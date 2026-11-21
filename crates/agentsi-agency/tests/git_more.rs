//! The summary, the diff of a commit, branches, switch, fetch, pull and push on temporary
//! repos, with a local bare repo as the remote. The test process ignores the global and
//! system git config of the machine, and has no `git.toml`.

use agentsi_agency::gitpanel::{self, Error};
use agentsi_proto::git_api::GitOutcome;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

static INIT: Once = Once::new();

fn isolate() {
    INIT.call_once(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        // SAFETY: the first call in this test binary runs before any git child starts, and
        // each test calls `isolate` first. The values never change after that.
        unsafe {
            std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
            std::env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");
            std::env::set_var("XDG_CONFIG_HOME", &dir);
        }
    });
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn configure(path: &Path) {
    git(path, &["config", "user.name", "Test"]);
    git(path, &["config", "user.email", "test@example.invalid"]);
    git(path, &["config", "commit.gpgsign", "false"]);
}

/// A bare remote and two clones of it, `a` and `b`, with one commit on `main`.
struct Net {
    _dir: tempfile::TempDir,
    a: PathBuf,
    b: PathBuf,
}

fn net() -> Net {
    isolate();
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let bare = base.join("remote.git");
    git(&base, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
    let seed = base.join("seed");
    std::fs::create_dir(&seed).unwrap();
    git(&seed, &["init", "-q", "-b", "main"]);
    configure(&seed);
    write(&seed, "a.txt", "one\n");
    commit_all(&seed, "first");
    // A file URL makes git use the pack protocol, which prints progress.
    let url = format!("file://{}", bare.display());
    git(&seed, &["remote", "add", "origin", &url]);
    git(&seed, &["push", "-q", "origin", "main"]);
    let clone = |name: &str| {
        git(&base, &["clone", "-q", &url, name]);
        let p = base.join(name);
        configure(&p);
        p
    };
    let (a, b) = (clone("a"), clone("b"));
    Net { _dir: dir, a, b }
}

fn write(dir: &Path, name: &str, text: &str) {
    let p = dir.join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn commit_all(dir: &Path, msg: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
}

fn head(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD"])
}

fn quiet(_: &str) {}

#[test]
fn the_summary_gives_the_branch_the_count_and_ahead_and_behind() {
    let n = net();
    let s = gitpanel::summary(&n.a).unwrap().unwrap();
    assert_eq!(s.branch.as_deref(), Some("main"));
    assert_eq!(s.upstream.as_deref(), Some("origin/main"));
    assert_eq!((s.ahead, s.behind, s.changed), (Some(0), Some(0), 0));
    assert_eq!(Path::new(&s.repo), n.a);

    write(&n.a, "a.txt", "two\n");
    write(&n.a, "new/x.txt", "x\n");
    write(&n.a, "new/y.txt", "y\n");
    let s = gitpanel::summary(&n.a.join("new")).unwrap().unwrap();
    // The new folder counts once, as in the normal mode of git status.
    assert_eq!(s.changed, 2);

    let outside = tempfile::tempdir().unwrap();
    assert_eq!(gitpanel::summary(outside.path()).unwrap(), None);
}

#[test]
fn show_gives_the_files_and_the_patch_of_a_commit_and_of_the_first_commit() {
    let n = net();
    let first = head(&n.a);
    write(&n.a, "a.txt", "one\ntwo\n");
    write(&n.a, "b c.txt", "new\n");
    git(&n.a, &["mv", "a.txt", "moved.txt"]);
    commit_all(&n.a, "feat: more");
    let r = gitpanel::show(&n.a, &head(&n.a)).unwrap();
    assert_eq!(r.commit.subject, "feat: more");
    let mut files: Vec<_> = r
        .files
        .iter()
        .map(|f| (f.letter.as_str(), f.path.as_str(), f.orig_path.as_deref()))
        .collect();
    files.sort();
    assert_eq!(
        files,
        vec![("A", "b c.txt", None), ("R", "moved.txt", Some("a.txt"))]
    );
    assert!(r.patch.contains("+two"), "{}", r.patch);
    assert!(!r.truncated);

    let root = gitpanel::show(&n.a, &first).unwrap();
    assert_eq!(root.files.len(), 1);
    assert_eq!(root.files[0].letter, "A");
    assert!(root.patch.contains("+one"));

    for bad in ["--all", "main..HEAD", "", "nope"] {
        assert!(
            matches!(gitpanel::show(&n.a, bad), Err(Error::Invalid(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn branches_lists_the_current_branch_first_the_remote_ones_and_the_worktrees() {
    let n = net();
    gitpanel::branch_create(&n.a, "aaa", None).unwrap();
    gitpanel::branch_create(&n.a, "from-remote", Some("origin/main")).unwrap();
    assert!(matches!(
        gitpanel::branch_create(&n.a, "aaa", None),
        Err(Error::Invalid(_))
    ));
    for bad in ["-x", "a b", "a..b", ""] {
        assert!(
            matches!(
                gitpanel::branch_create(&n.a, bad, None),
                Err(Error::Invalid(_))
            ),
            "{bad:?}"
        );
    }
    write(&n.a, "a.txt", "two\n");
    commit_all(&n.a, "ahead");
    let wt = n.a.parent().unwrap().join("wt");
    git(
        &n.a,
        &["worktree", "add", "-q", wt.to_str().unwrap(), "aaa"],
    );

    let r = gitpanel::branches(&n.a).unwrap();
    let local: Vec<_> = r.local.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(local, ["main", "aaa", "from-remote"]);
    assert!(r.local[0].current);
    assert_eq!(r.local[0].upstream.as_deref(), Some("origin/main"));
    assert_eq!((r.local[0].ahead, r.local[0].behind), (Some(1), Some(0)));
    assert_eq!(r.local[0].subject, "ahead");
    assert_eq!(r.local[1].upstream, None);
    // A branch from a remote branch tracks nothing, so a push cannot send it there.
    assert_eq!(r.local[2].upstream, None);
    let remote: Vec<_> = r.remote.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(remote, ["origin/main"]);
    assert_eq!(r.worktrees.len(), 2);
    assert_eq!(r.worktrees[1].branch.as_deref(), Some("aaa"));
}

#[test]
fn a_switch_with_a_clean_tree_works_and_a_switch_with_changes_gives_dirty() {
    let n = net();
    gitpanel::branch_create(&n.a, "feat", None).unwrap();
    // An untracked file does not stop a switch.
    write(&n.a, "untracked.txt", "u\n");
    let r = gitpanel::switch(&n.a, "feat").unwrap();
    assert_eq!(r.outcome, GitOutcome::Done, "{}", r.output);
    assert_eq!(git(&n.a, &["branch", "--show-current"]), "feat");

    write(&n.a, "a.txt", "changed\n");
    let r = gitpanel::switch(&n.a, "main").unwrap();
    assert_eq!(r.outcome, GitOutcome::Dirty);
    let paths: Vec<_> = r.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["a.txt"]);
    assert_eq!(git(&n.a, &["branch", "--show-current"]), "feat");
    assert_eq!(
        std::fs::read_to_string(n.a.join("a.txt")).unwrap(),
        "changed\n"
    );

    assert!(matches!(
        gitpanel::switch(&n.a, "nope"),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn a_switch_to_a_remote_branch_makes_a_local_branch_that_tracks_it() {
    let n = net();
    git(&n.b, &["switch", "-q", "-c", "topic"]);
    write(&n.b, "t.txt", "t\n");
    commit_all(&n.b, "topic");
    git(&n.b, &["push", "-q", "origin", "topic"]);
    let r = gitpanel::fetch(&n.a, &mut quiet);
    assert_eq!(r.outcome, GitOutcome::Done, "{}", r.output);
    let r = gitpanel::switch(&n.a, "origin/topic").unwrap();
    assert_eq!(r.outcome, GitOutcome::Done, "{}", r.output);
    assert_eq!(git(&n.a, &["branch", "--show-current"]), "topic");
    assert_eq!(
        git(&n.a, &["rev-parse", "--abbrev-ref", "topic@{upstream}"]),
        "origin/topic"
    );
}

#[test]
fn a_switch_to_a_remote_branch_whose_local_branch_exists_switches_to_the_local_one() {
    let n = net();
    git(&n.a, &["switch", "-q", "-c", "other"]);
    let r = gitpanel::switch(&n.a, "origin/main").unwrap();
    assert_eq!(r.outcome, GitOutcome::Done, "{}", r.output);
    assert_eq!(git(&n.a, &["branch", "--show-current"]), "main");
}

#[test]
fn a_push_to_an_upstream_of_another_name_and_a_push_after_the_head_moved_are_refused() {
    let n = net();
    git(
        &n.a,
        &["switch", "-q", "-c", "feature", "--track", "origin/main"],
    );
    write(&n.a, "f.txt", "f\n");
    commit_all(&n.a, "feature");
    assert!(matches!(gitpanel::outgoing(&n.a), Err(Error::Invalid(_))));
    assert!(matches!(
        gitpanel::push(&n.a, None, &mut quiet),
        Err(Error::Invalid(_))
    ));
    let main = git(&n.b, &["rev-parse", "HEAD"]);
    assert_eq!(git(&n.a, &["ls-remote", "origin", "main"])[..40], main);

    git(&n.a, &["switch", "-q", "main"]);
    write(&n.a, "a.txt", "two\n");
    commit_all(&n.a, "one out");
    let o = gitpanel::outgoing(&n.a).unwrap();
    assert_eq!(o.head, head(&n.a));
    write(&n.a, "a.txt", "three\n");
    commit_all(&n.a, "not confirmed");
    assert!(matches!(
        gitpanel::push(&n.a, Some(("main", &o.head)), &mut quiet),
        Err(Error::Invalid(_))
    ));
    assert_eq!(git(&n.a, &["ls-remote", "origin", "main"])[..40], main);
    let now = head(&n.a);
    let r = gitpanel::push(&n.a, Some(("main", &now)), &mut quiet).unwrap();
    assert_eq!(r.outcome, GitOutcome::Done, "{}", r.output);
}

#[test]
fn fetch_and_pull_bring_the_commits_of_the_remote() {
    let n = net();
    write(&n.b, "b.txt", "b\n");
    commit_all(&n.b, "from b");
    git(&n.b, &["push", "-q"]);

    let r = gitpanel::fetch(&n.a, &mut quiet);
    assert_eq!(r.outcome, GitOutcome::Done, "{}", r.output);
    let s = gitpanel::summary(&n.a).unwrap().unwrap();
    assert_eq!((s.ahead, s.behind), (Some(0), Some(1)));

    let mut lines = Vec::new();
    let r = gitpanel::pull(&n.a, &mut |l: &str| lines.push(l.to_string()));
    assert_eq!(r.outcome, GitOutcome::Done, "{}", r.output);
    assert_eq!(head(&n.a), head(&n.b));
    assert!(n.a.join("b.txt").exists());
}

#[test]
fn a_pull_that_cannot_fast_forward_gives_diverged_and_leaves_the_tree() {
    let n = net();
    write(&n.b, "b.txt", "b\n");
    commit_all(&n.b, "from b");
    git(&n.b, &["push", "-q"]);
    write(&n.a, "a.txt", "mine\n");
    commit_all(&n.a, "from a");
    write(&n.a, "a.txt", "work in progress\n");
    let before = head(&n.a);

    let r = gitpanel::pull(&n.a, &mut quiet);
    assert_eq!(r.outcome, GitOutcome::Diverged, "{}", r.output);
    assert_eq!(head(&n.a), before);
    assert!(!n.a.join("b.txt").exists());
    assert_eq!(
        std::fs::read_to_string(n.a.join("a.txt")).unwrap(),
        "work in progress\n"
    );
    assert_eq!(git(&n.a, &["status", "--porcelain"]), "M a.txt");
}

#[test]
fn push_lists_the_outgoing_commits_and_sets_the_upstream_of_a_new_branch() {
    let n = net();
    write(&n.a, "a.txt", "two\n");
    commit_all(&n.a, "one out");
    write(&n.a, "a.txt", "three\n");
    commit_all(&n.a, "two out");
    let o = gitpanel::outgoing(&n.a).unwrap();
    assert_eq!(
        (
            o.branch.as_str(),
            o.remote.as_str(),
            o.remote_branch.as_str()
        ),
        ("main", "origin", "main")
    );
    assert!(!o.set_upstream);
    assert_eq!(o.count, 2);
    let subjects: Vec<_> = o.commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["two out", "one out"]);

    let mut lines = Vec::new();
    let r = gitpanel::push(&n.a, None, &mut |l: &str| lines.push(l.to_string())).unwrap();
    assert_eq!(r.outcome, GitOutcome::Done, "{}", r.output);
    assert!(!lines.is_empty(), "the push sent no progress line");
    assert_eq!(
        git(&n.b, &["ls-remote", "origin", "main"])[..40],
        head(&n.a)
    );
    assert_eq!(gitpanel::outgoing(&n.a).unwrap().count, 0);

    git(&n.a, &["switch", "-q", "-c", "new-branch"]);
    write(&n.a, "n.txt", "n\n");
    commit_all(&n.a, "on new");
    let o = gitpanel::outgoing(&n.a).unwrap();
    assert!(o.set_upstream);
    assert_eq!(o.count, 1);
    let r = gitpanel::push(&n.a, None, &mut quiet).unwrap();
    assert_eq!(r.outcome, GitOutcome::Done, "{}", r.output);
    assert_eq!(
        git(
            &n.a,
            &["rev-parse", "--abbrev-ref", "new-branch@{upstream}"]
        ),
        "origin/new-branch"
    );
}

#[test]
fn a_push_that_the_remote_refuses_fails_and_does_not_force() {
    let n = net();
    write(&n.b, "b.txt", "b\n");
    commit_all(&n.b, "from b");
    git(&n.b, &["push", "-q"]);
    write(&n.a, "a.txt", "mine\n");
    commit_all(&n.a, "from a");
    let r = gitpanel::push(&n.a, None, &mut quiet).unwrap();
    assert_eq!(r.outcome, GitOutcome::Failed);
    assert!(r.output.contains("rejected"), "{}", r.output);
    assert_eq!(
        git(&n.a, &["ls-remote", "origin", "main"])[..40],
        head(&n.b)
    );

    git(&n.a, &["switch", "-q", "--detach"]);
    assert!(matches!(
        gitpanel::push(&n.a, None, &mut quiet),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn no_git_call_of_the_panel_forces() {
    let source = include_str!("../src/gitpanel.rs");
    let force = concat!("--", "force");
    assert!(!source.contains(force));
    assert!(!source.contains("\"-f\""));
}
