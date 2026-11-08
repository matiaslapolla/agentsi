//! The rules of the Postgres import, the port of
//! `packages/core/src/data-import.ts`. The source of the rows is a trait, so
//! the rules run in tests with no Postgres. `agentsi-core import-postgres`
//! gives the Postgres source.
//!
//! 1. Each row that the database lacks, by primary key, is inserted. A row
//!    that exists is newer and is never replaced.
//! 2. A minutes id that another product holds becomes `<id>-<product>`, and
//!    the rows of that product that name the minutes take the new id.
//! 3. Each table must have at least as many rows as the source.

use crate::db;
use crate::error::{Error, Result};
use crate::schema::{TABLES, Table};
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, Transaction};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Where the rows come from. Each value is already in its SQLite form (the
/// type map of the data spec), in the order of `table.columns`.
pub trait Source {
    fn rows(&mut self, table: &'static Table) -> Result<Vec<Vec<Value>>>;
    fn count(&mut self, table: &'static Table) -> Result<i64>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableReport {
    pub table: &'static str,
    pub source: i64,
    pub sqlite: i64,
    pub inserted: u64,
}

impl TableReport {
    pub fn ok(&self) -> bool {
        self.sqlite >= self.source
    }

    /// `<table> pg=<n> sqlite=<n> ok|FAIL`, the line of the data spec.
    pub fn line(&self) -> String {
        format!(
            "{} pg={} sqlite={} {}",
            self.table,
            self.source,
            self.sqlite,
            if self.ok() { "ok" } else { "FAIL" }
        )
    }
}

/// A minutes id that the import stored under a new id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rename {
    pub product: String,
    pub from: String,
    pub to: String,
    /// The product that already held `from`.
    pub holder: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub tables: Vec<TableReport>,
    pub renames: Vec<Rename>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.tables.iter().all(TableReport::ok)
    }

    pub fn inserted(&self) -> u64 {
        self.tables.iter().map(|t| t.inserted).sum()
    }
}

fn text(v: &Value) -> Option<&str> {
    match v {
        Value::Text(s) => Some(s),
        _ => None,
    }
}

/// Sets `field` of a JSON object in text form, and keeps each other field.
fn set_json_field(v: &mut Value, field: &str, to: &str) -> Result<()> {
    let Value::Text(s) = v else { return Ok(()) };
    let mut json: serde_json::Value = serde_json::from_str(s)
        .map_err(|e| Error::Other(format!("source JSON does not parse: {e}")))?;
    if let Some(obj) = json.as_object_mut()
        && obj.contains_key(field)
    {
        obj.insert(field.to_string(), serde_json::Value::String(to.to_string()));
        *s = json.to_string();
    }
    Ok(())
}

/// The renamed minutes, by old id. A Postgres minutes id is unique, so one
/// old id names one product.
struct Renames(HashMap<String, Rename>);

impl Renames {
    fn get(&self, row: &[Value], table: &Table, col: &str) -> Option<&Rename> {
        let id = text(&row[table.column(col)?])?;
        self.0.get(id)
    }

    fn of_product(&self, row: &[Value], table: &Table, col: &str) -> Option<&Rename> {
        let r = self.get(row, table, col)?;
        (text(&row[table.column("product")?]) == Some(r.product.as_str())).then_some(r)
    }

    /// Gives the new minutes id to each column of `row` that names a renamed minutes.
    fn apply(&self, table: &Table, row: &mut [Value]) -> Result<()> {
        if self.0.is_empty() {
            return Ok(());
        }
        let set = |row: &mut [Value], col: &str, to: &str| {
            if let Some(i) = table.column(col) {
                row[i] = Value::Text(to.to_string());
            }
        };
        match table.name {
            "minutes" => {
                if let Some(to) = self.of_product(row, table, "id").map(|r| r.to.clone()) {
                    set(row, "id", &to);
                    if let Some(i) = table.column("data") {
                        set_json_field(&mut row[i], "id", &to)?;
                    }
                }
            }
            "priorities" | "usage" => {
                if let Some(to) = self.get(row, table, "minutes_id").map(|r| r.to.clone()) {
                    set(row, "minutes_id", &to);
                }
            }
            "decisions" | "snapshots" | "listens" | "meeting_log" => {
                if let Some(to) = self
                    .of_product(row, table, "minutes_id")
                    .map(|r| r.to.clone())
                {
                    set(row, "minutes_id", &to);
                    if let Some(i) = table.column("entry") {
                        set_json_field(&mut row[i], "minutes_id", &to)?;
                    }
                }
            }
            // The indexer gives each minutes document the minutes id as its ref.
            "archive_records" | "documents" => {
                let minutes = table
                    .column("kind")
                    .map(|i| text(&row[i]) == Some("minutes"))
                    .unwrap_or(false);
                if minutes
                    && let Some(to) = self.of_product(row, table, "ref").map(|r| r.to.clone())
                {
                    set(row, "ref", &to);
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn count(tx: &Connection, table: &Table) -> Result<i64> {
    Ok(tx.query_row(
        &format!("select count(*) from \"{}\"", table.name),
        [],
        |r| r.get(0),
    )?)
}

/// Finds the minutes ids that another product already holds in the database.
fn find_renames(
    tx: &Transaction,
    minutes: &[Vec<Value>],
    log: &mut dyn FnMut(&str),
) -> Result<Renames> {
    let mut map = HashMap::new();
    let table = crate::schema::table("minutes").expect("the minutes table");
    let (id_col, product_col) = (table.column("id"), table.column("product"));
    let mut holder = tx.prepare("select product from minutes where id = ?1")?;
    for row in minutes {
        let id = id_col.and_then(|i| text(&row[i]));
        let product = product_col.and_then(|i| text(&row[i]));
        let (Some(id), Some(product)) = (id, product) else {
            continue;
        };
        let held: Option<String> = holder.query_row([id], |r| r.get(0)).optional()?;
        if let Some(held) = held.filter(|h| h != product) {
            let rename = Rename {
                product: product.into(),
                from: id.into(),
                to: format!("{id}-{product}"),
                holder: held,
            };
            log(&format!(
                "minutes {} of {}: {} holds this id, so the import uses {}",
                rename.from, rename.product, rename.holder, rename.to
            ));
            map.insert(id.to_string(), rename);
        }
    }
    Ok(Renames(map))
}

/// Inserts each source row that `tx` lacks, and counts each table on both sides.
pub fn import(
    tx: &Transaction,
    source: &mut dyn Source,
    log: &mut dyn FnMut(&str),
) -> Result<Report> {
    let mut report = Report::default();
    let mut renames = Renames(HashMap::new());
    // Usage has no key: a minutes that has usage rows already keeps them, and
    // gets none from the source.
    let has_usage: HashSet<String> = {
        let mut stmt = tx.prepare("select distinct minutes_id from usage")?;
        stmt.query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    for table in TABLES {
        let mut rows = source.rows(table)?;
        if table.name == "minutes" {
            renames = find_renames(tx, &rows, log)?;
            report.renames = renames.0.values().cloned().collect();
            report.renames.sort_by(|a, b| a.to.cmp(&b.to));
        }
        let cols: Vec<String> = table
            .columns
            .iter()
            .map(|c| format!("\"{}\"", c.name))
            .collect();
        let marks: Vec<String> = (1..=cols.len()).map(|i| format!("?{i}")).collect();
        let mut insert = tx.prepare(&format!(
            "insert into \"{}\" ({}) values ({}) on conflict do nothing",
            table.name,
            cols.join(", "),
            marks.join(", ")
        ))?;
        let mut inserted = 0u64;
        for row in rows.iter_mut() {
            if row.len() != table.columns.len() {
                return Err(Error::Other(format!(
                    "{}: a source row has {} values",
                    table.name,
                    row.len()
                )));
            }
            renames.apply(table, row)?;
            let minutes_id = table.column("minutes_id").and_then(|i| text(&row[i]));
            if table.name == "usage" && minutes_id.is_some_and(|id| has_usage.contains(id)) {
                continue;
            }
            inserted += insert.execute(rusqlite::params_from_iter(row.iter()))? as u64;
        }
        report.tables.push(TableReport {
            table: table.name,
            source: source.count(table)?,
            sqlite: count(tx, table)?,
            inserted,
        });
    }
    Ok(report)
}

/// The file that the import writes before it moves it into place.
pub fn work_file(db_path: &Path) -> PathBuf {
    side_file(db_path, ".import")
}

fn side_file(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Imports into a copy of `db_path`. Only when each table is ok and a row
/// was inserted does the copy take the place of `db_path`. The caller must
/// make sure that no other process has `db_path` open.
pub fn import_file(
    db_path: &Path,
    source: &mut dyn Source,
    log: &mut dyn FnMut(&str),
) -> Result<Report> {
    run(db_path, source, log, false)
}

/// Imports into a new empty file, the path of the switch-over. If `db_path`
/// exists, it stops with an error and changes nothing. Only when each table
/// is ok does the new file become `db_path`.
pub fn import_fresh(
    db_path: &Path,
    source: &mut dyn Source,
    log: &mut dyn FnMut(&str),
) -> Result<Report> {
    if db_path.exists() {
        return Err(exists(db_path));
    }
    run(db_path, source, log, true)
}

fn exists(db_path: &Path) -> Error {
    Error::Other(format!(
        "{} exists. --fresh writes only a new database file",
        db_path.display()
    ))
}

fn run(
    db_path: &Path,
    source: &mut dyn Source,
    log: &mut dyn FnMut(&str),
    fresh: bool,
) -> Result<Report> {
    let work = work_file(db_path);
    for f in [work.clone(), side_file(&work, "-journal")] {
        match std::fs::remove_file(&f) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
    }
    let existed = !fresh && db_path.exists();
    if existed {
        let current = Connection::open(db_path)?;
        db::configure(&current)?;
        current.backup(rusqlite::MAIN_DB, &work, None)?;
        current.close().map_err(|(_, e)| Error::from(e))?;
    }
    let result = (|| {
        let mut conn = Connection::open(&work)?;
        db::configure(&conn)?;
        conn.pragma_update(None, "journal_mode", "DELETE")?;
        db::migrate(&mut conn)?;
        let tx = conn.transaction()?;
        let report = import(&tx, source, log)?;
        if report.ok() {
            tx.commit()?;
        } else {
            tx.rollback()?;
        }
        conn.close().map_err(|(_, e)| Error::from(e))?;
        Ok(report)
    })();
    let report = match result {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_file(&work);
            return Err(e);
        }
    };
    if !report.ok() || (existed && report.inserted() == 0) {
        std::fs::remove_file(&work)?;
        return Ok(report);
    }
    // A WAL file left next to the target would be applied to the new file.
    for side in ["-wal", "-shm"] {
        let f = side_file(db_path, side);
        if f.exists() {
            std::fs::remove_file(&work)?;
            return Err(Error::Other(format!(
                "{} exists: another process has the database open",
                f.display()
            )));
        }
    }
    std::fs::File::open(&work)?.sync_all()?;
    if fresh {
        // A link fails when `db_path` exists, so a fresh import never takes
        // the place of a file.
        let linked = std::fs::hard_link(&work, db_path);
        std::fs::remove_file(&work)?;
        linked.map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => exists(db_path),
            _ => Error::from(e),
        })?;
    } else {
        std::fs::rename(&work, db_path)?;
    }
    if let Some(dir) = db_path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::File::open(dir)?.sync_all()?;
    }
    Ok(report)
}
