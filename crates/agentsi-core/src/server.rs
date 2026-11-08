//! The socket of the core: the folder, the lock, the peer check, and the two modes.

use crate::methods::Ctx;
use crate::session::Session;
use crate::state::{Core, Subscriber, lock};
use crate::terminal::Term;
use crate::terminal::stream::{Attach, Next};
use agentsi_proto::frame::{self, Decoder, Frame, Kind, MAX_PAYLOAD};
use agentsi_proto::rpc::{Id, Outcome, Request, Response, RpcError, V2};
use agentsi_proto::socket::SocketPath;
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;

/// Messages that wait for the writer of one connection.
const OUTBOX: usize = 256;

#[derive(Debug)]
pub enum BindError {
    /// Another core holds the lock of this socket.
    AlreadyRunning,
    Io(io::Error),
}

impl From<io::Error> for BindError {
    fn from(e: io::Error) -> BindError {
        BindError::Io(e)
    }
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindError::AlreadyRunning => write!(f, "another core runs on this socket"),
            BindError::Io(e) => write!(f, "{e}"),
        }
    }
}

pub fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Makes the folder of the socket. The default folder gets mode 0700. A
/// folder from `AGENTSI_CORE_SOCKET` gets mode 0700 only if the core makes it.
pub fn prepare_folder(socket: &SocketPath) -> io::Result<()> {
    let folder = socket
        .path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !folder.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(folder)?;
    }
    if socket.default {
        let meta = std::fs::metadata(folder)?;
        if meta.uid() != euid() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "{} belongs to uid {}, not to this user",
                    folder.display(),
                    meta.uid()
                ),
            ));
        }
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// A socket that this core owns. Drop removes the socket file, then frees the lock.
pub struct Bound {
    listener: Option<UnixListener>,
    socket: SocketPath,
    /// `serve` takes it out, to hand it to its caller.
    lock: Option<File>,
}

/// The lock of the socket after `serve`. Keep it until the work of the core has
/// stopped: while it lives, a second core exits with `AlreadyRunning`.
#[must_use]
pub struct HeldLock(#[allow(dead_code)] File);

/// How often a serving core checks that its socket file still exists.
const SOCKET_WATCH: std::time::Duration = std::time::Duration::from_secs(1);

/// Takes the lock, removes a stale socket and listens with mode 0600.
pub fn bind(socket: &SocketPath) -> Result<Bound, BindError> {
    prepare_folder(socket)?;
    let mut lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(socket.lock())?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Err(BindError::AlreadyRunning),
        Err(std::fs::TryLockError::Error(e)) => return Err(e.into()),
    }
    if std::os::unix::net::UnixStream::connect(&socket.path).is_ok() {
        return Err(BindError::AlreadyRunning);
    }
    match std::fs::remove_file(&socket.path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
        _ => {}
    }
    lock.set_len(0)?;
    writeln!(lock, "{}", std::process::id())?;
    let listener = UnixListener::bind(&socket.path)?;
    let bound = Bound {
        listener: Some(listener),
        socket: socket.clone(),
        lock: Some(lock),
    };
    std::fs::set_permissions(&socket.path, std::fs::Permissions::from_mode(0o600))?;
    if let Some(l) = &bound.listener {
        l.set_nonblocking(true)?;
    }
    Ok(bound)
}

impl Drop for Bound {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket.path);
    }
}

impl Bound {
    /// Serves until `core.shutdown` or until `stop` is ready. Then it removes
    /// the socket and returns the lock, which stays held. A connection from a user
    /// other than `owner_uid` is closed before the core reads from it.
    pub async fn serve(
        mut self,
        core: Arc<Core>,
        owner_uid: u32,
        stop: impl Future<Output = ()>,
    ) -> io::Result<HeldLock> {
        let listener =
            tokio::net::UnixListener::from_std(self.listener.take().expect("serve runs one time"))?;
        core.terminals.set_socket(self.socket.path.clone());
        tokio::pin!(stop);
        let mut watch = tokio::time::interval(SOCKET_WATCH);
        loop {
            tokio::select! {
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        tokio::spawn(connection(core.clone(), stream, owner_uid));
                    }
                    Err(e) => {
                        eprintln!("core: accept failed: {e}");
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                },
                () = core.stopped() => break,
                () = &mut stop => break,
                _ = watch.tick() => if !self.socket.path.exists() {
                    eprintln!("core: {} is gone, so no client can reach this core. Stopping.", self.socket.path.display());
                    break;
                },
            }
        }
        // The work that a stop ends keeps its wait. The links do not.
        core.close_links();
        Ok(HeldLock(self.lock.take().expect("bind takes the lock")))
    }
}

