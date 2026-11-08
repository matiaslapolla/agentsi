//! The writes of a meeting. The rules copy `reserveMinutesId` and `storeMinutes`
//! of `minutes.ts`, `writeLedger` of `meeting-log.ts`, and `appendFacts` of
//! `memory.ts`. Each function takes the transaction of the caller, so a meeting
//! writes its minutes, memory, log line and snapshot as one.

use crate::error::Result;
use crate::read;
use rusqlite::{Transaction, params};
use serde_json::Value;
use std::collections::hash_map::RandomState;
use std::collections::{BTreeSet, HashSet};
use std::hash::{BuildHasher, Hasher};

/// The keys in Postgres `jsonb` order: shorter keys first, then by bytes. A reader of
/// stored minutes, such as the index, walks the roles in this order.
pub(crate) fn jsonb_order(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(b.0)));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(k, v)| (k.clone(), jsonb_order(v)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(jsonb_order).collect()),
        other => other.clone(),
    }
}

/// The columns of the first write of a minutes row.
pub struct MinutesHead<'a> {
    pub id: &'a str,
    pub product: &'a str,
    pub scope: Option<&'a str>,
    pub question: &'a str,
    pub status: &'a str,
    pub created_at: &'a str,
}

/// Claims `head.id`, or `<id>-2`, `<id>-3` and so on, for the product of `head`:
/// the first id that is free, or that this product holds already.
pub fn reserve_minutes_id(tx: &Transaction, head: &MinutesHead) -> Result<String> {
    for n in 1.. {
        let id = if n == 1 {
            head.id.to_string()
        } else {
            format!("{}-{n}", head.id)
        };
        let holder: String = tx.query_row(
            "insert into minutes (id, product, scope, question, status, created_at) values (?1, ?2, ?3, ?4, ?5, ?6)
             on conflict (id) do update set id = minutes.id returning product",
            params![id, head.product, head.scope, head.question, head.status, head.created_at],
            |r| r.get(0),
        )?;
        if holder == head.product {
            return Ok(id);
        }
    }
    unreachable!("the ids never end")
}

/// One usage row of a meeting.
pub struct UsageWrite<'a> {
    pub role: &'a str,
    pub model: &'a str,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: f64,
    pub duration_ms: u64,
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// Writes the minutes row, its priorities, decisions and usage rows. `minutes`
/// is the JSON of the minutes, and `usage_json` the JSON of the usage column.
pub fn store_minutes(
    tx: &Transaction,
    minutes: &Value,
    usage: &[UsageWrite],
    usage_json: &str,
) -> Result<()> {
    let id = text(minutes, "id").unwrap_or_default();
    let product = text(minutes, "product").unwrap_or_default();
    tx.execute("delete from priorities where minutes_id = ?1", [id])?;
    tx.execute("delete from decisions where minutes_id = ?1", [id])?;
    tx.execute("delete from usage where minutes_id = ?1", [id])?;
    tx.execute(
        "update minutes set product = ?2, scope = ?3, question = ?4, status = ?5, created_at = ?6, data = ?7, usage = ?8 where id = ?1",
        params![
            id,
            product,
            text(minutes, "scope"),
            text(minutes, "question"),
            text(minutes, "status"),
            text(minutes, "created_at"),
            jsonb_order(minutes).to_string(),
            usage_json
        ],
    )?;
    let empty = Vec::new();
    let list = |key: &str| minutes.get(key).and_then(Value::as_array).unwrap_or(&empty);
    for (i, p) in list("priorities").iter().enumerate() {
        tx.execute(
            "insert into priorities (minutes_id, idx, title, why, sources, owner_role, objective, mark, is_new)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                i as i64,
                text(p, "title"),
                text(p, "why"),
                p.get("sources").unwrap_or(&Value::Null).to_string(),
                text(p, "owner_role"),
                text(p, "objective"),
                text(p, "mark"),
                p.get("is_new").and_then(Value::as_bool).unwrap_or(false)
            ],
        )?;
    }
    for d in list("decisions") {
        tx.execute(
            "insert into decisions (product, minutes_id, text, why, action, status, decided_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                product,
                id,
                text(d, "text"),
                text(d, "why"),
                text(d, "action"),
                text(d, "status").unwrap_or("pending"),
                text(d, "decided_at")
            ],
        )?;
    }
    for u in usage {
        tx.execute(
            "insert into usage (minutes_id, role, model, input_tokens, output_tokens, cache_read, cache_write, cost_usd, duration_ms)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                u.role,
                u.model,
                u.input_tokens as i64,
                u.output_tokens as i64,
                u.cache_read as i64,
                u.cache_write as i64,
                u.cost_usd,
                u.duration_ms as i64
            ],
        )?;
    }
    Ok(())
}

/// Stores the meeting log line and the frozen snapshot of one meeting. A second
/// write of the same meeting replaces both, and the line keeps its position.
pub fn write_meeting_log(
    tx: &Transaction,
    product: &str,
    minutes_id: &str,
    entry: &Value,
    snapshot: &str,
) -> Result<()> {
    tx.execute(
        "insert into meeting_log (product, minutes_id, position, entry)
         values (?1, ?2, (select coalesce(max(position), 0) + 1 from meeting_log), ?3)
         on conflict (product, minutes_id) do update set entry = excluded.entry",
        params![product, minutes_id, jsonb_order(entry).to_string()],
    )?;
    tx.execute(
        "insert into snapshots (product, minutes_id, body) values (?1, ?2, ?3)
         on conflict (product, minutes_id) do update set body = excluded.body",
        params![product, minutes_id, snapshot],
    )?;
    Ok(())
}

