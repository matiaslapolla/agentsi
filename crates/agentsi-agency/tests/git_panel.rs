//! The git screen logic on temporary repos. The test process ignores the global and system git
//! config of the machine, and has no `git.toml`.

use agentsi_agency::gitpanel;
use agentsi_proto::git_api::DIFF_MAX_BYTES;
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
    String::from_utf8_lossy(&out.stdout).into_owned()
}

struct Repo {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn repo() -> Repo {
    isolate();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    git(&path, &["init", "-q", "-b", "main"]);
    git(&path, &["config", "user.name", "Test"]);
    git(&path, &["config", "user.email", "test@example.invalid"]);
    git(&path, &["config", "commit.gpgsign", "false"]);
    Repo { _dir: dir, path }
}

fn write(r: &Repo, name: &str, text: &str) {
    let p = r.path.join(name);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn commit_all(r: &Repo, msg: &str) {
    git(&r.path, &["add", "-A"]);
    git(&r.path, &["commit", "-q", "-m", msg]);
}

type Row = (String, Option<String>, Option<String>);

fn rows(r: &Repo) -> Vec<Row> {
    let mut v: Vec<Row> = gitpanel::status(&r.path)
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.staged, f.unstaged))
        .collect();
    v.sort();
    v
}

fn row(path: &str, staged: Option<&str>, unstaged: Option<&str>) -> Row {
    (
        path.into(),
        staged.map(Into::into),
        unstaged.map(Into::into),
    )
}

#[test]
fn status_shows_added_changed_deleted_renamed_untracked_and_a_path_with_a_space() {
    let r = repo();
    write(&r, "keep.txt", "one\n");
    write(&r, "change.txt", "one\n");
    write(&r, "gone.txt", "one\n");
    write(
        &r,
        "old name.txt",
        "a long enough body so that git sees a rename\nline 2\nline 3\n",
    );
    commit_all(&r, "first");
    write(&r, "change.txt", "two\n");
    std::fs::remove_file(r.path.join("gone.txt")).unwrap();
    git(&r.path, &["mv", "old name.txt", "new name.txt"]);
    write(&r, "added.txt", "new\n");
    git(&r.path, &["add", "added.txt"]);
    write(&r, "dir/untracked file.txt", "x\n");

    let status = gitpanel::status(&r.path).unwrap();
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert!(status.head.is_some());
    assert_eq!(status.upstream, None);
    assert_eq!(status.worktrees.len(), 1);
    assert_eq!(status.worktrees[0].branch.as_deref(), Some("main"));
    assert_eq!(
        rows(&r),
        vec![
            row("added.txt", Some("A"), None),
            row("change.txt", None, Some("M")),
            row("dir/untracked file.txt", None, Some("?")),
            row("gone.txt", None, Some("D")),
            row("new name.txt", Some("R"), None),
        ]
    );
    let renamed = status
        .files
        .iter()
        .find(|f| f.path == "new name.txt")
        .unwrap();
    assert_eq!(renamed.orig_path.as_deref(), Some("old name.txt"));
}

#[test]
fn status_of_a_repo_with_no_commit_and_of_a_folder_that_is_no_repo() {
    let r = repo();
    write(&r, "a.txt", "a\n");
    let s = gitpanel::status(&r.path).unwrap();
    assert_eq!((s.head, s.branch.as_deref()), (None, Some("main")));
    assert_eq!(rows(&r), vec![row("a.txt", None, Some("?"))]);
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        gitpanel::status(dir.path()),
        Err(gitpanel::Error::Git(_))
    ));
}

#[test]
fn diff_gives_the_staged_the_unstaged_and_the_untracked_patch() {
    let r = repo();
    write(&r, "a b.txt", "one\n");
    commit_all(&r, "first");
    write(&r, "a b.txt", "one\ntwo\n");
    git(&r.path, &["add", "a b.txt"]);
    write(&r, "a b.txt", "one\ntwo\nthree\n");
    write(&r, "new.txt", "fresh\n");

    let staged = gitpanel::diff(&r.path, "a b.txt", true).unwrap();
    assert!(
        staged.patch.contains("+two") && !staged.patch.contains("+three"),
        "{}",
        staged.patch
    );
    let unstaged = gitpanel::diff(&r.path, "a b.txt", false).unwrap();
    assert!(unstaged.patch.contains("+three") && !unstaged.patch.contains("+two\n"));
    assert!(!staged.truncated);
    let untracked = gitpanel::diff(&r.path, "new.txt", false).unwrap();
    assert!(untracked.patch.contains("+fresh"), "{}", untracked.patch);
    assert!(untracked.patch.contains("b/new.txt"), "{}", untracked.patch);
    assert!(!untracked.patch.contains(r.path.to_str().unwrap()));
    assert_eq!(
        gitpanel::diff(&r.path, "nothing.txt", false).unwrap().patch,
        ""
    );
}

#[test]
fn a_diff_over_512_kib_arrives_cut_with_the_flag() {
    let r = repo();
    write(&r, "big.txt", "x\n");
    commit_all(&r, "first");
    write(&r, "big.txt", &"a line of text\n".repeat(60_000));
    let d = gitpanel::diff(&r.path, "big.txt", false).unwrap();
    assert!(d.truncated);
    assert!(d.patch.len() <= DIFF_MAX_BYTES && d.patch.len() > DIFF_MAX_BYTES - 4);
}

