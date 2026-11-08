//! The logic of the git screen: status, diff, stage, unstage, commit and log of one repo of a
//! product. Each call runs `git` through `story::git::git_limited`, so it takes the git
//! config of the product (`git_env`). Spec: `docs/specs/agentsi-desktop-git.md`.

use crate::registry::Product;
use crate::story::git::{Out, git_limited};
use agentsi_proto::git_api::{
    DIFF_MAX_BYTES, GitCommitInfo, GitCommitResult, GitDiffResult, GitFile, GitStatusResult,
    GitWorktree, LOG_MAX,
};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// The time that a read gets, as `exec.ts` gives it.
const LIMIT: Duration = Duration::from_secs(20);
/// A commit runs the hooks of the repo, and a hook can take longer than a read.
const COMMIT_LIMIT: Duration = Duration::from_secs(120);

#[derive(Debug)]
pub enum Error {
    /// The request is wrong: a path outside the repo, a repo outside the product, no message.
    Invalid(String),
    /// Git failed, or the folder is not a repo.
    Git(String),
}

pub type Result<T> = std::result::Result<T, Error>;

fn git(repo: &Path, args: &[&str], reads: bool, limit: Duration) -> Out {
    let env: Vec<(String, String)> = if reads {
        vec![("GIT_OPTIONAL_LOCKS".into(), "0".into())]
    } else {
        Vec::new()
    };
    git_limited(repo, args, &env, limit)
}

fn must(out: Out, what: &str) -> Result<String> {
    if out.ok {
        return Ok(out.stdout);
    }
    let why = if out.stderr.trim().is_empty() {
        out.stdout
    } else {
        out.stderr
    };
    Err(Error::Git(format!("git {what} failed: {}", why.trim())))
}

/// The repo to work in: `repo` when it is a path of the product, else the first path.
pub fn resolve_repo(product: &Product, repo: Option<&str>) -> Result<PathBuf> {
    let norm = |p: &str| p.trim_end_matches('/').to_string();
    match repo {
        Some(r) => product
            .paths
            .iter()
            .find(|p| norm(p) == norm(r))
            .map(PathBuf::from)
            .ok_or_else(|| Error::Invalid(format!("{r} is not a repo of product {}.", product.id))),
        None => product
            .paths
            .first()
            .map(PathBuf::from)
            .ok_or_else(|| Error::Invalid(format!("Product {} has no path.", product.id))),
    }
}

/// A path from the root of the repo that stays inside it.
pub fn check_path(path: &str) -> Result<()> {
    let p = Path::new(path);
    let outside = path.is_empty()
        || path.contains('\0')
        || p.is_absolute()
        || p.components().any(|c| matches!(c, Component::ParentDir));
    if outside {
        return Err(Error::Invalid(format!(
            "{path:?} is not a path inside the repo."
        )));
    }
    Ok(())
}

/// A patch with the same look on each machine: `a/` and `b/` prefixes, no color, no external tool.
const DIFF: [&str; 7] = [
    "-c",
    "diff.mnemonicPrefix=false",
    "-c",
    "diff.noprefix=false",
    "diff",
    "--no-color",
    "--no-ext-diff",
];

/// The pathspec of a path: from the root of the repo, with no glob or magic.
fn spec(path: &str) -> String {
    format!(":(top,literal){path}")
}

pub fn status(repo: &Path) -> Result<GitStatusResult> {
    let out = must(
        git(
            repo,
            &[
                "status",
                "--porcelain=v2",
                "--branch",
                "-z",
                "--untracked-files=all",
            ],
            true,
            LIMIT,
        ),
        "status",
    )?;
    let mut result = parse_status(&out);
    result.repo = repo.to_string_lossy().into_owned();
    let list = git(repo, &["worktree", "list", "--porcelain"], true, LIMIT);
    if list.ok {
        result.worktrees = parse_worktrees(&list.stdout);
    }
    Ok(result)
}

fn letter(c: char) -> Option<String> {
    (c != '.').then(|| c.to_string())
}