/// What the writer of one connection sends.
enum Out {
    Event(Arc<str>),
    Reply(Reply),
    /// A frame of a stream channel. Only a framed connection has streams.
    Frame(Frame),
}

/// The reply to one request, and the changes that come after it. One item
/// keeps them in order: no event can come between the reply and the change.
struct Reply {
    /// `None` for a request without an id.
    text: Option<Arc<str>>,
    /// The messages after this reply go in frames.
    framed: bool,
    /// Stop the core after this reply, also if the client is gone.
    shutdown: bool,
}

async fn connection(core: Arc<Core>, stream: UnixStream, owner_uid: u32) {
    let _link = core.open_link();
    let uid = match stream.peer_cred() {
        Ok(cred) => cred.uid(),
        Err(e) => {
            eprintln!("core: closed a connection with no peer credentials: {e}");
            return;
        }
    };
    if uid != owner_uid {
        eprintln!("core: closed a connection from uid {uid}");
        return;
    }
    let (read, write) = stream.into_split();
    let (tx, rx) = mpsc::channel(OUTBOX);
    let writer = tokio::spawn(write_loop(write, rx, core.clone()));
    let session = Arc::new(Session::default());
    let mut reader = BufReader::new(read);
    // A request that a stop cuts off keeps its work: meetings, stories and renders run on
    // blocking threads that the stop cancels, and they write their own end state.
    tokio::select! {
        result = read_loop(&core, &session, &mut reader, &tx) => {
            if let Err(e) = result {
                eprintln!("core: connection error: {e}");
            }
        }
        () = async {
            core.links_closing().await;
            drain(&session).await;
        } => {}
    }
    session.stop_forwarder();
    session.stop_streams();
    drop(tx);
    let _ = writer.await;
}

/// How long a stop lets a request that is in flight answer. The work that a stop cancels
/// ends in milliseconds, and a step with no cancel must not hold the link.
pub const STOP_GRACE: Duration = Duration::from_secs(1);

/// Waits until the connection has no request in flight, or until `STOP_GRACE` ends.
async fn drain(session: &Session) {
    let deadline = tokio::time::Instant::now() + STOP_GRACE;
    while session.requests_in_flight() > 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn read_loop(
    core: &Arc<Core>,
    session: &Arc<Session>,
    reader: &mut BufReader<OwnedReadHalf>,
    tx: &mpsc::Sender<Out>,
) -> io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = (&mut *reader)
            .take(MAX_PAYLOAD as u64 + 1)
            .read_until(b'\n', &mut line)
            .await?;
        if n == 0 {
            return Ok(());
        }
        let message = line.strip_suffix(b"\n").unwrap_or(&line);
        if message.len() > MAX_PAYLOAD {
            let error = reply(
                None,
                Err(RpcError::invalid_request("a line is longer than 16 MiB")),
            );
            let _ = tx
                .send(Out::Reply(Reply {
                    text: Some(error),
                    framed: false,
                    shutdown: false,
                }))
                .await;
            return Ok(());
        }
        if message.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let _request = session.begin_request();
        let (text, opened) = handle(core, session, message).await;
        let framed = session.take_framed();
        respond(session, tx, text, framed, opened).await?;
        if framed {
            session.set_in_frames();
            return framed_loop(core, session, reader, tx).await;
        }
    }
}

/// The bytes of the requests that the core runs at the same time on one
/// connection. A client that does not read its replies stops here.
const IN_FLIGHT_BYTES: usize = 32 << 20;

