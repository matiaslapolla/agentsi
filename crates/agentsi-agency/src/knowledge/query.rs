//! The reads behind the five MCP tools: `search`, `get_doc`, `list_minutes`,
//! `pending_decisions` and `role_memory`. The rules copy `search.ts`,
//! `packages/mcp/src/list-minutes.ts` and `pending-decisions.ts`.

use super::memory_text::{MEMORY_MAX_CHARS, render_for_prompt};
use crate::error::{Error, Result};
use crate::status::marks;
use agentsi_proto::knowledge_api::{
    KbDoc, KbMarkCount, KbPendingResult, KbPendingRow, KbRecentMinutes, KbSearchHit,
};
use agentsi_store::read;
use agentsi_store::search::{self, SearchOptions};
use rusqlite::Connection;
use std::collections::BTreeSet;

/// The hits for all terms. When there are none, the hits for any term, and `true`.
pub fn search_with_fallback(
    c: &Connection,
    product: &str,
    query: &str,
    kind: Option<&str>,
    limit: u32,
) -> Result<(Vec<KbSearchHit>, bool)> {
    let mut opts = SearchOptions {
        kind,
        limit: Some(limit),
        any_term: false,
    };
    let mut hits = search::search(c, product, query, &opts)?;
    let any_term = hits.is_empty();
    if any_term {
        opts.any_term = true;
        hits = search::search(c, product, query, &opts)?;
    }
    let hits = hits
        .into_iter()
        .map(|h| KbSearchHit {
            kind: h.kind,
            reference: h.reference,
            scope: h.scope,
            title: h.title,
            rank: h.rank,
            snippet: h.snippet,
        })
        .collect();
    Ok((hits, any_term))
}

pub fn get_doc(c: &Connection, product: &str, kind: &str, reference: &str) -> Result<KbDoc> {
    search::get_doc(c, product, kind, reference)?
        .map(|d| KbDoc {
            product: d.product,
            scope: d.scope,
            kind: d.kind,
            reference: d.reference,
            title: d.title,
            body: d.body,
            updated_at: d.updated_at,
        })
        .ok_or_else(|| {
            Error::NotFound(format!(
                "No {kind} \"{reference}\" in {product}. Use search to find valid refs."
            ))
        })
}

fn archived(c: &Connection, product: &str, kind: &str) -> Result<BTreeSet<String>> {
    Ok(read::archived(c, product)?
        .into_iter()
        .filter(|a| a.kind == kind)
        .map(|a| a.reference)
        .collect())
}

/// The newest `limit` minutes. Archived ones are left out unless asked for.
pub fn recent_minutes(
    c: &Connection,
    product: &str,
    limit: u32,
    include_archived: bool,
) -> Result<Vec<KbRecentMinutes>> {
    let hidden = archived(c, product, "minutes")?;
    let mut shown = Vec::new();
    for m in read::minutes_list(c, product)? {
        let is_archived = hidden.contains(&m.id);
        if is_archived && !include_archived {
            continue;
        }
        if shown.len() >= limit as usize {
            break;
        }
        shown.push(KbRecentMinutes {
            marks: marks(c, &m.id)?
                .into_iter()
                .map(|(mark, n)| KbMarkCount {
                    mark,
                    count: n.max(0) as u64,
                })
                .collect(),
            id: m.id,
            status: m.status,
            created_at: m.created_at,
            question: m.question,
            archived: is_archived,
        });
    }
    Ok(shown)
}

/// The pending decisions that are not archived: by minutes id with the
/// decisions of no minutes last, then by id.
pub fn pending_decisions(c: &Connection, product: &str, limit: u32) -> Result<KbPendingResult> {
    let hidden = archived(c, product, "decision")?;
    let mut stmt = c.prepare_cached(
        "select id, minutes_id, text, why, action from decisions
         where product = ?1 and status = 'pending'
         order by minutes_id is null, minutes_id, id",
    )?;
    let rows = stmt.query_map([product], |r| {
        Ok(KbPendingRow {
            id: r.get(0)?,
            minutes_id: r.get(1)?,
            text: r.get(2)?,
            why: r.get(3)?,
            action: r.get(4)?,
        })
    })?;
    let mut kept = Vec::new();
    for row in rows {
        let row = row?;
        if !hidden.contains(&row.id.to_string()) {
            kept.push(row);
        }
    }
    let total = kept.len() as u64;
    kept.truncate(limit as usize);
    Ok(KbPendingResult {
        hidden: total - kept.len() as u64,
        decisions: kept,
    })
}

/// What one role learned about the product, as the meeting prompt shows it.
pub fn role_memory(c: &Connection, product: &str, role: &str) -> Result<String> {
    Ok(render_for_prompt(
        &read::memory(c, product, role)?,
        Some(MEMORY_MAX_CHARS),
    ))
}