/// `git status --porcelain=v2 --branch -z`.
pub fn parse_status(text: &str) -> GitStatusResult {
    let mut result = GitStatusResult {
        repo: String::new(),
        branch: None,
        head: None,
        upstream: None,
        ahead: None,
        behind: None,
        worktrees: Vec::new(),
        files: Vec::new(),
    };
    let mut parts = text.split('\0');
    while let Some(rec) = parts.next() {
        if rec.is_empty() {
            continue;
        }
        if let Some(h) = rec.strip_prefix("# ") {
            let (key, val) = h.split_once(' ').unwrap_or((h, ""));
            match key {
                "branch.oid" if val != "(initial)" => {
                    result.head = Some(val.chars().take(8).collect());
                }
                "branch.head" if val != "(detached)" => result.branch = Some(val.into()),
                "branch.upstream" => result.upstream = Some(val.into()),
                "branch.ab" => {
                    let num = |s: Option<&str>| s.and_then(|s| s[1..].parse().ok());
                    let mut it = val.split(' ');
                    result.ahead = num(it.next());
                    result.behind = num(it.next());
                }
                _ => {}
            }
            continue;
        }
        let kind = rec.chars().next().unwrap_or(' ');
        match kind {
            '1' => {
                // 1 XY sub mH mI mW hH hI path
                let f: Vec<&str> = rec.splitn(9, ' ').collect();
                if let [_, xy, .., path] = f[..] {
                    result.files.push(file(path, None, xy));
                }
            }
            '2' => {
                // 2 XY sub mH mI mW hH hI Xscore path, then the old path.
                let f: Vec<&str> = rec.splitn(10, ' ').collect();
                let orig = parts.next().unwrap_or_default();
                if let [_, xy, .., path] = f[..] {
                    result.files.push(file(path, Some(orig), xy));
                }
            }
            'u' => {
                // u XY sub m1 m2 m3 mW h1 h2 h3 path
                let f: Vec<&str> = rec.splitn(11, ' ').collect();
                if let [_, _, .., path] = f[..] {
                    result.files.push(GitFile {
                        path: path.into(),
                        orig_path: None,
                        staged: None,
                        unstaged: Some("U".into()),
                    });
                }
            }
            '?' => result.files.push(GitFile {
                path: rec[2..].into(),
                orig_path: None,
                staged: None,
                unstaged: Some("?".into()),
            }),
            _ => {}
        }
    }
    result
}

fn file(path: &str, orig: Option<&str>, xy: &str) -> GitFile {
    let mut c = xy.chars();
    GitFile {
        path: path.into(),
        orig_path: orig.filter(|o| !o.is_empty()).map(String::from),
        staged: c.next().and_then(letter),
        unstaged: c.next().and_then(letter),
    }
}

fn parse_worktrees(text: &str) -> Vec<GitWorktree> {
    text.split("\n\n")
        .filter_map(|block| {
            let mut w = GitWorktree {
                path: String::new(),
                branch: None,
                head: String::new(),
            };
            for line in block.lines() {
                if let Some(p) = line.strip_prefix("worktree ") {
                    w.path = p.into();
                } else if let Some(h) = line.strip_prefix("HEAD ") {
                    w.head = h.chars().take(8).collect();
                } else if let Some(b) = line.strip_prefix("branch ") {
                    w.branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).into());
                }
            }
            (!w.path.is_empty()).then_some(w)
        })
        .collect()
}

pub fn diff(repo: &Path, path: &str, staged: bool) -> Result<GitDiffResult> {
    check_path(path)?;
    let spec = spec(path);
    let mut args = DIFF.to_vec();
    if staged {
        args.push("--cached");
    }
    args.extend(["--", &spec]);
    let mut patch = must(git(repo, &args, true, LIMIT), "diff")?;
    if patch.is_empty() && !staged {
        patch = untracked_patch(repo, path, &spec)?;
    }
    Ok(cut(patch))
}

/// The patch of an untracked file, as a new file. A file that is not untracked gives "".
fn untracked_patch(repo: &Path, path: &str, spec: &str) -> Result<String> {
    let found = must(
        git(
            repo,
            &[
                "ls-files",
                "--others",
                "--exclude-standard",
                "-z",
                "--",
                spec,
            ],
            true,
            LIMIT,
        ),
        "ls-files",
    )?;
    if found.is_empty() {
        return Ok(String::new());
    }
    // `--no-index` exits 1 when the files differ, so the text decides, not the exit code. It
    // names the file as it is given, so it runs at the root of the repo with the repo path.
    let top = must(
        git(repo, &["rev-parse", "--show-toplevel"], true, LIMIT),
        "rev-parse",
    )?;
    let mut args = DIFF.to_vec();
    args.extend(["--no-index", "--", "/dev/null", path]);
    let out = git(Path::new(top.trim()), &args, true, LIMIT);
    if out.stdout.is_empty() && !out.ok && !out.stderr.is_empty() {
        return Err(Error::Git(format!(
            "git diff failed: {}",
            out.stderr.trim()
        )));
    }
    Ok(out.stdout)
}

fn cut(mut patch: String) -> GitDiffResult {
    let truncated = patch.len() > DIFF_MAX_BYTES;
    if truncated {
        let mut at = DIFF_MAX_BYTES;
        while !patch.is_char_boundary(at) {
            at -= 1;
        }
        patch.truncate(at);
    }
    GitDiffResult { patch, truncated }
}

pub fn stage(repo: &Path, paths: &[String]) -> Result<()> {
    paths.iter().try_for_each(|p| check_path(p))?;
    if paths.is_empty() {
        return Ok(());
    }
    let specs: Vec<String> = paths.iter().map(|p| spec(p)).collect();
    let mut args = vec!["add", "--"];
    args.extend(specs.iter().map(String::as_str));
    must(git(repo, &args, false, LIMIT), "add").map(drop)
}

