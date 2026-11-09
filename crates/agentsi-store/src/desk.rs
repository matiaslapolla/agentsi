//! The desk conversation of each product: its desk session (`desk_sessions`) and its items
//! (`desk_items`). The body of an item is the JSON of the item of the protocol.

use crate::error::Result;
use rusqlite::{Connection, OptionalExtension, params};

/// One stored item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeskRow {
    pub seq: i64,
    pub kind: String,
    pub body: String,
    pub sent: bool,
    pub at: String,
}

/// The session uuid of a product.
pub fn session(c: &Connection, product: &str) -> Result<Option<String>> {
    Ok(c.query_row(
        "select session from desk_sessions where product = ?1",
        [product],
        |r| r.get(0),
    )
    .optional()?)
}

/// The product of a session uuid.
pub fn product_of(c: &Connection, session: &str) -> Result<Option<String>> {
    Ok(c.query_row(
        "select product from desk_sessions where session = ?1",
        [session],
        |r| r.get(0),
    )
    .optional()?)
}

/// Gives the product the session `session`, in place of the one it had.
pub fn set_session(c: &Connection, product: &str, session: &str, at: &str) -> Result<()> {
    c.execute(
        "insert into desk_sessions (product, session, created_at) values (?1, ?2, ?3)
         on conflict (product) do update set session = excluded.session, created_at = excluded.created_at",
        params![product, session, at],
    )?;
    Ok(())
}

/// Stores an item after the last one, and gives its `seq`. A note starts unsent.
pub fn add_item(c: &Connection, product: &str, kind: &str, body: &str, at: &str) -> Result<i64> {
    let seq: i64 = c.query_row(
        "select coalesce(max(seq), 0) + 1 from desk_items where product = ?1",
        [product],
        |r| r.get(0),
    )?;
    c.execute(
        "insert into desk_items (product, seq, kind, body, sent, at) values (?1, ?2, ?3, ?4, 0, ?5)",
        params![product, seq, kind, body, at],
    )?;
    Ok(seq)
}

/// The last `limit` items with a `seq` below `before`, oldest first.
pub fn items(
    c: &Connection,
    product: &str,
    before: Option<i64>,
    limit: u32,
) -> Result<Vec<DeskRow>> {
    let mut stmt = c.prepare(
        "select seq, kind, body, sent, at from desk_items
         where product = ?1 and seq < ?2 order by seq desc limit ?3",
    )?;
    let mut rows = stmt
        .query_map(params![product, before.unwrap_or(i64::MAX), limit], |r| {
            Ok(DeskRow {
                seq: r.get(0)?,
                kind: r.get(1)?,
                body: r.get(2)?,
                sent: r.get::<_, i64>(3)? != 0,
                at: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.reverse();
    Ok(rows)
}

/// True when the conversation has an item of this kind.
pub fn has_kind(c: &Connection, product: &str, kind: &str) -> Result<bool> {
    Ok(c.query_row(
        "select exists (select 1 from desk_items where product = ?1 and kind = ?2)",
        params![product, kind],
        |r| r.get(0),
    )?)
}

/// Marks the unsent notes sent, and gives their bodies, oldest first.
pub fn take_notes(c: &Connection, product: &str) -> Result<Vec<String>> {
    let mut stmt = c.prepare(
        "update desk_items set sent = 1 where product = ?1 and kind = 'note' and sent = 0
         returning seq, body",
    )?;
    let mut notes = stmt
        .query_map([product], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    notes.sort();
    Ok(notes.into_iter().map(|(_, body)| body).collect())
}

/// Deletes the items of a product and gives it the session `session`.
pub fn reset(c: &Connection, product: &str, session: &str, at: &str) -> Result<()> {
    c.execute("delete from desk_items where product = ?1", [product])?;
    set_session(c, product, session, at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("agentsi.db")).unwrap();
        (dir, store)
    }

    #[test]
    fn a_session_maps_both_ways_and_a_reset_replaces_it() {
        let (_d, s) = store();
        s.write(|tx| {
            assert_eq!(session(tx, "alpha")?, None);
            set_session(tx, "alpha", "u1", "t0")?;
            assert_eq!(session(tx, "alpha")?.as_deref(), Some("u1"));
            assert_eq!(product_of(tx, "u1")?.as_deref(), Some("alpha"));
            add_item(
                tx,
                "alpha",
                "owner",
                r#"{"kind":"owner","text":"hi"}"#,
                "t1",
            )?;
            reset(tx, "alpha", "u2", "t2")?;
            assert_eq!(product_of(tx, "u1")?, None);
            assert_eq!(session(tx, "alpha")?.as_deref(), Some("u2"));
            assert!(items(tx, "alpha", None, 10)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn items_count_up_by_product_and_page_back_oldest_first() {
        let (_d, s) = store();
        s.write(|tx| {
            for n in 1..=5 {
                assert_eq!(add_item(tx, "alpha", "reply", "{}", &format!("t{n}"))?, n);
            }
            assert_eq!(add_item(tx, "beta", "reply", "{}", "t")?, 1);
            let seqs = |v: Vec<DeskRow>| v.into_iter().map(|r| r.seq).collect::<Vec<_>>();
            assert_eq!(seqs(items(tx, "alpha", None, 2)?), [4, 5]);
            assert_eq!(seqs(items(tx, "alpha", Some(4), 2)?), [2, 3]);
            assert!(has_kind(tx, "alpha", "reply")?);
            assert!(!has_kind(tx, "alpha", "end")?);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_unsent_notes_go_out_once_in_order() {
        let (_d, s) = store();
        s.write(|tx| {
            add_item(tx, "alpha", "note", r#""a""#, "t")?;
            add_item(tx, "alpha", "owner", "{}", "t")?;
            add_item(tx, "alpha", "note", r#""b""#, "t")?;
            add_item(tx, "beta", "note", r#""c""#, "t")?;
            assert_eq!(take_notes(tx, "alpha")?, [r#""a""#, r#""b""#]);
            assert!(take_notes(tx, "alpha")?.is_empty());
            let sent: Vec<bool> = items(tx, "alpha", None, 9)?
                .iter()
                .map(|r| r.sent)
                .collect();
            assert_eq!(sent, [true, false, true]);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_bad_kind_or_body_is_refused() {
        let (_d, s) = store();
        assert!(
            s.write(|tx| add_item(tx, "alpha", "other", "{}", "t"))
                .is_err()
        );
        assert!(
            s.write(|tx| add_item(tx, "alpha", "note", "{x", "t"))
                .is_err()
        );
    }
}
