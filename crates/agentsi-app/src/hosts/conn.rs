//! The connection of one SSH host, on its own thread.
//!
//! The thread runs `ssh -T <host> agentsi-core connect` with one end of a socket pair as
//! its stdin and stdout, so the link of the local core works unchanged on it. It reads the
//! product list and hands the link to the window. It then sends each `voice.*` event of the
//! remote core to the local core, and each other event to the window.
//! When the connection fails or stops, the thread reports "device off" and tries again
//! after 1, 2, 4 and up to 60 seconds. Nothing here runs on the UI thread, so a slow or
//! dead host never blocks the window.

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader};
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Poll, Wake, Waker};
use std::time::Duration;
use std::time::Instant;

use agentsi_proto::ErrorKind;
use agentsi_proto::core_api::{Empty, Subscribe, SubscribeParams, Topic};
use agentsi_proto::desk_api::{ProductInfo, ProductList};
use agentsi_proto::events::{VoiceSpeak, VoiceStop};
use agentsi_proto::hosts_api::{VoiceRelay, VoiceRelayParams, is_relayed};
use agentsi_proto::rpc::Notification;
use futures::StreamExt;
use futures::channel::mpsc;
use futures::executor::block_on;

use super::config::{self, HostConfig};
use crate::link::{CallError, Link, LinkEvent};
use crate::model::HostState;

/// What the thread of a host reports to the window.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    State(HostState),
    Products(Vec<ProductInfo>),
    /// A remark for the settings screen, or `None` to clear it.
    Note(Option<String>),
    /// The link of a new connection. `State(Up)` follows it.
    Link(Linked),
    /// An event of the remote core that is not a voice event.
    Event(Notification),
}

/// The link of one connection. Two are equal when they are the same link.
#[derive(Clone)]
pub struct Linked(pub Arc<Link>);

impl PartialEq for Linked {
    fn eq(&self, other: &Linked) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for Linked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Linked")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub host: usize,
    pub change: Change,
}

/// The time limits of a connection.
#[derive(Debug, Clone)]
pub struct Limits {
    /// From the start of `ssh` to the hello reply: the login, and the start of a remote core.
    pub hello: Duration,
    /// A call to the remote core or to the local core.
    pub call: Duration,
    /// The waits before each new try. The last one repeats.
    pub backoff: Vec<Duration>,
    /// A connection that lasted this long starts the waits again at the first one.
    pub stable: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            hello: Duration::from_secs(20),
            call: Duration::from_secs(10),
            backoff: [1, 2, 4, 8, 16, 32, 60].map(Duration::from_secs).to_vec(),
            stable: Duration::from_secs(10),
        }
    }
}

impl Limits {
    /// The wait after `failures` tries in a row that did not connect.
    pub fn delay(&self, failures: usize) -> Duration {
        let last = self.backoff.len().saturating_sub(1);
        self.backoff
            .get(failures.min(last))
            .copied()
            .unwrap_or_default()
    }
}

/// One open transport to a remote core.
pub struct Dialed {
    pub stream: UnixStream,
    /// The `ssh` process. The connection kills it at its end.
    pub child: Option<Child>,
    /// The last line that `ssh` wrote to stderr: the reason of a failed login.
    pub stderr: Arc<Stderr>,
}

pub type Dial = Box<dyn FnMut() -> io::Result<Dialed> + Send>;

/// The stderr of an `ssh` process, as a thread reads it.
#[derive(Default)]
pub struct Stderr {
    last: Mutex<String>,
    /// The reader reached the end: `last` is the last line.
    done: std::sync::atomic::AtomicBool,
}