pub fn unstage(repo: &Path, paths: &[String]) -> Result<()> {
    paths.iter().try_for_each(|p| check_path(p))?;
    if paths.is_empty() {
        return Ok(());
    }
    let specs: Vec<String> = paths.iter().map(|p| spec(p)).collect();
    // A repo with no commit has no HEAD to restore from.
    let has_head = git(
        repo,
        &["rev-parse", "--verify", "--quiet", "HEAD"],
        true,
        LIMIT,
    )
    .ok;
    let mut args = if has_head {
        vec!["restore", "--staged", "--"]
    } else {
        vec!["rm", "--cached", "-r", "-q", "--ignore-unmatch", "--"]
    };
    args.extend(specs.iter().map(String::as_str));
    must(git(repo, &args, false, LIMIT), "restore").map(drop)
}

pub fn commit(repo: &Path, message: &str) -> Result<GitCommitResult> {
    if message.trim().is_empty() {
        return Err(Error::Invalid("The commit message is empty.".into()));
    }
    let out = git(repo, &["commit", "-m", message], false, COMMIT_LIMIT);
    if !out.ok {
        let output = [out.stdout.trim(), out.stderr.trim()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(GitCommitResult {
            committed: false,
            commit: None,
            output,
        });
    }
    let id = must(git(repo, &["rev-parse", "HEAD"], true, LIMIT), "rev-parse")?;
    Ok(GitCommitResult {
        committed: true,
        commit: Some(id.trim().into()),
        output: out.stdout.trim().into(),
    })
}

pub fn log(repo: &Path, limit: u32) -> Result<Vec<GitCommitInfo>> {
    if !git(
        repo,
        &["rev-parse", "--verify", "--quiet", "HEAD"],
        true,
        LIMIT,
    )
    .ok
    {
        // A repo with no commit has an empty history. A folder that is no repo fails here.
        must(
            git(repo, &["rev-parse", "--git-dir"], true, LIMIT),
            "rev-parse",
        )?;
        return Ok(Vec::new());
    }
    let n = limit.clamp(1, LOG_MAX).to_string();
    let out = must(
        git(
            repo,
            &["log", "-n", &n, "--format=%H%x1f%an%x1f%at%x1f%s%x1e"],
            true,
            LIMIT,
        ),
        "log",
    )?;
    Ok(out
        .split('\x1e')
        .filter_map(|rec| {
            let mut f = rec.trim_start_matches('\n').split('\x1f');
            Some(GitCommitInfo {
                id: f.next().filter(|s| !s.is_empty())?.into(),
                author: f.next()?.into(),
                time: f.next()?.parse().ok()?,
                subject: f.next()?.into(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_that_leaves_the_repo_is_refused() {
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "a\0b"] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
        for good in ["a.txt", "dir/a b.txt", "..x/y", ".github/ci.yml"] {
            assert!(check_path(good).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn the_porcelain_text_gives_the_branch_and_the_files() {
        let text = "# branch.oid 0123456789abcdef\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
            1 M. N... 100644 100644 100644 aaa bbb staged.txt\0\
            1 .M N... 100644 100644 100644 aaa bbb dir/a b.txt\0\
            2 R. N... 100644 100644 100644 aaa bbb R100 new.txt\0old.txt\0\
            u UU N... 1 2 3 4 aaa bbb clash.txt\0\
            ? new file.txt\0";
        let r = parse_status(text);
        assert_eq!(r.branch.as_deref(), Some("main"));
        assert_eq!(r.head.as_deref(), Some("01234567"));
        assert_eq!(r.upstream.as_deref(), Some("origin/main"));
        assert_eq!((r.ahead, r.behind), (Some(2), Some(1)));
        let got: Vec<_> = r
            .files
            .iter()
            .map(|f| {
                (
                    f.path.as_str(),
                    f.orig_path.as_deref(),
                    f.staged.as_deref(),
                    f.unstaged.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("staged.txt", None, Some("M"), None),
                ("dir/a b.txt", None, None, Some("M")),
                ("new.txt", Some("old.txt"), Some("R"), None),
                ("clash.txt", None, None, Some("U")),
                ("new file.txt", None, None, Some("?")),
            ]
        );
    }

    #[test]
    fn a_detached_head_and_an_initial_repo_have_no_branch_or_head() {
        let r = parse_status("# branch.oid (initial)\0# branch.head (detached)\0");
        assert_eq!((r.branch, r.head, r.ahead), (None, None, None));
    }

    #[test]
    fn a_patch_over_the_limit_is_cut_on_a_character() {
        let big = "é".repeat(DIFF_MAX_BYTES);
        let r = cut(big);
        assert!(r.truncated);
        assert!(r.patch.len() <= DIFF_MAX_BYTES);
        assert_eq!(
            cut("short".into()),
            GitDiffResult {
                patch: "short".into(),
                truncated: false
            }
        );
    }

    #[test]
    fn a_repo_must_be_a_path_of_the_product() {
        let p = Product {
            id: "a".into(),
            name: "A".into(),
            paths: vec!["/work/a".into(), "/work/b/".into()],
            status: "active".into(),
            roles: vec![],
        };
        assert_eq!(resolve_repo(&p, None).unwrap(), PathBuf::from("/work/a"));
        assert!(resolve_repo(&p, Some("/work/b")).is_ok());
        assert!(resolve_repo(&p, Some("/work/c")).is_err());
        assert!(resolve_repo(&p, Some("/work/a/..")).is_err());
    }
}
