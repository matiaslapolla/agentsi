//! The runner sandbox of ADR 0002, a port of `packages/core/src/sandbox.ts`:
//! bubblewrap with a read-only root, the worktree writable, its `node_modules`
//! read-only, a private `/tmp`, `/run` hidden, the secret folders masked and no
//! network. The argument list is the TypeScript one plus `--unshare-pid`; a test
//! compares them.
//!
//! bubblewrap is Linux only. On another system `works` is false, and a story
//! run refuses (see `docs/adr/0002-story-sessions-write-product-repos.md`).

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

pub const BWRAP: &str = "bwrap";

/// Folders of the home dir that hold the owner's keys, tokens and tool settings.
const SECRET_DIRS: [&str; 5] = [".ssh", ".aws", ".gnupg", ".config", ".claude"];
/// Folders inside the secret folders that commands still read: git takes the commit identity from `~/.config/git`.
const KEPT_DIRS: [&str; 1] = [".config/git"];

/// Paths that a sandboxed command must not read, and the paths inside them that it still reads.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Mask {
    pub hide: Vec<String>,
    pub keep: Vec<String>,
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[derive(PartialEq)]
enum Kind {
    Dir,
    File,
}

/// What `statSync` sees: it follows a symbolic link.
fn kind(p: &str) -> Option<Kind> {
    let m = std::fs::metadata(p).ok()?;
    Some(if m.is_dir() { Kind::Dir } else { Kind::File })
}

/// The `node_modules` folders of a checkout, without the ones nested in another `node_modules`.
pub fn node_modules_dirs(dir: &Path) -> Vec<String> {
    fn walk(d: &Path, found: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(d) else {
            return;
        };
        for e in entries.flatten() {
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
            let name = e.file_name();
            if !is_dir || name == ".git" {
                continue;
            }
            if name == "node_modules" {
                found.push(s(&e.path()));
            } else {
                walk(&e.path(), found);
            }
        }
    }
    let mut found = Vec::new();
    if dir.exists() {
        walk(dir, &mut found);
    }
    found.sort();
    found
}

/// The `.env*` files of a checkout, outside `node_modules`, `.git` and
/// `.worktrees`. A `.env*` symlink gives its own path and the file it points
/// to: bwrap cannot mount on a symlink, so `bwrap_args` hides the target. A
/// folder that cannot be read is skipped.
pub fn env_files(dir: &Path) -> Vec<String> {
    fn walk(d: &Path, found: &mut BTreeSet<String>) {
        let Ok(entries) = std::fs::read_dir(d) else {
            return;
        };
        for e in entries.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() && ![".git", "node_modules", ".worktrees"].contains(&name.as_str()) {
                walk(&path, found);
            } else if t.is_file() && name.starts_with(".env") {
                found.insert(s(&path));
            } else if t.is_symlink()
                && name.starts_with(".env")
                && kind(&s(&path)) == Some(Kind::File)
            {
                found.insert(s(&path));
                if let Ok(real) = std::fs::canonicalize(&path) {
                    found.insert(s(&real));
                }
            }
        }
    }
    let mut found = BTreeSet::new();
    if dir.exists() {
        walk(dir, &mut found);
    }
    found.into_iter().collect()
}

/// True when `path` is `dir` or one of its parent folders.
fn holds(path: &str, dir: &str) -> bool {
    let with_sep = if path.ends_with('/') {
        path.to_string()
    } else {
        format!("{path}/")
    };
    dir == path || dir.starts_with(&with_sep)
}

/// The mask of a story on `repo`: the secret folders, the data root and the
/// `.env*` files of `repo`. A path that is `repo`, holds it or holds its
/// worktrees stays visible: a worktree reads the git data of `repo` through
/// its `.git` file.
pub fn story_mask(repo: &Path, home: &Path, root: &Path) -> Mask {
    let repo_s = s(repo);
    let worktrees = s(&repo.join(".worktrees"));
    let hide = SECRET_DIRS
        .iter()
        .map(|d| s(&home.join(d)))
        .chain(std::iter::once(s(root)))
        .chain(env_files(repo))
        .filter(|p| !holds(p, &repo_s) && !holds(p, &worktrees))
        .collect();
    Mask {
        hide,
        keep: KEPT_DIRS.iter().map(|d| s(&home.join(d))).collect(),
    }
}

/// The real paths, each one once, in the first order.
fn real(paths: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for p in paths {
        let r = std::fs::canonicalize(&p).map(|r| s(&r)).unwrap_or(p);
        if seen.insert(r.clone()) {
            out.push(r);
        }
    }
    out
}

