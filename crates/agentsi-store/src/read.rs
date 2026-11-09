//! The read stores: typed queries for the front desk and the CLI. Each
//! function takes a connection, so it runs in `Store::read` or in a test.
//! JSON columns come back as `serde_json::Value`, as the TypeScript stores
//! return them.

use crate::error::{Error, Result};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::Serialize;
use serde_json::Value;

fn json(text: Option<String>) -> Result<Option<Value>> {
    text.map(|t| {
        serde_json::from_str(&t)
            .map_err(|e| Error::Other(format!("stored JSON does not parse: {e}")))
    })
    .transpose()
}

fn all<T>(
    c: &Connection,
    sql: &str,
    args: impl rusqlite::Params,
    f: impl FnMut(&Row) -> Result<T>,
) -> Result<Vec<T>> {
    let mut f = f;
    let mut stmt = c.prepare_cached(sql)?;
    let mut rows = stmt.query(args)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(f(row)?);
    }
    Ok(out)
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Product {
    pub id: String,
    pub name: Option<String>,
    pub paths: Option<Value>,
    pub status: Option<String>,
}

pub fn products(c: &Connection) -> Result<Vec<Product>> {
    all(
        c,
        "select id, name, paths, status from products order by id",
        [],
        |r| {
            Ok(Product {
                id: r.get(0)?,
                name: r.get(1)?,
                paths: json(r.get(2)?)?,
                status: r.get(3)?,
            })
        },
    )
}

/// One row of the minutes table, without the JSON body.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct MinutesRow {
    pub id: String,
    pub product: String,
    pub scope: Option<String>,
    pub question: Option<String>,
    pub status: Option<String>,
    pub created_at: Option<String>,
    pub file: Option<String>,
}

/// The minutes of `product`, newest first.
pub fn minutes_list(c: &Connection, product: &str) -> Result<Vec<MinutesRow>> {
    all(
        c,
        "select id, product, scope, question, status, created_at, file from minutes
         where product = ?1 order by created_at desc, id desc",
        [product],
        |r| {
            Ok(MinutesRow {
                id: r.get(0)?,
                product: r.get(1)?,
                scope: r.get(2)?,
                question: r.get(3)?,
                status: r.get(4)?,
                created_at: r.get(5)?,
                file: r.get(6)?,
            })
        },
    )
}

