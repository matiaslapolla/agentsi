//! What waits for the owner: the unmarked priorities and the pending
//! decisions. A port of `pendingItems` of `pending.ts`.

use super::Result;
use agentsi_proto::marks_api::{
    MarkPendingResult, PendingDecisionItem, PendingItem, PendingPriority,
};
use agentsi_store::read;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::BTreeSet;

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(String::from)
}

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// `!p.mark` of TypeScript: no mark, or an empty one.
pub fn is_unmarked(priority: &Value) -> bool {
    priority
        .get("mark")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
}

type Valid = Vec<(String, Value)>;

/// The valid minutes of `product` with a stored body, oldest id first.
fn valid_minutes(c: &Connection, product: &str) -> Result<Valid> {
    let mut stmt = c.prepare_cached(
        "select id, data from minutes where product = ?1 and data is not null order by id",
    )?;
    let mut rows = stmt.query([product])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let (id, data): (String, String) = (row.get(0)?, row.get(1)?);
        let value: Value = serde_json::from_str(&data)?;
        if value.get("status").and_then(Value::as_str) == Some("valid") {
            out.push((id, value));
        }
    }
    Ok(out)
}

/// The unmarked priorities that a decision links in its minutes. Old minutes
/// and decisions of a product link none.
fn linked(valid: &[(String, Value)], d: &read::Decision) -> Vec<u32> {
    let Some((_, minutes)) = valid
        .iter()
        .find(|(id, _)| Some(id) == d.minutes_id.as_ref())
    else {
        return Vec::new();
    };
    let Some(linked) = list(minutes, "decisions")
        .iter()
        // `x.text === r.text` of TypeScript: a missing text matches nothing.
        .find(|x| d.text.is_some() && text(x, "text") == d.text)
        .map(|x| list(x, "priorities"))
    else {
        return Vec::new();
    };
    let priorities = list(minutes, "priorities");
    let mut out: Vec<u32> = Vec::new();
    for n in linked {
        let Some(i) = n.as_u64().filter(|i| (*i as usize) < priorities.len()) else {
            continue;
        };
        if is_unmarked(&priorities[i as usize]) && !out.contains(&(i as u32)) {
            out.push(i as u32);
        }
    }
    out
}

fn decision_item(valid: &[(String, Value)], d: &read::Decision) -> PendingItem {
    PendingItem::Decision(PendingDecisionItem {
        id: d.id,
        minutes_id: d.minutes_id.clone(),
        text: d.text.clone().unwrap_or_default(),
        why: d.why.clone(),
        action: d.action.clone(),
        priorities: linked(valid, d),
    })
}

/// The pending decisions of `product` that are not archived, in id order.
fn pending_rows(c: &Connection, product: &str) -> Result<Vec<read::Decision>> {
    let archived: BTreeSet<String> = read::archived(c, product)?
        .into_iter()
        .filter(|r| r.kind == "decision")
        .map(|r| r.reference)
        .collect();
    Ok(read::decisions(c, product, Some("pending"))?
        .into_iter()
        .filter(|d| !archived.contains(&d.id.to_string()))
        .collect())
}

/// Unmarked priorities and pending decisions, oldest minutes first. The
/// decisions of no minutes come last. A decision that is archived is left out,
/// but the priorities of an archived minutes stay, as `pendingItems` has it.
pub fn pending(c: &Connection, product: &str) -> MarkPendingResult {
    // The minutes are read first and kept when the decisions fail, as `pendingItems` does.
    let mut decisions_error = None;
    let mut rows: Vec<read::Decision> = Vec::new();
    let valid = match valid_minutes(c, product) {
        Ok(v) => v,
        Err(e) => {
            decisions_error = Some(e.to_string());
            Vec::new()
        }
    };
    if decisions_error.is_none() {
        match pending_rows(c, product) {
            Ok(r) => rows = r,
            Err(e) => decisions_error = Some(e.to_string()),
        }
    }

    let mut items = Vec::new();
    let mut used: BTreeSet<i64> = BTreeSet::new();
    for (id, m) in &valid {
        for (idx, p) in list(m, "priorities").iter().enumerate() {
            if !is_unmarked(p) {
                continue;
            }
            items.push(PendingItem::Priority(PendingPriority {
                minutes_id: id.clone(),
                idx: idx as u32,
                title: text(p, "title").unwrap_or_default(),
                why: text(p, "why").unwrap_or_default(),
                owner_role: text(p, "owner_role").unwrap_or_default(),
                objective: text(p, "objective"),
                sources: list(p, "sources").to_vec(),
            }));
        }
        for d in rows.iter().filter(|d| d.minutes_id.as_ref() == Some(id)) {
            used.insert(d.id);
            items.push(decision_item(&valid, d));
        }
    }
    for d in rows.iter().filter(|d| !used.contains(&d.id)) {
        items.push(decision_item(&valid, d));
    }
    MarkPendingResult {
        items,
        decisions_error,
    }
}
