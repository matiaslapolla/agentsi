//! The writes of reflection: the memory of an agent, the reflection entries and
//! the overlay of an approved proposal. The rules copy `replaceItems` of
//! `memory.ts`, `saveReflections` of `reflections.ts` and `writeOverlay` of
//! `overlays.ts`. Each function takes the transaction of the caller, so one
//! reflection writes as one.

use crate::error::Result;
use crate::meeting::jsonb_order;
use crate::read::MemoryItem;
use rusqlite::{Transaction, params};
use serde_json::{Number, Value};

/// Makes the stored items of one agent exactly `items`, in that order.
pub fn write_memory(
    tx: &Transaction,
    product: &str,
    agent: &str,
    items: &[MemoryItem],
) -> Result<()> {
    tx.execute(
        "delete from memory where product = ?1 and agent = ?2",
        [product, agent],
    )?;
    for (position, i) in items.iter().enumerate() {
        tx.execute(
            "insert into memory (product, agent, id, position, text, kind, helpful, harmful, source, created_at, retired_at, retired_why)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                product,
                agent,
                i.id,
                position as i64,
                i.text,
                i.kind,
                i.helpful,
                i.harmful,
                i.source,
                i.created_at,
                i.retired_at,
                i.retired_why
            ],
        )?;
    }
    Ok(())
}

/// A whole float as an integer, as `JSON.stringify` and Postgres `jsonb` write it.
fn whole_numbers(v: Value) -> Value {
    match v {
        Value::Number(n) => match n.as_f64() {
            Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 9e15 => {
                Value::Number(Number::from(f as i64))
            }
            _ => Value::Number(n),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(whole_numbers).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, whole_numbers(v)))
                .collect(),
        ),
        other => other,
    }
}

/// Makes the stored entries of the product exactly `entries`, in that order.
pub fn write_reflections(tx: &Transaction, product: &str, entries: &[Value]) -> Result<()> {
    tx.execute("delete from reflections where product = ?1", [product])?;
    for (position, entry) in entries.iter().enumerate() {
        tx.execute(
            "insert into reflections (product, id, position, entry) values (?1, ?2, ?3, ?4)",
            params![
                product,
                entry.get("id").and_then(Value::as_str),
                position as i64,
                jsonb_order(&whole_numbers(entry.clone())).to_string()
            ],
        )?;
    }
    Ok(())
}

/// Stores the overlay of one agent, in place of the one it had.
pub fn write_overlay(tx: &Transaction, product: &str, agent: &str, text: &str) -> Result<()> {
    tx.execute(
        "insert into agent_overlays (product, agent, body) values (?1, ?2, ?3)
         on conflict (product, agent) do update set body = excluded.body",
        params![product, agent, text],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_writer;
    use crate::read;
    use serde_json::json;

    fn item(id: &str) -> MemoryItem {
        MemoryItem {
            id: id.into(),
            text: format!("text {id}"),
            kind: "fact".into(),
            helpful: 1,
            harmful: 2,
            source: "m1".into(),
            created_at: "t".into(),
            retired_at: None,
            retired_why: None,
        }
    }

    #[test]
    fn the_memory_of_an_agent_is_replaced_and_keeps_its_order() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("agentsi.db")).unwrap();
        let tx = c.transaction().unwrap();
        write_memory(&tx, "a", "dev", &[item("m-1"), item("m-2")]).unwrap();
        write_memory(&tx, "a", "qa", &[item("m-9")]).unwrap();
        write_memory(&tx, "a", "dev", &[item("m-3"), item("m-1")]).unwrap();
        let ids = |agent: &str| -> Vec<String> {
            read::memory(&tx, "a", agent)
                .unwrap()
                .into_iter()
                .map(|i| i.id)
                .collect()
        };
        assert_eq!(ids("dev"), ["m-3", "m-1"]);
        assert_eq!(ids("qa"), ["m-9"]);
    }

    #[test]
    fn entries_keep_their_order_and_a_whole_cost_has_no_point() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("agentsi.db")).unwrap();
        let tx = c.transaction().unwrap();
        let entries = [json!({"id": "r-2", "cost_usd": 1.0}), json!({"id": "r-1"})];
        write_reflections(&tx, "a", &entries).unwrap();
        write_reflections(&tx, "b", &[json!({"id": "r-3"})]).unwrap();
        let back = read::reflections(&tx, "a").unwrap();
        assert_eq!(back[0]["id"], "r-2");
        assert_eq!(back[1]["id"], "r-1");
        assert_eq!(back[0]["cost_usd"].to_string(), "1");
        assert_eq!(read::reflections(&tx, "b").unwrap().len(), 1);
    }

    #[test]
    fn an_overlay_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("agentsi.db")).unwrap();
        let tx = c.transaction().unwrap();
        write_overlay(&tx, "a", "dev", "one").unwrap();
        write_overlay(&tx, "a", "dev", "two").unwrap();
        assert_eq!(read::overlay(&tx, "a", "dev").unwrap().unwrap(), "two");
    }
}
