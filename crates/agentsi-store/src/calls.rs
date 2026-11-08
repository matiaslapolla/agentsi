//! The writes of calls (U26): the listened mark. The reads are in `read`.
//! The rules copy `recordListen` and `removeListenRecord` of `inbox-data.ts`.

use crate::error::Result;
use rusqlite::{Connection, params};

/// Stores that the owner heard the call. Returns false when a row exists already, and
/// keeps its first time.
pub fn record_listen(c: &Connection, product: &str, minutes_id: &str, at: &str) -> Result<bool> {
    let n = c.execute(
        "insert into listens (product, minutes_id, at) values (?1, ?2, ?3)
         on conflict (product, minutes_id) do nothing",
        params![product, minutes_id, at],
    )?;
    Ok(n > 0)
}

/// Removes the mark of one call and no other. Returns false when there was none.
pub fn remove_listen(c: &Connection, product: &str, minutes_id: &str) -> Result<bool> {
    let n = c.execute(
        "delete from listens where product = ?1 and minutes_id = ?2",
        params![product, minutes_id],
    )?;
    Ok(n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read;

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let c = crate::db::open_writer(&dir.path().join("agentsi.db")).unwrap();
        (dir, c)
    }

    #[test]
    fn a_listen_is_stored_once_and_keeps_its_first_time() {
        let (_d, c) = db();
        assert!(record_listen(&c, "p", "m1", "2026-10-20T10:00:00.000Z").unwrap());
        assert!(!record_listen(&c, "p", "m1", "2026-10-21T10:00:00.000Z").unwrap());
        let rows = read::listens(&c, "p").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].at, "2026-10-20T10:00:00.000Z");
    }

    #[test]
    fn removing_a_listen_keeps_the_others() {
        let (_d, c) = db();
        record_listen(&c, "p", "m1", "2026-10-20T10:00:00.000Z").unwrap();
        record_listen(&c, "p", "m2", "2026-10-20T11:00:00.000Z").unwrap();
        record_listen(&c, "q", "m1", "2026-10-20T12:00:00.000Z").unwrap();
        assert!(remove_listen(&c, "p", "m1").unwrap());
        assert!(!remove_listen(&c, "p", "m1").unwrap());
        let ids: Vec<String> = read::listens(&c, "p")
            .unwrap()
            .into_iter()
            .map(|l| l.minutes_id)
            .collect();
        assert_eq!(ids, ["m2"]);
        assert_eq!(read::listens(&c, "q").unwrap().len(), 1);
    }
}
