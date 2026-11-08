//! Backups with the SQLite online backup API: `<root>/backups/agentsi-<UTC date>.db`
//! and a copy of the registry, `agentsi.config-<UTC date>.json`.

use crate::error::{Error, Result};
use rusqlite::Connection;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The number of backups to keep, as `KEEP_BACKUPS` in `packages/core/src/backup.ts`.
pub const KEEP_BACKUPS: usize = 14;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupResult {
    pub file: PathBuf,
    pub registry: Option<PathBuf>,
    pub removed: Vec<PathBuf>,
}

pub fn backup_dir(root: &Path) -> PathBuf {
    root.join("backups")
}

/// `YYYY-MM-DD` of `at`, in UTC.
pub fn utc_date(at: SystemTime) -> String {
    let secs = at
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // The civil-from-days algorithm of Howard Hinnant.
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn dated(name: &str) -> Option<&str> {
    let date = name.strip_prefix("agentsi-")?.strip_suffix(".db")?;
    let ok = date.len() == 10
        && date.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    ok.then_some(date)
}

/// The dates of the backups in `root`, oldest first.
pub fn backup_dates(root: &Path) -> Result<Vec<String>> {
    let dir = backup_dir(root);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut dates: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().and_then(dated).map(str::to_string))
        .collect();
    dates.sort();
    Ok(dates)
}

/// True when `root` has no backup of the UTC date of `now`. The core asks
/// this when it starts and at 03:00 local time.
pub fn backup_due(root: &Path, now: SystemTime) -> Result<bool> {
    Ok(!backup_dates(root)?.contains(&utc_date(now)))
}

/// Copies the database of `conn` and the registry of `root` into
/// `<root>/backups/`, then keeps the newest `keep` backups. A second backup
/// on the same UTC date takes the place of the first.
pub fn backup(
    conn: &Connection,
    root: &Path,
    keep: usize,
    now: SystemTime,
) -> Result<BackupResult> {
    if keep < 1 {
        return Err(Error::Other("keep must be 1 or more".into()));
    }
    let dir = backup_dir(root);
    std::fs::create_dir_all(&dir)?;
    let date = utc_date(now);
    let file = dir.join(format!("agentsi-{date}.db"));
    // The process id keeps two processes (the core and `agentsi-core backup`) off one file.
    let partial = dir.join(format!("agentsi-{date}.db.{}.partial", std::process::id()));
    let written = conn
        .backup(rusqlite::MAIN_DB, &partial, None)
        .map_err(Error::from)
        .and_then(|()| {
            std::fs::File::open(&partial)?.sync_all()?;
            std::fs::rename(&partial, &file)?;
            Ok(())
        });
    if let Err(e) = written {
        let _ = std::fs::remove_file(&partial);
        return Err(e);
    }

    let config = root.join("agentsi.config.json");
    let registry = config
        .exists()
        .then(|| dir.join(format!("agentsi.config-{date}.json")));
    if let Some(copy) = &registry {
        std::fs::copy(&config, copy)?;
    }

    let dates = backup_dates(root)?;
    let mut removed = Vec::new();
    for old in &dates[..dates.len().saturating_sub(keep)] {
        for f in [
            dir.join(format!("agentsi-{old}.db")),
            dir.join(format!("agentsi.config-{old}.json")),
        ] {
            if f.exists() {
                std::fs::remove_file(&f)?;
                removed.push(f);
            }
        }
    }
    Ok(BackupResult {
        file,
        registry,
        removed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn utc_date_counts_days_from_the_epoch() {
        let at = |s: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(s);
        assert_eq!(utc_date(at(0)), "1970-01-01");
        assert_eq!(utc_date(at(951_782_400)), "2000-02-29");
        assert_eq!(utc_date(at(1_793_620_799)), "2026-11-02");
        assert_eq!(dated("agentsi-2026-11-02.db"), Some("2026-11-02"));
        assert_eq!(dated("agentsi-2026-11-02-0300.sql.gz"), None);
    }
}