/// The bwrap arguments that run `command` with only `worktree` writable: a
/// read-only root, the `read_only` folders bound read-only again inside the
/// worktree, a private /tmp, /dev and /proc, and no network. Each hidden
/// folder reads as an empty read-only folder. Each hidden file is bound to
/// /dev/null on a nodev mount, so a read of it fails. /run is hidden too: it
/// holds the sockets of docker, dbus and the owner's agents, and a read-only
/// bind does not stop a connect to a socket. The command runs in a pid namespace of its own.
pub fn bwrap_args(worktree: &str, command: &str, read_only: &[String], mask: &Mask) -> Vec<String> {
    let dirs = real(
        mask.hide
            .iter()
            .filter(|p| kind(p) == Some(Kind::Dir))
            .cloned(),
    );
    let files = real(
        mask.hide
            .iter()
            .filter(|p| kind(p) == Some(Kind::File))
            .cloned(),
    );
    let keep = real(mask.keep.iter().filter(|p| kind(p).is_some()).cloned());
    let mut a: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| a.extend(items.iter().map(|i| i.to_string()));
    push(&["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc"]);
    push(&["--tmpfs", "/tmp", "--tmpfs", "/run"]);
    for d in &dirs {
        push(&["--tmpfs", d]);
    }
    for f in &files {
        push(&["--ro-bind", "/dev/null", f]);
    }
    for d in &keep {
        push(&["--ro-bind", d, d]);
    }
    // After the masks, so that a worktree inside a hidden folder stays visible.
    push(&["--bind", worktree, worktree]);
    for d in read_only {
        push(&["--ro-bind", d, d]);
    }
    // The remount is not recursive, so the binds inside a hidden folder keep their own access.
    for d in &dirs {
        push(&["--remount-ro", d]);
    }
    // The owner chose it over `sandbox.ts`: a child that leaves the process group still ends
    // with the sandbox, because the kernel kills every process of a pid namespace with its init.
    push(&[
        "--unshare-net",
        "--unshare-pid",
        "--die-with-parent",
        "--new-session",
    ]);
    push(&["--chdir", worktree, "--", "sh", "-c", command]);
    a
}

/// The end of a command run.
#[derive(Debug, Clone)]
pub struct Output {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// How long a finished command may keep its pipes open. A child outside its group holds
/// them: `bwrap --new-session` moves the command to a session of its own.
const DRAIN: Duration = Duration::from_secs(2);

/// Kills `command` with SIGKILL when the thread that starts it ends, so a session that
/// the core leaves behind spends nothing more.
pub fn die_with_parent(command: &mut Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: prctl is safe to call between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = command;
}

/// The bytes of a pipe, read on a thread, and a signal at its end.
struct Pipe {
    bytes: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}

impl Pipe {
    fn read(mut source: impl Read + Send + 'static) -> Pipe {
        let bytes: Arc<Mutex<Vec<u8>>> = Arc::default();
        let (tx, done) = mpsc::channel();
        let sink = bytes.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n) = source.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                sink.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend_from_slice(&chunk[..n]);
            }
            let _ = tx.send(());
        });
        Pipe { bytes, done }
    }

    fn text(self) -> String {
        let _ = self.done.recv_timeout(DRAIN);
        let bytes = self.bytes.lock().unwrap_or_else(|e| e.into_inner());
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

/// How a child ended. `code` is `None` for a signal or a timeout.
pub struct Finished {
    pub ok: bool,
    pub code: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Waits for `child` with a timeout, and then kills its process group. `on_start` gets
/// the pid of the group, and `None` at the end.
pub fn finish(mut child: Child, timeout: Duration, on_start: &dyn Fn(Option<u32>)) -> Finished {
    on_start(Some(child.id()));
    let out = child.stdout.take().map(Pipe::read);
    let err = child.stderr.take().map(Pipe::read);
    let deadline = Instant::now() + timeout;
    let (status, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (Some(status), false),
            Ok(None) if Instant::now() > deadline => {
                kill_group(child.id());
                let _ = child.wait();
                break (None, true);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => break (None, false),
        }
    };
    on_start(None);
    Finished {
        ok: status.is_some_and(|s| s.success()),
        code: status.and_then(|s| s.code()),
        timed_out,
        stdout: out.map(Pipe::text).unwrap_or_default(),
        stderr: err.map(Pipe::text).unwrap_or_default(),
    }
}

/// Runs `program` in `cwd` in a process group of its own, with a timeout.
/// `on_start` gets the pid of the group, so a caller can stop it.
pub fn run_command(
    program: &str,
    args: &[String],
    cwd: &Path,
    timeout: Duration,
    on_start: &dyn Fn(Option<u32>),
) -> Output {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    die_with_parent(&mut command);
    let child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Output {
                ok: false,
                stdout: String::new(),
                stderr: e.to_string(),
            };
        }
    };
    let f = finish(child, timeout, on_start);
    let mut stderr = f.stderr;
    if f.timed_out {
        stderr.push_str(&format!("\nTimed out after {}s.", timeout.as_secs()));
    }
    Output {
        ok: f.ok,
        stdout: f.stdout,
        stderr,
    }
}

