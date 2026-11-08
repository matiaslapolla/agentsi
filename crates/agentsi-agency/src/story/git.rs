//! `git` of `exec.ts` for the story code: a run that gives its output, a run
//! that must work, and the main branch of a repo.

use super::{Error, Result};
use crate::git_env;
use crate::knowledge::exec;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// Hooks of a repo run on the host, and a story can write them in the worktree. So the
/// commits of the runner and the merge of `accept` run no hook.
pub const NO_HOOKS: [&str; 2] = ["-c", "core.hooksPath=/dev/null"];

pub struct Out {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

pub fn git(cwd: &Path, args: &[&str]) -> Out {
    match Command::new("git")
        // A path with an accent stays as it is, so it can equal a criterion test path.
        .args(["-c", "core.quotePath=false"])
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .envs(git_env::env_for_cwd(cwd))
        .output()
    {
        Ok(out) => Out {
            ok: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        Err(e) => Out {
            ok: false,
            stdout: String::new(),
            stderr: e.to_string(),
        },
    }
}

/// Like `git`, with a time limit and more names in the environment. The git screen uses it.
pub fn git_limited(cwd: &Path, args: &[&str], env: &[(String, String)], limit: Duration) -> Out {
    let mut all: Vec<&str> = vec!["-c", "core.quotePath=false"];
    all.extend_from_slice(args);
    let mut vars = git_env::env_for_cwd(cwd);
    vars.extend_from_slice(env);
    let out = exec::run_env("git", &all, cwd, limit, &vars);
    Out {
        ok: out.ok,
        stdout: out.stdout,
        stderr: out.stderr,
    }
}

/// The trimmed stdout of a git run that must work.
pub fn git_out(cwd: &Path, args: &[&str]) -> Result<String> {
    let r = git(cwd, args);
    if !r.ok {
        let why = if r.stderr.is_empty() {
            &r.stdout
        } else {
            &r.stderr
        };
        return Err(Error::invalid(format!(
            "git {} failed in {}: {}",
            args.join(" "),
            cwd.display(),
            why.trim()
        )));
    }
    Ok(r.stdout.trim().to_string())
}

pub fn branch_exists(repo: &Path, branch: &str) -> bool {
    git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .ok
}

/// The product's main branch: the remote default branch, else `main`, else `master`.
pub fn main_branch(repo: &Path) -> Result<String> {
    let remote = git(
        repo,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    );
    let name = remote.stdout.trim();
    if remote.ok && !name.is_empty() {
        return Ok(name.strip_prefix("origin/").unwrap_or(name).to_string());
    }
    for name in ["main", "master"] {
        if branch_exists(repo, name) {
            return Ok(name.into());
        }
    }
    Err(Error::invalid(format!(
        "{} has no main or master branch.",
        repo.display()
    )))
}

/// `commitFiles` of `exec.ts`: stages `paths` and commits them alone. `Ok(false)`
/// when nothing changed.
pub fn commit_files(cwd: &Path, paths: &[&str], message: &str) -> Result<bool> {
    let with = |head: &[&str]| -> Vec<String> {
        head.iter()
            .chain(std::iter::once(&"--"))
            .chain(paths.iter())
            .map(|s| s.to_string())
            .collect()
    };
    let run = |head: &[&str]| {
        let args = with(head);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        git(cwd, &refs)
    };
    let add = run(&["add"]);
    if !add.ok {
        return Err(Error::invalid(add.stderr.trim().to_string()));
    }
    if run(&["diff", "--cached", "--quiet"]).ok {
        return Ok(false);
    }
    let res = run(&[NO_HOOKS[0], NO_HOOKS[1], "commit", "-m", message]);
    if !res.ok {
        let why = if res.stderr.is_empty() {
            res.stdout
        } else {
            res.stderr
        };
        return Err(Error::invalid(why.trim().to_string()));
    }
    Ok(true)
}
