//! The tracker: the rules and the SQL of the `ticket.*` methods. Spec:
//! `docs/specs/agentsi-desktop-tracker.md`.
//!
//! Each write takes a `&Transaction` and the product of the registry, whose roster
//! the assignee rule reads. The caller commits on `Ok` and rolls back on `Err`.

pub mod text;
pub mod write;

pub use crate::marks::{Error, Result};

use agentsi_proto::ticket_api::{Author, Link, Ticket, TicketKind, TicketListParams, TicketState};
use rusqlite::{Connection, OptionalExtension, Row};

pub const DEFAULT_LIMIT: u32 = 50;
pub const MAX_LIMIT: u32 = 200;

const COLUMNS: &str = "product, id, scope, kind, state, title, body, waiting_on, assignee, \
    author, updated_by, links, objective, created_at, updated_at, closed_at";

/// The order of `ticket.list`: the state, then the newest write, then the highest id.
const ORDER: &str = "case state when 'open' then 0 when 'waiting' then 1 when 'done' then 2 \
    else 3 end, updated_at desc, cast(substr(id, 2) as integer) desc";

fn json<T: serde::de::DeserializeOwned>(text: String, column: &str) -> rusqlite::Result<T> {
    serde_json::from_str(&text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("tickets.{column}: {e}").into(),
        )
    })
}

fn enum_of<T>(text: String, parse: fn(&str) -> Option<T>) -> rusqlite::Result<T> {
    parse(&text).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown ticket value {text:?}").into(),
        )
    })
}

fn ticket_of(r: &Row) -> rusqlite::Result<Ticket> {
    Ok(Ticket {
        product: r.get(0)?,
        id: r.get(1)?,
        scope: r.get(2)?,
        kind: enum_of(r.get(3)?, TicketKind::parse)?,
        state: enum_of(r.get(4)?, TicketState::parse)?,
        title: r.get(5)?,
        body: r.get(6)?,
        waiting_on: r.get(7)?,
        assignee: r.get(8)?,
        author: json::<Author>(r.get(9)?, "author")?,
        updated_by: json::<Author>(r.get(10)?, "updated_by")?,
        links: json::<Vec<Link>>(r.get(11)?, "links")?,
        objective: r.get(12)?,
        created_at: r.get(13)?,
        updated_at: r.get(14)?,
        closed_at: r.get(15)?,
    })
}

/// One ticket, or none.
pub fn find(c: &Connection, product: &str, id: &str) -> Result<Option<Ticket>> {
    Ok(c.prepare_cached(&format!(
        "select {COLUMNS} from tickets where product = ?1 and id = ?2"
    ))?
    .query_row([product, id], ticket_of)
    .optional()?)
}

/// One ticket, or `not_found`.
pub fn get(c: &Connection, product: &str, id: &str) -> Result<Ticket> {
    find(c, product, id)?.ok_or_else(|| Error::NotFound(format!("No ticket {id} in {product}.")))
}

/// The tickets of `ticket.list`. The caller checks the product.
pub fn list(c: &Connection, p: &TicketListParams) -> Result<Vec<Ticket>> {
    let limit = p.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(Error::Invalid(format!(
            "The limit must be 1 to {MAX_LIMIT}, not {limit}."
        )));
    }
    let states: Vec<&str> = p
        .states
        .clone()
        .unwrap_or_else(|| vec![TicketState::Open, TicketState::Waiting, TicketState::Done])
        .into_iter()
        .map(TicketState::as_str)
        .collect();
    let link = p
        .link
        .as_ref()
        .map(|l| (l.kind.as_str(), l.reference.as_str()));
    let sql = format!(
        "select {COLUMNS} from tickets
         where product = ?1
           and state in (select value from json_each(?2))
           and (?3 is null or kind = ?3)
           and (?4 is null or assignee = ?4)
           and (?5 is null or json_extract(author, '$.by') = ?5)
           and (?6 is null or exists (select 1 from json_each(links)
                 where json_extract(value, '$.type') = ?6 and json_extract(value, '$.ref') = ?7))
         order by {ORDER}
         limit ?8"
    );
    let mut stmt = c.prepare_cached(&sql)?;
    let rows = stmt.query_map(
        rusqlite::params![
            p.product,
            serde_json::to_string(&states)?,
            p.kind.map(TicketKind::as_str),
            p.assignee,
            p.author.map(|a| a.as_str()),
            link.map(|l| l.0),
            link.map(|l| l.1),
            limit,
        ],
        ticket_of,
    )?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The open tickets of a product, in the order of `ticket.list`. With a scope, only the
/// tickets of that scope and the tickets with no scope.
pub fn open(c: &Connection, product: &str, scope: Option<&str>) -> Result<Vec<Ticket>> {
    let mut stmt = c.prepare_cached(&format!(
        "select {COLUMNS} from tickets
         where product = ?1 and state in ('open', 'waiting')
           and (?2 is null or scope is null or scope = ?2)
         order by {ORDER}"
    ))?;
    let rows = stmt.query_map(rusqlite::params![product, scope], ticket_of)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// true when the product has a ticket with this id, in any state.
pub fn exists(c: &Connection, product: &str, id: &str) -> Result<bool> {
    Ok(
        c.prepare_cached("select 1 from tickets where product = ?1 and id = ?2")?
            .exists([product, id])?,
    )
}

#[cfg(test)]
mod tests;
