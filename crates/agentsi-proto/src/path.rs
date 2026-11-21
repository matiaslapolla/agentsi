//! Finds a program on the `PATH`. Every caller of AgentSI uses this lookup, so a
//! program that one part finds, every part finds. Spec: `docs/specs/agentsi-path.md`.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// A `claude` program that wins over the one on the `PATH`.
pub const CLAUDE_BIN_ENV: &str = "AGENTSI_CLAUDE_BIN";

/// The `PATH` of this process.
pub fn current() -> Option<OsString> {
    std::env::var_os("PATH")
}

/// The first executable file `name` in a folder of the `PATH`. A name with a `/` is
/// checked as it is.
pub fn which(name: &str) -> Option<PathBuf> {
    which_in(name, current().as_deref())
}

/// [`which`] on the given `PATH`.
pub fn which_in(name: &str, path: Option<&OsStr>) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    if name.contains('/') {
        return Some(PathBuf::from(name)).filter(|p| is_executable_file(p));
    }
    std::env::split_paths(path?)
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(name))
        .find(|p| is_executable_file(p))
}

pub fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The `claude` program that AgentSI starts: `AGENTSI_CLAUDE_BIN`, else the one on the `PATH`.
pub fn claude() -> Result<PathBuf, String> {
    claude_in(current())
}

/// [`claude`] for a child that gets `path` as its `PATH`.
pub fn claude_in(path: Option<OsString>) -> Result<PathBuf, String> {
    claude_with(std::env::var_os(CLAUDE_BIN_ENV), path)
}

/// The program that `AGENTSI_CLAUDE_BIN` names, or `None` when it is not set.
pub fn claude_bin() -> Option<Result<PathBuf, String>> {
    bin_of(std::env::var_os(CLAUDE_BIN_ENV))
}

fn bin_of(bin: Option<OsString>) -> Option<Result<PathBuf, String>> {
    let bin = PathBuf::from(bin.filter(|b| !b.is_empty())?);
    Some(if is_executable_file(&bin) {
        Ok(bin)
    } else {
        Err(format!(
            "{CLAUDE_BIN_ENV} is {}, and it is not an executable file",
            bin.display()
        ))
    })
}

fn claude_with(bin: Option<OsString>, path: Option<OsString>) -> Result<PathBuf, String> {
    if let Some(bin) = bin_of(bin) {
        return bin;
    }
    which_in("claude", path.as_deref()).ok_or_else(|| {
        format!(
            "cannot find `claude` on the PATH of the core: {}",
            path.as_deref()
                .map_or_else(|| "(no PATH)".into(), |p| p.to_string_lossy().into_owned())
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn program(dir: &Path, name: &str, mode: u32) -> PathBuf {
        let file = dir.join(name);
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
        file
    }

    fn join(dirs: &[&Path]) -> OsString {
        std::env::join_paths(dirs).unwrap()
    }

    #[test]
    fn the_first_executable_file_on_the_path_wins() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        program(a.path(), "tool", 0o644);
        let found = program(b.path(), "tool", 0o755);
        program(b.path(), "other", 0o755);
        let path = join(&[a.path(), b.path()]);
        assert_eq!(which_in("tool", Some(&path)), Some(found));
        assert_eq!(which_in("none", Some(&path)), None);
        assert_eq!(which_in("tool", None), None);
    }

    #[test]
    fn a_folder_with_the_name_is_not_a_program() {
        let a = tempfile::tempdir().unwrap();
        std::fs::create_dir(a.path().join("tool")).unwrap();
        assert_eq!(which_in("tool", Some(&join(&[a.path()]))), None);
    }

    #[test]
    fn a_name_with_a_slash_skips_the_path() {
        let a = tempfile::tempdir().unwrap();
        let file = program(a.path(), "tool", 0o755);
        let name = file.to_str().unwrap();
        assert_eq!(which_in(name, None), Some(file.clone()));
        assert_eq!(which_in("", Some(&join(&[a.path()]))), None);
    }

    #[test]
    fn claude_bin_wins_over_the_path() {
        let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        program(a.path(), "claude", 0o755);
        let own = program(b.path(), "my-claude", 0o755);
        let path = join(&[a.path()]);
        assert_eq!(
            claude_with(Some(own.clone().into()), Some(path.clone())),
            Ok(own)
        );
        assert_eq!(
            claude_with(Some("".into()), Some(path.clone())),
            Ok(a.path().join("claude"))
        );
        let err = claude_with(Some(b.path().join("none").into()), Some(path)).unwrap_err();
        assert!(err.starts_with(CLAUDE_BIN_ENV), "{err}");
    }

    #[test]
    fn a_missing_claude_names_the_path_of_the_core() {
        let a = tempfile::tempdir().unwrap();
        let path = join(&[a.path(), Path::new("/nowhere")]);
        let err = claude_with(None, Some(path)).unwrap_err();
        assert_eq!(
            err,
            format!(
                "cannot find `claude` on the PATH of the core: {}:/nowhere",
                a.path().display()
            )
        );
    }
}