/// The minutes JSON with this id. Minutes ids are global. A row without
/// `data` (from before migration 008) gives `None`, as `findMinutes` does.
pub fn minutes(c: &Connection, id: &str) -> Result<Option<Value>> {
    let text: Option<String> = c
        .query_row(
            "select data from minutes where id = ?1 and data is not null",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    json(text)
}

/// The per-role usage array stored with the minutes, or an empty list.
pub fn minutes_usage(c: &Connection, id: &str) -> Result<Vec<Value>> {
    let text: Option<Option<String>> = c
        .query_row("select usage from minutes where id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    match json(text.flatten())? {
        Some(Value::Array(rows)) => Ok(rows),
        _ => Ok(Vec::new()),
    }
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Priority {
    pub minutes_id: String,
    pub idx: i64,
    pub title: Option<String>,
    pub why: Option<String>,
    pub sources: Option<Value>,
    pub owner_role: Option<String>,
    pub mark: Option<String>,
    pub is_new: Option<bool>,
    pub objective: Option<String>,
}

pub fn priorities(c: &Connection, minutes_id: &str) -> Result<Vec<Priority>> {
    all(
        c,
        "select minutes_id, idx, title, why, sources, owner_role, mark, is_new, objective
         from priorities where minutes_id = ?1 order by idx",
        [minutes_id],
        |r| {
            Ok(Priority {
                minutes_id: r.get(0)?,
                idx: r.get(1)?,
                title: r.get(2)?,
                why: r.get(3)?,
                sources: json(r.get(4)?)?,
                owner_role: r.get(5)?,
                mark: r.get(6)?,
                is_new: r.get::<_, Option<i64>>(7)?.map(|v| v != 0),
                objective: r.get(8)?,
            })
        },
    )
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Decision {
    pub id: i64,
    pub product: String,
    pub minutes_id: Option<String>,
    pub text: Option<String>,
    pub why: Option<String>,
    pub status: Option<String>,
    pub decided_at: Option<String>,
    pub kind: String,
    pub reflection_id: Option<String>,
    pub action: Option<String>,
}

const DECISION: &str = "select id, product, minutes_id, text, why, status, decided_at, kind, reflection_id, action from decisions";

fn decision(r: &Row) -> Result<Decision> {
    Ok(Decision {
        id: r.get(0)?,
        product: r.get(1)?,
        minutes_id: r.get(2)?,
        text: r.get(3)?,
        why: r.get(4)?,
        status: r.get(5)?,
        decided_at: r.get(6)?,
        kind: r.get(7)?,
        reflection_id: r.get(8)?,
        action: r.get(9)?,
    })
}

/// The decisions of `product` in id order. With `status`, only that status.
pub fn decisions(c: &Connection, product: &str, status: Option<&str>) -> Result<Vec<Decision>> {
    all(
        c,
        &format!("{DECISION} where product = ?1 and (?2 is null or status = ?2) order by id"),
        params![product, status],
        decision,
    )
}

pub fn decisions_of_minutes(c: &Connection, minutes_id: &str) -> Result<Vec<Decision>> {
    all(
        c,
        &format!("{DECISION} where minutes_id = ?1 order by id"),
        [minutes_id],
        decision,
    )
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct UsageRow {
    pub minutes_id: String,
    pub role: Option<String>,
    pub model: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<i64>,
}

pub fn usage(c: &Connection, minutes_id: &str) -> Result<Vec<UsageRow>> {
    all(
        c,
        "select minutes_id, role, model, input_tokens, output_tokens, cache_read, cache_write, cost_usd, duration_ms
         from usage where minutes_id = ?1 order by rowid",
        [minutes_id],
        |r| {
            Ok(UsageRow {
                minutes_id: r.get(0)?,
                role: r.get(1)?,
                model: r.get(2)?,
                input_tokens: r.get(3)?,
                output_tokens: r.get(4)?,
                cache_read: r.get(5)?,
                cache_write: r.get(6)?,
                cost_usd: r.get(7)?,
                duration_ms: r.get(8)?,
            })
        },
    )
}

/// The sum of `usage.cost_usd` over the minutes of `product`.
pub fn cost_usd(c: &Connection, product: &str) -> Result<f64> {
    Ok(c.query_row(
        "select coalesce(sum(u.cost_usd), 0) from usage u join minutes m on m.id = u.minutes_id where m.product = ?1",
        [product],
        |r| r.get(0),
    )?)
}

/// The story JSON of each story of `product`, in id order.
pub fn stories(c: &Connection, product: &str) -> Result<Vec<Value>> {
    all(
        c,
        "select data from stories where product = ?1 order by id",
        [product],
        |r| json(r.get(0)?).map(|v| v.unwrap_or(Value::Null)),
    )
}

pub fn story(c: &Connection, product: &str, id: &str) -> Result<Option<Value>> {
    let text: Option<String> = c
        .query_row(
            "select data from stories where product = ?1 and id = ?2",
            [product, id],
            |r| r.get(0),
        )
        .optional()?;
    json(text)
}

pub fn story_log(c: &Connection, product: &str, story_id: &str) -> Result<Option<String>> {
    Ok(c.query_row(
        "select body from story_logs where product = ?1 and story_id = ?2",
        [product, story_id],
        |r| r.get(0),
    )
    .optional()?)
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct MemoryItem {
    pub id: String,
    pub text: String,
    pub kind: String,
    pub helpful: i64,
    pub harmful: i64,
    pub source: String,
    pub created_at: String,
    pub retired_at: Option<String>,
    pub retired_why: Option<String>,
}

/// The agents of `product` that have memory items.
pub fn memory_agents(c: &Connection, product: &str) -> Result<Vec<String>> {
    all(
        c,
        "select distinct agent from memory where product = ?1 order by agent",
        [product],
        |r| Ok(r.get(0)?),
    )
}

/// The memory items of one agent, in position order.
pub fn memory(c: &Connection, product: &str, agent: &str) -> Result<Vec<MemoryItem>> {
    all(
        c,
        "select id, text, kind, helpful, harmful, source, created_at, retired_at, retired_why
         from memory where product = ?1 and agent = ?2 order by position",
        [product, agent],
        |r| {
            Ok(MemoryItem {
                id: r.get(0)?,
                text: r.get(1)?,
                kind: r.get(2)?,
                helpful: r.get(3)?,
                harmful: r.get(4)?,
                source: r.get(5)?,
                created_at: r.get(6)?,
                retired_at: r.get(7)?,
                retired_why: r.get(8)?,
            })
        },
    )
}

pub fn overlay(c: &Connection, product: &str, agent: &str) -> Result<Option<String>> {
    Ok(c.query_row(
        "select body from agent_overlays where product = ?1 and agent = ?2",
        [product, agent],
        |r| r.get(0),
    )
    .optional()?)
}

pub fn brief(c: &Connection, product: &str) -> Result<Option<Value>> {
    let text: Option<String> = c
        .query_row(
            "select data from briefs where product = ?1",
            [product],
            |r| r.get(0),
        )
        .optional()?;
    json(text)
}

/// The meeting log entries of `product`, oldest first.
pub fn meeting_log(c: &Connection, product: &str) -> Result<Vec<Value>> {
    all(
        c,
        "select entry from meeting_log where product = ?1 order by position",
        [product],
        |r| json(r.get(0)?).map(|v| v.unwrap_or(Value::Null)),
    )
}

pub fn snapshot(c: &Connection, product: &str, minutes_id: &str) -> Result<Option<String>> {
    Ok(c.query_row(
        "select body from snapshots where product = ?1 and minutes_id = ?2",
        [product, minutes_id],
        |r| r.get(0),
    )
    .optional()?)
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct Listen {
    pub minutes_id: String,
    pub at: String,
}

pub fn listens(c: &Connection, product: &str) -> Result<Vec<Listen>> {
    all(
        c,
        "select minutes_id, at from listens where product = ?1 order by at",
        [product],
        |r| {
            Ok(Listen {
                minutes_id: r.get(0)?,
                at: r.get(1)?,
            })
        },
    )
}

/// The reflection entries of `product`, oldest first.
pub fn reflections(c: &Connection, product: &str) -> Result<Vec<Value>> {
    all(
        c,
        "select entry from reflections where product = ?1 order by position",
        [product],
        |r| json(r.get(0)?).map(|v| v.unwrap_or(Value::Null)),
    )
}

#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct ArchiveRecord {
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
}

/// The ids of the products that the archive record holds.
pub fn archived_products(c: &Connection) -> Result<Vec<String>> {
    all(
        c,
        "select product from archive_records where kind = 'product' order by product",
        [],
        |r| Ok(r.get(0)?),
    )
}

pub fn archived(c: &Connection, product: &str) -> Result<Vec<ArchiveRecord>> {
    all(
        c,
        "select kind, ref from archive_records where product = ?1 order by kind, ref",
        [product],
        |r| {
            Ok(ArchiveRecord {
                kind: r.get(0)?,
                reference: r.get(1)?,
            })
        },
    )
}

/// The row count of each table, in the order of `schema::all`.
pub fn counts(c: &Connection) -> Result<Vec<(&'static str, i64)>> {
    crate::schema::all()
        .map(|t| {
            Ok((
                t.name,
                c.query_row(&format!("select count(*) from \"{}\"", t.name), [], |r| {
                    r.get(0)
                })?,
            ))
        })
        .collect()
}
