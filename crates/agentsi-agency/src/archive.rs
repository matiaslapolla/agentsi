//! `agentsi archive` and `agentsi restore`. A port of `archive.ts`: an
//! archive change writes or deletes one `archive_records` row and nothing
//! else. The record is hidden from the lists and the counts, not deleted.

use crate::marks::{Error, Result};
use agentsi_proto::marks_api::ArchiveKind;
use regex::Regex;
use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::Value;
use std::sync::LazyLock;

static MINUTES_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_][A-Za-z0-9._-]*$").expect("a valid pattern"));

/// A story id: `S001`, `T012`.
static STORY_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[ST]\d{3,}$").expect("a valid pattern"));

/// Flips one record. It is an error when the record is already in the wanted
/// state. `label` names it in the message.
fn flip(
    tx: &Transaction,
    product: &str,
    kind: &str,
    reference: &str,
    archive: bool,
    label: &str,
) -> Result<()> {
    let changed = if archive {
        tx.execute(
            "insert into archive_records (product, kind, ref) values (?1, ?2, ?3) on conflict do nothing",
            params![product, kind, reference],
        )?
    } else {
        tx.execute(
            "delete from archive_records where product = ?1 and kind = ?2 and ref = ?3",
            params![product, kind, reference],
        )?
    };
    if changed == 0 {
        return Err(Error::Conflict(format!(
            "{label} is {}.",
            if archive {
                "already archived"
            } else {
                "not archived"
            }
        )));
    }
    Ok(())
}

fn exists(tx: &Transaction, sql: &str, args: impl rusqlite::Params) -> Result<bool> {
    Ok(tx.query_row(sql, args, |r| r.get(0))?)
}

fn minutes(tx: &Transaction, product: &str, id: &str, archive: bool) -> Result<()> {
    if !MINUTES_ID.is_match(id) || id.contains("..") {
        return Err(Error::Invalid(format!("Bad minutes id \"{id}\".")));
    }
    if !exists(
        tx,
        "select exists(select 1 from minutes where id = ?1 and product = ?2 and data is not null)",
        params![id, product],
    )? {
        return Err(Error::NotFound(format!("No minutes {id} for {product}.")));
    }
    flip(
        tx,
        product,
        "minutes",
        id,
        archive,
        &format!("Minutes {id}"),
    )
}

fn decision(tx: &Transaction, product: &str, id: &str, archive: bool) -> Result<()> {
    match id.parse::<i64>() {
        Ok(n) if n >= 1 && n.to_string() == id => flip(
            tx,
            product,
            "decision",
            id,
            archive,
            &format!("Decision {id}"),
        ),
        _ => Err(Error::Invalid(format!("Bad decision id \"{id}\"."))),
    }
}

/// A story that has a session running cannot be archived.
fn story(tx: &Transaction, product: &str, id: &str, archive: bool) -> Result<()> {
    if !STORY_ID.is_match(id) {
        return Err(Error::Invalid(format!("Invalid story id \"{id}\"")));
    }
    let data: Option<String> = tx
        .query_row(
            "select data from stories where product = ?1 and id = ?2",
            params![product, id],
            |r| r.get(0),
        )
        .optional()?;
    let data =
        data.ok_or_else(|| Error::NotFound(format!("Unknown story \"{id}\" in {product}")))?;
    let json: Value = serde_json::from_str(&data)?;
    // `isRunningSession`: a session that is neither failed nor ready.
    let running = json
        .get("session")
        .and_then(|s| s.get("state"))
        .and_then(Value::as_str)
        .is_some_and(|s| s != "failed" && s != "ready");
    if archive && running {
        return Err(Error::Conflict(format!(
            "Story {id} has a story session running; wait for it to finish before archiving."
        )));
    }
    flip(tx, product, "story", id, archive, &format!("Story {id}"))
}

/// Archives or restores one record. `id` is left out for the whole product.
pub fn change(
    tx: &Transaction,
    product: &str,
    kind: ArchiveKind,
    id: Option<&str>,
    archive: bool,
) -> Result<()> {
    match (kind, id) {
        (ArchiveKind::Product, _) => flip(
            tx,
            product,
            "product",
            "",
            archive,
            &format!("Product {product}"),
        ),
        (_, None) => Err(Error::Invalid(format!(
            "An id is needed to {} a {}.",
            if archive { "archive" } else { "restore" },
            kind.as_str()
        ))),
        (ArchiveKind::Minutes, Some(id)) => minutes(tx, product, id, archive),
        (ArchiveKind::Decision, Some(id)) => decision(tx, product, id, archive),
        (ArchiveKind::Story, Some(id)) => story(tx, product, id, archive),
    }
}

/// The product that archived the decision, looking through `products`.
pub fn archived_decision_product(
    c: &rusqlite::Connection,
    products: &[String],
    id: i64,
) -> Result<Option<String>> {
    let wanted = id.to_string();
    for p in products {
        let hit = agentsi_store::read::archived(c, p)?
            .iter()
            .any(|r| r.kind == "decision" && r.reference == wanted);
        if hit {
            return Ok(Some(p.clone()));
        }
    }
    Ok(None)
}