#[test]
fn stage_and_unstage_move_a_file_between_the_lists() {
    let r = repo();
    write(&r, "a.txt", "a\n");
    write(&r, "dir/b c.txt", "b\n");
    // No commit yet: the unstage must still work.
    gitpanel::stage(&r.path, &["a.txt".into(), "dir/b c.txt".into()]).unwrap();
    assert_eq!(
        rows(&r),
        vec![
            row("a.txt", Some("A"), None),
            row("dir/b c.txt", Some("A"), None)
        ]
    );
    gitpanel::unstage(&r.path, &["a.txt".into()]).unwrap();
    assert_eq!(
        rows(&r),
        vec![
            row("a.txt", None, Some("?")),
            row("dir/b c.txt", Some("A"), None)
        ]
    );
    commit_all(&r, "first");
    write(&r, "a.txt", "changed\n");
    gitpanel::stage(&r.path, &["a.txt".into()]).unwrap();
    assert_eq!(rows(&r), vec![row("a.txt", Some("M"), None)]);
    gitpanel::unstage(&r.path, &["a.txt".into()]).unwrap();
    assert_eq!(rows(&r), vec![row("a.txt", None, Some("M"))]);
}

#[test]
fn a_path_is_a_literal_path_and_stays_in_the_repo() {
    let r = repo();
    write(&r, "a.txt", "a\n");
    write(&r, "b.txt", "b\n");
    // A glob stages nothing: the path is literal.
    assert!(gitpanel::stage(&r.path, &["*.txt".into()]).is_err());
    assert_eq!(
        rows(&r),
        vec![row("a.txt", None, Some("?")), row("b.txt", None, Some("?"))]
    );
    for bad in ["../outside.txt", "/etc/hostname", "a/../../x"] {
        let e = gitpanel::stage(&r.path, &[bad.into()]).unwrap_err();
        assert!(matches!(e, gitpanel::Error::Invalid(_)), "{bad}");
        assert!(gitpanel::unstage(&r.path, &[bad.into()]).is_err());
        assert!(gitpanel::diff(&r.path, bad, false).is_err());
    }
}

#[test]
fn a_path_works_from_a_folder_inside_the_repo() {
    let r = repo();
    write(&r, "sub/a.txt", "a\n");
    commit_all(&r, "first");
    write(&r, "sub/a.txt", "b\n");
    let sub = r.path.join("sub");
    let s = gitpanel::status(&sub).unwrap();
    assert_eq!(s.files[0].path, "sub/a.txt");
    gitpanel::stage(&sub, &["sub/a.txt".into()]).unwrap();
    assert_eq!(rows(&r), vec![row("sub/a.txt", Some("M"), None)]);
}

#[test]
fn commit_gives_the_new_id_and_the_log_lists_it() {
    let r = repo();
    assert!(
        gitpanel::log(&r.path, 10).unwrap().is_empty(),
        "no commit yet"
    );
    write(&r, "a.txt", "a\n");
    gitpanel::stage(&r.path, &["a.txt".into()]).unwrap();
    let done = gitpanel::commit(&r.path, "feat: add a\n\nbody").unwrap();
    assert!(done.committed);
    let head = git(&r.path, &["rev-parse", "HEAD"]);
    assert_eq!(done.commit.as_deref(), Some(head.trim()));
    assert!(rows(&r).is_empty());
    let log = gitpanel::log(&r.path, 10).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(
        (log[0].subject.as_str(), log[0].author.as_str()),
        ("feat: add a", "Test")
    );
    assert_eq!(log[0].id, head.trim());
    assert!(log[0].time > 1_700_000_000);
    write(&r, "b.txt", "b\n");
    commit_all(&r, "second");
    assert_eq!(gitpanel::log(&r.path, 1).unwrap().len(), 1);
    assert_eq!(gitpanel::log(&r.path, 1000).unwrap().len(), 2);
}

#[test]
fn a_commit_with_an_empty_message_or_nothing_staged_does_not_commit() {
    let r = repo();
    write(&r, "a.txt", "a\n");
    assert!(matches!(
        gitpanel::commit(&r.path, "  \n"),
        Err(gitpanel::Error::Invalid(_))
    ));
    let nothing = gitpanel::commit(&r.path, "x").unwrap();
    assert!(!nothing.committed && nothing.commit.is_none());
    assert!(!nothing.output.is_empty());
}

#[test]
fn a_hook_that_exits_1_refuses_the_commit_and_its_output_comes_back() {
    use std::os::unix::fs::PermissionsExt;
    let r = repo();
    write(&r, "a.txt", "a\n");
    let hook = r.path.join(".git/hooks/pre-commit");
    std::fs::write(
        &hook,
        "#!/bin/sh\necho 'lint failed: a.txt'\necho 'second line' >&2\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    gitpanel::stage(&r.path, &["a.txt".into()]).unwrap();
    let done = gitpanel::commit(&r.path, "feat: a").unwrap();
    assert!(!done.committed && done.commit.is_none());
    assert!(
        done.output.contains("lint failed: a.txt"),
        "{}",
        done.output
    );
    assert!(done.output.contains("second line"), "{}", done.output);
    // The file stays staged, so the owner can fix the hook and commit again.
    assert_eq!(rows(&r), vec![row("a.txt", Some("A"), None)]);
}
