//! `agentsi mark`: sets the marks of priorities. A port of `setMarks`,
//! `persistMarks` and `applyMarks` of `minutes.ts`. The `priorities` rows and
//! the minutes JSON change in the one transaction of the caller.

use super::{Error, Result};
use agentsi_proto::marks_api::MarkEntry;
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::Value;

/// What a call changed: the number of marks, and the products of the minutes.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Marked {
    pub marked: u32,
    pub products: Vec<String>,
}

struct Loaded {
    id: String,
    product: String,
    data: Value,
}

fn load(tx: &Transaction, id: &str) -> Result<Loaded> {
    let row: Option<(String, String)> = tx
        .query_row(
            "select product, data from minutes where id = ?1 and data is not null",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (product, data) =
        row.ok_or_else(|| Error::NotFound(format!("Minutes \"{id}\" not found")))?;
    Ok(Loaded {
        id: id.to_string(),
        product,
        data: serde_json::from_str(&data)?,
    })
}

fn priorities(m: &Loaded) -> &[Value] {
    m.data
        .get("priorities")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// Checks every entry before any change, as `setMarks` does. Then sets the
/// marks in memory.
fn set_in_memory(loaded: &mut [Loaded], entries: &[MarkEntry], unmarked_only: bool) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for e in entries {
        let m = loaded
            .iter()
            .find(|m| m.id == e.minutes_id)
            .ok_or_else(|| Error::NotFound(format!("Minutes \"{}\" not found", e.minutes_id)))?;
        let list = priorities(m);
        let Some(p) = list.get(e.idx as usize) else {
            return Err(Error::NotFound(format!(
                "Minutes {} has no priority #{} (0..{})",
                e.minutes_id,
                e.idx,
                list.len() as i64 - 1
            )));
        };
        let key = format!("{}#{}", e.minutes_id, e.idx);
        if !seen.insert(key.clone()) {
            return Err(Error::Invalid(format!(
                "Priority {key} appears twice in one batch"
            )));
        }
        let have = p
            .get("mark")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        if unmarked_only
            && let Some(have) = have
            && (have != e.mark.as_str() || p.get("is_new") != Some(&Value::Bool(e.is_new)))
        {
            return Err(Error::Conflict(format!(
                "Priority {key} is already marked {have}"
            )));
        }
    }
    for e in entries {
        let m = loaded
            .iter_mut()
            .find(|m| m.id == e.minutes_id)
            .expect("checked above");
        if let Some(p) = m
            .data
            .get_mut("priorities")
            .and_then(|l| l.get_mut(e.idx as usize))
            .and_then(Value::as_object_mut)
        {
            p.insert("mark".into(), Value::String(e.mark.as_str().into()));
            p.insert("is_new".into(), Value::Bool(e.is_new));
        }
    }
    Ok(())
}

/// Sets several marks. An error means that nothing was written, once the
/// caller rolls the transaction back.
pub fn set(tx: &Transaction, entries: &[MarkEntry], unmarked_only: bool) -> Result<Marked> {
    if entries.is_empty() {
        return Ok(Marked::default());
    }
    let mut loaded: Vec<Loaded> = Vec::new();
    for e in entries {
        if !loaded.iter().any(|m| m.id == e.minutes_id) {
            loaded.push(load(tx, &e.minutes_id)?);
        }
    }
    set_in_memory(&mut loaded, entries, unmarked_only)?;

    for e in entries {
        let guard = if unmarked_only {
            " and (mark is null or (mark = ?3 and is_new = ?4))"
        } else {
            ""
        };
        let args = params![e.minutes_id, e.idx, e.mark.as_str(), e.is_new];
        let changed = tx.execute(
            &format!("update priorities set mark = ?3, is_new = ?4 where minutes_id = ?1 and idx = ?2{guard}"),
            args,
        )?;
        if unmarked_only && changed == 0 {
            let exists: bool = tx.query_row(
                "select exists(select 1 from priorities where minutes_id = ?1 and idx = ?2)",
                params![e.minutes_id, e.idx],
                |r| r.get(0),
            )?;
            if exists {
                return Err(Error::Conflict(format!(
                    "Priority {}#{} is already marked",
                    e.minutes_id, e.idx
                )));
            }
        }
    }
    let mut products: Vec<String> = Vec::new();
    for m in &loaded {
        tx.execute(
            "update minutes set data = ?2 where id = ?1",
            params![m.id, serde_json::to_string(&m.data)?],
        )?;
        if !products.contains(&m.product) {
            products.push(m.product.clone());
        }
    }
    Ok(Marked {
        marked: entries.len() as u32,
        products,
    })
}
