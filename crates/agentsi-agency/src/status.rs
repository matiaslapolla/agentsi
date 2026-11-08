//! `product.status`: the Markdown of `agentsi status`, and the counts of the
//! index line of `agentsi desk`. The rules copy `packages/core/src/status.ts`.

use crate::clock;
use crate::error::Result;
use crate::registry::Product;
use agentsi_proto::agency_api::{IndexCounts, StatusResult};
use agentsi_store::read;
use rusqlite::Connection;
use std::collections::BTreeSet;

/// The last minutes that the status shows.
const SHOWN: usize = 3;

/// `Math.round` for a positive number: halves go up.
fn round(x: f64) -> i64 {
    (x + 0.5).floor() as i64
}

fn ago(at: Option<&str>, now_ms: i64) -> String {
    let Some(ms) = at.and_then(clock::parse_iso_ms) else {
        return "never".into();
    };
    let min = round((now_ms - ms) as f64 / 60_000.0);
    if min < 60 {
        format!("{min} min ago")
    } else if min < 48 * 60 {
        format!("{} h ago", round(min as f64 / 60.0))
    } else {
        format!("{} d ago", round(min as f64 / 1440.0))
    }
}

fn archived(c: &Connection, product: &str, kind: &str) -> Result<BTreeSet<String>> {
    Ok(read::archived(c, product)?
        .into_iter()
        .filter(|a| a.kind == kind)
        .map(|a| a.reference)
        .collect())
}

fn count(c: &Connection, sql: &str, product: &str) -> Result<u64> {
    Ok(c.query_row(sql, [product], |r| r.get::<_, i64>(0))?.max(0) as u64)
}

/// The marks of one minutes as `<n> <mark>` parts. Postgres `jsonb` orders
/// the keys by length, then by bytes, and the TypeScript text follows it.
pub(crate) fn marks(c: &Connection, minutes_id: &str) -> Result<Vec<(String, i64)>> {
    let mut stmt = c.prepare_cached(
        "select coalesce(mark, 'unmarked'), count(*) from priorities where minutes_id = ?1 group by 1",
    )?;
    let mut rows: Vec<(String, i64)> = stmt
        .query_map([minutes_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    rows.sort_by(|a, b| (a.0.len(), &a.0).cmp(&(b.0.len(), &b.0)));
    Ok(rows)
}

/// The counts that the index line of `agentsi desk` shows, from the store.
pub fn index_counts(c: &Connection, product: &str) -> Result<IndexCounts> {
    Ok(IndexCounts {
        repos: count(
            c,
            "select count(*) from index_state where product = ?1",
            product,
        )?,
        docs: count(
            c,
            "select count(*) from documents where product = ?1 and kind in ('doc', 'journal')",
            product,
        )?,
        commits: count(
            c,
            "select count(*) from documents where product = ?1 and kind = 'commit'",
            product,
        )?,
        issues: count(
            c,
            "select count(*) from documents where product = ?1 and kind = 'issue'",
            product,
        )?,
        minutes: count(
            c,
            "select count(*) from minutes where product = ?1 and data is not null",
            product,
        )?,
        memory: count(
            c,
            "select count(distinct agent) from memory where product = ?1 and retired_at is null",
            product,
        )?,
    })
}

pub fn status(c: &Connection, product: &Product, now_ms: i64) -> Result<StatusResult> {
    let id = product.id.as_str();
    let hidden_minutes = archived(c, id, "minutes")?;
    let hidden_decisions = archived(c, id, "decision")?;

    let mut kinds = std::collections::HashMap::<String, u64>::new();
    let mut stmt =
        c.prepare_cached("select kind, count(*) from documents where product = ?1 group by kind")?;
    for row in stmt.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (kind, n) = row?;
        kinds.insert(kind, n.max(0) as u64);
    }
    let n = |kind: &str| kinds.get(kind).copied().unwrap_or(0);

    let last_indexed: Option<String> = c.query_row(
        "select max(indexed_at) from index_state where product = ?1",
        [id],
        |r| r.get(0),
    )?;
    let last_commit: Option<(String, Option<String>)> = c
        .prepare_cached(
            "select title, updated_at from documents where product = ?1 and kind = 'commit'
             order by updated_at desc limit 1",
        )?
        .query_map([id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .next()
        .transpose()?;

    let pending: Vec<read::Decision> = read::decisions(c, id, Some("pending"))?
        .into_iter()
        .filter(|d| !hidden_decisions.contains(&d.id.to_string()))
        .collect();

    let total_minutes = read::minutes_list(c, id)?
        .into_iter()
        .filter(|m| !hidden_minutes.contains(&m.id))
        .collect::<Vec<_>>();

    let mut out = vec![
        format!("# {} — AgentSI status", product.name),
        String::new(),
    ];
    out.push(format!(
        "- Roles: {} · status {}",
        product.roles.join(", "),
        product.status
    ));
    out.push(format!(
        "- Index: {} docs, {} commits, {} open issues, indexed {}",
        n("doc"),
        n("commit"),
        n("issue"),
        ago(last_indexed.as_deref(), now_ms)
    ));
    out.push(format!(
        "- Last commit: {}",
        match &last_commit {
            Some((title, at)) => format!(
                "{} {title}",
                at.as_deref()
                    .unwrap_or_default()
                    .chars()
                    .take(10)
                    .collect::<String>()
            ),
            None => "none indexed".into(),
        }
    ));
    out.push(format!("- Pending decisions: {}", pending.len()));
    out.push(format!("- Minutes: {} total", total_minutes.len()));
    out.push(String::new());

    out.push("## Pending decisions".into());
    out.push(String::new());
    if pending.is_empty() {
        out.push("_None._".into());
    }
    for d in &pending {
        out.push(format!(
            "- [{}] {}{}",
            d.id,
            d.text.as_deref().unwrap_or("null"),
            d.minutes_id
                .as_ref()
                .map(|m| format!(" ({m})"))
                .unwrap_or_default()
        ));
    }
    out.push(String::new());

    out.push("## Last minutes".into());
    out.push(String::new());
    if total_minutes.is_empty() {
        out.push("_No meetings yet._".into());
    }
    for m in total_minutes.iter().take(SHOWN) {
        let parts: Vec<String> = marks(c, &m.id)?
            .into_iter()
            .map(|(k, v)| format!("{v} {k}"))
            .collect();
        out.push(format!(
            "- {} — {} — {} — {}",
            m.id,
            m.question.as_deref().unwrap_or("null"),
            m.status.as_deref().unwrap_or("null"),
            if parts.is_empty() {
                "no priorities".into()
            } else {
                parts.join(", ")
            }
        ));
    }
    Ok(StatusResult {
        markdown: out.join("\n") + "\n",
        index: index_counts(c, id)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_age_of_the_index_follows_the_typescript_rounding() {
        let now = clock::parse_iso_ms("2026-11-04T12:00:00.000Z").unwrap();
        let at = |s: &str| ago(Some(s), now);
        assert_eq!(ago(None, now), "never");
        assert_eq!(at("2026-11-04T11:59:30.000Z"), "1 min ago");
        assert_eq!(at("2026-11-04T11:15:00.000Z"), "45 min ago");
        assert_eq!(at("2026-11-04T09:30:00.000Z"), "3 h ago");
        assert_eq!(at("2026-11-02T13:00:00.000Z"), "47 h ago");
        assert_eq!(at("2026-11-02T12:00:00.000Z"), "2 d ago");
        assert_eq!(at("2026-10-01T12:00:00.000Z"), "34 d ago");
    }
}
