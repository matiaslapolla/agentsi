//! The login shell PATH. A core that systemd or launchd starts has a fixed `PATH`, and a
//! program that only the owner's login shell finds (a mise wrapper of `claude`) stays out of
//! it. At start the core reads the `PATH` of `$SHELL -l -i` once and adds its new folders.
//! Spec: `docs/specs/agentsi-path.md`.

use agentsi_proto::ops_api::PathSource;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// `1` turns off the read. The tests set it.
pub const NO_SHELL_PATH_ENV: &str = "AGENTSI_NO_SHELL_PATH";
pub const SHELL_TIMEOUT: Duration = Duration::from_secs(5);

const BEGIN: &str = "__AGENTSI_PATH_BEGIN__";
const END: &str = "__AGENTSI_PATH_END__";

static SOURCE: OnceLock<PathSource> = OnceLock::new();

/// Where the `PATH` of this process comes from.
pub fn source() -> PathSource {
    SOURCE.get().copied().unwrap_or(PathSource::Process)
}

/// Sets the `PATH` of this process to the merged PATH. It must run before the process
/// starts any thread, because it changes the environment.
pub fn apply() {
    let source = if std::env::var_os(NO_SHELL_PATH_ENV).is_some_and(|v| v == "1") {
        PathSource::Process
    } else {
        match login_path() {
            Ok(shell) => {
                let merged = merge(std::env::var_os("PATH").as_deref(), &shell);
                // SAFETY: the core has no other thread yet, so nothing reads the environment.
                unsafe { std::env::set_var("PATH", merged) };
                PathSource::Merged
            }
            Err(e) => {
                eprintln!("agentsi-core: the PATH stays as it is, because {e}");
                PathSource::ShellReadFailed
            }
        }
    };
    let _ = SOURCE.set(source);
}

fn login_path() -> Result<OsString, String> {
    let shell = std::env::var_os("SHELL")
        .filter(|s| !s.is_empty())
        .ok_or("SHELL is not set")?;
    read(Path::new(&shell), SHELL_TIMEOUT)
}