async fn framed_loop(
    core: &Arc<Core>,
    session: &Arc<Session>,
    reader: &mut BufReader<OwnedReadHalf>,
    tx: &mpsc::Sender<Out>,
) -> io::Result<()> {
    let budget = Arc::new(Semaphore::new(IN_FLIGHT_BYTES));
    let mut requests = JoinSet::new();
    let mut decoder = Decoder::default();
    let mut buf = vec![0u8; 64 * 1024];
    let result = loop {
        while requests.try_join_next().is_some() {}
        match decoder.next_frame() {
            Err(e) => break Err(io::Error::new(io::ErrorKind::InvalidData, e)),
            Ok(Some(f)) if f.channel == frame::CONTROL && f.kind == Kind::DATA => {
                let permits = f.payload.len().clamp(1, IN_FLIGHT_BYTES) as u32;
                let permit = budget
                    .clone()
                    .acquire_many_owned(permits)
                    .await
                    .expect("the budget stays open");
                let (core, session, tx) = (core.clone(), session.clone(), tx.clone());
                let in_flight = session.clone();
                requests.spawn(async move {
                    let _permit = permit;
                    let _request = in_flight.begin_request();
                    let (text, opened) = handle(&core, &session, &f.payload).await;
                    let _ = respond(&session, &tx, text, false, opened).await;
                });
                continue;
            }
            Ok(Some(f)) if f.channel == frame::CONTROL => continue,
            Ok(Some(f)) => {
                session.stream_frame(f);
                continue;
            }
            Ok(None) => {}
        }
        match reader.read(&mut buf).await {
            Ok(0) if decoder.buffered() == 0 => break Ok(()),
            Ok(0) => {
                break Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the stream ended inside a frame",
                ));
            }
            Ok(n) => decoder.push(&buf[..n]),
            Err(e) => break Err(e),
        }
    };
    // The replies still go out. Then no task can start a forwarder.
    while requests.join_next().await.is_some() {}
    result
}

/// Queues the reply, then starts the events of a first `core.subscribe`.
async fn respond(
    session: &Arc<Session>,
    tx: &mpsc::Sender<Out>,
    text: Option<Arc<str>>,
    framed: bool,
    opened: Vec<u32>,
) -> io::Result<()> {
    let shutdown = session.take_shutdown();
    tx.send(Out::Reply(Reply {
        text,
        framed,
        shutdown,
    }))
    .await
    .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the writer stopped"))?;
    if let Some(subscriber) = session.take_new_events() {
        let forwarder = tokio::spawn(forward(subscriber, tx.downgrade()));
        session.set_forwarder(forwarder.abort_handle());
    }
    // Only the channels of this request: the reply that names a channel comes before its frames.
    for channel in opened {
        if let Some((attach, term)) = session.stream(channel) {
            let task = tokio::spawn(pump(session.clone(), attach, term, tx.downgrade()));
            session.set_pump(channel, task.abort_handle());
        }
    }
    Ok(())
}

/// Sends the frames of one stream channel within its credit window. Like
/// the forwarder, it holds a weak sender.
async fn pump(
    session: Arc<Session>,
    attach: Arc<Attach>,
    term: Arc<Term>,
    tx: mpsc::WeakSender<Out>,
) {
    'stream: loop {
        match attach.next() {
            Next::Send(frames) => {
                let Some(tx) = tx.upgrade() else { break };
                for f in frames {
                    if tx.send(Out::Frame(f)).await.is_err() {
                        break 'stream;
                    }
                }
            }
            Next::Resync => term.resync(&attach),
            Next::Wait => attach.ready().await,
            Next::End => break,
        }
    }
    session.close_stream(attach.channel);
}

/// The forwarder holds a weak sender. Thus it never keeps a connection open.
async fn forward(subscriber: Arc<Subscriber>, tx: mpsc::WeakSender<Out>) {
    loop {
        let message = subscriber.next().await;
        let Some(tx) = tx.upgrade() else { return };
        if tx.send(Out::Event(message)).await.is_err() {
            return;
        }
    }
}

fn reply(id: Option<Id>, result: Result<Value, RpcError>) -> Arc<str> {
    let outcome = match result {
        Ok(v) => Outcome::Result(v),
        Err(e) => Outcome::Error(e),
    };
    serde_json::to_string(&Response {
        jsonrpc: V2,
        id,
        outcome,
    })
    .unwrap_or_default()
    .into()
}

/// Runs one JSON-RPC message. Gives the reply, `None` for a notification,
/// and the stream channels that the request opened.
async fn handle(
    core: &Arc<Core>,
    session: &Arc<Session>,
    text: &[u8],
) -> (Option<Arc<str>>, Vec<u32>) {
    let opened = Arc::new(Mutex::new(Vec::new()));
    let text = run(core, session, text, &opened).await;
    let opened = std::mem::take(&mut *lock(&opened));
    (text, opened)
}