impl Stderr {
    /// The last line, after the reader reached the end or after `limit`.
    fn last(&self, limit: Duration) -> String {
        let deadline = Instant::now() + limit;
        while !self.done.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        self.last.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

/// The options that keep a dead host from hanging the connection: no password prompt,
/// a connect timeout, and a keepalive that ends a link whose network is gone.
const SSH_OPTIONS: [&str; 8] = [
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=10",
    "-o",
    "ServerAliveInterval=5",
    "-o",
    "ServerAliveCountMax=3",
];

/// The argv of the `ssh` process of a host.
pub fn ssh_argv(cfg: &HostConfig) -> Vec<String> {
    let mut argv = vec!["ssh".to_string(), "-T".to_string()];
    argv.extend(SSH_OPTIONS.map(String::from));
    argv.extend(cfg.args.iter().cloned());
    argv.push(cfg.ssh.clone());
    argv.push(cfg.command.clone());
    argv
}

/// A dial that runs `argv` with one end of a socket pair as its stdin and stdout.
pub fn spawn_dial(argv: Vec<String>) -> Dial {
    Box::new(move || {
        let (ours, theirs) = UnixStream::pair()?;
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .stdin(Stdio::from(OwnedFd::from(theirs.try_clone()?)))
            .stdout(Stdio::from(OwnedFd::from(theirs)))
            .stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        // The command holds the other end until it drops. Then the end of `ssh` gives EOF.
        drop(cmd);
        let stderr = Arc::new(Stderr::default());
        if let Some(pipe) = child.stderr.take() {
            let out = stderr.clone();
            let reader = std::thread::Builder::new()
                .name("host-stderr".into())
                .spawn(move || {
                    for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                        let line = line.trim().to_string();
                        if !line.is_empty() {
                            *out.last.lock().unwrap_or_else(|p| p.into_inner()) = line;
                        }
                    }
                    out.done.store(true, Ordering::Release);
                });
            if let Err(e) = reader {
                kill(child);
                return Err(e);
            }
        }
        Ok(Dialed {
            stream: ours,
            child: Some(child),
            stderr,
        })
    })
}

/// The `ssh` processes of the open connections. At quit the app kills them: an `ssh` whose
/// remote command never ends does not stop at the end of its input.
/// After `kill_all`, a new `ssh` dies at once and the host threads end.
#[derive(Default)]
struct Children {
    quit: bool,
    open: BTreeMap<u64, Child>,
}

static CHILDREN: Mutex<Children> = Mutex::new(Children {
    quit: false,
    open: BTreeMap::new(),
});
static NEXT_CHILD: AtomicU64 = AtomicU64::new(0);

fn children() -> std::sync::MutexGuard<'static, Children> {
    CHILDREN.lock().unwrap_or_else(|p| p.into_inner())
}

fn quitting() -> bool {
    children().quit
}

#[cfg(test)]
fn quit_reset() {
    children().quit = false;
}

fn kill(mut c: Child) {
    let _ = c.kill();
    let _ = c.wait();
}

/// Kills each `ssh` process of the hosts. Call it when the app quits.
pub fn kill_all() {
    let all = {
        let mut c = children();
        c.quit = true;
        std::mem::take(&mut c.open)
    };
    all.into_values().for_each(kill);
}

/// Kills and reaps the `ssh` process when a connection ends.
struct Reap(Option<u64>);

impl Reap {
    fn new(child: Option<Child>) -> Reap {
        let Some(child) = child else {
            return Reap(None);
        };
        let mut c = children();
        if c.quit {
            drop(c);
            kill(child);
            return Reap(None);
        }
        let key = NEXT_CHILD.fetch_add(1, Ordering::Relaxed);
        c.open.insert(key, child);
        Reap(Some(key))
    }

