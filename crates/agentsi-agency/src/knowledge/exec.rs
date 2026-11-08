//! Runs a program with a time limit: `git`, `gh` and the code graph binary.
//! The rules copy `packages/core/src/exec.ts`.

use crate::git_env;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The time that `git` gets, as `exec.ts` gives it.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(20);

pub struct Out {
    /// The program started, stopped in time, and exited with code 0.
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Reads a pipe to its end on a thread, and gives the text through a channel.
fn drain(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<String> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        let _ = send.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    receive
}

pub fn run(program: &str, args: &[&str], cwd: &Path, timeout: Duration) -> Out {
    run_env(program, args, cwd, timeout, &[])
}

/// Like `run`, with more names in the environment of the program.
pub fn run_env(
    program: &str,
    args: &[&str],
    cwd: &Path,
    timeout: Duration,
    env: &[(String, String)],
) -> Out {
    let failed = |stderr: String| Out {
        ok: false,
        stdout: String::new(),
        stderr,
    };
    let spawned = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(e) => return failed(e.to_string()),
    };
    let stdout = drain(child.stdout.take().expect("stdout is piped"));
    let stderr = drain(child.stderr.take().expect("stderr is piped"));
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            // The readers stay behind: a grandchild can hold the pipes open.
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return failed(format!("{program} timed out"));
            }
        }
    };
    // A grandchild can keep a pipe open after the child exits. Wait for the text only until the deadline.
    let left = |d: Instant| d.saturating_duration_since(Instant::now());
    let (Ok(stdout), Ok(stderr)) = (
        stdout.recv_timeout(left(deadline)),
        stderr.recv_timeout(left(deadline)),
    ) else {
        return failed(format!("{program} left a pipe open"));
    };
    Out {
        ok: status.success(),
        stdout,
        stderr,
    }
}

pub fn git(cwd: &Path, args: &[&str]) -> Out {
    run_env(
        "git",
        args,
        cwd,
        DEFAULT_TIMEOUT,
        &git_env::env_for_cwd(cwd),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grandchild_that_keeps_the_pipe_open_does_not_hold_the_run_past_its_time() {
        let started = Instant::now();
        let out = run(
            "sh",
            &["-c", "sleep 30 & echo done"],
            Path::new("."),
            Duration::from_millis(500),
        );
        assert!(!out.ok);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn a_program_that_exits_gives_its_text() {
        let out = run(
            "sh",
            &["-c", "echo out; echo err >&2"],
            Path::new("."),
            DEFAULT_TIMEOUT,
        );
        assert_eq!(
            (out.ok, out.stdout.trim(), out.stderr.trim()),
            (true, "out", "err")
        );
    }
}
