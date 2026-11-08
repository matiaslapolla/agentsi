//! Voice mode of a Claude Code session that runs outside the app.
//!
//! One empty file for each session holds the state: `<data root>/voice/sessions/<id>`.
//! The file survives a restart of the core, and the hook bridge tests it without a
//! socket. A session that nobody turned off leaves its file. The core removes files
//! that no hook touched for `STALE` when a session turns voice mode on.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const STALE: Duration = Duration::from_secs(14 * 24 * 3600);

/// The session id as a file name, or `None` when it is not a plain id: 1 to 100 ASCII
/// letters, digits, `_` and `-`, with at least one letter or digit. The id is the name,
/// so two ids never share a file. The TypeScript CLI took `\w` and `-` too.
pub fn safe_name(session: &str) -> Option<String> {
    let plain = (1..=100).contains(&session.len())
        && session
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && session.bytes().any(|b| b.is_ascii_alphanumeric());
    plain.then(|| session.to_string())
}

pub fn dir(root: &Path) -> PathBuf {
    root.join("voice").join("sessions")
}

/// True when some session has voice mode on. The bridge tests this before it reads stdin.
pub fn any_on(root: &Path) -> bool {
    std::fs::read_dir(dir(root)).is_ok_and(|mut entries| entries.next().is_some())
}

pub fn is_on(root: &Path, session: &str) -> bool {
    safe_name(session).is_some_and(|name| dir(root).join(name).exists())
}

/// Turns voice mode of `session` on or off, or reads it when `on` is `None`.
pub fn set(root: &Path, session: &str, on: Option<bool>) -> std::io::Result<bool> {
    let name = safe_name(session)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad session id"))?;
    let file = dir(root).join(name);
    match on {
        None => Ok(file.exists()),
        Some(true) => {
            std::fs::create_dir_all(dir(root))?;
            prune(root);
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file)?;
            touch(&file);
            Ok(true)
        }
        Some(false) => match std::fs::remove_file(&file) {
            Ok(()) => Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        },
    }
}

/// Marks the session as in use, so `prune` keeps its file.
pub fn touch(file: &Path) {
    if let Ok(f) = std::fs::OpenOptions::new().append(true).open(file) {
        let _ = f.set_modified(SystemTime::now());
    }
}

pub fn file(root: &Path, session: &str) -> Option<PathBuf> {
    safe_name(session).map(|name| dir(root).join(name))
}

fn prune(root: &Path) {
    let Ok(entries) = std::fs::read_dir(dir(root)) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn a_session_turns_on_and_off_and_the_file_is_the_state() {
        let r = root();
        assert!(!any_on(r.path()));
        assert!(!set(r.path(), "s-1", None).unwrap());
        assert!(set(r.path(), "s-1", Some(true)).unwrap());
        assert!(is_on(r.path(), "s-1") && any_on(r.path()));
        assert!(!is_on(r.path(), "s-2"));
        assert!(!set(r.path(), "s-1", Some(false)).unwrap());
        assert!(!any_on(r.path()));
        assert!(!set(r.path(), "s-1", Some(false)).unwrap());
    }

    #[test]
    fn only_letters_digits_underscore_and_hyphen_make_an_id() {
        let r = root();
        let long = "a".repeat(101);
        let cjk = "漢".repeat(100);
        for bad in [
            "", "   ", "///", "..", "---", "___", "_-_", "../../x", ".hidden", "a/b", "a b", "é",
            &long, &cjk,
        ] {
            assert!(set(r.path(), bad, Some(true)).is_err(), "{bad}");
            assert!(!is_on(r.path(), bad), "{bad}");
        }
        assert!(!dir(r.path()).exists() || !any_on(r.path()));
        let ok = "a".repeat(100);
        for good in [
            "s-1",
            "0f3c1e2a-7b9d-4c11-8a55-0123456789ab",
            "A_b-9",
            ok.as_str(),
        ] {
            set(r.path(), good, Some(true)).unwrap();
            assert!(is_on(r.path(), good), "{good}");
        }
    }

    #[test]
    fn two_ids_never_share_a_file() {
        let r = root();
        set(r.path(), "a_b", Some(true)).unwrap();
        assert!(set(r.path(), "a/b", Some(true)).is_err());
        assert!(!is_on(r.path(), "a-b"));
    }

    #[test]
    fn turning_on_removes_the_files_that_no_hook_touched_for_two_weeks() {
        let r = root();
        set(r.path(), "old", Some(true)).unwrap();
        let old = file(r.path(), "old").unwrap();
        let f = std::fs::OpenOptions::new().append(true).open(&old).unwrap();
        f.set_modified(SystemTime::now() - STALE - Duration::from_secs(60))
            .unwrap();
        set(r.path(), "fresh", Some(true)).unwrap();
        assert!(!is_on(r.path(), "old") && is_on(r.path(), "fresh"));
    }
}
