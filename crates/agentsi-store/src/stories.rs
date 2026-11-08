//! The writes of stories mode (U23): the story JSON and the story log. The
//! reads are in `read`.

use crate::error::Result;
use rusqlite::{Connection, params};

/// Inserts the story JSON, or replaces the story with the same product and id.
pub fn upsert_story(c: &Connection, product: &str, id: &str, data: &str) -> Result<()> {
    c.execute(
        "insert into stories (product, id, data) values (?1, ?2, ?3)
         on conflict (product, id) do update set data = excluded.data",
        params![product, id, data],
    )?;
    Ok(())
}

/// Appends `text` to the log of one story.
pub fn append_story_log(c: &Connection, product: &str, story_id: &str, text: &str) -> Result<()> {
    c.execute(
        "insert into story_logs (product, story_id, body) values (?1, ?2, ?3)
         on conflict (product, story_id) do update set body = story_logs.body || excluded.body",
        params![product, story_id, text],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read;

    #[test]
    fn a_story_is_replaced_and_its_log_grows() {
        let dir = tempfile::tempdir().unwrap();
        let c = crate::db::open_writer(&dir.path().join("agentsi.db")).unwrap();
        upsert_story(&c, "p", "S001", r#"{"v":1}"#).unwrap();
        upsert_story(&c, "p", "S001", r#"{"v":2}"#).unwrap();
        assert_eq!(read::story(&c, "p", "S001").unwrap().unwrap()["v"], 2);
        append_story_log(&c, "p", "S001", "a\n").unwrap();
        append_story_log(&c, "p", "S001", "b\n").unwrap();
        assert_eq!(
            read::story_log(&c, "p", "S001").unwrap().as_deref(),
            Some("a\nb\n")
        );
    }
}
