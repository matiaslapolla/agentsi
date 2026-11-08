//! The framed connection of the app to its core.
//!
//! One socket carries the requests, the events and the terminal streams. A reader
//! thread splits the frames: replies go to the waiting call, events go to one queue,
//! and the bytes of each stream channel go to the queue of its pane. The core sends
//! no stream frame before the reply that names the channel, so the reader opens the
//! queue of a channel when it reads the reply of `terminal.attach`.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc as std_mpsc};
use std::time::{Duration, Instant};

use agentsi_proto::Method;
use agentsi_proto::core_api::{Hello, HelloParams};
use agentsi_proto::frame::{CONTROL, Frame, Kind, read_frame};
use agentsi_proto::rpc::{Id, Incoming, Notification, Outcome, Request, RpcError, V2};
use agentsi_proto::terminal_api::{Attach, AttachParams, AttachResult, CloseReason};
use futures::channel::{mpsc, oneshot};

/// What the stream of one pane carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamItem {
    /// Bytes of the program, with the time that the reader read them.
    Data(Instant, Vec<u8>),
    /// The size of the terminal, before the bytes at that size.
    Resize(u16, u16),
    /// The stream ended: the program stopped (`Some`), or the link to the core broke (`None`).
    Close(Option<CloseReason>),
}

/// What the app gets from the core apart from replies.
#[derive(Debug, Clone, PartialEq)]
pub enum LinkEvent {
    Notification(Notification),
    /// The core closed the connection, or the socket failed.
    Lost(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallError {
    Rpc(RpcError),
    /// The link to the core broke, or the core sent something that is not the protocol.
    Link(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::Rpc(e) => write!(f, "{e}"),
            CallError::Link(e) => write!(f, "{e}"),
        }
    }
}

/// The reason of a link that ended with an EOF.
pub const CLOSED: &str = "the core closed the connection";

type Reply = Result<serde_json::Value, CallError>;
type Stream = mpsc::UnboundedReceiver<StreamItem>;

enum Waiter {
    Plain(oneshot::Sender<Reply>),
    Attach(oneshot::Sender<Result<(u32, Stream), CallError>>),
}

#[derive(Default)]
struct Routes {
    waiters: HashMap<i64, Waiter>,
    streams: HashMap<u32, mpsc::UnboundedSender<StreamItem>>,
    /// `Some` after the link broke: the reason.
    lost: Option<String>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// How the frames reach the socket.
enum Writer {
    /// A link that never had a socket: see [`Link::lost`].
    None,
    /// The core of this device: a frame is on the socket when `send` returns, also at quit.
    Socket(Mutex<UnixStream>),
    /// A peer that can stall: a thread writes, so a full socket never blocks the window.
    Thread(std_mpsc::Sender<Vec<u8>>),
}

fn write_loop(mut stream: UnixStream, frames: std_mpsc::Receiver<Vec<u8>>) {
    for bytes in frames {
        if stream.write_all(&bytes).is_err() {
            // The reader then ends and reports the loss.
            let _ = stream.shutdown(Shutdown::Both);
            return;
        }
    }
}

/// The connection. Clone the `Arc` into each view that calls the core.
pub struct Link {
    writer: Writer,
    routes: Arc<Mutex<Routes>>,
    next_id: AtomicI64,
}

impl Link {
    /// Switches `stream` to framed mode and starts the reader thread. The events
    /// come out of the returned queue. `hello_timeout` marks a peer that can hang, an SSH
    /// host: a core that does not answer the hello in time is an error, and a thread
    /// writes the frames.
    pub fn start(
        mut stream: UnixStream,
        client: &str,
        hello_timeout: Option<Duration>,
    ) -> io::Result<(Arc<Link>, mpsc::UnboundedReceiver<LinkEvent>)> {
        let hello = Request {
            jsonrpc: V2,
            id: Some(Id::Number(0)),
            method: Hello::NAME.into(),
            params: Some(serde_json::to_value(HelloParams {
                framed: true,
                client: Some(client.into()),
            })?),
        };
        let mut line = serde_json::to_string(&hello)?;
        line.push('\n');
        // The timeout is on the socket, so it also covers the write. The reader thread
        // starts after the reset, so it never sees the timeout.
        stream.set_read_timeout(hello_timeout)?;
        stream.set_write_timeout(hello_timeout)?;
        stream.write_all(line.as_bytes())?;
        // The reply is one line, and frames follow it: read it byte by byte.
        let mut reply = Vec::new();
        let mut b = [0u8; 1];
        while b[0] != b'\n' {
            stream.read_exact(&mut b)?;
            reply.push(b[0]);
        }
        match serde_json::from_slice::<Incoming>(&reply) {
            Ok(Incoming::Response(r)) if matches!(r.outcome, Outcome::Result(_)) => {}
            _ => {
                return Err(io::Error::other(format!(
                    "the core refused framed mode: {}",
                    String::from_utf8_lossy(&reply).trim_end()
                )));
            }
        }
        stream.set_read_timeout(None)?;
        stream.set_write_timeout(None)?;
        let routes = Arc::new(Mutex::new(Routes::default()));
        let (events, rx) = mpsc::unbounded();
        let reader = stream.try_clone()?;
        let r = routes.clone();
        std::thread::Builder::new()
            .name("core-link".into())
            .spawn(move || read_loop(reader, &r, &events))?;
        let writer = if hello_timeout.is_some() {
            let (tx, frames) = std_mpsc::channel();
            std::thread::Builder::new()
                .name("core-link-write".into())
                .spawn(move || write_loop(stream, frames))?;
            Writer::Thread(tx)
        } else {
            Writer::Socket(Mutex::new(stream))
        };
        Ok((
            Arc::new(Link {
                writer,
                routes,
                next_id: AtomicI64::new(1),
            }),
            rx,
        ))
    }

