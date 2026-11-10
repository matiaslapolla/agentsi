//! The long-lived desk process: one `claude -p --input-format stream-json` that reads owner
//! messages on stdin and writes stream-json lines on stdout.

use crate::sandbox::{die_with_parent, kill_group};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The bytes of stderr that the process keeps, for the message of a failure.
const STDERR_KEEP: usize = 4096;

/// The tries of a spawn that fails with `ETXTBSY`.
const BUSY_TRIES: u32 = 5;

/// Spawns `command`, and tries again after a short pause while the program file is busy
/// (`ETXTBSY`): another thread's fork still holds the file open for writing, or `claude`
/// is updating itself.
pub fn spawn(command: &mut Command) -> std::io::Result<Child> {
    let mut pause = Duration::from_millis(25);
    for _ in 1..BUSY_TRIES {
        match command.spawn() {
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                std::thread::sleep(pause);
                pause *= 2;
            }
            other => return other,
        }
    }
    command.spawn()
}

/// What the process gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcEvent {
    /// One stdout line, without its newline.
    Line(String),
    /// Stdout closed: the process stopped. `stderr` holds its last bytes.
    Closed { stderr: String },
}

pub struct DeskProcess {
    child: Child,
    pid: u32,
    stdin: Option<ChildStdin>,
    events: Receiver<ProcEvent>,
    closed: bool,
}

impl DeskProcess {
    /// Starts `program` in its own process group. The process gets SIGKILL when the thread
    /// that calls `start` ends, so that thread must live as long as the process.
    pub fn start(
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
    ) -> std::io::Result<DeskProcess> {
        use std::os::unix::process::CommandExt;
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(cwd)
            .envs(env.iter().map(|(k, v)| (k, v)))
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        die_with_parent(&mut command);
        let mut child = spawn(&mut command)?;
        let pid = child.id();
        let stdin = child.stdin.take();
        let stderr = keep_tail(child.stderr.take().expect("stderr is piped"));
        let stdout = child.stdout.take().expect("stdout is piped");
        let (send, events) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut bytes = Vec::new();
            loop {
                bytes.clear();
                match reader.read_until(b'\n', &mut bytes) {
                    Ok(n) if n > 0 => {
                        let line = String::from_utf8_lossy(&bytes).trim_end().to_string();
                        if send.send(ProcEvent::Line(line)).is_err() {
                            return;
                        }
                    }
                    _ => break,
                }
            }
            // The stderr reader ends soon after stdout. A child that keeps stderr open
            // gives only what it wrote so far.
            std::thread::sleep(Duration::from_millis(50));
            let tail = stderr.lock().map(|b| b.clone()).unwrap_or_default();
            let _ = send.send(ProcEvent::Closed {
                stderr: String::from_utf8_lossy(&tail).into_owned(),
            });
        });
        Ok(DeskProcess {
            child,
            pid,
            stdin,
            events,
            closed: false,
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Writes one stdin line, such as `args::user_line`.
    pub fn send(&mut self, line: &str) -> std::io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| std::io::Error::other("the stdin of the process is closed"))?;
        stdin.write_all(line.as_bytes())?;
        stdin.flush()
    }

    /// The next event, or `None` when none came within `wait`. After `Closed`, it gives
    /// `None` at once.
    pub fn next(&mut self, wait: Duration) -> Option<ProcEvent> {
        if self.closed {
            return None;
        }
        match self.events.recv_timeout(wait) {
            Ok(e) => {
                self.closed = matches!(e, ProcEvent::Closed { .. });
                Some(e)
            }
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                self.closed = true;
                Some(ProcEvent::Closed {
                    stderr: String::new(),
                })
            }
        }
    }

    /// Stops the process group and waits for the process.
    pub fn kill(&mut self) {
        self.stdin = None;
        kill_group(self.pid);
        let _ = self.child.wait();
    }

    /// The exit code, once the process stopped.
    pub fn exit_code(&mut self) -> Option<i32> {
        self.child.try_wait().ok().flatten().and_then(|s| s.code())
    }
}

impl Drop for DeskProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

fn keep_tail(mut pipe: impl Read + Send + 'static) -> Arc<Mutex<Vec<u8>>> {
    let tail: Arc<Mutex<Vec<u8>>> = Arc::default();
    let sink = tail.clone();
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = pipe.read(&mut chunk) {
            if n == 0 {
                break;
            }
            let mut kept = sink.lock().unwrap_or_else(|e| e.into_inner());
            kept.extend_from_slice(&chunk[..n]);
            let over = kept.len().saturating_sub(STDERR_KEEP);
            kept.drain(..over);
        }
    });
    tail
}

/// The first line of a failure text that is not empty, at most 300 characters.
pub fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.chars().take(300).collect())
}
