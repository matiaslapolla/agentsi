//! The logic of the git screen: status, diff, stage, unstage, commit and log of one repo of a
//! product, its summary, the diff of a commit, branches, and fetch, pull and push. Each call
//! runs `git` with the git config of the product (`git_env`) and a time limit.
//! Specs: `docs/specs/agentsi-desktop-git.md`, `docs/specs/agentsi-desktop-git-more.md`.
//! No call forces: a push sends full ref names, so no refspec can start with `+`.

use crate::registry::Product;
use crate::story::git::{Out, git_limited};
use crate::git_env;
use agentsi_proto::git_api::{
    DIFF_MAX_BYTES, GitBranch, GitBranchesResult, GitCommitInfo, GitCommitResult, GitDiffResult,
    GitFile, GitOutcome, GitOutgoingResult, GitShowFile, GitShowResult, GitStatusResult,
    GitSummaryInfo, GitSwitchResult, GitSyncResult, GitWorktree, LOG_MAX,
};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The time that a read gets, as `exec.ts` gives it.
const LIMIT: Duration = Duration::from_secs(20);
/// A commit runs the hooks of the repo, and a hook can take longer than a read.
const COMMIT_LIMIT: Duration = Duration::from_secs(120);
/// A fetch, a pull or a push waits for the network.
pub const NETWORK_LIMIT: Duration = Duration::from_secs(120);
/// The shortest time between two progress lines of one call.
const PROGRESS_GAP: Duration = Duration::from_millis(100);

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

const LOG_FORMAT: &str = "--format=%H%x1f%an%x1f%at%x1f%s%x1e";

fn parse_log(out: &str) -> Vec<GitCommitInfo> {
    out.split('\x1e')
        .filter_map(|rec| {
            let mut f = rec.trim_start_matches('\n').split('\x1f');
            Some(GitCommitInfo {
                id: f.next().filter(|s| !s.is_empty())?.into(),
                author: f.next()?.into(),
                time: f.next()?.parse().ok()?,
                subject: f.next()?.into(),
            })
        })
        .collect()
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
    let out = must(git(repo, &["log", "-n", &n, LOG_FORMAT], true, LIMIT), "log")?;
    Ok(parse_log(&out))
}

// The summary, the diff of a commit, branches, fetch, pull and push.

/// The git summary of the repo that holds `dir`. A folder outside a repo gives `None`.
/// Untracked files count in the normal mode of git: a new folder counts once.
pub fn summary(dir: &Path) -> Result<Option<GitSummaryInfo>> {
    let top = git(dir, &["rev-parse", "--show-toplevel"], true, LIMIT);
    if !top.ok {
        if top.stderr.contains("not a git repository") {
            return Ok(None);
        }
        return must(top, "rev-parse").map(|_| None);
    }
    let out = must(
        git(
            dir,
            &["status", "--porcelain=v2", "--branch", "-z"],
            true,
            LIMIT,
        ),
        "status",
    )?;
    let s = parse_status(&out);
    Ok(Some(GitSummaryInfo {
        repo: top.stdout.trim().into(),
        branch: s.branch,
        head: s.head,
        upstream: s.upstream,
        ahead: s.ahead,
        behind: s.behind,
        changed: u32::try_from(s.files.len()).unwrap_or(u32::MAX),
    }))
}

/// A branch, a commit id or another revision that git cannot read as an option.
fn check_rev(rev: &str) -> Result<()> {
    let bad = rev.is_empty()
        || rev.starts_with('-')
        || rev.contains("..")
        || rev.chars().any(|c| c.is_whitespace() || c.is_control());
    if bad {
        return Err(Error::Invalid(format!("{rev:?} is not a branch or a commit.")));
    }
    Ok(())
}

/// The full id of the commit that `rev` names.
fn commit_id(repo: &Path, rev: &str) -> Result<String> {
    check_rev(rev)?;
    let spec = format!("{rev}^{{commit}}");
    let out = git(
        repo,
        &["rev-parse", "--verify", "--quiet", "--end-of-options", &spec],
        true,
        LIMIT,
    );
    if !out.ok {
        return Err(Error::Invalid(format!("{rev} is not a commit of the repo.")));
    }
    Ok(out.stdout.trim().into())
}

/// `git diff --name-status -z` of two trees.
fn parse_name_status(text: &str) -> Vec<GitShowFile> {
    let mut parts = text.split('\0').filter(|p| !p.is_empty());
    let mut files = Vec::new();
    while let Some(status) = parts.next() {
        let letter: String = status.chars().take(1).collect();
        let Some(first) = parts.next() else { break };
        let (path, orig_path) = if matches!(letter.as_str(), "R" | "C") {
            match parts.next() {
                Some(new) => (new.to_string(), Some(first.to_string())),
                None => break,
            }
        } else {
            (first.to_string(), None)
        };
        files.push(GitShowFile {
            path,
            orig_path,
            letter,
        });
    }
    files
}