    /// A link that fails each call at once with `why`. A host that has not connected yet
    /// holds one, so a screen of that host can never reach another core.
    pub fn lost(why: &str) -> Arc<Link> {
        Arc::new(Link {
            writer: Writer::None,
            routes: Arc::new(Mutex::new(Routes {
                lost: Some(why.to_string()),
                ..Routes::default()
            })),
            next_id: AtomicI64::new(1),
        })
    }

    /// A link with no reader, for tests of the views and the model.
    #[cfg(test)]
    pub fn for_tests(stream: UnixStream) -> Link {
        Link {
            writer: Writer::Socket(Mutex::new(stream)),
            routes: Arc::default(),
            next_id: AtomicI64::new(1),
        }
    }

    /// Sends one frame. A write error ends nothing here: the reader sees the broken socket.
    pub fn send(&self, frame: &Frame) {
        match &self.writer {
            Writer::None => {}
            Writer::Socket(s) => {
                let _ = lock(s).write_all(&frame.encode());
            }
            Writer::Thread(tx) => {
                let _ = tx.send(frame.encode());
            }
        }
    }

    /// Calls method `M` and waits for the reply in a future.
    pub fn call<M: Method>(
        &self,
        params: &M::Params,
    ) -> impl std::future::Future<Output = Result<M::Result, CallError>> + use<M> {
        let (tx, rx) = oneshot::channel();
        self.request(M::NAME, params, Waiter::Plain(tx));
        async move {
            let value = rx
                .await
                .map_err(|_| CallError::Link("the link to the core broke".into()))??;
            serde_json::from_value(value).map_err(|e| CallError::Link(e.to_string()))
        }
    }

    /// `terminal.attach`: the channel and the queue of its stream.
    pub fn attach(
        &self,
        id: &str,
    ) -> impl std::future::Future<Output = Result<(u32, Stream), CallError>> + use<> {
        let (tx, rx) = oneshot::channel();
        let params = AttachParams {
            id: id.into(),
            observe: false,
        };
        self.request(Attach::NAME, &params, Waiter::Attach(tx));
        async move {
            rx.await
                .map_err(|_| CallError::Link("the link to the core broke".into()))?
        }
    }

