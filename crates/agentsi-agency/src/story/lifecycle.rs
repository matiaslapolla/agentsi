//! The story actions with no model call: `decideStory` of `story-write.ts`
//! (approve, discard and the note of a change request), and `reviewStory`,
//! `acceptStory`, `rejectStory` and `recordStoryResult` of `story-run.ts`.

use super::git::{NO_HOOKS, branch_exists, git, git_out, main_branch};
use super::record::{action_error, has_active_session, is_running_session};
use super::store::{self, read_with, transaction};
use super::{Error, Result, run};
use crate::clock;
use crate::registry::Product;
use agentsi_proto::story_api::{Story, StoryConfig, StoryResult};
use agentsi_store::Store;
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

const STOP_WAIT: Duration = Duration::from_secs(5);
pub const MERGED_RESULTS: [&str; 2] = ["merged_changed", "merged_unchanged"];

fn check(s: &Story, action: &str) -> Result<()> {
    match action_error(s, action) {
        Some(e) => Err(Error::invalid(e)),
        None => Ok(()),
    }
}

/// Loads the story, applies `change` and saves it, in one story transaction.
pub fn update(
    store: &Store,
    product: &Product,
    id: &str,
    change: impl FnOnce(Story) -> Result<Story>,
) -> Result<Story> {
    transaction(&product.id, || {
        let story = read_with(store, |c| store::load(c, &product.id, id))?;
        store::save(store, product, change(story)?)
    })
}

/// `story approve`, `story discard`, and the note of a change request.
pub fn decide(
    store: &Store,
    product: &Product,
    id: &str,
    verdict: &str,
    note: &str,
) -> Result<Story> {
    update(store, product, id, |s| match verdict {
        "approve" => {
            check(&s, "approve")?;
            Ok(Story {
                approval: "approved".into(),
                ..s
            })
        }
        "discard" => {
            check(&s, "discard")?;
            Ok(Story {
                status: "discarded".into(),
                ..s
            })
        }
        _ => request_changes(s, note),
    })
}

/// The story with the change request recorded. It writes nothing.
pub fn request_changes(s: Story, note: &str) -> Result<Story> {
    check(&s, "changes")?;
    if note.trim().is_empty() {
        return Err(Error::invalid("A change request needs a note."));
    }
    Ok(Story {
        approval: "changes_requested".into(),
        changes_note: Some(note.trim().into()),
        ..s
    })
}

pub struct Review {
    pub story: Story,
    pub stat: String,
    pub diff: String,
}

pub fn review(store: &Store, product: &str, id: &str, config: &StoryConfig) -> Result<Review> {
    let story = read_with(store, |c| store::load(c, product, id))?;
    let Some(s) = &story.session else {
        return Err(Error::invalid(format!("Story {id} has no story session.")));
    };
    let repo = Path::new(&config.repo);
    let (stat, diff) = if branch_exists(repo, &s.branch) {
        let range = format!("{}..{}", s.base, s.branch);
        (
            git_out(repo, &["diff", "--stat", &range])?,
            git(repo, &["diff", &range]).stdout,
        )
    } else {
        (String::new(), String::new())
    };
    Ok(Review { story, stat, diff })
}

pub struct Accepted {
    pub story: Story,
    pub result: String,
    pub commit: String,
}

/// Merges the story branch with `--no-ff` into the main branch of the main
/// checkout, records the result and removes the worktree and the branch.
/// It refuses a dirty main checkout or worktree.
pub fn accept(
    store: &Store,
    product: &Product,
    id: &str,
    config: &StoryConfig,
) -> Result<Accepted> {
    let repo = Path::new(&config.repo);
    transaction(&product.id, || {
        let story = read_with(store, |c| store::load(c, &product.id, id))?;
        check(&story, "accept")?;
        store::assert_valid(product, &story)?;
        let s = story
            .session
            .clone()
            .unwrap_or_else(|| unreachable!("accept needs a session"));
        let main = main_branch(repo)?;
        let current = git_out(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?;
        if current != main {
            return Err(Error::invalid(format!(
                "The main checkout {} is on {current}, not {main}.",
                config.repo
            )));
        }
        if !git_out(repo, &["status", "--porcelain", "--untracked-files=no"])?.is_empty() {
            return Err(Error::invalid(format!(
                "The main checkout {} has uncommitted changes.",
                config.repo
            )));
        }
        let wt = Path::new(&s.worktree);
        if wt.exists() && !git_out(wt, &["status", "--porcelain"])?.is_empty() {
            return Err(Error::invalid(format!(
                "The worktree {} has uncommitted changes. Commit or discard them first.",
                s.worktree
            )));
        }
        let head = git_out(repo, &["rev-parse", &s.branch])?;
        let result = if Some(&head) == s.final_commit.as_ref() {
            "merged_unchanged"
        } else {
            "merged_changed"
        };
        let message = format!("Merge {}: {}", s.branch, story.title);
        let merge = git(
            repo,
            &[
                NO_HOOKS[0],
                NO_HOOKS[1],
                "merge",
                "--no-ff",
                "-m",
                &message,
                &s.branch,
            ],
        );
        if !merge.ok {
            git(repo, &["merge", "--abort"]);
            return Err(Error::invalid(format!(
                "The merge of {} failed: {}",
                s.branch,
                format!("{}{}", merge.stdout, merge.stderr).trim()
            )));
        }
        let commit = git_out(repo, &["rev-parse", "HEAD"])?;
        if wt.exists() {
            git_out(repo, &["worktree", "remove", &s.worktree])?;
        }
        git_out(repo, &["branch", "-d", &s.branch])?;
        let mut results = story.results.clone();
        results.push(StoryResult {
            result: result.into(),
            at: clock::iso(clock::now_ms()),
            commit: Some(commit.clone()),
        });
        let saved = store::save(
            store,
            product,
            Story {
                status: "implemented".into(),
                results,
                ..story
            },
        )?;
        Ok(Accepted {
            story: saved,
            result: result.into(),
            commit,
        })
    })
}

static COMMIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[0-9a-f]{4,64}$").unwrap());

/// Records a merge that happened outside `story accept`, as accept records it.
/// It refuses a commit that is not on the main branch, and an active session.
pub fn record_result(
    store: &Store,
    product: &Product,
    id: &str,
    kind: &str,
    commit: &str,
    config: &StoryConfig,
) -> Result<Story> {
    if !MERGED_RESULTS.contains(&kind) {
        return Err(Error::invalid(format!(
            "Result \"{kind}\" is not one of {}.",
            MERGED_RESULTS.join(", ")
        )));
    }
    if !COMMIT.is_match(commit) {
        return Err(Error::invalid(format!(
            "Commit \"{commit}\" is not a commit hash."
        )));
    }
    let repo = Path::new(&config.repo);
    transaction(&product.id, || {
        let story = read_with(store, |c| store::load(c, &product.id, id))?;
        if has_active_session(&story) {
            let state = story
                .session
                .as_ref()
                .map(|s| s.state.as_str())
                .unwrap_or_default();
            return Err(Error::invalid(format!(
                "Story {id} has a story session in state {state}. Run agentsi story accept {id} or agentsi story reject {id}."
            )));
        }
        let found = git(
            repo,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{commit}^{{commit}}"),
            ],
        );
        if !found.ok {
            return Err(Error::invalid(format!(
                "Commit {commit} is not in {}.",
                config.repo
            )));
        }
        let full = found.stdout.trim().to_string();
        let main = main_branch(repo)?;
        if !git(repo, &["merge-base", "--is-ancestor", &full, &main]).ok {
            return Err(Error::invalid(format!(
                "Commit {commit} is not on the main branch {main} of {}.",
                config.repo
            )));
        }
        let mut results = story.results.clone();
        results.push(StoryResult {
            result: kind.into(),
            at: clock::iso(clock::now_ms()),
            commit: Some(full),
        });
        store::save(
            store,
            product,
            Story {
                status: "implemented".into(),
                results,
                ..story
            },
        )
    })
}