/// Kills the process group that `run_command` made.
pub fn kill_group(pgid: u32) {
    if let Ok(p) = libc::pid_t::try_from(pgid) {
        // SAFETY: a signal to a process group that this process started.
        unsafe {
            libc::kill(-p, libc::SIGKILL);
        }
    }
}

/// Runs `command` in the worktree sandbox. Its `node_modules` folders stay read-only, and
/// the paths that `mask` hides read as empty. The `.git` file of the worktree stays
/// read-only too: the runner runs git on the host, and a changed link would make it read
/// a config that the command wrote.
pub fn sandboxed(
    worktree: &Path,
    command: &str,
    timeout: Duration,
    mask: &Mask,
    on_start: &dyn Fn(Option<u32>),
) -> Output {
    let mut read_only = node_modules_dirs(worktree);
    let link = worktree.join(".git");
    if link.exists() {
        read_only.push(s(&link));
    }
    let args = bwrap_args(&s(worktree), command, &read_only, mask);
    run_command(BWRAP, &args, worktree, timeout, on_start)
}

/// True when bwrap is on the PATH and can start a sandbox with no network.
pub fn works() -> bool {
    let tmp = std::env::temp_dir();
    let args = bwrap_args(&s(&tmp), "true", &[], &Mask::default());
    run_command(BWRAP, &args, &tmp, Duration::from_secs(30), &|_| {}).ok
}

/// The home dir of the owner of the process.
pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timeout_returns_though_a_child_outside_the_group_holds_the_pipe() {
        let start = Instant::now();
        let args: Vec<String> = ["-c", "setsid sleep 20 & echo started; sleep 20"]
            .iter()
            .map(|a| a.to_string())
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let r = run_command("sh", &args, dir.path(), Duration::from_millis(300), &|_| {});
        assert!(!r.ok);
        assert_eq!(r.stdout.trim(), "started", "the output so far stays");
        assert!(r.stderr.contains("Timed out"));
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_child_dies_with_the_thread_that_started_it() {
        let mut child = std::thread::spawn(|| {
            let mut c = Command::new("sleep");
            c.arg("20");
            die_with_parent(&mut c);
            c.spawn().unwrap()
        })
        .join()
        .unwrap();
        let start = Instant::now();
        let status = child.wait().unwrap();
        assert_eq!(status.code(), None, "a signal ended it");
        assert!(start.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_path_that_holds_the_repo_stays_visible() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let repo = home.join(".config/code/app");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::write(repo.join(".env"), "K=1").unwrap();
        let mask = story_mask(&repo, &home, &dir.path().join("root"));
        assert!(!mask.hide.contains(&s(&home.join(".config"))));
        assert!(mask.hide.contains(&s(&home.join(".ssh"))));
        assert!(mask.hide.contains(&s(&repo.join(".env"))));
        assert_eq!(mask.keep, vec![s(&home.join(".config/git"))]);
    }

    #[test]
    fn the_masks_come_before_the_worktree_and_the_remounts_after() {
        let dir = tempfile::tempdir().unwrap();
        let hidden = dir.path().join("secret");
        std::fs::create_dir_all(&hidden).unwrap();
        let file = dir.path().join(".env");
        std::fs::write(&file, "K=1").unwrap();
        let mask = Mask {
            hide: vec![s(&hidden), s(&file), s(&dir.path().join("missing"))],
            keep: vec![],
        };
        let a = bwrap_args("/wt", "true", &["/wt/node_modules".into()], &mask);
        let at = |x: &str| a.iter().position(|y| y == x).unwrap();
        assert!(at("--tmpfs") < at("--bind"));
        assert!(at("--bind") < at("--remount-ro"));
        assert!(a.contains(&"--unshare-net".to_string()));
        assert!(!a.iter().any(|x| x.ends_with("missing")));
        assert_eq!(&a[a.len() - 4..], ["--", "sh", "-c", "true"]);
    }
}
