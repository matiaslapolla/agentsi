//! `commitFiles` of `exec.ts`: commits only the named files of the data root.

use agentsi_proto::roles_api::CommitInfo;
use std::path::Path;
use std::process::Command;

struct Git {
    ok: bool,
    stdout: String,
    stderr: String,
}

fn git(cwd: &Path, args: &[&str]) -> Git {
    match Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .envs(crate::git_env::env_for_cwd(cwd))
        .output()
    {
        Ok(out) => Git {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        Err(e) => Git {
            ok: false,
            stdout: String::new(),
            stderr: e.to_string(),
        },
    }
}

/// Stages `paths` (relative to `cwd`) and commits them alone. Nothing else
/// that is staged goes in. A failure is in `error`, never a panic: the files
/// are already written.
pub fn commit_files(cwd: &Path, paths: &[&str], message: &str) -> CommitInfo {
    let with_paths = |head: &[&str]| -> Vec<String> {
        head.iter()
            .map(|s| s.to_string())
            .chain(std::iter::once("--".to_string()))
            .chain(paths.iter().map(|p| p.to_string()))
            .collect()
    };
    let run = |head: &[&str]| {
        let args = with_paths(head);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        git(cwd, &refs)
    };
    let add = run(&["add"]);
    if !add.ok {
        return CommitInfo {
            committed: false,
            error: Some(add.stderr.trim().into()),
        };
    }
    if run(&["diff", "--cached", "--quiet"]).ok {
        return CommitInfo::default();
    }
    let res = run(&["commit", "-m", message]);
    if res.ok {
        return CommitInfo {
            committed: true,
            error: None,
        };
    }
    let text = if res.stderr.is_empty() {
        &res.stdout
    } else {
        &res.stderr
    };
    CommitInfo {
        committed: false,
        error: Some(text.trim().into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        assert!(git(dir.path(), &["init", "-q"]).ok);
        for (k, v) in [
            ("user.name", "T"),
            ("user.email", "t@example.invalid"),
            ("commit.gpgsign", "false"),
        ] {
            assert!(git(dir.path(), &["config", k, v]).ok);
        }
        dir
    }

    #[test]
    fn it_commits_only_the_named_file_and_skips_a_file_with_no_change() {
        let dir = repo();
        std::fs::write(dir.path().join("a.json"), "1").unwrap();
        std::fs::write(dir.path().join("other.txt"), "x").unwrap();
        git(dir.path(), &["add", "other.txt"]);
        let first = commit_files(dir.path(), &["a.json"], "first");
        assert!(first.committed, "{:?}", first.error);
        let shown = git(dir.path(), &["show", "--name-only", "--format=%s"]);
        assert_eq!(shown.stdout.trim(), "first\n\na.json");
        let again = commit_files(dir.path(), &["a.json"], "second");
        assert_eq!(again, CommitInfo::default());
    }

    #[test]
    fn a_folder_with_no_repo_gives_an_error_and_no_commit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.json"), "1").unwrap();
        let r = commit_files(dir.path(), &["a.json"], "m");
        assert!(!r.committed);
        assert!(r.error.unwrap().contains("not a git repository"));
    }
}
