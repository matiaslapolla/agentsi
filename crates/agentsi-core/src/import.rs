//! `agentsi-core import-postgres <url>`: copies the Postgres data into the
//! database file, with the rules of `agentsi_store::import`.
//!
//! The import reads Postgres in one read-only transaction. Thus it cannot
//! change Postgres. The counts and the rows come from one snapshot. The
//! import holds the lock of the core socket, so no core opens the database
//! file while it runs.

use agentsi_store::import::{Source, import_file, import_fresh};
use agentsi_store::schema::{Kind, Table};
use agentsi_store::{Error, paths};
use postgres::{IsolationLevel, NoTls};
use rusqlite::types::Value;
use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::process::ExitCode;

/// The select expression that gives the column `name` in its SQLite form.
fn expr(name: &str, kind: Kind) -> String {
    let c = format!("\"{name}\"");
    match kind {
        Kind::Text | Kind::Bool => c,
        Kind::Json => format!("{c}::text"),
        Kind::Int => format!("{c}::int8"),
        Kind::Real => format!("{c}::float8"),
        // Milliseconds, as JavaScript wrote them. A value with microseconds
        // keeps them, so no import loses time.
        Kind::Time => format!(
            "case when extract(microseconds from {c})::int8 % 1000 = 0 \
             then to_char({c} at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') \
             else to_char({c} at time zone 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') end"
        ),
    }
}

struct PgSource<'a> {
    tx: postgres::Transaction<'a>,
}

fn pg(e: postgres::Error) -> Error {
    Error::Other(format!("postgres: {e}"))
}

impl Source for PgSource<'_> {
    fn rows(&mut self, table: &'static Table) -> agentsi_store::Result<Vec<Vec<Value>>> {
        let cols: Vec<String> = table.columns.iter().map(|c| expr(c.name, c.kind)).collect();
        let order: Vec<String> = match table.key {
            [] => table
                .columns
                .iter()
                .map(|c| format!("\"{}\"", c.name))
                .collect(),
            key => key.iter().map(|k| format!("\"{k}\"")).collect(),
        };
        let sql = format!(
            "select {} from \"{}\" order by {}",
            cols.join(", "),
            table.name,
            order.join(", ")
        );
        let rows = self.tx.query(sql.as_str(), &[]).map_err(pg)?;
        rows.iter()
            .map(|row| {
                table
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        Ok(match c.kind {
                            Kind::Text | Kind::Json | Kind::Time => row
                                .try_get::<_, Option<String>>(i)
                                .map_err(pg)?
                                .map_or(Value::Null, Value::Text),
                            Kind::Bool => row
                                .try_get::<_, Option<bool>>(i)
                                .map_err(pg)?
                                .map_or(Value::Null, |b| Value::Integer(i64::from(b))),
                            Kind::Int => row
                                .try_get::<_, Option<i64>>(i)
                                .map_err(pg)?
                                .map_or(Value::Null, Value::Integer),
                            Kind::Real => row
                                .try_get::<_, Option<f64>>(i)
                                .map_err(pg)?
                                .map_or(Value::Null, Value::Real),
                        })
                    })
                    .collect()
            })
            .collect()
    }

    fn count(&mut self, table: &'static Table) -> agentsi_store::Result<i64> {
        let row = self
            .tx
            .query_one(
                format!("select count(*) from \"{}\"", table.name).as_str(),
                &[],
            )
            .map_err(pg)?;
        row.try_get(0).map_err(pg)
    }
}

/// Holds the lock that `agentsi-core serve` takes, or says why it cannot.
fn lock_core() -> Result<std::fs::File, String> {
    let socket = agentsi_proto::socket::socket_path()?;
    crate::server::prepare_folder(&socket).map_err(|e| e.to_string())?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(socket.lock())
        .map_err(|e| e.to_string())?;
    match lock.try_lock() {
        Ok(()) => Ok(lock),
        Err(std::fs::TryLockError::WouldBlock) => Err(format!(
            "a core runs on {}. Stop it first: only one process may open the database file",
            socket.path.display()
        )),
        Err(std::fs::TryLockError::Error(e)) => Err(e.to_string()),
    }
}

fn import(url: &str, root: &Path, fresh: bool) -> Result<bool, String> {
    std::fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let db = paths::db_path(root);
    let _lock = lock_core()?;
    if fresh && db.exists() {
        return Err(format!(
            "{} exists. --fresh writes only a new database file",
            db.display()
        ));
    }
    let mut client = postgres::Client::connect(url, NoTls).map_err(|e| format!("postgres: {e}"))?;
    let tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .map_err(|e| format!("postgres: {e}"))?;
    let mut source = PgSource { tx };
    let run = if fresh { import_fresh } else { import_file };
    let report =
        run(&db, &mut source, &mut |line| println!("{line}")).map_err(|e| e.to_string())?;
    for table in &report.tables {
        println!("{}", table.line());
    }
    match (report.ok(), report.inserted()) {
        (false, _) => eprintln!(
            "agentsi-core import-postgres: a table has fewer rows than Postgres. {} stays as it was",
            db.display()
        ),
        (true, 0) => println!("import: 0 rows inserted, {} stays as it was", db.display()),
        (true, n) => println!("import: {n} rows inserted into {}", db.display()),
    }
    source.tx.rollback().map_err(|e| format!("postgres: {e}"))?;
    Ok(report.ok())
}

struct Args<'a> {
    url: &'a str,
    root: Option<&'a Path>,
    fresh: bool,
}

fn parse<'a>(args: &[&'a str]) -> Option<Args<'a>> {
    let mut url = None;
    let mut root = None;
    let mut fresh = false;
    let mut rest = args;
    while let [first, tail @ ..] = rest {
        rest = tail;
        match *first {
            "--fresh" => fresh = true,
            "--data-root" => {
                let [dir, tail @ ..] = rest else { return None };
                root = Some(Path::new(*dir));
                rest = tail;
            }
            flag if flag.starts_with("--") => return None,
            value if url.is_none() => url = Some(value),
            _ => return None,
        }
    }
    Some(Args {
        url: url?,
        root,
        fresh,
    })
}

/// `import-postgres <url> [--fresh] [--data-root <dir>]`.
pub fn run(args: &[&str]) -> ExitCode {
    let Some(args) = parse(args) else {
        eprintln!("usage: agentsi-core import-postgres <url> [--fresh] [--data-root <dir>]");
        return ExitCode::from(2);
    };
    let result = paths::data_root(args.root)
        .map_err(|e| e.to_string())
        .and_then(|root| import(args.url, &root, args.fresh));
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("agentsi-core import-postgres: {e}");
            ExitCode::FAILURE
        }
    }
}