pub fn normalize_text(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A new item id `m-<4 hex>`, or `m-<6 hex>` after five ids that were taken.
pub fn new_item_id(taken: &BTreeSet<String>) -> Result<String> {
    for digits in [4, 4, 4, 4, 4, 6] {
        let random = RandomState::new().build_hasher().finish();
        let id = format!("m-{:0digits$x}", random & ((1u64 << (digits * 4)) - 1));
        if !taken.contains(&id) {
            return Ok(id);
        }
    }
    Err(crate::Error::Other(
        "Could not allocate a memory item id".into(),
    ))
}

/// Adds each note as a `fact` item, and skips a text equal to an active item
/// after the white space is normalized. Returns the count of new items.
pub fn append_facts(
    tx: &Transaction,
    product: &str,
    agent: &str,
    notes: &[String],
    source: &str,
    created_at: &str,
) -> Result<usize> {
    let mut items = read::memory(tx, product, agent)?;
    let before = items.len();
    let mut seen: HashSet<String> = items
        .iter()
        .filter(|i| i.retired_at.is_none())
        .map(|i| normalize_text(&i.text))
        .collect();
    let mut taken: BTreeSet<String> = items.iter().map(|i| i.id.clone()).collect();
    for note in notes {
        let text = normalize_text(note);
        if text.is_empty() || !seen.insert(text.clone()) {
            continue;
        }
        let id = new_item_id(&taken)?;
        taken.insert(id.clone());
        items.push(read::MemoryItem {
            id,
            text,
            kind: "fact".into(),
            helpful: 0,
            harmful: 0,
            source: source.into(),
            created_at: created_at.into(),
            retired_at: None,
            retired_why: None,
        });
    }
    if items.len() == before {
        return Ok(0);
    }
    crate::reflection::write_memory(tx, product, agent, &items)?;
    Ok(items.len() - before)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_writer;
    use serde_json::json;

    fn db() -> (tempfile::TempDir, rusqlite::Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_writer(&dir.path().join("agentsi.db")).unwrap();
        (dir, conn)
    }

    fn head<'a>(id: &'a str, product: &'a str) -> MinutesHead<'a> {
        MinutesHead {
            id,
            product,
            scope: None,
            question: "q",
            status: "invalid",
            created_at: "2026-11-04T10:00:00.000Z",
        }
    }

    #[test]
    fn stored_keys_come_in_jsonb_order() {
        let v = serde_json::json!({"roles": {"dev": 1, "pm": 2, "qa": [{"zz": 1, "a": 2}]}});
        assert_eq!(
            jsonb_order(&v).to_string(),
            r#"{"roles":{"pm":2,"qa":[{"a":2,"zz":1}],"dev":1}}"#
        );
    }

    #[test]
    fn an_id_of_another_product_moves_to_the_next_number() {
        let (_d, mut c) = db();
        let tx = c.transaction().unwrap();
        assert_eq!(reserve_minutes_id(&tx, &head("m", "a")).unwrap(), "m");
        assert_eq!(reserve_minutes_id(&tx, &head("m", "b")).unwrap(), "m-2");
        assert_eq!(reserve_minutes_id(&tx, &head("m", "a")).unwrap(), "m");
        assert_eq!(reserve_minutes_id(&tx, &head("m", "c")).unwrap(), "m-3");
    }

    #[test]
    fn minutes_write_their_priorities_decisions_usage_and_log() {
        let (_d, mut c) = db();
        let tx = c.transaction().unwrap();
        reserve_minutes_id(&tx, &head("m", "a")).unwrap();
        let minutes = json!({
            "id": "m", "product": "a", "scope": null, "question": "q", "created_at": "t", "status": "valid",
            "priorities": [{"title": "T", "why": "W", "sources": [{"type": "path", "ref": "x"}],
                            "owner_role": "pm", "objective": null, "mark": null, "is_new": false}],
            "decisions": [{"text": "D", "why": "Y", "action": "A"}, {"text": "E", "why": "Z"}],
        });
        let usage = [UsageWrite {
            role: "pm",
            model: "haiku",
            input_tokens: 1,
            output_tokens: 2,
            cache_read: 3,
            cache_write: 4,
            cost_usd: 0.5,
            duration_ms: 6,
        }];
        store_minutes(&tx, &minutes, &usage, "[]").unwrap();
        store_minutes(&tx, &minutes, &usage, "[]").unwrap();
        write_meeting_log(&tx, "a", "m", &json!({"minutes_id": "m"}), "snap").unwrap();
        write_meeting_log(&tx, "a", "m", &json!({"minutes_id": "m", "n": 2}), "snap 2").unwrap();
        let count = |sql: &str| tx.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(count("select count(*) from priorities"), 1);
        assert_eq!(count("select count(*) from decisions"), 2);
        assert_eq!(count("select count(*) from usage"), 1);
        assert_eq!(count("select count(*) from meeting_log"), 1);
        let status: String = tx
            .query_row("select status from decisions limit 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(read::snapshot(&tx, "a", "m").unwrap().unwrap(), "snap 2");
    }

    #[test]
    fn facts_skip_a_text_that_an_active_item_holds() {
        let (_d, mut c) = db();
        let tx = c.transaction().unwrap();
        let notes = ["One  fact".to_string(), "One fact".into(), "  ".into()];
        assert_eq!(
            append_facts(&tx, "a", "dev", &notes, "m1", "t1").unwrap(),
            1
        );
        let again = ["One fact".to_string(), "Two".into()];
        assert_eq!(
            append_facts(&tx, "a", "dev", &again, "m2", "t2").unwrap(),
            1
        );
        let items = read::memory(&tx, "a", "dev").unwrap();
        assert_eq!(
            items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            ["One fact", "Two"]
        );
        assert!(items[0].id.starts_with("m-") && items[0].id.len() == 6);
        assert_eq!(items[1].source, "m2");
    }
}
