//! `ticket.add`, `ticket.update` and `ticket.move`: the checks of each value, the move
//! table, the session rule and the link checks.

use super::{Error, Result, get};
use crate::clock;
use crate::registry::Product;
use crate::roster;
use agentsi_proto::ticket_api::{
    Author, AuthorBy, Link, LinkType, Ticket, TicketAddParams, TicketMoveParams, TicketState,
    TicketUpdateParams,
};
use rusqlite::{Connection, Transaction, params};
use serde_json::Value;

pub const MAX_TITLE: usize = 120;

/// The session id that matches no ticket.
pub const UNKNOWN_SESSION: &str = "unknown";

fn trimmed(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn title(text: &str) -> Result<String> {
    let t = text.trim();
    let n = t.chars().count();
    if n == 0 || n > MAX_TITLE {
        return Err(Error::Invalid(format!(
            "A ticket title has 1 to {MAX_TITLE} characters, not {n}."
        )));
    }
    Ok(t.to_string())
}

fn author(a: &Author) -> Result<Author> {
    let id = trimmed(&a.id)
        .ok_or_else(|| Error::Invalid(format!("The author {} has no id.", a.by.as_str())))?;
    Ok(Author { by: a.by, id })
}

/// `owner`, `claude` or an agent id of the roster. Empty is none.
fn assignee(product: &Product, text: &str) -> Result<Option<String>> {
    let Some(a) = trimmed(text) else {
        return Ok(None);
    };
    if a == "owner" || a == "claude" || product.roles.contains(&a) {
        return Ok(Some(a));
    }
    Err(Error::Invalid(format!(
        "The assignee {a:?} is not owner, claude or an agent of the roster of {} ({}).",
        product.id,
        product.roles.join(", ")
    )))
}

/// An objective id of the brief. Empty is none.
fn objective(c: &Connection, product: &str, text: &str) -> Result<Option<String>> {
    let Some(id) = trimmed(text) else {
        return Ok(None);
    };
    let brief = roster::read_brief(c, product).map_err(|e| Error::Unavailable(e.to_string()))?;
    let known = brief
        .as_ref()
        .and_then(|b| b.objectives.as_array())
        .is_some_and(|list| {
            list.iter()
                .any(|o| o.get("id").and_then(Value::as_str) == Some(id.as_str()))
        });
    if known {
        Ok(Some(id))
    } else {
        Err(Error::NotFound(format!(
            "The brief of {product} has no objective {id}."
        )))
    }
}

fn found(c: &Connection, sql: &str, p: impl rusqlite::Params) -> Result<bool> {
    Ok(c.prepare_cached(sql)?.exists(p)?)
}

fn link(c: &Connection, product: &str, l: &Link) -> Result<Link> {
    let reference = trimmed(&l.reference)
        .ok_or_else(|| Error::Invalid(format!("A {} link has no ref.", l.kind.as_str())))?;
    let r = reference.as_str();
    let exists = match l.kind {
        LinkType::Story => found(
            c,
            "select 1 from stories where product = ?1 and id = ?2",
            params![product, r],
        )?,
        LinkType::Decision => match r.parse::<i64>() {
            Ok(id) => found(
                c,
                "select 1 from decisions where product = ?1 and id = ?2",
                params![product, id],
            )?,
            Err(_) => return Err(Error::Invalid(format!("Bad decision id {r:?}."))),
        },
        LinkType::Minutes => found(
            c,
            "select 1 from minutes where product = ?1 and id = ?2",
            params![product, r],
        )?,
        LinkType::Priority => {
            let Some((minutes, idx)) = r
                .rsplit_once('#')
                .and_then(|(m, i)| Some((m, i.parse::<i64>().ok()?)))
            else {
                return Err(Error::Invalid(format!(
                    "A priority link has the form <minutes id>#<idx>, not {r:?}."
                )));
            };
            found(
                c,
                "select 1 from priorities p join minutes m on m.id = p.minutes_id
                 where m.product = ?1 and p.minutes_id = ?2 and p.idx = ?3",
                params![product, minutes, idx],
            )?
        }
        LinkType::Ticket => super::exists(c, product, r)?,
        LinkType::Url => {
            if !(r.starts_with("http://") || r.starts_with("https://")) {
                return Err(Error::Invalid(format!(
                    "A url link needs http(s), not {r:?}."
                )));
            }
            true
        }
        LinkType::Commit | LinkType::Path | LinkType::Issue => true,
    };
    if !exists {
        return Err(Error::NotFound(format!(
            "The {} {r} does not exist in {product}.",
            l.kind.as_str()
        )));
    }
    Ok(Link {
        kind: l.kind,
        reference,
    })
}

fn links(c: &Connection, product: &str, list: &[Link]) -> Result<Vec<Link>> {
    list.iter().map(|l| link(c, product, l)).collect()
}

/// The next id of the product: the highest number plus 1, from `C001`.
fn next_id(c: &Connection, product: &str) -> Result<String> {
    let max: Option<i64> = c
        .prepare_cached(
            "select max(cast(substr(id, 2) as integer)) from tickets where product = ?1",
        )?
        .query_row([product], |r| r.get(0))?;
    Ok(format!("C{:03}", max.unwrap_or(0) + 1))
}

/// A session changes only its own tickets and the tickets assigned to `claude`.
pub fn check_writer(ticket: &Ticket, by: &Author) -> Result<()> {
    if by.by != AuthorBy::Session {
        return Ok(());
    }
    let own = ticket.author.by == AuthorBy::Session
        && ticket.author.id == by.id
        && by.id != UNKNOWN_SESSION;
    if own || ticket.assignee.as_deref() == Some("claude") {
        return Ok(());
    }
    Err(Error::Conflict(format!(
        "Ticket {} belongs to the owner.",
        ticket.id
    )))
}

fn save(tx: &Transaction, t: &Ticket) -> Result<()> {
    tx.prepare_cached(
        "insert or replace into tickets (product, id, scope, kind, state, title, body, waiting_on,
           assignee, author, updated_by, links, objective, created_at, updated_at, closed_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
    )?
    .execute(params![
        t.product,
        t.id,
        t.scope,
        t.kind.as_str(),
        t.state.as_str(),
        t.title,
        t.body,
        t.waiting_on,
        t.assignee,
        serde_json::to_string(&t.author)?,
        serde_json::to_string(&t.updated_by)?,
        serde_json::to_string(&t.links)?,
        t.objective,
        t.created_at,
        t.updated_at,
        t.closed_at,
    ])?;
    Ok(())
}

pub fn add(
    tx: &Transaction,
    product: &Product,
    p: &TicketAddParams,
    now_ms: i64,
) -> Result<Ticket> {
    let pid = product.id.as_str();
    let author = author(&p.author)?;
    let now = clock::iso(now_ms);
    let ticket = Ticket {
        product: pid.to_string(),
        id: next_id(tx, pid)?,
        scope: p.scope.as_deref().and_then(trimmed),
        kind: p.kind,
        state: TicketState::Open,
        title: title(&p.title)?,
        body: p.body.clone().unwrap_or_default(),
        waiting_on: None,
        assignee: assignee(product, p.assignee.as_deref().unwrap_or(""))?,
        updated_by: author.clone(),
        author,
        links: links(tx, pid, p.links.as_deref().unwrap_or(&[]))?,
        objective: objective(tx, pid, p.objective.as_deref().unwrap_or(""))?,
        created_at: now.clone(),
        updated_at: now,
        closed_at: None,
    };
    save(tx, &ticket)?;
    Ok(ticket)
}

pub fn update(
    tx: &Transaction,
    product: &Product,
    p: &TicketUpdateParams,
    now_ms: i64,
) -> Result<Ticket> {
    let pid = product.id.as_str();
    let by = author(&p.by)?;
    let mut t = get(tx, pid, &p.id)?;
    check_writer(&t, &by)?;
    if let Some(text) = &p.title {
        t.title = title(text)?;
    }
    if let Some(body) = &p.body {
        t.body = body.clone();
    }
    if let Some(kind) = p.kind {
        t.kind = kind;
    }
    if let Some(a) = &p.assignee {
        t.assignee = assignee(product, a)?;
    }
    if let Some(list) = &p.links {
        t.links = links(tx, pid, list)?;
    }
    if let Some(scope) = &p.scope {
        t.scope = trimmed(scope);
    }
    if let Some(o) = &p.objective {
        t.objective = objective(tx, pid, o)?;
    }
    t.updated_by = by;
    t.updated_at = clock::iso(now_ms);
    save(tx, &t)?;
    Ok(t)
}

/// The ticket after a move, and whether the row changed.
pub struct Moved {
    pub ticket: Ticket,
    pub changed: bool,
}

pub fn move_to(
    tx: &Transaction,
    product: &Product,
    p: &TicketMoveParams,
    now_ms: i64,
) -> Result<Moved> {
    use TicketState::*;
    let by = author(&p.by)?;
    let mut t = get(tx, &product.id, &p.id)?;
    check_writer(&t, &by)?;
    let waiting_on = p.waiting_on.as_deref().and_then(trimmed);
    if waiting_on.is_some() && p.state != Waiting {
        return Err(Error::Invalid(format!(
            "waiting_on goes only with the state waiting, not {}.",
            p.state.as_str()
        )));
    }
    let now = clock::iso(now_ms);
    match (t.state, p.state) {
        (from, to) if from == to => {
            if to != Waiting || waiting_on.is_none() || waiting_on == t.waiting_on {
                return Ok(Moved {
                    ticket: t,
                    changed: false,
                });
            }
            t.waiting_on = waiting_on;
        }
        (Open, Waiting) => {
            t.waiting_on = Some(waiting_on.ok_or_else(|| {
                Error::Invalid(format!(
                    "Ticket {} needs waiting_on to wait: who or what it waits on.",
                    t.id
                ))
            })?);
        }
        (Waiting, Open) => t.waiting_on = None,
        (Open | Waiting, Done | Dropped) => {
            t.waiting_on = None;
            t.closed_at = Some(now.clone());
        }
        (Done | Dropped, Open) => t.closed_at = None,
        (from, to) => {
            return Err(Error::Invalid(format!(
                "Ticket {} cannot move from {} to {}.",
                t.id,
                from.as_str(),
                to.as_str()
            )));
        }
    }
    t.state = p.state;
    t.updated_by = by;
    t.updated_at = now;
    save(tx, &t)?;
    Ok(Moved {
        ticket: t,
        changed: true,
    })
}
