//! Clear data: the rows and files that a delete removes. Spec:
//! `docs/specs/agentsi-desktop-clear-data.md`.
//!
//! A plan is a dry run of the delete: the same statements run in a transaction that
//! rolls back, so the counts of the plan are the counts of the delete, also when two
//! items share rows. The files of a plan are relative to the data root, and only the
//! folders in `ALLOWED` can hold them.

use crate::backup::backup_dates;
use crate::error::{Error, Result};
use rusqlite::{Connection, params};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;

/// The only places of the data root where a delete removes files.
pub const ALLOWED: [&str; 4] = ["workspace", "voice/transcripts", "logs", "backups"];

/// Each table, children first.
pub const TABLES: [&str; 18] = [
    "priorities",
    "usage",
    "documents",
    "index_state",
    "minutes",
    "decisions",
    "stories",
    "story_logs",
    "memory",
    "agent_overlays",
    "briefs",
    "meeting_log",
    "snapshots",
    "listens",
    "reflections",
    "archive_records",
    "tickets",
    "products",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Product,
    Meetings,
    Stories,
    RoleLearning,
    CallAudio,
    VoiceTranscripts,
    Logs,
    Backups,
    Everything,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub kind: Kind,
    pub product: Option<String>,
    /// `YYYY-MM-DD`: only what is older than this day (UTC).
    pub before: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Relative to the data root, with `/` between the parts.
    pub path: String,
    pub bytes: u64,
    pub dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemPlan {
    pub item: Item,
    /// The rows per table. A table with no row is left out.
    pub tables: Vec<(&'static str, u64)>,
    pub files: Vec<FileEntry>,
}

impl ItemPlan {
    pub fn rows(&self) -> u64 {
        self.tables.iter().map(|t| t.1).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failed {
    pub path: String,
    pub error: String,
}

/// A name that cannot leave its folder: `safeSegment` of the web app.
pub fn safe_segment(s: &str) -> bool {
    let mut chars = s.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !s.contains("..")
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(Error::Other(message.into()))
}

/// Checks the item. The message is for a person.
pub fn validate(item: &Item) -> Result<()> {
    let needs_product = matches!(
        item.kind,
        Kind::Product | Kind::Meetings | Kind::Stories | Kind::RoleLearning
    );
    match (&item.product, needs_product) {
        (None, true) => return invalid("This item needs a product."),
        (Some(_), false) if item.kind != Kind::CallAudio => {
            return invalid("This item has no product choice.");
        }
        (Some(p), _) if !safe_segment(p) => return invalid(format!("Bad product id \"{p}\".")),
        _ => {}
    }
    match (&item.before, item.kind) {
        (Some(d), Kind::Meetings | Kind::VoiceTranscripts) => {
            parse_day(d)
                .ok_or_else(|| Error::Other(format!("Bad date \"{d}\". Use YYYY-MM-DD.")))?;
        }
        (Some(_), _) => return invalid("This item has no date choice."),
        (None, _) => {}
    }
    Ok(())
}

/// Seconds from the epoch to 00:00 UTC of a `YYYY-MM-DD` day.
fn parse_day(day: &str) -> Option<i64> {
    let b = day.as_bytes();
    let digits = b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        });
    if !digits {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (
        day[..4].parse().ok()?,
        day[5..7].parse().ok()?,
        day[8..].parse().ok()?,
    );
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    // The days-from-civil algorithm of Howard Hinnant.
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some((era * 146_097 + doe - 719_468) * 86_400)
}

fn del(
    tx: &Connection,
    tables: &mut Vec<(&'static str, u64)>,
    table: &'static str,
    sql: &str,
    args: impl rusqlite::Params,
) -> Result<()> {
    let n = tx.execute(sql, args)? as u64;
    if n > 0 {
        match tables.iter_mut().find(|t| t.0 == table) {
            Some(t) => t.1 += n,
            None => tables.push((table, n)),
        }
    }
    Ok(())
}

fn ids_json(ids: &[String]) -> String {
    serde_json::to_string(ids).unwrap_or_else(|_| "[]".into())
}

fn strings(tx: &Connection, sql: &str, args: impl rusqlite::Params) -> Result<Vec<String>> {
    let mut stmt = tx.prepare(sql)?;
    let rows = stmt.query_map(args, |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<_, _>>()?)
}

/// The log files of the data root that the item Logs deletes.
const LOG_FILES: [&str; 2] = ["logs/hook.log", "logs/setup.jsonl"];

fn log_files(root: &Path) -> impl Iterator<Item = FileEntry> + '_ {
    LOG_FILES
        .iter()
        .filter_map(move |rel| sized(root, rel.to_string()))
        .filter(|e| !e.dir)
}

fn sized(root: &Path, rel: String) -> Option<FileEntry> {
    let meta = std::fs::symlink_metadata(root.join(&rel)).ok()?;
    if meta.is_dir() {
        Some(FileEntry {
            bytes: tree_bytes(&root.join(&rel)),
            path: rel,
            dir: true,
        })
    } else {
        Some(FileEntry {
            bytes: if meta.is_file() { meta.len() } else { 0 },
            path: rel,
            dir: false,
        })
    }
}

/// The bytes of the files under `dir`. It never follows a link.
pub fn tree_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => tree_bytes(&e.path()),
            Ok(m) if m.is_file() => m.len(),
            _ => 0,
        })
        .sum()
}

