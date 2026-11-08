//! A blocking client in line mode, and the start of the core by a client.
//! It needs no async runtime, so each program can use it.

use crate::method::Method;
use crate::rpc::{Id, Incoming, Notification, Outcome, Request, RpcError, V2};
use crate::socket::{SOCKET_ENV, SocketPath};
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The variable that names the `agentsi-core` program to start.
pub const CORE_BIN_ENV: &str = "AGENTSI_CORE_BIN";
/// The exit code of a core that finds another core on its socket.
pub const EXIT_ALREADY_RUNNING: i32 = 3;
/// How long a client waits for a core that it started.
pub const START_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a core that stops waits for its work, as the protocol says.
pub const CORE_STOP_WAIT: Duration = Duration::from_secs(10);
/// The first pause before a client starts another core, after a core found the lock held.
/// The pause doubles up to `RESTART_PAUSE_MAX`.
const RESTART_PAUSE: Duration = Duration::from_millis(50);
const RESTART_PAUSE_MAX: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub enum ClientError {
    Io(io::Error),
    /// The core answered with an error.
    Rpc(RpcError),
    /// The core sent something that is not the protocol.
    Protocol(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Io(e) => write!(f, "{e}"),
            ClientError::Rpc(e) => write!(f, "{e}"),
            ClientError::Protocol(e) => write!(f, "protocol error: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<io::Error> for ClientError {
    fn from(e: io::Error) -> ClientError {
        ClientError::Io(e)
    }
}

/// One connection in line mode. It keeps the notifications that come
/// before a reply, for `next_notification`.
pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: i64,
    notifications: VecDeque<Notification>,
}

impl Client {
    pub fn new(stream: UnixStream) -> io::Result<Client> {
        Ok(Client {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
            next_id: 1,
            notifications: VecDeque::new(),
        })
    }

    pub fn connect(path: &Path) -> io::Result<Client> {
        Client::new(UnixStream::connect(path)?)
    }

    /// A limit for each read and write. `None` waits for ever.
    pub fn set_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        self.writer.set_read_timeout(timeout)?;
        self.writer.set_write_timeout(timeout)
    }

    pub fn call<M: Method>(&mut self, params: &M::Params) -> Result<M::Result, ClientError> {
        let id = Id::Number(self.next_id);
        self.next_id += 1;
        let params =
            serde_json::to_value(params).map_err(|e| ClientError::Protocol(e.to_string()))?;
        let request = Request {
            jsonrpc: V2,
            id: Some(id.clone()),
            method: M::NAME.into(),
            params: Some(params),
        };
        let mut line =
            serde_json::to_string(&request).map_err(|e| ClientError::Protocol(e.to_string()))?;
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        loop {
            match self.read_incoming()? {
                Incoming::Notification(n) => self.notifications.push_back(n),
                Incoming::Response(r) if r.id.as_ref() == Some(&id) => {
                    return match r.outcome {
                        Outcome::Result(v) => serde_json::from_value(v)
                            .map_err(|e| ClientError::Protocol(e.to_string())),
                        Outcome::Error(e) => Err(ClientError::Rpc(e)),
                    };
                }
                Incoming::Response(r) => {
                    return Err(ClientError::Protocol(format!("a reply to id {:?}", r.id)));
                }
            }
        }
    }

    /// The next event. It waits until one comes.
    pub fn next_notification(&mut self) -> Result<Notification, ClientError> {
        if let Some(n) = self.notifications.pop_front() {
            return Ok(n);
        }
        match self.read_incoming()? {
            Incoming::Notification(n) => Ok(n),
            Incoming::Response(r) => Err(ClientError::Protocol(format!(
                "a reply to id {:?} with no request",
                r.id
            ))),
        }
    }

    fn read_incoming(&mut self) -> Result<Incoming, ClientError> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(ClientError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the core closed the connection",
            )));
        }
        serde_json::from_str(&line)
            .map_err(|e| ClientError::Protocol(format!("{e}: {}", line.trim_end())))
    }
}

/// Connects to the core. If no core listens, it starts `core_bin serve --detach`
/// and waits up to `START_TIMEOUT` for the socket.
///
/// A started core that stops with `EXIT_ALREADY_RUNNING` found the lock held. Then a core
/// starts, or a core stops and still holds the lock after it removed its socket. The client
/// then waits up to `CORE_STOP_WAIT` more, and starts a core again after a pause that doubles.
pub fn connect_or_start(socket: &SocketPath, core_bin: &Path) -> io::Result<UnixStream> {
    match UnixStream::connect(&socket.path) {
        Ok(stream) => return Ok(stream),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) => {}
        Err(e) => return Err(e),
    }
    let started = Instant::now();
    let mut deadline = started + START_TIMEOUT;
    let mut lock_held = false;
    let mut child = spawn_core(socket, core_bin)?;
    let mut last = None;
    let mut pause = RESTART_PAUSE;
    // When the child has stopped with code 3: the time to start the next one.
    let mut restart_at: Option<Instant> = None;
    while Instant::now() < deadline {
        match UnixStream::connect(&socket.path) {
            Ok(stream) => {
                // A long-running client reaps the core when the core stops.
                std::thread::spawn(move || child.wait());
                return Ok(stream);
            }
            Err(e) => last = Some(e),
        }
        match restart_at {
            Some(at) if Instant::now() >= at => {
                child = spawn_core(socket, core_bin)?;
                restart_at = None;
            }
            Some(_) => {}
            None => match child.try_wait()? {
                None => {}
                Some(status) if status.code() == Some(EXIT_ALREADY_RUNNING) => {
                    if !lock_held {
                        lock_held = true;
                        deadline = Instant::now() + CORE_STOP_WAIT + START_TIMEOUT;
                    }
                    restart_at = Some(Instant::now() + pause);
                    pause = (pause * 2).min(RESTART_PAUSE_MAX);
                }
                Some(status) => {
                    return Err(io::Error::other(format!(
                        "the core stopped with {status}. Read {}",
                        socket.log().display()
                    )));
                }
            },
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "no core on {} after {:?} ({last:?}). Read {}",
            socket.path.display(),
            started.elapsed(),
            socket.log().display()
        ),
    ))
}

fn spawn_core(socket: &SocketPath, core_bin: &Path) -> io::Result<std::process::Child> {
    let mut command = Command::new(core_bin);
    command
        .args(["serve", "--detach"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !socket.default {
        command.env(SOCKET_ENV, &socket.path);
    }
    command.spawn().map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("cannot start {}: {e}", core_bin.display()),
        )
    })
}

/// The `agentsi-core` program: `AGENTSI_CORE_BIN`, else the one next to this
/// program, else the first one on the `PATH`.
pub fn find_core_bin() -> Option<PathBuf> {
    if let Some(bin) = std::env::var_os(CORE_BIN_ENV).filter(|v| !v.is_empty()) {
        return Some(bin.into());
    }
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("agentsi-core")));
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|dir| dir.join("agentsi-core"));
    sibling
        .into_iter()
        .chain(on_path)
        .find(|p| is_executable_file(p))
}

pub fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
