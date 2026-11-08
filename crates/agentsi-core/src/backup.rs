//! `agentsi-core backup [--keep <n>] [--data-root <dir>]`: the backup of
//! the data spec, until the `data.backup` method of M2. Also the daily backup
//! of a running core: the first one when it starts, then one at 03:00 local time.

use crate::state::Core;
use agentsi_store::backup::{BackupResult, KEEP_BACKUPS, backup, backup_due};
use agentsi_store::paths;
use agentsi_store::{Result as StoreResult, Store};
use rusqlite::Connection;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// The local hour of the nightly backup.
const NIGHT_HOUR: i32 = 3;
/// The longest sleep. The loop reads the clock again after each one, because the
/// monotonic clock of the sleep stops while the machine sleeps.
const TICK: Duration = Duration::from_secs(15 * 60);

/// Writes today's backup when `root` has none for the UTC date of `now`, on the
/// writer connection of `store`. Returns `None` when a backup of today exists.
pub fn backup_if_due(
    store: &Store,
    root: &Path,
    now: SystemTime,
) -> StoreResult<Option<BackupResult>> {
    if !backup_due(root, now)? {
        return Ok(None);
    }
    let root = root.to_path_buf();
    store
        .with_writer(move |conn| backup(conn, &root, KEEP_BACKUPS, now))
        .map(Some)
}

/// The next `hour`:00 in the local time zone after `now`.
fn next_local_hour(now: SystemTime, hour: i32) -> SystemTime {
    let secs = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as libc::time_t);
    // SAFETY: `tm` is plain data. `localtime_r` and `mktime` only read and write it.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        let at = |tm: &mut libc::tm| {
            tm.tm_hour = hour;
            tm.tm_min = 0;
            tm.tm_sec = 0;
            tm.tm_isdst = -1;
            libc::mktime(tm)
        };
        let mut target = at(&mut tm);
        if target <= secs {
            tm.tm_mday += 1;
            target = at(&mut tm);
        }
        SystemTime::UNIX_EPOCH + Duration::from_secs(target.max(secs + 1) as u64)
    }
}

fn once(core: &Arc<Core>) -> Result<Option<BackupResult>, String> {
    let store = core
        .store()
        .map_err(|e| e.to_string())?
        .ok_or("no agentsi.db yet")?;
    let root = paths::data_root(None).map_err(|e| e.to_string())?;
    backup_if_due(&store, &root, SystemTime::now()).map_err(|e| e.to_string())
}

/// Backs up when the core starts, then at 03:00 local time each day. A day that has
/// a backup is skipped. It writes one line to the log of the core for each check.
pub async fn daily(core: Arc<Core>) {
    loop {
        let c = core.clone();
        match crate::blocking::spawn(move || once(&c)).await {
            Ok(Ok(Some(done))) => eprintln!(
                "core: backup written to {} ({} old removed)",
                done.file.display(),
                done.removed.len()
            ),
            Ok(Ok(None)) => eprintln!("core: the backup of today exists"),
            Ok(Err(e)) => eprintln!("core: no backup: {e}"),
            Err(e) => eprintln!("core: the backup task failed: {e}"),
        }
        let wake = next_local_hour(SystemTime::now(), NIGHT_HOUR);
        while let Ok(left) = wake.duration_since(SystemTime::now()) {
            tokio::time::sleep(left.min(TICK)).await;
        }
    }
}

fn parse<'a>(args: &[&'a str]) -> Option<(usize, Option<&'a Path>)> {
    let mut keep = KEEP_BACKUPS;
    let mut root = None;
    let mut rest = args;
    while let [flag, value, tail @ ..] = rest {
        match *flag {
            "--keep" => keep = value.parse().ok().filter(|k| *k >= 1)?,
            "--data-root" => root = Some(Path::new(*value)),
            _ => return None,
        }
        rest = tail;
    }
    rest.is_empty().then_some((keep, root))
}

fn run_backup(keep: usize, root: Option<&Path>) -> Result<(), String> {
    let root = paths::data_root(root).map_err(|e| e.to_string())?;
    let db = paths::db_path(&root);
    if !db.exists() {
        return Err(format!("{} does not exist", db.display()));
    }
    // The online backup API reads a consistent copy while a core writes.
    let conn = Connection::open(&db).map_err(|e| e.to_string())?;
    agentsi_store::db::configure(&conn).map_err(|e| e.to_string())?;
    let result =
        backup(&conn, &root, keep, std::time::SystemTime::now()).map_err(|e| e.to_string())?;
    println!("backup: {}", result.file.display());
    if let Some(registry) = result.registry {
        println!("registry: {}", registry.display());
    }
    for f in result.removed {
        println!("removed: {}", f.display());
    }
    Ok(())
}

pub fn run(args: &[&str]) -> ExitCode {
    let Some((keep, root)) = parse(args) else {
        eprintln!("usage: agentsi-core backup [--keep <n>] [--data-root <dir>]");
        return ExitCode::from(2);
    };
    match run_backup(keep, root) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("agentsi-core backup: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("agentsi.db")).unwrap();
        (dir, store)
    }

    #[test]
    fn the_first_check_of_a_day_writes_a_backup_and_the_second_writes_none() {
        let (dir, store) = store();
        let day = UNIX_EPOCH + Duration::from_secs(1_793_700_000);
        let first = backup_if_due(&store, dir.path(), day).unwrap().unwrap();
        assert!(first.file.exists());
        assert!(
            backup_if_due(&store, dir.path(), day + Duration::from_secs(3600))
                .unwrap()
                .is_none()
        );
        // The next UTC day is due again.
        let next = day + Duration::from_secs(86_400);
        assert!(backup_if_due(&store, dir.path(), next).unwrap().is_some());
    }

    #[test]
    fn the_core_keeps_14_backups() {
        let (dir, store) = store();
        let start = UNIX_EPOCH + Duration::from_secs(1_793_700_000);
        for n in 0..16 {
            backup_if_due(&store, dir.path(), start + Duration::from_secs(86_400 * n)).unwrap();
        }
        let dates = agentsi_store::backup::backup_dates(dir.path()).unwrap();
        assert_eq!(dates.len(), 14);
        assert_eq!(dates[0], "2026-11-05");
    }

    #[test]
    fn the_night_is_the_next_03_00_of_the_local_time_zone() {
        let now = SystemTime::now();
        let wake = next_local_hour(now, NIGHT_HOUR);
        let ahead = wake.duration_since(now).unwrap();
        // A day and a daylight-saving hour at most.
        assert!(ahead > Duration::ZERO && ahead <= Duration::from_secs(25 * 3600));
        let secs = wake.duration_since(UNIX_EPOCH).unwrap().as_secs() as libc::time_t;
        // SAFETY: plain data in and out.
        let tm = unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            libc::localtime_r(&secs, &mut tm);
            tm
        };
        assert_eq!((tm.tm_hour, tm.tm_min, tm.tm_sec), (3, 0, 0));
    }
}