    fn request<P: serde::Serialize>(&self, method: &str, params: &P, waiter: Waiter) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut routes = lock(&self.routes);
            if let Some(why) = routes.lost.clone() {
                fail(waiter, CallError::Link(why));
                return;
            }
            routes.waiters.insert(id, waiter);
        }
        let request = Request {
            jsonrpc: V2,
            id: Some(Id::Number(id)),
            method: method.into(),
            params: serde_json::to_value(params).ok(),
        };
        match serde_json::to_string(&request) {
            Ok(json) => self.send(&Frame::message(&json)),
            Err(e) => {
                if let Some(w) = lock(&self.routes).waiters.remove(&id) {
                    fail(w, CallError::Link(e.to_string()));
                }
            }
        }
    }
}

fn fail(waiter: Waiter, e: CallError) {
    match waiter {
        Waiter::Plain(tx) => {
            let _ = tx.send(Err(e));
        }
        Waiter::Attach(tx) => {
            let _ = tx.send(Err(e));
        }
    }
}

fn read_loop(
    mut stream: UnixStream,
    routes: &Mutex<Routes>,
    events: &mpsc::UnboundedSender<LinkEvent>,
) {
    let why = loop {
        match read_frame(&mut stream) {
            Ok(Some(frame)) => route(frame, routes, events),
            Ok(None) => break CLOSED.to_string(),
            Err(e) => break format!("the connection to the core failed: {e}"),
        }
    };
    let mut r = lock(routes);
    r.lost = Some(why.clone());
    for (_, w) in r.waiters.drain() {
        fail(w, CallError::Link(why.clone()));
    }
    for (_, s) in r.streams.drain() {
        let _ = s.unbounded_send(StreamItem::Close(None));
    }
    drop(r);
    let _ = events.unbounded_send(LinkEvent::Lost(why));
}