fn names(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .map(|it| {
            it.filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

fn audio_dirs(root: &Path, product: &str, only: Option<&[String]>) -> Vec<FileEntry> {
    let base = root.join("workspace").join(product).join("audio");
    let wanted: Vec<String> = match only {
        Some(ids) => ids.iter().filter(|id| safe_segment(id)).cloned().collect(),
        None => names(&base),
    };
    wanted
        .into_iter()
        .filter(|id| safe_segment(id))
        .filter_map(|id| sized(root, format!("workspace/{product}/audio/{id}")))
        .filter(|e| e.dir)
        .collect()
}

fn transcripts(root: &Path, before: Option<&str>) -> Vec<FileEntry> {
    let limit = before.and_then(parse_day);
    let dir = root.join("voice").join("transcripts");
    names(&dir)
        .into_iter()
        .filter(|n| n.ends_with(".md"))
        .filter_map(|n| {
            let meta = std::fs::symlink_metadata(dir.join(&n)).ok()?;
            if !meta.is_file() {
                return None;
            }
            if let Some(limit) = limit {
                let at = meta
                    .modified()
                    .ok()?
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64);
                if at >= limit {
                    return None;
                }
            }
            Some(FileEntry {
                path: format!("voice/transcripts/{n}"),
                bytes: meta.len(),
                dir: false,
            })
        })
        .collect()
}

fn old_backups(root: &Path) -> Result<Vec<FileEntry>> {
    let dates = backup_dates(root)?;
    let old = &dates[..dates.len().saturating_sub(1)];
    Ok(old
        .iter()
        .flat_map(|d| {
            [
                format!("backups/agentsi-{d}.db"),
                format!("backups/agentsi.config-{d}.json"),
            ]
        })
        .filter_map(|rel| sized(root, rel).filter(|e| !e.dir))
        .collect())
}

fn minutes_scope(tx: &Connection, product: &str, before: Option<&str>) -> Result<Vec<String>> {
    match before {
        Some(day) => strings(
            tx,
            "select id from minutes where product = ?1 and created_at < ?2 order by id",
            params![product, day],
        ),
        None => strings(
            tx,
            "select id from minutes where product = ?1 order by id",
            params![product],
        ),
    }
}

/// Runs the statements of one item and lists its files.
fn apply(tx: &Connection, root: &Path, item: &Item) -> Result<ItemPlan> {
    let mut tables = Vec::new();
    let mut files = Vec::new();
    let product = item.product.as_deref().unwrap_or_default();
    match item.kind {
        Kind::Product => {
            let t = &mut tables;
            del(
                tx,
                t,
                "priorities",
                "delete from priorities where minutes_id in (select id from minutes where product = ?1)",
                [product],
            )?;
            del(
                tx,
                t,
                "usage",
                "delete from usage where minutes_id in (select id from minutes where product = ?1)",
                [product],
            )?;
            for (table, sql) in [
                ("documents", "delete from documents where product = ?1"),
                ("index_state", "delete from index_state where product = ?1"),
                ("minutes", "delete from minutes where product = ?1"),
                ("decisions", "delete from decisions where product = ?1"),
                ("stories", "delete from stories where product = ?1"),
                ("story_logs", "delete from story_logs where product = ?1"),
                ("memory", "delete from memory where product = ?1"),
                (
                    "agent_overlays",
                    "delete from agent_overlays where product = ?1",
                ),
                ("briefs", "delete from briefs where product = ?1"),
                ("meeting_log", "delete from meeting_log where product = ?1"),
                ("snapshots", "delete from snapshots where product = ?1"),
                ("listens", "delete from listens where product = ?1"),
                ("reflections", "delete from reflections where product = ?1"),
                (
                    "archive_records",
                    "delete from archive_records where product = ?1",
                ),
                ("tickets", "delete from tickets where product = ?1"),
                ("products", "delete from products where id = ?1"),
            ] {
                del(tx, t, table, sql, [product])?;
            }
            files.extend(sized(root, format!("workspace/{product}")));
        }
        Kind::Meetings => {
            let ids = minutes_scope(tx, product, item.before.as_deref())?;
            let json = ids_json(&ids);
            let decisions = strings(
                tx,
                "select cast(id as text) from decisions where product = ?1 and minutes_id in (select value from json_each(?2))",
                params![product, json],
            )?;
            let dec_json = ids_json(&decisions);
            let t = &mut tables;
            del(
                tx,
                t,
                "priorities",
                "delete from priorities where minutes_id in (select value from json_each(?1))",
                [&json],
            )?;
            del(
                tx,
                t,
                "usage",
                "delete from usage where minutes_id in (select value from json_each(?1))",
                [&json],
            )?;
            del(
                tx,
                t,
                "decisions",
                "delete from decisions where product = ?1 and minutes_id in (select value from json_each(?2))",
                params![product, json],
            )?;
            del(
                tx,
                t,
                "snapshots",
                "delete from snapshots where product = ?1 and minutes_id in (select value from json_each(?2))",
                params![product, json],
            )?;
            del(
                tx,
                t,
                "listens",
                "delete from listens where product = ?1 and minutes_id in (select value from json_each(?2))",
                params![product, json],
            )?;
            del(
                tx,
                t,
                "meeting_log",
                "delete from meeting_log where product = ?1 and minutes_id in (select value from json_each(?2))",
                params![product, json],
            )?;
            del(
                tx,
                t,
                "documents",
                "delete from documents where product = ?1 and kind = 'minutes' and ref in (select value from json_each(?2))",
                params![product, json],
            )?;
            del(
                tx,
                t,
                "archive_records",
                "delete from archive_records where product = ?1 and ((kind = 'minutes' and ref in (select value from json_each(?2))) or (kind = 'decision' and ref in (select value from json_each(?3))))",
                params![product, json, dec_json],
            )?;
            del(
                tx,
                t,
                "minutes",
                "delete from minutes where product = ?1 and id in (select value from json_each(?2))",
                params![product, json],
            )?;
            files.extend(audio_dirs(root, product, Some(&ids)));
        }
        Kind::Stories => {
            let ids = strings(
                tx,
                "select id from stories where product = ?1 and json_extract(data, '$.status') in ('implemented', 'discarded') order by id",
                [product],
            )?;
            let json = ids_json(&ids);
            let t = &mut tables;
            del(
                tx,
                t,
                "stories",
                "delete from stories where product = ?1 and id in (select value from json_each(?2))",
                params![product, json],
            )?;
            del(
                tx,
                t,
                "story_logs",
                "delete from story_logs where product = ?1 and story_id in (select value from json_each(?2))",
                params![product, json],
            )?;
            del(
                tx,
                t,
                "archive_records",
                "delete from archive_records where product = ?1 and kind = 'story' and ref in (select value from json_each(?2))",
                params![product, json],
            )?;
        }
        Kind::RoleLearning => {
            let t = &mut tables;
            for (table, sql) in [
                ("memory", "delete from memory where product = ?1"),
                (
                    "agent_overlays",
                    "delete from agent_overlays where product = ?1",
                ),
                ("briefs", "delete from briefs where product = ?1"),
                ("reflections", "delete from reflections where product = ?1"),
                (
                    "documents",
                    "delete from documents where product = ?1 and kind = 'memory'",
                ),
            ] {
                del(tx, t, table, sql, [product])?;
            }
        }
        Kind::CallAudio => {
            if item.product.is_some() {
                del(
                    tx,
                    &mut tables,
                    "listens",
                    "delete from listens where product = ?1",
                    [product],
                )?;
                files.extend(audio_dirs(root, product, None));
            } else {
                del(tx, &mut tables, "listens", "delete from listens", [])?;
                for p in names(&root.join("workspace"))
                    .into_iter()
                    .filter(|p| safe_segment(p))
                {
                    files.extend(audio_dirs(root, &p, None));
                }
            }
        }
        Kind::VoiceTranscripts => files.extend(transcripts(root, item.before.as_deref())),
        Kind::Logs => files.extend(log_files(root)),
        Kind::Backups => files.extend(old_backups(root)?),
        Kind::Everything => {
            for table in TABLES {
                del(tx, &mut tables, table, &format!("delete from {table}"), [])?;
            }
            for p in names(&root.join("workspace")) {
                files.extend(sized(root, format!("workspace/{p}")));
            }
            files.extend(transcripts(root, None));
            files.extend(log_files(root));
            files.extend(old_backups(root)?);
        }
    }
    Ok(ItemPlan {
        item: item.clone(),
        tables,
        files,
    })
}

fn within(a: &str, b: &str) -> bool {
    a == b || a.strip_prefix(b).is_some_and(|rest| rest.starts_with('/'))
}

/// Runs the items one after the other in one transaction, and commits it only when
/// `commit` is true. A file that an earlier item lists, or that lies in a folder that it
/// lists, is not listed again.
pub fn run(
    conn: &mut Connection,
    root: &Path,
    items: &[Item],
    commit: bool,
) -> Result<Vec<ItemPlan>> {
    for item in items {
        validate(item)?;
    }
    let tx = conn.transaction()?;
    let mut plans: Vec<ItemPlan> = Vec::new();
    for item in items {
        let mut plan = apply(&tx, root, item)?;
        plan.files.retain(|f| {
            !plans
                .iter()
                .flat_map(|p| &p.files)
                .any(|seen| within(&f.path, &seen.path))
        });
        plans.push(plan);
    }
    if commit {
        tx.commit()?;
    }
    Ok(plans)
}

/// Every product id the tables hold, for the `data.changed` events.
pub fn product_ids(conn: &Connection) -> Result<Vec<String>> {
    let mut ids = BTreeSet::new();
    for sql in [
        "select id from products",
        "select distinct product from minutes",
        "select distinct product from documents",
        "select distinct product from stories",
        "select distinct product from memory",
        "select distinct product from decisions",
        "select distinct product from tickets",
    ] {
        ids.extend(strings(conn, sql, [])?);
    }
    Ok(ids.into_iter().collect())
}

/// Removes the files of the plans. A path outside `ALLOWED`, or one with a part that is
/// not a plain name, is refused. A link goes, and what it points to stays. A path in
/// `keep` stays. A file that is gone already is not a failure.
pub fn remove_files(root: &Path, plans: &[ItemPlan], keep: &[PathBuf]) -> (u64, Vec<Failed>) {
    let mut removed = 0;
    let mut failed = Vec::new();
    let root_real = std::fs::canonicalize(root).ok();
    for entry in plans.iter().flat_map(|p| &p.files) {
        match remove_one(root, root_real.as_deref(), &entry.path, keep) {
            Ok(true) => removed += 1,
            Ok(false) => {}
            Err(error) => failed.push(Failed {
                path: entry.path.clone(),
                error,
            }),
        }
    }
    (removed, failed)
}

fn remove_one(
    root: &Path,
    root_real: Option<&Path>,
    rel: &str,
    keep: &[PathBuf],
) -> std::result::Result<bool, String> {
    let rel_path = Path::new(rel);
    let plain = rel
        .split('/')
        .all(|s| !s.is_empty() && s != "." && s != "..")
        && rel_path
            .components()
            .all(|c| matches!(c, Component::Normal(_)));
    if !plain
        || rel_path.components().count() < 2
        || !ALLOWED.iter().any(|a| within(rel, a) && rel != *a)
    {
        return Err("outside the folders that a delete may touch".into());
    }
    let path = root.join(rel_path);
    if keep.iter().any(|k| k == &path) {
        return Ok(false);
    }
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    // The parent must resolve inside the data root, so a linked folder cannot lead out.
    let parent_real = path
        .parent()
        .and_then(|p| std::fs::canonicalize(p).ok())
        .ok_or("the folder is gone")?;
    if !root_real.is_some_and(|r| parent_real.starts_with(r)) {
        return Err("the folder leads out of the data root".into());
    }
    let done = if meta.is_dir() {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_file(&path)
    };
    done.map(|()| true).map_err(|e| e.to_string())
}

/// Folds the WAL into the database file and frees the pages that the delete left.
/// It takes the writer for as long as it runs: call it off the request thread.
pub fn compact(conn: &Connection) -> Result<()> {
    conn.query_row("pragma wal_checkpoint(truncate)", [], |_| Ok(()))?;
    conn.execute_batch("vacuum")?;
    conn.query_row("pragma wal_checkpoint(truncate)", [], |_| Ok(()))?;
    Ok(())
}

/// The bytes of the data root, without following links.
pub fn root_bytes(root: &Path) -> u64 {
    tree_bytes(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    struct World {
        dir: tempfile::TempDir,
        conn: Connection,
    }

    impl World {
        fn root(&self) -> &Path {
            self.dir.path()
        }
    }

    fn seed_product(c: &Connection, p: &str) {
        let (m1, m2) = (format!("m1{p}"), format!("m2{p}"));
        c.execute_batch(&format!(
            "insert into products values ('{p}', '{p}', '[]', 'active');
             insert into documents (product, kind, ref, title, body) values
               ('{p}', 'minutes', '{m1}', 't', 'b'), ('{p}', 'minutes', '{m2}', 't', 'b'),
               ('{p}', 'memory', 'dev', 't', 'b'), ('{p}', 'doc', 'README', 't', 'b');
             insert into index_state values ('{p}', 'a.rs', 'abc', 'x');
             insert into minutes (id, product, created_at, data) values
               ('{m1}', '{p}', '2026-01-01T00:00:00.000Z', '{{}}'),
               ('{m2}', '{p}', '2026-10-01T00:00:00.000Z', '{{}}');
             insert into priorities (minutes_id, idx) values ('{m1}', 0), ('{m2}', 0);
             insert into decisions (product, minutes_id, text) values ('{p}', '{m1}', 'd1'), ('{p}', '{m2}', 'd2');
             insert into usage (minutes_id, role) values ('{m1}', 'pm'), ('{m2}', 'pm');
             insert into stories values
               ('{p}', 'S001', '{{\"status\":\"implemented\"}}'),
               ('{p}', 'S002', '{{\"status\":\"discarded\"}}'),
               ('{p}', 'S003', '{{\"status\":\"not_implemented\"}}');
             insert into story_logs values ('{p}', 'S001', 'l'), ('{p}', 'S003', 'l');
             insert into memory values ('{p}', 'dev', 'i1', 0, 't', 'fact', 0, 0, 's', 'x', null, null);
             insert into agent_overlays values ('{p}', 'dev', 'o');
             insert into briefs values ('{p}', '{{}}');
             insert into meeting_log values ('{p}', '{m1}', 0, '{{}}'), ('{p}', '{m2}', 1, '{{}}');
             insert into snapshots values ('{p}', '{m1}', 's'), ('{p}', '{m2}', 's');
             insert into listens values ('{p}', '{m1}', 'x'), ('{p}', '{m2}', 'x');
             insert into reflections values ('{p}', 'r1', 0, '{{}}');
             insert into archive_records values
               ('{p}', 'minutes', '{m1}'), ('{p}', 'minutes', '{m2}'), ('{p}', 'story', 'S001'),
               ('{p}', 'story', 'S003'), ('{p}', 'decision', (select id from decisions where product = '{p}' and minutes_id = '{m1}'));
             insert into tickets (product, id, kind, state, title, author, updated_by, created_at, updated_at)
               values ('{p}', 'C001', 'note', 'open', 't', '{{}}', '{{}}', 'x', 'x');"
        ))
        .unwrap();
    }

    fn put(root: &Path, rel: &str, bytes: usize) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    fn age(root: &Path, rel: &str, secs: u64) {
        let f = std::fs::File::options()
            .write(true)
            .open(root.join(rel))
            .unwrap();
        f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    use std::time::Duration;

    fn world() -> World {
        let dir = tempfile::tempdir().unwrap();
        let conn = crate::db::open_writer(&dir.path().join("agentsi.db")).unwrap();
        for p in ["a", "b"] {
            seed_product(&conn, p);
            for m in ["m1", "m2"] {
                put(
                    dir.path(),
                    &format!("workspace/{p}/audio/{m}{p}/call.opus"),
                    100,
                );
            }
            put(dir.path(), &format!("workspace/{p}/notes.txt"), 7);
        }
        put(dir.path(), "voice/transcripts/old.md", 10);
        put(dir.path(), "voice/transcripts/new.md", 10);
        age(dir.path(), "voice/transcripts/old.md", 1_000_000_000); // 2001
        age(dir.path(), "voice/transcripts/new.md", 1_900_000_000); // 2030
        put(dir.path(), "logs/hook.log", 50);
        put(dir.path(), "logs/setup.jsonl", 20);
        for d in ["2026-11-01", "2026-11-02", "2026-11-03"] {
            put(dir.path(), &format!("backups/agentsi-{d}.db"), 20);
            put(dir.path(), &format!("backups/agentsi.config-{d}.json"), 5);
        }
        put(dir.path(), "agentsi.config.json", 3);
        World { dir, conn }
    }

    /// The rows of one product, per table.
    fn counts(c: &Connection, p: &str) -> BTreeMap<&'static str, i64> {
        let by_minutes = "minutes_id in (select id from minutes where product = ?1)";
        TABLES
            .iter()
            .map(|t| {
                let pred = match *t {
                    "priorities" | "usage" => by_minutes,
                    "products" => "id = ?1",
                    _ => "product = ?1",
                };
                let n = c
                    .query_row(
                        &format!("select count(*) from {t} where {pred}"),
                        [p],
                        |r| r.get(0),
                    )
                    .unwrap();
                (*t, n)
            })
            .collect()
    }

    fn item(kind: Kind, product: Option<&str>, before: Option<&str>) -> Item {
        Item {
            kind,
            product: product.map(String::from),
            before: before.map(String::from),
        }
    }

    fn files_of(root: &Path, dir: &str) -> Vec<String> {
        names(&root.join(dir))
    }

    fn do_clear(w: &mut World, items: &[Item]) -> Vec<ItemPlan> {
        let plans = run(&mut w.conn, w.dir.path(), items, true).unwrap();
        let (_, failed) = remove_files(w.dir.path(), &plans, &[]);
        assert!(failed.is_empty(), "{failed:?}");
        plans
    }

    fn rows_of(plans: &[ItemPlan]) -> BTreeMap<&'static str, u64> {
        let mut out = BTreeMap::new();
        for (t, n) in plans.iter().flat_map(|p| &p.tables) {
            *out.entry(*t).or_insert(0) += n;
        }
        out
    }

    #[test]
    fn a_product_goes_with_its_rows_and_folder_and_the_other_keeps_everything() {
        let mut w = world();
        let b_before = counts(&w.conn, "b");
        assert!(b_before.values().all(|n| *n > 0), "{b_before:?}");
        do_clear(&mut w, &[item(Kind::Product, Some("a"), None)]);
        assert!(
            counts(&w.conn, "a").values().all(|n| *n == 0),
            "{:?}",
            counts(&w.conn, "a")
        );
        assert_eq!(counts(&w.conn, "b"), b_before);
        assert!(!w.root().join("workspace/a").exists());
        assert_eq!(files_of(w.root(), "workspace/b"), ["audio", "notes.txt"]);
        let fts: i64 = w
            .conn
            .query_row(
                "select count(*) from documents_fts where documents_fts match 't'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(fts, 4, "the search index keeps only product b");
    }

    #[test]
    fn meetings_go_with_what_hangs_on_them_and_a_date_keeps_the_newer_ones() {
        let mut w = world();
        let b_before = counts(&w.conn, "b");
        do_clear(
            &mut w,
            &[item(Kind::Meetings, Some("a"), Some("2026-06-01"))],
        );
        let a = counts(&w.conn, "a");
        for t in [
            "minutes",
            "priorities",
            "usage",
            "decisions",
            "snapshots",
            "listens",
            "meeting_log",
        ] {
            assert_eq!(a[t], 1, "{t}");
        }
        assert_eq!(
            a["archive_records"], 3,
            "the minutes and the decision record went"
        );
        assert_eq!(
            a["documents"], 3,
            "the minutes document went, memory and doc stayed"
        );
        assert_eq!(files_of(w.root(), "workspace/a/audio"), ["m2a"]);
        assert_eq!(counts(&w.conn, "b"), b_before);

        do_clear(&mut w, &[item(Kind::Meetings, Some("a"), None)]);
        assert_eq!(counts(&w.conn, "a")["minutes"], 0);
        assert!(files_of(w.root(), "workspace/a/audio").is_empty());
        assert_eq!(counts(&w.conn, "a")["stories"], 3, "stories stay");
        assert_eq!(counts(&w.conn, "b"), b_before);
    }

    #[test]
    fn stories_go_when_done_or_rejected_and_the_open_one_stays() {
        let mut w = world();
        let b_before = counts(&w.conn, "b");
        do_clear(&mut w, &[item(Kind::Stories, Some("a"), None)]);
        let a = counts(&w.conn, "a");
        assert_eq!((a["stories"], a["story_logs"]), (1, 1));
        assert_eq!(a["archive_records"], 4, "the record of S001 went");
        assert_eq!(counts(&w.conn, "b"), b_before);
    }

    #[test]
    fn role_learning_goes_and_minutes_stay() {
        let mut w = world();
        let b_before = counts(&w.conn, "b");
        do_clear(&mut w, &[item(Kind::RoleLearning, Some("a"), None)]);
        let a = counts(&w.conn, "a");
        for t in ["memory", "agent_overlays", "briefs", "reflections"] {
            assert_eq!(a[t], 0, "{t}");
        }
        assert_eq!((a["minutes"], a["documents"]), (2, 3));
        assert_eq!(counts(&w.conn, "b"), b_before);
    }

    #[test]
    fn call_audio_goes_for_one_product_or_for_all_and_the_minutes_stay() {
        let mut w = world();
        do_clear(&mut w, &[item(Kind::CallAudio, Some("a"), None)]);
        assert!(files_of(w.root(), "workspace/a/audio").is_empty());
        assert_eq!(files_of(w.root(), "workspace/b/audio"), ["m1b", "m2b"]);
        assert_eq!(counts(&w.conn, "a")["listens"], 0);
        assert_eq!(counts(&w.conn, "a")["minutes"], 2);
        assert_eq!(counts(&w.conn, "b")["listens"], 2);
        do_clear(&mut w, &[item(Kind::CallAudio, None, None)]);
        assert!(files_of(w.root(), "workspace/b/audio").is_empty());
        assert_eq!(counts(&w.conn, "b")["listens"], 0);
        assert!(w.root().join("workspace/b/notes.txt").exists());
    }

    #[test]
    fn transcripts_go_all_or_by_date_and_logs_go() {
        let mut w = world();
        do_clear(
            &mut w,
            &[item(Kind::VoiceTranscripts, None, Some("2000-01-01"))],
        );
        assert_eq!(
            files_of(w.root(), "voice/transcripts"),
            ["new.md", "old.md"]
        );
        do_clear(
            &mut w,
            &[item(Kind::VoiceTranscripts, None, Some("2026-01-01"))],
        );
        assert_eq!(files_of(w.root(), "voice/transcripts"), ["new.md"]);
        do_clear(
            &mut w,
            &[
                item(Kind::VoiceTranscripts, None, None),
                item(Kind::Logs, None, None),
            ],
        );
        assert!(files_of(w.root(), "voice/transcripts").is_empty());
        assert!(!w.root().join("logs/hook.log").exists());
        assert!(!w.root().join("logs/setup.jsonl").exists());
    }

    #[test]
    fn backups_go_except_the_newest_with_its_registry_copy() {
        let mut w = world();
        do_clear(&mut w, &[item(Kind::Backups, None, None)]);
        assert_eq!(
            files_of(w.root(), "backups"),
            ["agentsi-2026-11-03.db", "agentsi.config-2026-11-03.json"]
        );
    }

    #[test]
    fn everything_empties_the_tables_and_keeps_the_root_the_newest_backup_and_the_registry_file() {
        let mut w = world();
        do_clear(&mut w, &[item(Kind::Everything, None, None)]);
        for p in ["a", "b"] {
            assert!(counts(&w.conn, p).values().all(|n| *n == 0));
        }
        assert!(files_of(w.root(), "workspace").is_empty());
        assert!(files_of(w.root(), "voice/transcripts").is_empty());
        assert_eq!(files_of(w.root(), "backups").len(), 2);
        assert!(
            w.root().join("agentsi.db").exists() && w.root().join("agentsi.config.json").exists()
        );
    }

    #[test]
    fn the_plan_counts_equal_the_deleted_rows_also_when_two_items_share_rows() {
        let mut w = world();
        let items = [
            item(Kind::Meetings, Some("a"), None),
            item(Kind::Product, Some("a"), None),
            item(Kind::CallAudio, Some("a"), None),
        ];
        let total = |c: &Connection| -> i64 {
            TABLES
                .iter()
                .map(|t| {
                    c.query_row(&format!("select count(*) from {t}"), [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .unwrap()
                })
                .sum()
        };
        let before = total(&w.conn);
        let planned = run(&mut w.conn, w.dir.path(), &items, false).unwrap();
        assert_eq!(total(&w.conn), before, "a plan writes nothing");
        assert!(
            w.root().join("workspace/a/audio/m1a").exists(),
            "a plan removes no file"
        );
        let again = run(&mut w.conn, w.dir.path(), &items, false).unwrap();
        assert_eq!(planned, again);
        let done = run(&mut w.conn, w.dir.path(), &items, true).unwrap();
        assert_eq!(done, planned);
        assert_eq!(
            before - total(&w.conn),
            planned.iter().map(|p| p.rows() as i64).sum::<i64>()
        );
        assert_eq!(rows_of(&done)["minutes"], 2);
        // The audio folders are listed once, in the first item that holds them.
        let listed: Vec<&str> = planned
            .iter()
            .flat_map(|p| &p.files)
            .map(|f| f.path.as_str())
            .collect();
        assert_eq!(
            listed,
            [
                "workspace/a/audio/m1a",
                "workspace/a/audio/m2a",
                "workspace/a"
            ]
        );
    }

    #[test]
    fn a_bad_item_is_refused() {
        let w = world();
        for bad in [
            item(Kind::Product, None, None),
            item(Kind::Product, Some("../b"), None),
            item(Kind::Meetings, Some("a"), Some("yesterday")),
            item(Kind::Logs, Some("a"), None),
            item(Kind::Stories, Some("a"), Some("2026-01-01")),
        ] {
            assert!(validate(&bad).is_err(), "{bad:?}");
        }
        assert!(validate(&item(Kind::CallAudio, None, None)).is_ok());
        assert_eq!(parse_day("1970-01-02"), Some(86_400));
        assert!(w.root().exists());
    }

    #[test]
    fn a_delete_never_leaves_the_data_root() {
        let w = world();
        let outside = tempfile::tempdir().unwrap();
        put(outside.path(), "keep.txt", 5);
        std::os::unix::fs::symlink(outside.path(), w.root().join("workspace/link")).unwrap();
        std::os::unix::fs::symlink(outside.path(), w.root().join("workspace/a/audio/m1a/out"))
            .unwrap();
        let entry = |p: &str| ItemPlan {
            item: item(Kind::Logs, None, None),
            tables: vec![],
            files: vec![FileEntry {
                path: p.into(),
                bytes: 0,
                dir: false,
            }],
        };
        let refused = [
            "../x",
            "/etc/passwd",
            "workspace/../agentsi.db",
            "workspace",
            "agentsi.db",
            "agentsi.config.json",
            "voice/sessions/x",
            "workspace//a",
        ];
        for p in refused {
            let (n, failed) = remove_files(w.root(), &[entry(p)], &[]);
            assert_eq!((n, failed.len()), (0, 1), "{p}");
        }
        assert!(w.root().join("agentsi.db").exists());
        // A link is removed and its target stays.
        let (n, failed) = remove_files(w.root(), &[entry("workspace/link")], &[]);
        assert_eq!((n, failed.len()), (1, 0));
        assert!(outside.path().join("keep.txt").exists());
        // A folder with a link inside goes, and the target of the link stays.
        let (n, _) = remove_files(w.root(), &[entry("workspace/a")], &[]);
        assert_eq!(n, 1);
        assert!(outside.path().join("keep.txt").exists());
        // A folder behind a linked parent is refused.
        std::os::unix::fs::symlink(
            outside.path(),
            w.root().join("workspace/b/audio/m1b/through"),
        )
        .unwrap();
        let (_, failed) = remove_files(
            w.root(),
            &[entry("workspace/b/audio/m1b/through/keep.txt")],
            &[],
        );
        assert_eq!(failed.len(), 1);
        assert!(outside.path().join("keep.txt").exists());
    }

    #[test]
    fn compaction_empties_the_wal() {
        let w = world();
        compact(&w.conn).unwrap();
        let wal = w.root().join("agentsi.db-wal");
        assert!(std::fs::metadata(wal).map_or(0, |m| m.len()) == 0);
    }
}