/// True while the process exists. A process of another user counts.
pub fn alive(pid: i64) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: kill with signal 0 only checks that the process exists.
    let r = unsafe { libc::kill(pid, 0) };
    r == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Kills the process group of a session process, or the process alone.
fn stop(pid: i64) -> Result<()> {
    let p = pid as libc::pid_t;
    // SAFETY: plain signal calls on a pid that the story recorded.
    unsafe {
        if libc::kill(-p, libc::SIGKILL) != 0 {
            libc::kill(p, libc::SIGKILL);
        }
    }
    let deadline = Instant::now() + STOP_WAIT;
    while alive(pid) {
        if Instant::now() > deadline {
            return Err(Error::invalid(format!("Process {pid} did not stop.")));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// True when `pid` is the detached `_exec` of this story, which the TypeScript CLI starts.
/// A pid alone says nothing: the old pid of a core can name another process now.
fn is_story_exec(pid: i64, id: &str) -> bool {
    std::fs::read(format!("/proc/{pid}/cmdline")).is_ok_and(|text| {
        let args: Vec<&[u8]> = text.split(|b| *b == 0).collect();
        args.contains(&b"_exec".as_slice()) && args.contains(&id.as_bytes())
    })
}

/// Stops a session of this core and waits for its thread, outside the lock of the
/// product, because the thread takes that lock at its next step.
fn stop_here(product: &str, id: &str) {
    run::cancel(product, id);
    let deadline = Instant::now() + STOP_WAIT;
    while run::is_running(product, id) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Records a `discarded` result, removes the worktree and the branch and clears
/// the session, so the story can run again. A session that still runs needs
/// `force`, which stops it first.
pub fn reject(
    store: &Store,
    product: &Product,
    id: &str,
    force: bool,
    config: &StoryConfig,
) -> Result<Story> {
    let repo = Path::new(&config.repo);
    if force {
        stop_here(&product.id, id);
    }
    transaction(&product.id, || {
        let story = read_with(store, |c| store::load(c, &product.id, id))?;
        check(&story, "reject")?;
        store::assert_valid(product, &story)?;
        let s = story
            .session
            .clone()
            .unwrap_or_else(|| unreachable!("reject needs a session"));
        let pid = s.pid.flatten();
        let here = run::is_running(&product.id, id);
        let elsewhere = pid.is_some_and(|p| is_story_exec(p, id));
        if is_running_session(&story) && (here || elsewhere) {
            if !force {
                return Err(Error::invalid(format!(
                    "The story session of {id} is in state {} and its process {} still runs. Wait for it, or stop it with: agentsi story reject {id} --force",
                    s.state,
                    pid.map_or("?".into(), |p| p.to_string())
                )));
            }
            if let (false, Some(pid)) = (here, pid) {
                stop(pid)?;
            }
        }
        if Path::new(&s.worktree).exists() {
            git_out(repo, &["worktree", "remove", "--force", &s.worktree])?;
        }
        git(repo, &["worktree", "prune"]);
        if branch_exists(repo, &s.branch) {
            git_out(repo, &["branch", "-D", &s.branch])?;
        }
        let mut results = story.results.clone();
        results.push(StoryResult {
            result: "discarded".into(),
            at: clock::iso(clock::now_ms()),
            commit: s.final_commit.clone(),
        });
        store::save(
            store,
            product,
            Story {
                session: None,
                results,
                ..story
            },
        )
    })
}