    /// The exit status of the `ssh` process, if it stops within `limit`.
    fn status(&self, limit: Duration) -> Option<ExitStatus> {
        let key = self.0?;
        let deadline = Instant::now() + limit;
        loop {
            let status = children().open.get_mut(&key)?.try_wait().ok().flatten();
            if status.is_some() || Instant::now() >= deadline {
                return status;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Reap {
    fn drop(&mut self) {
        let child = self.0.and_then(|key| children().open.remove(&key));
        if let Some(c) = child {
            kill(c);
        }
    }
}

/// Wakes the thread of `wait`.
#[derive(Default)]
struct Signal {
    woken: Mutex<bool>,
    cond: Condvar,
}

impl Wake for Signal {
    fn wake(self: Arc<Self>) {
        *self.woken.lock().unwrap_or_else(|p| p.into_inner()) = true;
        self.cond.notify_one();
    }
}

/// Polls `fut` on this thread until it ends, or for `limit` at most.
fn wait<T>(fut: impl std::future::Future<Output = T>, limit: Duration) -> Option<T> {
    let deadline = Instant::now() + limit;
    let signal = Arc::new(Signal::default());
    let waker = Waker::from(signal.clone());
    let mut cx = std::task::Context::from_waker(&waker);
    let mut fut = std::pin::pin!(fut);
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return Some(v);
        }
        let mut woken = signal.woken.lock().unwrap_or_else(|p| p.into_inner());
        while !*woken {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            woken = signal
                .cond
                .wait_timeout(woken, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        *woken = false;
    }
}

/// The thread of one host.
pub struct Host {
    pub index: usize,
    pub name: String,
    /// `hosts.toml`, to save a new product list. `None` in tests.
    pub file: Option<PathBuf>,
    /// The product list in the file.
    pub saved: Vec<ProductInfo>,
    pub local: Arc<Link>,
    pub limits: Limits,
    pub dial: Dial,
    pub updates: mpsc::UnboundedSender<Update>,
    /// The state that the window shows, so the thread reports only changes.
    shown: Option<HostState>,
    note: Option<String>,
}

/// How a connection ended.
struct Ended {
    /// How long the connection was up, after it read the products.
    up_for: Option<Duration>,
    why: String,
}

impl Host {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        index: usize,
        name: String,
        file: Option<PathBuf>,
        saved: Vec<ProductInfo>,
        local: Arc<Link>,
        limits: Limits,
        dial: Dial,
        updates: mpsc::UnboundedSender<Update>,
    ) -> Host {
        Host {
            index,
            name,
            file,
            saved,
            local,
            limits,
            dial,
            updates,
            shown: None,
            note: None,
        }
    }

    pub fn spawn(self) -> io::Result<()> {
        std::thread::Builder::new()
            .name(format!("host-{}", self.name))
            .spawn(move || self.run())
            .map(|_| ())
    }

    /// false when the window is gone: the thread ends.
    fn send(&self, change: Change) -> bool {
        self.updates
            .unbounded_send(Update {
                host: self.index,
                change,
            })
            .is_ok()
    }

    fn set_state(&mut self, state: HostState) -> bool {
        if self.shown.as_ref() == Some(&state) {
            return !self.updates.is_closed();
        }
        match &state {
            HostState::Off(why) => eprintln!("agentsi-app: host {}: device off: {why}", self.name),
            s => eprintln!("agentsi-app: host {}: {}", self.name, s.word()),
        }
        self.shown = Some(state.clone());
        self.send(Change::State(state))
    }

    fn set_note(&mut self, note: Option<String>) -> bool {
        if self.note == note {
            return !self.updates.is_closed();
        }
        self.note = note.clone();
        self.send(Change::Note(note))
    }

    pub fn run(mut self) {
        let mut failures = 0;
        if !self.set_state(HostState::Connecting) {
            return;
        }
        loop {
            let ended = self.connect_once();
            if !self.set_state(HostState::Off(ended.why)) {
                return;
            }
            if ended.up_for.is_some_and(|d| d >= self.limits.stable) {
                failures = 0;
            }
            std::thread::sleep(self.limits.delay(failures));
            failures += 1;
            if self.updates.is_closed() || quitting() {
                return;
            }
        }
    }

    fn connect_once(&mut self) -> Ended {
        let off = |why: String| Ended { up_for: None, why };
        let dialed = match (self.dial)() {
            Ok(d) => d,
            Err(e) => return off(format!("ssh does not start: {e}")),
        };
        let reap = Reap::new(dialed.child);
        let stderr = dialed.stderr;
        // The reason of a failed login is the last stderr line of an ssh that failed. A live
        // ssh, or one that stopped with 0, can have an old warning there.
        let reason = |why: String| {
            let failed = reap
                .status(Duration::from_millis(200))
                .is_some_and(|s| !s.success());
            let last = if failed {
                stderr.last(Duration::from_millis(200))
            } else {
                String::new()
            };
            if !last.is_empty() { last } else { why }
        };
        let (link, mut events) =
            match Link::start(dialed.stream, "agentsi-app", Some(self.limits.hello)) {
                Ok(l) => l,
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return off(reason(format!(
                        "no answer in {} seconds",
                        self.limits.hello.as_secs()
                    )));
                }
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    return off(reason("the connection closed".into()));
                }
                Err(e) => return off(reason(e.to_string())),
            };
        let subscribed = wait(
            link.call::<Subscribe>(&SubscribeParams {
                topics: vec![
                    Topic::Voice,
                    Topic::Terminal,
                    Topic::Data,
                    Topic::Meeting,
                    Topic::Desk,
                ],
            }),
            self.limits.call,
        );
        let products = wait(link.call::<ProductList>(&Empty {}), self.limits.call);
        let products = match (subscribed, products) {
            (Some(Ok(_)), Some(Ok(p))) => {
                self.set_note(None);
                crate::model::listed(p.products)
            }
            (Some(Ok(_)), Some(Err(CallError::Rpc(e)))) if e.kind() == ErrorKind::Unavailable => {
                self.set_note(Some(format!("The host has no product store: {e}")));
                Vec::new()
            }
            (Some(Err(e)), _) | (_, Some(Err(e))) => return off(format!("the remote core: {e}")),
            _ => return off("the remote core does not answer".into()),
        };
        if products != self.saved {
            self.save(&products);
            if !self.send(Change::Products(products)) {
                return off("the window closed".into());
            }
        }
        if !self.send(Change::Link(Linked(link.clone()))) || !self.set_state(HostState::Up) {
            return off("the window closed".into());
        }
        let up = Instant::now();
        let why = loop {
            match block_on(events.next()) {
                Some(LinkEvent::Notification(n)) if n.method.starts_with("voice.") => {
                    self.relay(&n)
                }
                Some(LinkEvent::Notification(n)) => {
                    self.send(Change::Event(n));
                }
                // An EOF comes from the remote core or from `ssh`: the link cannot tell which.
                Some(LinkEvent::Lost(why)) if why == crate::link::CLOSED => {
                    break "the connection to the host closed".into();
                }
                Some(LinkEvent::Lost(why)) => break why,
                None => break "the connection closed".into(),
            }
        };
        Ended {
            up_for: Some(up.elapsed()),
            why: reason(why),
        }
    }

    fn save(&mut self, products: &[ProductInfo]) {
        self.saved = products.to_vec();
        if let Some(file) = &self.file
            && let Err(e) = config::save_products(file, &self.name, products)
        {
            eprintln!(
                "agentsi-app: cannot save the products of {} to {}: {e}",
                self.name,
                file.display()
            );
        }
    }

    /// Sends one voice event of the remote core to the local core. An event that a core
    /// already relayed stays: two hosts that list each other must not loop.
    fn relay(&mut self, n: &Notification) {
        let params = if let Some(p) = n.parse::<VoiceSpeak>() {
            VoiceRelayParams {
                host: self.name.clone(),
                terminal: p.terminal,
                role: Some(p.role),
                text: Some(p.text),
            }
        } else if let Some(p) = n.parse::<VoiceStop>() {
            VoiceRelayParams {
                host: self.name.clone(),
                terminal: p.terminal,
                role: None,
                text: None,
            }
        } else {
            return;
        };
        if is_relayed(&params.terminal) {
            return;
        }
        match wait(self.local.call::<VoiceRelay>(&params), self.limits.call) {
            Some(Ok(_)) => {}
            Some(Err(CallError::Rpc(e))) if e.kind() == ErrorKind::Conflict => {
                self.set_note(Some(
                    "This host is this device: its voice is spoken once, by the local core.".into(),
                ));
            }
            Some(Err(e)) => eprintln!("agentsi-app: voice of {} not relayed: {e}", self.name),
            None => eprintln!(
                "agentsi-app: voice of {} not relayed: the local core does not answer",
                self.name
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::tests::{FakeCore, pair};
    use agentsi_proto::frame::Frame;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The tests that start processes run one at a time: `kill_all` kills every child.
    static PROCESSES: Mutex<()> = Mutex::new(());

    fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
        PROCESSES.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn quick() -> Limits {
        Limits {
            hello: Duration::from_millis(300),
            call: Duration::from_secs(2),
            backoff: vec![Duration::from_millis(50)],
            stable: Duration::from_secs(10),
        }
    }

    fn host(dial: Dial, local: Arc<Link>) -> (Host, mpsc::UnboundedReceiver<Update>) {
        let (tx, rx) = mpsc::unbounded();
        let h = Host::new(
            0,
            "studio".into(),
            None,
            Vec::new(),
            local,
            quick(),
            dial,
            tx,
        );
        (h, rx)
    }

    /// The next change, apart from the link of a connection.
    fn next(rx: &mut mpsc::UnboundedReceiver<Update>) -> Change {
        loop {
            let u = wait(rx.next(), Duration::from_secs(10))
                .flatten()
                .expect("an update");
            if !matches!(u.change, Change::Link(_)) {
                return u.change;
            }
        }
    }

    fn off(c: &Change) -> String {
        match c {
            Change::State(HostState::Off(why)) => why.clone(),
            other => panic!("not off: {other:?}"),
        }
    }

    #[test]
    fn the_backoff_doubles_up_to_60_seconds() {
        let l = Limits::default();
        let secs: Vec<u64> = (0..9).map(|n| l.delay(n).as_secs()).collect();
        assert_eq!(secs, [1, 2, 4, 8, 16, 32, 60, 60, 60]);
    }

    #[test]
    fn a_host_that_never_answers_is_off_after_the_hello_limit() {
        let _one = one_at_a_time();
        let (local, _events, _core) = pair();
        let (h, mut rx) = host(spawn_dial(vec!["sleep".into(), "30".into()]), local);
        let start = Instant::now();
        h.spawn().unwrap();
        assert_eq!(next(&mut rx), Change::State(HostState::Connecting));
        let why = off(&next(&mut rx));
        assert!(why.contains("no answer"), "{why}");
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn a_host_that_closes_at_once_is_off_with_the_ssh_reason_and_reports_it_once() {
        let _one = one_at_a_time();
        let (local, _events, _core) = pair();
        let tries = Arc::new(AtomicUsize::new(0));
        let mut inner = spawn_dial(vec![
            "sh".into(),
            "-c".into(),
            "echo 'ssh: Permission denied (publickey).' >&2; exit 255".into(),
        ]);
        let t = tries.clone();
        let dial: Dial = Box::new(move || {
            t.fetch_add(1, Ordering::SeqCst);
            inner()
        });
        let (h, mut rx) = host(dial, local);
        h.spawn().unwrap();
        assert_eq!(next(&mut rx), Change::State(HostState::Connecting));
        let why = off(&next(&mut rx));
        assert!(why.contains("Permission denied"), "{why}");
        std::thread::sleep(Duration::from_millis(600));
        assert!(tries.load(Ordering::SeqCst) >= 3, "the thread tries again");
        // The same reason each try: no new update, so the window draws nothing.
        assert!(rx.try_recv().is_err(), "an update between tries");
        drop(rx);
    }

    #[test]
    fn kill_all_ends_the_ssh_of_a_host_that_hangs() {
        let _one = one_at_a_time();
        let (local, _events, _core) = pair();
        let (tx, mut rx) = mpsc::unbounded();
        let limits = Limits {
            hello: Duration::from_secs(30),
            ..quick()
        };
        let dial = spawn_dial(vec!["sleep".into(), "30".into()]);
        let h = Host::new(0, "a".into(), None, Vec::new(), local, limits, dial, tx);
        h.spawn().unwrap();
        assert_eq!(next(&mut rx), Change::State(HostState::Connecting));
        // After the kill, a child that registers later dies at once too.
        let killed = Instant::now();
        kill_all();
        let update = wait(rx.next(), Duration::from_secs(5));
        quit_reset();
        let why = off(&update.flatten().expect("the host is off").change);
        assert!(killed.elapsed() < Duration::from_secs(5));
        // The reason is an EOF or a reset: the hello was written before the kill.
        assert!(!why.contains("no answer"), "{why}");
        drop(rx);
    }

    /// A remote core that answers the start of a connection, then closes it at once.
    fn flaky(mut core: FakeCore) {
        core.hello();
        for _ in 0..2 {
            let req = core.request();
            let result = match req["method"].as_str() {
                Some("core.subscribe") => json!({"topics": ["voice"]}),
                _ => json!({"products": []}),
            };
            core.reply(&req["id"], result);
        }
    }

    #[test]
    fn a_host_that_closes_after_each_start_backs_off() {
        let _one = one_at_a_time();
        let (local, _events, _core) = pair();
        let tries = Arc::new(AtomicUsize::new(0));
        let t = tries.clone();
        let dial: Dial = Box::new(move || {
            t.fetch_add(1, Ordering::SeqCst);
            let (a, b) = UnixStream::pair()?;
            std::thread::spawn(move || flaky(FakeCore(b)));
            Ok(Dialed {
                stream: a,
                child: None,
                stderr: Arc::default(),
            })
        });
        let (tx, rx) = mpsc::unbounded();
        let limits = Limits {
            backoff: [40, 80, 160, 320].map(Duration::from_millis).to_vec(),
            ..quick()
        };
        let h = Host::new(0, "a".into(), None, Vec::new(), local, limits, dial, tx);
        h.spawn().unwrap();
        std::thread::sleep(Duration::from_millis(700));
        let n = tries.load(Ordering::SeqCst);
        drop(rx);
        // With backoff: tries near 0, 40, 120, 280 and 600 ms. Without it: one every 40 ms.
        assert!(n <= 7, "{n} tries in 700 ms");
    }

    #[test]
    fn the_reason_of_an_ssh_that_fails_at_once_is_its_last_stderr_line_each_try() {
        let _one = one_at_a_time();
        let (local, _events, _core) = pair();
        let dial = spawn_dial(vec![
            "sh".into(),
            "-c".into(),
            "echo 'ssh: connect to host x port 1: Connection refused' >&2; exit 255".into(),
        ]);
        let (mut h, _rx) = host(dial, local);
        for _ in 0..20 {
            let why = h.connect_once().why;
            assert!(why.contains("Connection refused"), "{why}");
        }
    }

    #[test]
    fn a_stale_ssh_warning_does_not_hide_the_real_reason() {
        let _one = one_at_a_time();
        let (local, _events, _core) = pair();
        let dial = spawn_dial(vec![
            "sh".into(),
            "-c".into(),
            "echo 'Warning: Permanently added host to the list of known hosts.' >&2; sleep 30"
                .into(),
        ]);
        let (h, mut rx) = host(dial, local);
        h.spawn().unwrap();
        assert_eq!(next(&mut rx), Change::State(HostState::Connecting));
        let why = off(&next(&mut rx));
        assert!(why.contains("no answer"), "{why}");
        drop(rx);
    }

    #[test]
    fn an_ssh_that_starts_after_kill_all_is_killed_too() {
        let _one = one_at_a_time();
        kill_all();
        let (local, _events, _core) = pair();
        let (tx, mut rx) = mpsc::unbounded();
        let limits = Limits {
            hello: Duration::from_secs(30),
            ..quick()
        };
        let dial = spawn_dial(vec!["sleep".into(), "30".into()]);
        let h = Host::new(0, "a".into(), None, Vec::new(), local, limits, dial, tx);
        let start = Instant::now();
        h.spawn().unwrap();
        assert_eq!(next(&mut rx), Change::State(HostState::Connecting));
        let off_at = wait(rx.next(), Duration::from_secs(5));
        quit_reset();
        assert!(
            matches!(off_at, Some(Some(_))),
            "the ssh started after the quit and still runs"
        );
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    /// A remote core that answers the start, sends voice events, then stops.
    fn remote(mut core: FakeCore) {
        core.hello();
        for _ in 0..2 {
            let req = core.request();
            let result = match req["method"].as_str() {
                Some("core.subscribe") => json!({"topics": ["voice"]}),
                Some("product.list") => {
                    json!({"products": [{"id": "web", "name": "Web", "path": "/srv/web"}]})
                }
                other => panic!("{other:?}"),
            };
            core.reply(&req["id"], result);
        }
        let speak = |terminal: &str| {
            json!({"jsonrpc": "2.0", "method": "voice.speak",
                   "params": {"terminal": terminal, "role": "qa", "text": "Done."}})
            .to_string()
        };
        // An event that another app already relayed stays on that core.
        core.send(Frame::message(&speak("other/t1-zzzzzz")));
        core.send(Frame::message(&speak("t1-abc123")));
        let changed = json!({"jsonrpc": "2.0", "method": "data.changed",
                             "params": {"product": "web", "kind": "minutes"}});
        core.send(Frame::message(&changed.to_string()));
        std::thread::sleep(Duration::from_millis(200));
    }

    #[test]
    fn a_host_reports_its_products_relays_its_voice_once_and_is_off_when_it_stops() {
        let _one = one_at_a_time();
        let (local, _events, mut local_core) = pair();
        let mut first = true;
        let dial: Dial = Box::new(move || {
            if !first {
                return Err(io::Error::other("refused"));
            }
            first = false;
            let (a, b) = UnixStream::pair()?;
            std::thread::spawn(move || remote(FakeCore(b)));
            Ok(Dialed {
                stream: a,
                child: None,
                stderr: Arc::default(),
            })
        });
        let (h, mut rx) = host(dial, local);
        h.spawn().unwrap();
        assert_eq!(next(&mut rx), Change::State(HostState::Connecting));
        assert_eq!(
            next(&mut rx),
            Change::Products(vec![ProductInfo {
                id: "web".into(),
                name: "Web".into(),
                path: Some("/srv/web".into()),
                archived: false,
            }])
        );
        assert_eq!(next(&mut rx), Change::State(HostState::Up));
        // Only a voice event reaches the local core. Each other event goes to the window.
        assert!(matches!(next(&mut rx), Change::Event(n) if n.method == "data.changed"));
        let relay = local_core.request();
        assert_eq!(relay["method"], "voice.relay");
        assert_eq!(
            relay["params"],
            json!({"host": "studio", "terminal": "t1-abc123", "role": "qa", "text": "Done."})
        );
        local_core.reply(&relay["id"], json!({}));
        let stopped = Instant::now();
        off(&next(&mut rx));
        assert!(stopped.elapsed() < Duration::from_secs(2));
        // The next try fails: the reason changes, and no second relay reached the local core.
        assert_eq!(off(&next(&mut rx)), "ssh does not start: refused");
        local_core.0.set_nonblocking(true).unwrap();
        assert!(agentsi_proto::frame::read_frame(&mut local_core.0).is_err());
    }

    #[test]
    fn the_link_of_a_connection_reaches_the_window_before_the_host_is_up() {
        let _one = one_at_a_time();
        let (local, _events, _core) = pair();
        let mut first = true;
        let dial: Dial = Box::new(move || {
            if !first {
                return Err(io::Error::other("refused"));
            }
            first = false;
            let (a, b) = UnixStream::pair()?;
            std::thread::spawn(move || remote(FakeCore(b)));
            Ok(Dialed {
                stream: a,
                child: None,
                stderr: Arc::default(),
            })
        });
        let (h, mut rx) = host(dial, local);
        h.spawn().unwrap();
        let mut seen = Vec::new();
        while !seen.contains(&"up") {
            let u = wait(rx.next(), Duration::from_secs(10)).flatten().unwrap();
            match u.change {
                Change::Link(_) => seen.push("link"),
                Change::State(HostState::Up) => seen.push("up"),
                _ => {}
            }
        }
        assert_eq!(seen, ["link", "up"]);
    }
}
