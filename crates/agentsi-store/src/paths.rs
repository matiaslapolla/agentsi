//! Where the data root and the database file are.

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

fn expand(path: &str) -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    match path.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            home.join(rest.trim_start_matches('/'))
        }
        _ if Path::new(path).is_relative() => home.join(path),
        _ => PathBuf::from(path),
    }
}

/// `explicit`, then `AGENTSI_HOME`, then the first line of
/// `$XDG_CONFIG_HOME/agentsi/home`. The TypeScript CLI has the same order.
/// The Rust programs have no code checkout, so there is no other default.
pub fn data_root(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    if let Some(home) = std::env::var("AGENTSI_HOME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Ok(expand(&home));
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or_else(|| Error::Other("HOME is not set".into()))?;
    let file = config.join("agentsi").join("home");
    let line = std::fs::read_to_string(&file)
        .ok()
        .and_then(|s| s.lines().next().map(|l| l.trim().to_string()));
    match line.filter(|l| !l.is_empty()) {
        Some(l) => Ok(expand(&l)),
        None => Err(Error::Other(format!(
            "no data root: set AGENTSI_HOME, or write the path in {}",
            file.display()
        ))),
    }
}

/// `<data root>/agentsi.db`.
pub fn db_path(root: &Path) -> PathBuf {
    root.join(crate::db::DB_FILE)
}
