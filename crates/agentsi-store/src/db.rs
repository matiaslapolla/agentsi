//! The database file: connection settings, migrations, and the store with
//! one writer thread and a pool of readers.

use crate::error::{Error, Result};
use rusqlite::{Connection, OpenFlags, Transaction};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc;
use std::thread::JoinHandle;

/// Each migration, in order. The number of the last applied one is in
/// `PRAGMA user_version`.
pub const MIGRATIONS: &[(u32, &str)] = &[(1, include_str!("../sql/0001_init.sql"))];

pub const DB_FILE: &str = "agentsi.db";

/// The settings of the data spec for each connection.
pub fn configure(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    Ok(())
}

/// Opens a connection that can write, in WAL mode, with the migrations applied.
pub fn open_writer(path: &Path) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    configure(&conn)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    migrate(&mut conn)?;
    Ok(conn)
}

/// Applies the missing migrations, each one in a transaction.
pub fn migrate(conn: &mut Connection) -> Result<u32> {
    let known = MIGRATIONS.last().map(|m| m.0).unwrap_or(0);
    let current: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if current > known {
        return Err(Error::TooNew {
            found: current,
            known,
        });
    }
    for (number, sql) in MIGRATIONS.iter().filter(|m| m.0 > current) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", number)?;
        tx.commit()?;
    }
    Ok(known)
}

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// The database file of one host. Only the core opens it. Each write runs on
/// the writer thread, in one transaction. Reads use a pool of connections.
pub struct Store {
    path: PathBuf,
    jobs: Option<mpsc::Sender<Job>>,
    writer: Option<JoinHandle<()>>,
    readers: Mutex<Vec<Connection>>,
}

/// Connections kept in the pool. A read that finds the pool empty opens one more.
const POOL: usize = 4;

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let mut conn = open_writer(path)?;
        let (jobs, inbox) = mpsc::channel::<Job>();
        let writer = std::thread::Builder::new()
            .name("agentsi-store-writer".into())
            .spawn(move || {
                while let Ok(job) = inbox.recv() {
                    job(&mut conn);
                }
            })?;
        Ok(Store {
            path: path.to_path_buf(),
            jobs: Some(jobs),
            writer: Some(writer),
            readers: Mutex::new(Vec::new()),
        })
    }

    /// Runs `f` on a reader connection, which cannot write.
    pub fn read<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let pooled = self.readers.lock().map_err(|_| Error::Closed)?.pop();
        let conn = match pooled {
            Some(c) => c,
            None => {
                let c = Connection::open_with_flags(
                    &self.path,
                    OpenFlags::SQLITE_OPEN_READ_ONLY
                        | OpenFlags::SQLITE_OPEN_NO_MUTEX
                        | OpenFlags::SQLITE_OPEN_URI,
                )?;
                configure(&c)?;
                c
            }
        };
        let result = f(&conn);
        if let Ok(mut pool) = self.readers.lock()
            && pool.len() < POOL
        {
            pool.push(conn);
        }
        result
    }

    /// Runs `f` in one transaction on the writer thread. An error rolls it back.
    pub fn write<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Transaction) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        self.with_writer(move |conn| {
            let tx = conn.transaction()?;
            let value = f(&tx)?;
            tx.commit()?;
            Ok(value)
        })
    }

    /// Like `write`, for a function with its own error type. An `Err` of `f`
    /// rolls the transaction back, so a failed write leaves no row, and comes
    /// back as the inner `Err`. The outer `Err` is a failure of the store.
    pub fn try_write<T: Send + 'static, E: Send + 'static>(
        &self,
        f: impl FnOnce(&Transaction) -> std::result::Result<T, E> + Send + 'static,
    ) -> Result<std::result::Result<T, E>> {
        self.with_writer(move |conn| {
            let tx = conn.transaction()?;
            match f(&tx) {
                Ok(value) => {
                    tx.commit()?;
                    Ok(Ok(value))
                }
                Err(e) => {
                    tx.rollback()?;
                    Ok(Err(e))
                }
            }
        })
    }

    /// Runs `f` with the writer connection itself, outside a transaction.
    /// The online backup uses it.
    pub fn with_writer<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (reply, answer) = mpsc::channel();
        let job: Job = Box::new(move |conn| {
            let _ = reply.send(f(conn));
        });
        self.jobs
            .as_ref()
            .ok_or(Error::Closed)?
            .send(job)
            .map_err(|_| Error::Closed)?;
        answer.recv().map_err(|_| Error::Closed)?
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        self.readers.get_mut().map(|p| p.clear()).ok();
        drop(self.jobs.take());
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
    }
}
