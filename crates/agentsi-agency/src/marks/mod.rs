//! The writes of the owner: marks, decisions and (in `archive`) the archive
//! records. A port of `minutes.ts`, `pending.ts` and `archive.ts` of the
//! TypeScript core (U20).
//!
//! Each write takes a `&Transaction`. The caller commits when the function
//! returns `Ok` and rolls back when it returns `Err`, so a failed write leaves
//! no row (`Store::try_write`). The messages equal the TypeScript messages,
//! because the CLI prints them.

pub mod decide;
pub mod mark;
pub mod pending;

use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// The product, the minutes, the priority or the decision does not exist.
    NotFound(String),
    /// The input is wrong, for example a bad id.
    Invalid(String),
    /// The record is already in the wanted state, or another mark came first.
    Conflict(String),
    /// The store cannot be read or written, or the case is not ported yet.
    Unavailable(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound(m) | Error::Invalid(m) | Error::Conflict(m) | Error::Unavailable(m) => {
                f.write_str(m)
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Error {
        Error::Unavailable(e.to_string())
    }
}

impl From<agentsi_store::Error> for Error {
    fn from(e: agentsi_store::Error) -> Error {
        Error::Unavailable(e.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Error {
        Error::Unavailable(format!("stored JSON does not parse: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive;
    use agentsi_proto::marks_api::{ArchiveKind, Mark, MarkEntry, PendingItem, Verdict};
    use agentsi_store::Store;
    use rusqlite::Connection;

    const MINUTES: &str = r#"{"id":"m1","product":"p","status":"valid","priorities":[
        {"title":"a","why":"w","owner_role":"pm","objective":"o1","sources":[{"type":"path","ref":"x"}],"mark":null,"is_new":false},
        {"title":"b","why":"w","owner_role":"dev","objective":null,"sources":[]},
        {"title":"c","why":"w","owner_role":"dev","objective":null,"sources":[],"mark":"accepted","is_new":false}],
        "decisions":[{"text":"first","why":"w","priorities":[0,1,2,1,9]},{"text":"second","why":"w"}]}"#;

    fn seed(c: &Connection) {
        c.execute_batch(&format!(
            "insert into minutes (id, product, status, created_at, data) values ('m1', 'p', 'valid', '2026-10-01T10:00:00.000Z', '{}');
             insert into minutes (id, product, status, created_at, data) values ('bad', 'p', 'invalid', '2026-10-02T10:00:00.000Z', '{{\"status\":\"invalid\",\"priorities\":[{{\"title\":\"hidden\"}}]}}');
             insert into priorities (minutes_id, idx, title, mark) values ('m1', 0, 'a', null), ('m1', 1, 'b', null), ('m1', 2, 'c', 'accepted');
             insert into decisions (id, product, minutes_id, text, why, status, kind) values
               (1, 'p', 'm1', 'first', 'w', 'pending', 'minutes'),
               (2, 'p', 'm1', 'second', 'w', 'pending', 'minutes'),
               (3, 'p', null, 'loose', 'w', 'pending', 'product'),
               (4, 'p', 'm1', 'hidden', 'w', 'pending', 'minutes'),
               (5, 'p', null, 'reflect', 'w', 'pending', 'reflection');
             insert into archive_records (product, kind, ref) values ('p', 'decision', '4');",
            MINUTES.replace('\n', "")
        ))
        .unwrap();
    }

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agentsi.db");
        let store = Store::open(&path).unwrap();
        store
            .with_writer(|c| {
                seed(c);
                Ok(())
            })
            .unwrap();
        (dir, store)
    }

    fn entry(idx: u32, mark: Mark) -> MarkEntry {
        MarkEntry {
            minutes_id: "m1".into(),
            idx,
            mark,
            is_new: false,
        }
    }

    fn mark_of(store: &Store, idx: i64) -> Option<String> {
        store
            .read(|c| {
                Ok(c.query_row(
                    "select mark from priorities where minutes_id = 'm1' and idx = ?1",
                    [idx],
                    |r| r.get(0),
                )?)
            })
            .unwrap()
    }

    fn json_mark(store: &Store, idx: usize) -> Option<String> {
        let text: String = store
            .read(
                |c| Ok(c.query_row("select data from minutes where id = 'm1'", [], |r| r.get(0))?),
            )
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        v["priorities"][idx]["mark"].as_str().map(String::from)
    }

    #[test]
    fn pending_lists_the_unmarked_priorities_then_their_decisions_and_hides_the_archived() {
        let (_dir, store) = store();
        let got = store.read(|c| Ok(pending::pending(c, "p"))).unwrap();
        assert_eq!(got.decisions_error, None);
        let shape: Vec<String> = got
            .items
            .iter()
            .map(|i| match i {
                PendingItem::Priority(p) => format!("priority {}#{}", p.minutes_id, p.idx),
                PendingItem::Decision(d) => format!("decision {} {:?}", d.id, d.priorities),
            })
            .collect();
        assert_eq!(
            shape,
            [
                "priority m1#0",
                "priority m1#1",
                // The linked list drops the marked #2, the repeat of #1 and the out-of-range #9.
                "decision 1 [0, 1]",
                "decision 2 []",
                "decision 3 []",
                "decision 5 []",
            ]
        );
    }

    #[test]
    fn marks_change_the_row_and_the_json_together() {
        let (_dir, store) = store();
        let marked = store
            .try_write(|tx| {
                mark::set(
                    tx,
                    &[entry(0, Mark::Changed), entry(1, Mark::Rejected)],
                    false,
                )
            })
            .unwrap()
            .unwrap();
        assert_eq!(
            (marked.marked, marked.products.as_slice()),
            (2, ["p".to_string()].as_slice())
        );
        assert_eq!(mark_of(&store, 0).as_deref(), Some("changed"));
        assert_eq!(json_mark(&store, 1).as_deref(), Some("rejected"));
    }

    #[test]
    fn a_bad_batch_is_refused_before_it_writes() {
        let (_dir, store) = store();
        let bad = |entries: Vec<MarkEntry>, unmarked_only| {
            store
                .try_write(move |tx| mark::set(tx, &entries, unmarked_only))
                .unwrap()
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            bad(vec![entry(7, Mark::Accepted)], false),
            "Minutes m1 has no priority #7 (0..2)"
        );
        assert_eq!(
            bad(
                vec![entry(0, Mark::Accepted), entry(0, Mark::Changed)],
                false
            ),
            "Priority m1#0 appears twice in one batch"
        );
        assert_eq!(
            bad(vec![entry(2, Mark::Rejected)], true),
            "Priority m1#2 is already marked accepted"
        );
        let missing = MarkEntry {
            minutes_id: "zz".into(),
            ..entry(0, Mark::Accepted)
        };
        assert_eq!(bad(vec![missing], false), "Minutes \"zz\" not found");
        assert_eq!(mark_of(&store, 0), None);
    }

    #[test]
    fn the_same_mark_again_passes_with_unmarked_only() {
        let (_dir, store) = store();
        let accepted = entry(2, Mark::Accepted);
        let marked = store
            .try_write(move |tx| mark::set(tx, &[accepted], true))
            .unwrap()
            .unwrap();
        assert_eq!(marked.marked, 1);
    }

    #[test]
    fn a_failure_in_the_middle_of_a_write_leaves_no_partial_rows() {
        let (_dir, store) = store();
        // The JSON says #1 has no mark, but the row says it has one: the second
        // update fails after the first one wrote its row.
        store
            .with_writer(|c| {
                c.execute(
                    "update priorities set mark = 'rejected' where minutes_id = 'm1' and idx = 1",
                    [],
                )?;
                Ok(())
            })
            .unwrap();
        let err = store
            .try_write(|tx| {
                mark::set(
                    tx,
                    &[entry(0, Mark::Accepted), entry(1, Mark::Accepted)],
                    true,
                )
            })
            .unwrap()
            .unwrap_err();
        assert_eq!(
            err,
            Error::Conflict("Priority m1#1 is already marked".into())
        );
        assert_eq!(mark_of(&store, 0), None, "the first row was rolled back");
        assert_eq!(json_mark(&store, 0), None, "the JSON did not change");
    }

    #[test]
    fn deciding_sets_the_row_and_the_decision_in_the_json_by_position() {
        let (_dir, store) = store();
        let at = 1_788_000_000_000;
        let done = store
            .try_write(move |tx| decide::decide(tx, 2, Verdict::Rejected, at))
            .unwrap()
            .unwrap();
        assert_eq!(done.minutes_id.as_deref(), Some("m1"));
        let (status, decided): (String, String) = store
            .read(|c| {
                Ok(c.query_row(
                    "select status, decided_at from decisions where id = 2",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .unwrap();
        assert_eq!(status, "rejected");
        let text: String = store
            .read(
                |c| Ok(c.query_row("select data from minutes where id = 'm1'", [], |r| r.get(0))?),
            )
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        // Decision 2 is the second decision of m1 (the first has the lower id).
        assert_eq!(v["decisions"][1]["status"], "rejected");
        assert_eq!(v["decisions"][1]["decided_at"], decided.as_str());
        assert!(v["decisions"][0].get("status").is_none());
    }

    #[test]
    fn deciding_refuses_an_unknown_decision_and_a_reflection_with_no_id_and_changes_nothing() {
        let (_dir, store) = store();
        let run = |id| {
            store
                .try_write(move |tx| decide::decide(tx, id, Verdict::Approved, 0))
                .unwrap()
                .unwrap_err()
        };
        assert_eq!(run(99), Error::NotFound("Decision 99 not found".into()));
        assert_eq!(
            run(5),
            Error::Unavailable(
                "Decision 5 is a reflection decision without a reflection id".into()
            )
        );
        let pending: i64 = store
            .read(|c| {
                Ok(c.query_row(
                    "select count(*) from decisions where status = 'pending'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(pending, 5);
    }

    #[test]
    fn a_decision_whose_minutes_vanished_rolls_back_its_row() {
        let (_dir, store) = store();
        store
            .with_writer(|c| {
                c.execute("update minutes set data = null where id = 'm1'", [])?;
                Ok(())
            })
            .unwrap();
        let err = store
            .try_write(|tx| decide::decide(tx, 1, Verdict::Approved, 0))
            .unwrap()
            .unwrap_err();
        assert_eq!(err, Error::NotFound("Minutes \"m1\" not found".into()));
        let status: String = store
            .read(|c| {
                Ok(
                    c.query_row("select status from decisions where id = 1", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap();
        assert_eq!(status, "pending");
    }

    #[test]
    fn a_new_decision_is_pending_and_of_kind_product() {
        let (_dir, store) = store();
        let id = store
            .try_write(|tx| decide::add(tx, "p", "Pick a name", "It is open", Some("")))
            .unwrap()
            .unwrap();
        let row: (String, Option<String>, Option<String>) = store
            .read(|c| {
                Ok(c.query_row(
                    "select kind || '/' || status, minutes_id, action from decisions where id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )?)
            })
            .unwrap();
        assert_eq!(row, ("product/pending".into(), None, None));
    }

    #[test]
    fn archive_and_restore_flip_one_record_and_say_when_nothing_changes() {
        let (_dir, store) = store();
        let run = |kind, id: Option<&str>, archive| {
            let id = id.map(String::from);
            store
                .try_write(move |tx| archive::change(tx, "p", kind, id.as_deref(), archive))
                .unwrap()
        };
        run(ArchiveKind::Minutes, Some("m1"), true).unwrap();
        assert_eq!(
            run(ArchiveKind::Minutes, Some("m1"), true)
                .unwrap_err()
                .to_string(),
            "Minutes m1 is already archived."
        );
        run(ArchiveKind::Minutes, Some("m1"), false).unwrap();
        assert_eq!(
            run(ArchiveKind::Minutes, Some("m1"), false)
                .unwrap_err()
                .to_string(),
            "Minutes m1 is not archived."
        );
        assert_eq!(
            run(ArchiveKind::Minutes, Some("nope"), true)
                .unwrap_err()
                .to_string(),
            "No minutes nope for p."
        );
        assert_eq!(
            run(ArchiveKind::Minutes, Some("../x"), true)
                .unwrap_err()
                .to_string(),
            "Bad minutes id \"../x\"."
        );
        run(ArchiveKind::Decision, Some("2"), true).unwrap();
        assert_eq!(
            run(ArchiveKind::Decision, Some("0"), true)
                .unwrap_err()
                .to_string(),
            "Bad decision id \"0\"."
        );
        run(ArchiveKind::Product, None, true).unwrap();
        assert_eq!(
            run(ArchiveKind::Product, None, true)
                .unwrap_err()
                .to_string(),
            "Product p is already archived."
        );
    }

    #[test]
    fn a_failed_decisions_read_keeps_the_priorities_and_reports_the_error() {
        let (_dir, store) = store();
        store
            .with_writer(|c| {
                c.execute_batch("drop table archive_records")?;
                Ok(())
            })
            .unwrap();
        let got = store.read(|c| Ok(pending::pending(c, "p"))).unwrap();
        assert!(got.decisions_error.is_some());
        let kinds: Vec<&str> = got
            .items
            .iter()
            .map(|i| match i {
                PendingItem::Priority(_) => "priority",
                PendingItem::Decision(_) => "decision",
            })
            .collect();
        assert_eq!(kinds, ["priority", "priority"]);
    }

    #[test]
    fn a_decision_with_no_text_links_no_priorities() {
        let (_dir, store) = store();
        store
            .with_writer(|c| {
                c.execute_batch(
                    "update decisions set text = null where id = 1;
                     update minutes set data = json_set(data, '$.decisions[0].text', null)
                       where id = 'm1';",
                )?;
                Ok(())
            })
            .unwrap();
        let got = store.read(|c| Ok(pending::pending(c, "p"))).unwrap();
        let linked = got.items.iter().find_map(|i| match i {
            PendingItem::Decision(d) if d.id == 1 => Some(d.priorities.clone()),
            _ => None,
        });
        assert_eq!(linked, Some(vec![]));
    }
}