async fn run(
    core: &Arc<Core>,
    session: &Arc<Session>,
    text: &[u8],
    opened: &Arc<Mutex<Vec<u32>>>,
) -> Option<Arc<str>> {
    let value: Value = match serde_json::from_slice(text) {
        Ok(v) => v,
        Err(e) => return Some(reply(None, Err(RpcError::parse_error(e.to_string())))),
    };
    if !value.is_object() {
        return Some(reply(
            None,
            Err(RpcError::invalid_request(
                "send one JSON object, not a batch",
            )),
        ));
    }
    let id: Option<Id> = serde_json::from_value(value["id"].clone()).ok();
    let request: Request = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(e) => return Some(reply(id, Err(RpcError::invalid_request(e.to_string())))),
    };
    let params = match request.params {
        None | Some(Value::Null) => Value::Object(Default::default()),
        Some(p @ Value::Object(_)) => p,
        Some(_) => {
            let error = RpcError::invalid_params(
                "params must be an object: the protocol takes params by name",
            );
            return request.id.map(|id| reply(Some(id), Err(error)));
        }
    };
    let ctx = Ctx {
        core: core.clone(),
        session: session.clone(),
        opened: opened.clone(),
    };
    let result = core.router.call(ctx, &request.method, params).await;
    request.id.map(|id| reply(Some(id), result))
}

/// After a failed write, the writer reads on and sends nothing. Thus a
/// shutdown still stops the core when its client is gone.
async fn write_loop(mut write: OwnedWriteHalf, mut rx: mpsc::Receiver<Out>, core: Arc<Core>) {
    let mut framed = false;
    let mut broken = false;
    while let Some(out) = rx.recv().await {
        let (text, after) = match out {
            Out::Event(text) => (Some(text), None),
            Out::Reply(r) => (r.text.clone(), Some(r)),
            Out::Frame(f) => {
                if !broken {
                    broken = write.write_all(&f.encode()).await.is_err();
                }
                continue;
            }
        };
        if let Some(text) = text
            && !broken
        {
            let bytes = if framed {
                Frame::message(&text).encode()
            } else {
                format!("{text}\n").into_bytes()
            };
            broken = write.write_all(&bytes).await.is_err();
        }
        if let Some(r) = after {
            framed |= r.framed;
            if r.shutdown {
                let _ = write.flush().await;
                core.ask_stop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    /// B3 of the U22 review: the lock is never free between the end of `serve` and
    /// the end of the work of the core.
    #[tokio::test]
    async fn serve_removes_the_socket_and_keeps_the_lock_for_its_caller() {
        let dir = tempfile::tempdir().unwrap();
        let socket = SocketPath {
            path: dir.path().join("core.sock"),
            default: false,
        };
        let bound = bind(&socket).unwrap();
        let held = bound
            .serve(Core::new(), euid(), std::future::ready(()))
            .await
            .unwrap();
        assert!(!socket.path.exists());
        assert!(matches!(bind(&socket), Err(BindError::AlreadyRunning)));
        drop(held);
        assert!(bind(&socket).is_ok());
    }

    /// B2 of the review: the mode changes after the text of its reply, and
    /// before the next event.
    #[tokio::test]
    async fn the_writer_changes_to_frames_right_after_the_reply() {
        let (a, mut b) = UnixStream::pair().unwrap();
        let (_, write) = a.into_split();
        let (tx, rx) = mpsc::channel(8);
        let writer = tokio::spawn(write_loop(write, rx, Core::new()));
        tx.send(Out::Event("e1".into())).await.unwrap();
        tx.send(Out::Reply(Reply {
            text: Some("r".into()),
            framed: true,
            shutdown: false,
        }))
        .await
        .unwrap();
        tx.send(Out::Event("e2".into())).await.unwrap();
        drop(tx);
        writer.await.unwrap();
        let mut out = Vec::new();
        b.read_to_end(&mut out).await.unwrap();
        let mut expected = b"e1\nr\n".to_vec();
        expected.extend(Frame::message("e2").encode());
        assert_eq!(out, expected);
    }
}