fn route(frame: Frame, routes: &Mutex<Routes>, events: &mpsc::UnboundedSender<LinkEvent>) {
    if frame.channel == CONTROL {
        if frame.kind != Kind::DATA {
            return;
        }
        match serde_json::from_slice::<Incoming>(&frame.payload) {
            Ok(Incoming::Notification(n)) => {
                let _ = events.unbounded_send(LinkEvent::Notification(n));
            }
            Ok(Incoming::Response(r)) => {
                let Some(Id::Number(id)) = r.id else {
                    return;
                };
                let reply = match r.outcome {
                    Outcome::Result(v) => Ok(v),
                    Outcome::Error(e) => Err(CallError::Rpc(e)),
                };
                let mut routes = lock(routes);
                match routes.waiters.remove(&id) {
                    Some(Waiter::Plain(tx)) => {
                        let _ = tx.send(reply);
                    }
                    Some(Waiter::Attach(tx)) => {
                        let opened = reply.and_then(|v| {
                            serde_json::from_value::<AttachResult>(v)
                                .map_err(|e| CallError::Link(e.to_string()))
                        });
                        let _ = tx.send(opened.map(|a| {
                            let (s, rx) = mpsc::unbounded();
                            routes.streams.insert(a.channel, s);
                            (a.channel, rx)
                        }));
                    }
                    None => {}
                }
            }
            // A message that is not the protocol fails only its own call.
            Err(e) => {
                let id = serde_json::from_slice::<serde_json::Value>(&frame.payload)
                    .ok()
                    .and_then(|v| v.get("id")?.as_i64());
                eprintln!("agentsi-app: the core sent a bad message: {e}");
                if let Some(w) = id.and_then(|id| lock(routes).waiters.remove(&id)) {
                    fail(w, CallError::Link(format!("a bad reply: {e}")));
                }
            }
        }
        return;
    }
    let mut routes = lock(routes);
    let item = match frame.kind {
        Kind::DATA => StreamItem::Data(Instant::now(), frame.payload),
        Kind::RESIZE => match frame.as_resize() {
            Some((cols, rows)) => StreamItem::Resize(cols, rows),
            None => return,
        },
        Kind::CLOSE => {
            let reason = serde_json::from_slice(&frame.payload).ok();
            if let Some(s) = routes.streams.remove(&frame.channel) {
                let _ = s.unbounded_send(StreamItem::Close(reason));
            }
            return;
        }
        _ => return,
    };
    if let Some(s) = routes.streams.get(&frame.channel)
        && s.unbounded_send(item).is_err()
    {
        routes.streams.remove(&frame.channel);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use agentsi_proto::core_api::Empty;
    use agentsi_proto::terminal_api::List;
    use futures::StreamExt;
    use futures::executor::block_on;
    use serde_json::{Value, json};

    /// A fake core on the other end of a socket pair.
    pub(crate) struct FakeCore(pub UnixStream);

    impl FakeCore {
        pub(crate) fn hello(&mut self) {
            let mut line = Vec::new();
            let mut b = [0u8; 1];
            while b[0] != b'\n' {
                self.0.read_exact(&mut b).unwrap();
                line.push(b[0]);
            }
            let v: Value = serde_json::from_slice(&line).unwrap();
            assert_eq!(v["params"]["framed"], true);
            writeln!(
                self.0,
                r#"{{"jsonrpc":"2.0","id":0,"result":{{"protocol":1,"version":"0","features":[]}}}}"#
            )
            .unwrap();
        }
        pub(crate) fn request(&mut self) -> Value {
            let f = read_frame(&mut self.0).unwrap().unwrap();
            serde_json::from_slice(&f.payload).unwrap()
        }
        pub(crate) fn send(&mut self, f: Frame) {
            self.0.write_all(&f.encode()).unwrap();
        }
        pub(crate) fn reply(&mut self, id: &Value, result: Value) {
            let v = json!({"jsonrpc": "2.0", "id": id, "result": result});
            self.send(Frame::message(&v.to_string()));
        }
    }

    pub(crate) fn pair() -> (Arc<Link>, mpsc::UnboundedReceiver<LinkEvent>, FakeCore) {
        let (a, b) = UnixStream::pair().unwrap();
        let mut core = FakeCore(b);
        let t = std::thread::spawn(move || {
            core.hello();
            core
        });
        let (link, events) = Link::start(a, "test", None).unwrap();
        (link, events, t.join().unwrap())
    }

    #[test]
    fn a_call_gets_its_reply_and_an_event_goes_to_the_queue() {
        let (link, mut events, mut core) = pair();
        let reply = link.call::<List>(&Empty {});
        let req = core.request();
        assert_eq!(req["method"], "terminal.list");
        let note = json!({"jsonrpc": "2.0", "method": "terminal.exit", "params": {"id": "t1-a", "code": 0}});
        core.send(Frame::message(&note.to_string()));
        core.reply(&req["id"], json!({"terminals": []}));
        assert_eq!(block_on(reply).unwrap().terminals, vec![]);
        match block_on(events.next()) {
            Some(LinkEvent::Notification(n)) => assert_eq!(n.method, "terminal.exit"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stream_frames_right_after_the_attach_reply_reach_the_pane() {
        let (link, _events, mut core) = pair();
        let opened = link.attach("t1-a");
        let req = core.request();
        assert_eq!(req["method"], "terminal.attach");
        core.reply(&req["id"], json!({"channel": 3}));
        core.send(Frame::resize(3, 100, 30));
        core.send(Frame::new(3, Kind::DATA, b"hi".to_vec()));
        core.send(Frame::close(3, r#"{"reason":"exit","code":2}"#));
        let (channel, stream) = block_on(opened).unwrap();
        assert_eq!(channel, 3);
        let items: Vec<StreamItem> = block_on(stream.collect());
        assert_eq!(items[0], StreamItem::Resize(100, 30));
        assert!(matches!(&items[1], StreamItem::Data(_, b) if b == b"hi"));
        assert_eq!(
            items[2],
            StreamItem::Close(Some(CloseReason::Exit { code: Some(2) }))
        );
    }

    #[test]
    fn a_lost_link_fails_the_calls_closes_the_streams_and_says_so() {
        let (link, mut events, mut core) = pair();
        let opened = link.attach("t1-a");
        let req = core.request();
        core.reply(&req["id"], json!({"channel": 1}));
        let (_, mut stream) = block_on(opened).unwrap();
        let waiting = link.call::<List>(&Empty {});
        let _ = core.request();
        drop(core);
        assert!(matches!(block_on(waiting), Err(CallError::Link(_))));
        assert_eq!(block_on(stream.next()), Some(StreamItem::Close(None)));
        assert!(matches!(block_on(events.next()), Some(LinkEvent::Lost(_))));
        // A call after the loss fails at once.
        assert!(matches!(
            block_on(link.call::<List>(&Empty {})),
            Err(CallError::Link(_))
        ));
    }

    #[test]
    fn a_host_that_reads_nothing_never_blocks_the_sender_and_the_frames_keep_their_order() {
        let (a, b) = UnixStream::pair().unwrap();
        let mut core = FakeCore(b);
        let t = std::thread::spawn(move || {
            core.hello();
            core
        });
        let (link, _events) = Link::start(a, "test", Some(Duration::from_secs(5))).unwrap();
        let mut core = t.join().unwrap();
        // Far more than a socket buffer holds, and the core does not read yet.
        let start = Instant::now();
        for n in 0..64u8 {
            link.send(&Frame::new(1, Kind::DATA, vec![n; 256 * 1024]));
        }
        assert!(start.elapsed() < Duration::from_secs(2));
        for n in 0..64u8 {
            let f = read_frame(&mut core.0).unwrap().unwrap();
            assert_eq!(f.payload[0], n);
        }
    }

    #[test]
    fn a_lost_link_fails_each_call_and_each_attach_with_its_reason() {
        let link = Link::lost("not connected yet");
        link.send(&Frame::message("{}"));
        match block_on(link.call::<List>(&Empty {})) {
            Err(CallError::Link(why)) => assert_eq!(why, "not connected yet"),
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            block_on(link.attach("t1-a")),
            Err(CallError::Link(_))
        ));
    }

    #[test]
    fn a_bad_reply_fails_only_its_call_and_the_link_stays_up() {
        let (link, _events, mut core) = pair();
        let bad = link.call::<List>(&Empty {});
        let req = core.request();
        core.send(Frame::message(
            &json!({"jsonrpc": "2.0", "id": req["id"], "error": {"code": 1}}).to_string(),
        ));
        assert!(matches!(block_on(bad), Err(CallError::Link(_))));
        let good = link.call::<List>(&Empty {});
        let req = core.request();
        core.reply(&req["id"], json!({"terminals": []}));
        assert!(block_on(good).is_ok());
    }

    #[test]
    fn a_core_that_does_not_answer_the_hello_is_an_error_after_the_limit() {
        let (a, _b) = UnixStream::pair().unwrap();
        let t = Instant::now();
        let e = Link::start(a, "test", Some(Duration::from_millis(200)))
            .err()
            .expect("no hello reply");
        assert!(
            matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ),
            "{e:?}"
        );
        assert!(t.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn an_error_reply_is_an_rpc_error() {
        let (link, _events, mut core) = pair();
        let reply = link.attach("t9-zzzzzz");
        let req = core.request();
        let v = json!({"jsonrpc": "2.0", "id": req["id"], "error": {"code": -32001, "message": "no terminal", "data": {"kind": "not_found", "hint": "List the terminals."}}});
        core.send(Frame::message(&v.to_string()));
        match block_on(reply) {
            Err(CallError::Rpc(e)) => assert_eq!(e.kind(), agentsi_proto::ErrorKind::NotFound),
            Err(other) => panic!("{other:?}"),
            Ok(_) => panic!("an attach of an unknown id gave a channel"),
        }
    }
}