/// The files and the patch of a commit, against its first parent. A first commit compares
/// with the empty tree.
pub fn show(repo: &Path, rev: &str) -> Result<GitShowResult> {
    let id = commit_id(repo, rev)?;
    let info = must(
        git(repo, &["show", "-s", LOG_FORMAT, &id], true, LIMIT),
        "show",
    )?;
    let commit = parse_log(&info)
        .into_iter()
        .next()
        .ok_or_else(|| Error::Git(format!("git show gave no commit for {id}")))?;
    let parent = format!("{id}^1");
    let base = match git(
        repo,
        &["rev-parse", "--verify", "--quiet", &parent],
        true,
        LIMIT,
    ) {
        p if p.ok => p.stdout.trim().to_string(),
        _ => must(
            git(repo, &["hash-object", "-t", "tree", "/dev/null"], true, LIMIT),
            "hash-object",
        )?
        .trim()
        .to_string(),
    };
    let names = must(
        git(
            repo,
            &["diff", "--no-ext-diff", "-M", "--name-status", "-z", &base, &id],
            true,
            LIMIT,
        ),
        "diff",
    )?;
    let mut args = DIFF.to_vec();
    args.extend(["-M", &base, &id]);
    let patch = cut(must(git(repo, &args, true, LIMIT), "diff")?);
    Ok(GitShowResult {
        commit,
        files: parse_name_status(&names),
        patch: patch.patch,
        truncated: patch.truncated,
    })
}

/// `ahead 2, behind 1` of `%(upstream:track,nobracket)`. A gone upstream gives none.
fn track(text: &str) -> (Option<u32>, Option<u32>) {
    if text == "gone" {
        return (None, None);
    }
    let (mut ahead, mut behind) = (0, 0);
    for part in text.split(", ") {
        match part.split_once(' ') {
            Some(("ahead", n)) => ahead = n.parse().unwrap_or(0),
            Some(("behind", n)) => behind = n.parse().unwrap_or(0),
            _ => {}
        }
    }
    (Some(ahead), Some(behind))
}

/// `git for-each-ref` of the branches, with the fields of `BRANCH_FORMAT`.
fn parse_branches(text: &str) -> (Vec<GitBranch>, Vec<GitBranch>) {
    let (mut local, mut remote) = (Vec::new(), Vec::new());
    for rec in text.split('\x1e') {
        let f: Vec<&str> = rec.trim_start_matches('\n').split('\x1f').collect();
        let [refname, head, upstream, tracked, current, subject] = f[..] else {
            continue;
        };
        let upstream = (!upstream.is_empty()).then(|| upstream.to_string());
        let (ahead, behind) = match &upstream {
            Some(_) => track(tracked),
            None => (None, None),
        };
        let branch = |name: &str| GitBranch {
            name: name.into(),
            head: head.into(),
            subject: subject.into(),
            current: current == "*",
            upstream: upstream.clone(),
            ahead,
            behind,
        };
        if let Some(name) = refname.strip_prefix("refs/heads/") {
            local.push(branch(name));
        } else if let Some(name) = refname.strip_prefix("refs/remotes/") {
            // `origin/HEAD` names the default branch of the remote, not a branch.
            if !name.ends_with("/HEAD") {
                remote.push(branch(name));
            }
        }
    }
    local.sort_by_key(|b| !b.current);
    (local, remote)
}

const BRANCH_FORMAT: &str = "--format=%(refname)%1f%(objectname:short)%1f%(upstream:short)%1f%(upstream:track,nobracket)%1f%(HEAD)%1f%(subject)%1e";

pub fn branches(repo: &Path) -> Result<GitBranchesResult> {
    let out = must(
        git(
            repo,
            &["for-each-ref", BRANCH_FORMAT, "refs/heads", "refs/remotes"],
            true,
            LIMIT,
        ),
        "for-each-ref",
    )?;
    let (local, remote) = parse_branches(&out);
    let list = must(
        git(repo, &["worktree", "list", "--porcelain"], true, LIMIT),
        "worktree list",
    )?;
    Ok(GitBranchesResult {
        local,
        remote,
        worktrees: parse_worktrees(&list),
    })
}

fn has_ref(repo: &Path, full: &str) -> bool {
    git(
        repo,
        &["show-ref", "--verify", "--quiet", full],
        true,
        LIMIT,
    )
    .ok
}

