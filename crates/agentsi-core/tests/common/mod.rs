//! Helpers for tests that run a real core on a socket in a temp folder.
//! Each guard stops its core and removes its folder on drop.

#![allow(dead_code)]

pub mod framed;

use agentsi_proto::socket::SocketPath;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

pub const BIN: &str = env!("CARGO_BIN_EXE_agentsi-core");
pub const HELLO: &str = r#"{"jsonrpc":"2.0","id":1,"method":"core.hello","params":{}}"#;
pub const SHUTDOWN: &str = r#"{"jsonrpc":"2.0","id":99,"method":"core.shutdown"}"#;

pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new() -> TempDir {
        static N: AtomicU32 = AtomicU32::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("agentsi-u3-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    /// A socket path whose folder does not exist yet.
    pub fn socket(&self) -> SocketPath {
        SocketPath {
            path: self.0.join("agentsi").join("core.sock"),
            default: false,
        }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn wait_until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Sends lines, closes the write half, and reads until the core closes.
pub fn exchange(socket: &Path, lines: &[&str]) -> String {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    for line in lines {
        writeln!(stream, "{line}").unwrap();
    }
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).unwrap();
    out
}

/// A core process. Drop sends `core.shutdown`, then kills the process if it
/// still runs after 2 seconds.
pub struct CoreProc {
    pub child: Option<Child>,
    pub socket: SocketPath,
    pub log: PathBuf,
}

impl CoreProc {
    pub fn command(socket: &SocketPath) -> Command {
        let mut c = Command::new(BIN);
        c.arg("serve")
            .env("AGENTSI_CORE_SOCKET", &socket.path)
            .stdin(Stdio::null())
            .stdout(Stdio::null());
        c
    }

    pub fn start(dir: &TempDir) -> CoreProc {
        CoreProc::start_with(dir, &[])
    }

    /// A core with more names in its environment.
    pub fn start_with(dir: &TempDir, env: &[(&str, &Path)]) -> CoreProc {
        let socket = dir.socket();
        let log = dir.0.join("core.err");
        let mut c = CoreProc::command(&socket);
        c.envs(env.iter().copied());
        c.stderr(std::fs::File::create(&log).unwrap());
        let child = c.spawn().unwrap();
        let core = CoreProc {
            child: Some(child),
            socket,
            log,
        };
        wait_until("the core listens", || {
            UnixStream::connect(&core.socket.path).is_ok()
        });
        core
    }

    /// A core that another process started: the guard finds it by its lock file.
    pub fn adopt(socket: SocketPath) -> CoreProc {
        CoreProc {
            child: None,
            log: socket.log(),
            socket,
        }
    }

    pub fn pid(&self) -> i32 {
        match &self.child {
            Some(c) => c.id() as i32,
            None => std::fs::read_to_string(self.socket.lock())
                .unwrap()
                .trim()
                .parse()
                .unwrap(),
        }
    }

    pub fn stderr(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }
}

impl Drop for CoreProc {
    fn drop(&mut self) {
        let pid = std::fs::read_to_string(self.socket.lock())
            .ok()
            .and_then(|p| p.trim().parse::<i32>().ok());
        if let Ok(mut s) = UnixStream::connect(&self.socket.path) {
            let _ = writeln!(s, "{SHUTDOWN}");
            let _ = s.read(&mut [0u8; 256]);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        match &mut self.child {
            Some(child) => {
                while child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                let _ = child.kill();
                let _ = child.wait();
            }
            None => {
                let Some(pid) = pid else { return };
                while alive(pid) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                if alive(pid) {
                    // SAFETY: the pid is the core that this test started.
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                }
            }
        }
    }
}