/// The `PATH` that `shell -l -i` prints, within `timeout`.
pub fn read(shell: &Path, timeout: Duration) -> Result<OsString, String> {
    let script = format!("printf '\\n%s\\n%s\\n%s\\n' '{BEGIN}' \"$PATH\" '{END}'");
    let mut command = Command::new(shell);
    command
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // A new session has no terminal, so an interactive shell cannot stop on job control.
    // SAFETY: setsid is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("{} does not start: {e}", shell.display()))?;
    let Some(mut stdout) = child.stdout.take() else {
        return Err("no stdout".into());
    };
    let deadline = Instant::now() + timeout;
    let mut out = Vec::new();
    let found = loop {
        if let Some(path) = between_markers(&out) {
            break Ok(path);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break Err(format!(
                "{} gave no PATH in {}s",
                shell.display(),
                timeout.as_secs()
            ));
        }
        let mut poll = libc::pollfd {
            fd: stdout.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let wait = i32::try_from(left.as_millis()).unwrap_or(i32::MAX).max(1);
        // SAFETY: one valid pollfd.
        if unsafe { libc::poll(&mut poll, 1, wait) } <= 0 {
            continue;
        }
        let mut chunk = [0u8; 4096];
        match stdout.read(&mut chunk) {
            Ok(0) => {
                break Err(format!("{} ended with no PATH", shell.display()));
            }
            Ok(n) => out.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => break Err(format!("cannot read {}: {e}", shell.display())),
        }
    };
    // The leader is not reaped yet, so its id still names its group and its session.
    kill_session(child.id() as libc::pid_t);
    let _ = child.wait();
    found
}

/// Kills the session `sid`: the shell's group, and the jobs that the shell put in groups of
/// their own. Outside Linux, only the shell's group.
fn kill_session(sid: libc::pid_t) {
    // SAFETY: kill takes plain values.
    unsafe { libc::kill(-sid, libc::SIGKILL) };
    #[cfg(target_os = "linux")]
    for _ in 0..3 {
        let members = session_members(sid);
        if members.is_empty() {
            break;
        }
        for pid in members {
            // SAFETY: kill takes plain values.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

/// The live processes of the session `sid`, from `/proc/<pid>/stat`.
#[cfg(target_os = "linux")]
fn session_members(sid: libc::pid_t) -> Vec<libc::pid_t> {
    let Ok(dir) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    dir.flatten()
        .filter_map(|entry| {
            let pid: libc::pid_t = entry.file_name().to_str()?.parse().ok()?;
            let stat = std::fs::read_to_string(entry.path().join("stat")).ok()?;
            // After the name in parentheses: state, ppid, pgrp, session.
            let mut fields = stat.get(stat.rfind(')')? + 2..)?.split(' ');
            let state = fields.next()?;
            let session: libc::pid_t = fields.nth(2)?.parse().ok()?;
            (session == sid && state != "Z" && state != "X").then_some(pid)
        })
        .collect()
}

/// The line between the markers. An interactive shell can print other lines around them.
fn between_markers(out: &[u8]) -> Option<OsString> {
    let mut lines = out
        .split(|b| *b == b'\n')
        .map(|l| l.strip_suffix(b"\r").unwrap_or(l));
    lines.find(|l| *l == BEGIN.as_bytes())?;
    let path = lines.next()?;
    (lines.next()? == END.as_bytes() && !path.is_empty())
        .then(|| OsStr::from_bytes(path).to_os_string())
}

/// The folders of `process`, then each folder of `shell` that is not in it yet.
pub fn merge(process: Option<&OsStr>, shell: &OsStr) -> OsString {
    let process: Vec<PathBuf> = process
        .map(|p| std::env::split_paths(p).collect())
        .unwrap_or_default();
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dir in process.into_iter().chain(std::env::split_paths(shell)) {
        if !dir.as_os_str().is_empty() && !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    std::env::join_paths(dirs).unwrap_or_else(|_| shell.to_os_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fake_shell(dir: &Path, body: &str) -> PathBuf {
        let file = dir.join("fake-shell");
        std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        file
    }

    #[test]
    fn the_merged_path_keeps_the_process_folders_first_and_no_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        // The fake runs the command of the core (`-l -i -c <command>`) after its noise.
        let shell = fake_shell(
            dir.path(),
            "echo 'Welcome back'\necho '__AGENTSI_PATH_BEGIN__ is not alone here'\n\
             PATH=/shell/a:/usr/bin:/shell/b:/shell/a\nexport PATH\neval \"$4\"\necho bye",
        );
        let shell_path = read(&shell, SHELL_TIMEOUT).unwrap();
        assert_eq!(shell_path, "/shell/a:/usr/bin:/shell/b:/shell/a");
        let merged = merge(Some(OsStr::new("/usr/bin:/bin:/usr/bin")), &shell_path);
        assert_eq!(merged, "/usr/bin:/bin:/shell/a:/shell/b");
    }

    #[test]
    fn a_slow_shell_stops_at_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "sleep 30");
        let start = Instant::now();
        let err = read(&shell, SHELL_TIMEOUT).unwrap_err();
        assert!(
            start.elapsed() < Duration::from_secs(6),
            "{:?}",
            start.elapsed()
        );
        assert!(err.contains("gave no PATH in 5s"), "{err}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_job_in_its_own_group_ends_with_the_shell() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("job");
        let shell = dir.path().join("job-shell");
        std::fs::write(
            &shell,
            format!(
                "#!/bin/bash\nset -m\nsleep 100 &\necho $! > {}\neval \"$4\"\n",
                pid_file.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
        read(&shell, SHELL_TIMEOUT).unwrap();
        let job: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // The job's parent is gone, so init reaps it: it is gone or a zombie for a moment.
        let deadline = Instant::now() + Duration::from_secs(5);
        while std::fs::read_to_string(format!("/proc/{job}/stat"))
            .is_ok_and(|s| s.contains("sleep") && !s.contains(") Z "))
        {
            assert!(Instant::now() < deadline, "the job {job} still runs");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn a_shell_with_no_markers_or_no_program_fails() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "echo hello");
        assert!(
            read(&shell, SHELL_TIMEOUT)
                .unwrap_err()
                .contains("ended with no PATH")
        );
        assert!(
            read(&dir.path().join("none"), SHELL_TIMEOUT)
                .unwrap_err()
                .contains("does not start")
        );
    }

    #[test]
    fn the_markers_need_a_path_between_them() {
        assert_eq!(
            between_markers(b"x\n__AGENTSI_PATH_BEGIN__\n/a:/b\r\n__AGENTSI_PATH_END__\n"),
            Some("/a:/b".into())
        );
        assert_eq!(
            between_markers(b"__AGENTSI_PATH_BEGIN__\n\n__AGENTSI_PATH_END__\n"),
            None
        );
        assert_eq!(between_markers(b"__AGENTSI_PATH_BEGIN__\n/a\n"), None);
    }

    #[test]
    fn a_process_with_no_path_takes_the_shell_folders() {
        assert_eq!(merge(None, OsStr::new("/a::/b")), "/a:/b");
    }
}