pub fn branch_create(repo: &Path, name: &str, from: Option<&str>) -> Result<()> {
    let valid = !name.starts_with('-')
        && git(repo, &["check-ref-format", "--branch", name], true, LIMIT).ok;
    if !valid {
        return Err(Error::Invalid(format!("{name:?} is not a valid branch name.")));
    }
    if has_ref(repo, &format!("refs/heads/{name}")) {
        return Err(Error::Invalid(format!("The branch {name} exists.")));
    }
    let start = match from {
        Some(f) => {
            commit_id(repo, f)?;
            f
        }
        None => "HEAD",
    };
    must(git(repo, &["branch", name, start], false, LIMIT), "branch").map(drop)
}

fn joined(out: &Out) -> String {
    [out.stdout.trim(), out.stderr.trim()]
        .iter()
        .filter(|s| !s.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("\n")
}

/// Switches to a local branch, or to a new local branch that tracks a remote one. A tree
/// with a staged or unstaged change gives `dirty` and those files. Untracked files stay, as
/// `git switch` keeps them.
pub fn switch(repo: &Path, name: &str) -> Result<GitSwitchResult> {
    check_rev(name)?;
    let args: Vec<&str> = if has_ref(repo, &format!("refs/heads/{name}")) {
        vec!["switch", "--no-guess", name]
    } else if has_ref(repo, &format!("refs/remotes/{name}")) {
        vec!["switch", "--track", name]
    } else {
        return Err(Error::Invalid(format!("{name} is not a branch of the repo.")));
    };
    let dirty: Vec<GitFile> = status(repo)?
        .files
        .into_iter()
        .filter(|f| f.staged.is_some() || f.unstaged.as_deref().is_some_and(|u| u != "?"))
        .collect();
    if !dirty.is_empty() {
        return Ok(GitSwitchResult {
            outcome: GitOutcome::Dirty,
            files: dirty,
            output: String::new(),
        });
    }
    let out = git(repo, &args, false, COMMIT_LIMIT);
    Ok(GitSwitchResult {
        outcome: if out.ok {
            GitOutcome::Done
        } else {
            GitOutcome::Failed
        },
        files: Vec::new(),
        output: joined(&out),
    })
}

fn config_value(repo: &Path, key: &str) -> Option<String> {
    let out = git(repo, &["config", "--get", key], true, LIMIT);
    let value = out.stdout.trim();
    (out.ok && !value.is_empty()).then(|| value.to_string())
}

/// Where a push of the current branch goes.
struct Target {
    branch: String,
    remote: String,
    /// The full ref on the remote.
    remote_ref: String,
    set_upstream: bool,
}

fn target(repo: &Path) -> Result<Target> {
    let head = git(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"], true, LIMIT);
    let branch = head.stdout.trim().to_string();
    if !head.ok || branch.is_empty() {
        return Err(Error::Invalid(
            "The head is detached. Switch to a branch to push.".into(),
        ));
    }
    let remote = config_value(repo, &format!("branch.{branch}.remote"));
    let merge = config_value(repo, &format!("branch.{branch}.merge"));
    if let (Some(remote), Some(remote_ref)) = (remote, merge) {
        return Ok(Target {
            branch,
            remote,
            remote_ref,
            set_upstream: false,
        });
    }
    if !git(repo, &["remote", "get-url", "origin"], true, LIMIT).ok {
        return Err(Error::Invalid(
            "The branch has no upstream and the repo has no remote origin.".into(),
        ));
    }
    Ok(Target {
        remote_ref: format!("refs/heads/{branch}"),
        branch,
        remote: "origin".into(),
        set_upstream: true,
    })
}

/// The branch, the remote and the commits that `push` sends.
pub fn outgoing(repo: &Path) -> Result<GitOutgoingResult> {
    let t = target(repo)?;
    let upstream = format!("{}@{{upstream}}", t.branch);
    let not = if t.set_upstream {
        format!("--remotes={}", t.remote)
    } else {
        upstream
    };
    let range = ["HEAD", "--not", not.as_str()];
    let mut count_args = vec!["rev-list", "--count"];
    count_args.extend(range);
    let count = must(git(repo, &count_args, true, LIMIT), "rev-list")?
        .trim()
        .parse()
        .unwrap_or(0);
    let n = LOG_MAX.to_string();
    let mut log_args = vec!["log", "-n", &n, LOG_FORMAT];
    log_args.extend(range);
    let commits = parse_log(&must(git(repo, &log_args, true, LIMIT), "log")?);
    Ok(GitOutgoingResult {
        remote_branch: t
            .remote_ref
            .strip_prefix("refs/heads/")
            .unwrap_or(&t.remote_ref)
            .into(),
        branch: t.branch,
        remote: t.remote,
        set_upstream: t.set_upstream,
        commits,
        count,
    })
}

fn synced(out: Out) -> GitSyncResult {
    GitSyncResult {
        outcome: if out.ok {
            GitOutcome::Done
        } else {
            GitOutcome::Failed
        },
        output: joined(&out),
    }
}

pub fn fetch(repo: &Path, progress: &mut dyn FnMut(&str)) -> GitSyncResult {
    synced(streamed(repo, &["fetch", "--all", "--progress"], progress))
}

/// `git pull --ff-only`. A branch and an upstream that both moved give `diverged`: git
/// refuses that pull before it touches the tree.
pub fn pull(repo: &Path, progress: &mut dyn FnMut(&str)) -> GitSyncResult {
    let out = streamed(
        repo,
        &["pull", "--ff-only", "--no-rebase", "--progress"],
        progress,
    );
    let mut result = synced(out);
    if result.outcome == GitOutcome::Failed
        && let Ok(s) = status(repo)
        && s.ahead.unwrap_or(0) > 0
        && s.behind.unwrap_or(0) > 0
    {
        result.outcome = GitOutcome::Diverged;
    }
    result
}

/// Pushes the current branch, with no force. A branch with no upstream goes to origin with
/// `--set-upstream`.
pub fn push(repo: &Path, progress: &mut dyn FnMut(&str)) -> Result<GitSyncResult> {
    let t = target(repo)?;
    let spec = format!("refs/heads/{}:{}", t.branch, t.remote_ref);
    let mut args = vec!["push", "--progress"];
    if t.set_upstream {
        args.push("--set-upstream");
    }
    args.extend([t.remote.as_str(), spec.as_str()]);
    Ok(synced(streamed(repo, &args, progress)))
}

/// Runs git with the git config of the repo's product and the network limit. Each progress
/// line goes to `progress`, at most one each `PROGRESS_GAP`, and the last one always. The
/// output keeps the lines that end with a line break.
pub fn streamed(repo: &Path, args: &[&str], progress: &mut dyn FnMut(&str)) -> Out {
    streamed_within(repo, args, NETWORK_LIMIT, progress)
}

fn streamed_within(
    repo: &Path,
    args: &[&str],
    limit: Duration,
    progress: &mut dyn FnMut(&str),
) -> Out {
    let failed = |stderr: String| Out {
        ok: false,
        stdout: String::new(),
        stderr,
    };
    let spawned = Command::new("git")
        .args(["-c", "core.quotePath=false"])
        .args(args)
        .current_dir(repo)
        .env("GIT_TERMINAL_PROMPT", "0")
        .envs(git_env::env_for_cwd(repo))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => return failed(e.to_string()),
    };
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let (send_out, out_text) = mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        let _ = send_out.send(text);
    });
    let (send, lines) = mpsc::channel::<(String, bool)>();
    let mut stderr = child.stderr.take().expect("stderr is piped");
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut line = Vec::new();
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 {
                break;
            }
            for &b in &buf[..n] {
                if b == b'\r' || b == b'\n' {
                    let text = String::from_utf8_lossy(&line).into_owned();
                    line.clear();
                    if send.send((text, b == b'\n')).is_err() {
                        return;
                    }
                } else {
                    line.push(b);
                }
            }
        }
        if !line.is_empty() {
            let _ = send.send((String::from_utf8_lossy(&line).into_owned(), true));
        }
    });
    let deadline = Instant::now() + limit;
    let mut kept = String::new();
    let mut pending: Option<String> = None;
    let mut last = Instant::now() - PROGRESS_GAP;
    let mut flush = |pending: &mut Option<String>, force: bool| {
        if (force || last.elapsed() >= PROGRESS_GAP)
            && let Some(line) = pending.take()
        {
            progress(&line);
            last = Instant::now();
        }
    };
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            let _ = child.kill();
            let _ = child.wait();
            return failed(format!("git {} timed out after {limit:?}", args[0]));
        }
        match lines.recv_timeout(left.min(PROGRESS_GAP)) {
            Ok((line, ended)) => {
                if ended {
                    kept.push_str(&line);
                    kept.push('\n');
                }
                if !line.trim().is_empty() {
                    pending = Some(line.trim_end().to_string());
                }
                flush(&mut pending, false);
            }
            // A helper that git started can keep the pipe open after git exits.
            Err(mpsc::RecvTimeoutError::Timeout) => {
                flush(&mut pending, false);
                if matches!(child.try_wait(), Ok(Some(_))) {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    flush(&mut pending, true);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return failed(format!("git {} timed out after {limit:?}", args[0]));
            }
        }
    };
    Out {
        ok: status.success(),
        stdout: out_text
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_default(),
        stderr: kept,
    }
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
