//! The terminals of the core: a PTY, its program and one libghostty-vt
//! screen each.
//!
//! Threads of one terminal:
//! - The reader reads the PTY, feeds the screen through the query filter,
//!   and queues the bytes for each attached pane. It never waits for a pane
//!   or for the input of the program.
//! - The writer writes the replies of the screen, the input of the clients
//!   and the AUTO bytes of the panes to the PTY, in one order.
//! - The waiter reaps the program and ends the streams.
//!
//! Lock order: the screen of a terminal, then the queue of an attach.

mod filter;
pub mod stream;

use crate::state::{Core, lock};
use agentsi_agency::git_env;
use agentsi_proto::RpcError;
use agentsi_proto::events::{
    ProgramState, TerminalExit, TerminalExitParams, TerminalState, TerminalStateParams,
    TerminalTitle, TerminalTitleParams,
};
use agentsi_proto::rpc::ErrorKind;
use agentsi_proto::socket::SOCKET_ENV;
use agentsi_proto::terminal_api::{CloseReason, CreateParams, TERMINAL_ENV, TerminalInfo};
use agentsi_pty::{Pty, PtySize, SpawnOptions};
use agentsi_vt::{AgentState, BlockedKind, Key, KeyAction, KeyEvent, Mods};
use filter::Filter;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak, mpsc};
use std::time::Duration;
use stream::Attach;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// The input that waits for the program, from clients and panes. Past it,
/// `terminal.input` returns `busy` and the core drops the input of a pane.
const INPUT_BUDGET: usize = 8 << 20;
/// The replies of the screen that wait for the program. A program that does
/// not read its input loses the replies past it.
const REPLY_BUDGET: usize = 1 << 20;
/// The scrollback of the screen of the core.
const SCROLLBACK_BYTES: usize = 2_000_000;
/// After `terminal.kill`, the time before SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(2);
/// After the program exits, the time that the core waits for the rest of
/// its output. A process that keeps the PTY open can hold it longer.
const OUTPUT_GRACE: Duration = Duration::from_secs(2);
/// The time between the text and the Return key of `terminal.input` with submit.
/// Claude Code 2.1.317 took 30 ms as a new line in 4 of 10 runs, and 60 ms as a submit in 10 of
/// 10 (U11). 100 ms keeps a margin for a loaded machine.
const SUBMIT_DELAY: Duration = Duration::from_millis(100);
/// The length of the run mark in a terminal id.
const RUN_MARK_LEN: usize = 6;
/// DEC private mode: focus reports.
const FOCUS_MODE: u16 = 1004;
/// The prefixes of the names that a core inside a Claude Code session removes
/// from the child environment, so the child does not act as a tool call.
const CLAUDE_ENV: [&str; 4] = ["CLAUDECODE", "CLAUDE_CODE", "CLAUDE_JOB", "CLAUDE_BG"];

fn invalid(message: impl Into<String>, hint: &str) -> RpcError {
    RpcError::new(ErrorKind::Invalid, message, hint)
}

fn check_size(cols: u16, rows: u16) -> Result<PtySize, RpcError> {
    if cols == 0 || rows == 0 {
        return Err(invalid(
            "cols and rows must be 1 or more",
            "Give the size in cells.",
        ));
    }
    Ok(PtySize { cols, rows })
}

fn not_found(id: &str) -> RpcError {
    RpcError::new(
        ErrorKind::NotFound,
        format!("no terminal {id:?}"),
        "Call terminal.list for the terminals of this core.",
    )
}

/// The terminals of one core.
pub struct Terminals {
    map: Mutex<BTreeMap<u64, Arc<Term>>>,
    next: AtomicU64,
    next_attach: AtomicU64,
    socket: OnceLock<PathBuf>,
    /// The mark of this core process in each terminal id. A saved id of an
    /// earlier core never names a terminal of this one.
    run: String,
}

impl Default for Terminals {
    fn default() -> Terminals {
        Terminals {
            map: Mutex::default(),
            next: AtomicU64::default(),
            next_attach: AtomicU64::default(),
            socket: OnceLock::new(),
            run: run_mark(),
        }
    }
}

/// 6 base-36 characters from the start time and the pid of this process.
fn run_mark() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let mut v = nanos ^ (u64::from(std::process::id()) << 40);
    let mut mark = String::new();
    for _ in 0..RUN_MARK_LEN {
        let digit = (v % 36) as u32;
        mark.push(char::from_digit(digit, 36).unwrap_or('0'));
        v /= 36;
    }
    mark
}

impl Terminals {
    /// The socket that the children get in `AGENTSI_CORE_SOCKET`.
    pub fn set_socket(&self, path: PathBuf) {
        let _ = self.socket.set(path);
    }

    pub fn get(&self, id: &str) -> Result<Arc<Term>, RpcError> {
        id.strip_prefix('t')
            .and_then(|rest| rest.split_once('-'))
            .filter(|(_, run)| *run == self.run)
            .and_then(|(n, _)| n.parse::<u64>().ok())
            .and_then(|n| lock(&self.map).get(&n).cloned())
            .ok_or_else(|| not_found(id))
    }

    pub fn list(&self) -> Vec<TerminalInfo> {
        let terms: Vec<Arc<Term>> = lock(&self.map).values().cloned().collect();
        terms.iter().map(|t| t.info()).collect()
    }

    fn remove(&self, term: &Term) {
        lock(&self.map).remove(&term.n);
    }

    pub fn new_attach_id(&self) -> u64 {
        self.next_attach.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn create(&self, core: &Arc<Core>, params: CreateParams) -> Result<TerminalInfo, RpcError> {
        if params.argv.is_empty() || params.argv[0].is_empty() {
            return Err(invalid(
                "argv is empty",
                "Give the program and its arguments in argv.",
            ));
        }
        let cwd = PathBuf::from(&params.cwd);
        if !cwd.is_dir() {
            return Err(invalid(
                format!("cwd {:?} is not a folder", params.cwd),
                "Give a folder that exists.",
            ));
        }
        let size = check_size(params.cols.unwrap_or(80), params.rows.unwrap_or(24))?;
        let n = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let id = format!("t{n}-{}", self.run);
        let screen = agentsi_vt::Terminal::new(agentsi_vt::Options {
            cols: size.cols,
            rows: size.rows,
            scrollback_bytes: SCROLLBACK_BYTES,
        })
        .map_err(|e| RpcError::internal(format!("cannot make the screen: {e}")))?;

        let mut env: Vec<(OsString, OsString)> = params
            .env
            .iter()
            .filter(|(k, _)| k.as_str() != "PATH")
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let path = params
            .env
            .get("PATH")
            .map(OsString::from)
            .or_else(|| std::env::var_os("PATH"));
        if let Some(path) = child_path(path) {
            env.push(("PATH".into(), path));
        }
        // The git config of the product reaches `claude` and the owner's own git commands in
        // the pane. A caller that sets its own count keeps it.
        if !params.env.contains_key("GIT_CONFIG_COUNT") {
            let vars = match &params.product {
                Some(product) => git_env::env_for_product(product),
                None => git_env::env_for_cwd(&cwd),
            };
            env.extend(vars.into_iter().map(|(k, v)| (k.into(), v.into())));
        }
        env.push((TERMINAL_ENV.into(), id.clone().into()));
        if let Some(socket) = self.socket.get() {
            env.push((SOCKET_ENV.into(), socket.into()));
        }
        let env_remove = std::env::vars_os()
            .map(|(k, _)| k)
            .filter(|k| {
                let k = k.to_string_lossy();
                CLAUDE_ENV.iter().any(|p| k.starts_with(p)) && !params.env.contains_key(&*k)
            })
            .collect();
        let pty = Pty::spawn(&SpawnOptions {
            argv: params.argv.clone(),
            cwd: Some(cwd),
            env,
            env_remove,
            size,
        })
        .map_err(|e| {
            invalid(
                format!("cannot start {:?}: {e}", params.argv[0]),
                "Check argv[0] and the PATH of the core.",
            )
        })?;
        let pty = Arc::new(pty);
        let reader = pty
            .reader()
            .map_err(|e| RpcError::internal(format!("cannot read the PTY: {e}")))?;

        let (tx, rx) = mpsc::channel::<Chunk>();
        let term = Arc::new(Term {
            n,
            id,
            argv: params.argv,
            cwd: params.cwd,
            product: params.product,
            pid: pty.pid(),
            pty: pty.clone(),
            core: Arc::downgrade(core),
            screen: Mutex::new(Screen {
                vt: screen,
                filter: Filter::default(),
                attaches: Vec::new(),
                size,
                cell: (0, 0),
                title: String::new(),
                state: None,
                exit: None,
                killed: false,
                focus: None,
                osc: false,
                wait_tool: None,
            }),
            voice: Mutex::new(Voice::default()),
            writer: Mutex::new(tx),
            input_budget: Arc::new(Semaphore::new(INPUT_BUDGET)),
            replies: Arc::new(AtomicUsize::new(0)),
            output_done: (Mutex::new(false), Condvar::new()),
        });
        lock(&self.map).insert(n, term.clone());

        let replies = term.replies.clone();
        let writer_pty = pty.clone();
        spawn_thread("pty-writer", move || write_loop(&writer_pty, rx, &replies));
        let t = term.clone();
        spawn_thread("pty-reader", move || t.read_loop(reader));
        let t = term.clone();
        spawn_thread("pty-waiter", move || t.wait_loop());
        Ok(term.info())
    }
}

/// The `PATH` of a child. When `path` already finds this `agentsi-core` first, it stays as it
/// is. Else the folder of the core program goes first, so the hook bridge `agentsi-core hook`
/// runs in each Claude session of a terminal, and the order of the user's folders stays.
fn child_path(path: Option<OsString>) -> Option<OsString> {
    let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|e| e.canonicalize().ok())
    else {
        return path;
    };
    let Some(dir) = exe.parent().map(PathBuf::from) else {
        return path;
    };
    let dirs: Vec<PathBuf> = path
        .as_deref()
        .map(|p| std::env::split_paths(p).collect())
        .unwrap_or_default();
    let found = dirs
        .iter()
        .map(|d| d.join("agentsi-core"))
        .find(|c| c.is_file());
    if found.and_then(|c| c.canonicalize().ok()).as_ref() == Some(&exe) {
        return path;
    }
    let rest: Vec<PathBuf> = dirs.into_iter().filter(|d| *d != dir).collect();
    std::env::join_paths(std::iter::once(dir).chain(rest))
        .ok()
        .or(path)
}

fn spawn_thread(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(f) {
        eprintln!("core: cannot start the {name} thread: {e}");
    }
}

/// Bytes for the PTY. A permit holds a part of the input budget until the
/// writer wrote the bytes. Bytes without a permit are replies.
struct Chunk {
    bytes: Vec<u8>,
    /// Bytes that the writer writes `SUBMIT_DELAY` after `bytes`, with no other bytes between.
    then: Option<Vec<u8>>,
    permit: Option<OwnedSemaphorePermit>,
}

fn write_loop(pty: &Pty, rx: mpsc::Receiver<Chunk>, replies: &AtomicUsize) {
    let mut broken = false;
    for chunk in rx {
        let mut write = |bytes: &[u8]| {
            if !broken && let Err(e) = pty.write(bytes) {
                eprintln!("core: the PTY of pid {} takes no input: {e}", pty.pid());
                broken = true;
            }
        };
        write(&chunk.bytes);
        if let Some(then) = &chunk.then {
            std::thread::sleep(SUBMIT_DELAY);
            write(then);
        }
        if chunk.permit.is_none() {
            replies.fetch_sub(chunk.bytes.len(), Ordering::Relaxed);
        }
    }
}

/// The program state as the protocol gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    state: ProgramState,
    kind: Option<String>,
    msg: Option<String>,
}

struct Screen {
    vt: agentsi_vt::Terminal,
    filter: Filter,
    attaches: Vec<Arc<Attach>>,
    size: PtySize,
    /// The size of a cell in pixels, from the last resize that gave it.
    cell: (u16, u16),
    title: String,
    state: Option<State>,
    /// `Some` after the program stopped: its exit code, or `None` for a signal.
    exit: Option<Option<i32>>,
    killed: bool,
    /// The attach that sent the last FOCUS 1.
    focus: Option<u64>,
    /// true after the program sent an OSC 7501 record. Then the hooks never set the state.
    osc: bool,
    /// The tool of a `PermissionRequest` hook that set `blocked`. Only a `PostToolUse` of
    /// this tool ends the wait: tools run in parallel.
    wait_tool: Option<String>,
}

impl Screen {
    /// The bytes of a first paint: a reset, then the replay of the screen.
    fn first_paint(&mut self) -> Vec<u8> {
        let mut bytes = b"\x1bc".to_vec();
        match self.vt.replay() {
            Ok(replay) => bytes.extend(replay),
            Err(e) => eprintln!("core: no replay of the screen: {e}"),
        }
        bytes
    }

    /// The attach whose AUTO bytes go to the program.
    fn auto_owner(&self) -> Option<u64> {
        self.focus
            .filter(|id| self.attaches.iter().any(|a| a.id == *id))
            .or_else(|| self.attaches.iter().find(|a| !a.observe).map(|a| a.id))
    }
}

/// Voice mode of one terminal.
#[derive(Default)]
struct Voice {
    on: bool,
    /// The last prompt of the owner, for the transcript.
    prompt: String,
}

/// One terminal.
pub struct Term {
    n: u64,
    pub id: String,
    argv: Vec<String>,
    cwd: String,
    product: Option<String>,
    pid: u32,
    pty: Arc<Pty>,
    core: Weak<Core>,
    screen: Mutex<Screen>,
    voice: Mutex<Voice>,
    writer: Mutex<mpsc::Sender<Chunk>>,
    input_budget: Arc<Semaphore>,
    replies: Arc<AtomicUsize>,
    /// true after the reader read the end of the output.
    output_done: (Mutex<bool>, Condvar),
}

impl Term {
    pub fn info(&self) -> TerminalInfo {
        let s = lock(&self.screen);
        TerminalInfo {
            id: self.id.clone(),
            argv: self.argv.clone(),
            cwd: self.cwd.clone(),
            product: self.product.clone(),
            pid: self.pid,
            cols: s.size.cols,
            rows: s.size.rows,
            title: s.title.clone(),
            state: s.state.as_ref().map(|st| st.state),
            kind: s.state.as_ref().and_then(|st| st.kind.clone()),
            msg: s.state.as_ref().and_then(|st| st.msg.clone()),
            running: s.exit.is_none(),
            code: s.exit.flatten(),
        }
    }

    fn publish<E: agentsi_proto::Event>(&self, params: &E::Params) {
        if let Some(core) = self.core.upgrade() {
            core.publish::<E>(params);
        }
    }

    /// Queues a reply of the screen. It never waits.
    fn reply(&self, bytes: Vec<u8>) {
        if self.replies.fetch_add(bytes.len(), Ordering::Relaxed) + bytes.len() > REPLY_BUDGET {
            self.replies.fetch_sub(bytes.len(), Ordering::Relaxed);
            eprintln!("core: {} reads no input; a reply was dropped", self.id);
            return;
        }
        let n = bytes.len();
        if lock(&self.writer)
            .send(Chunk {
                bytes,
                then: None,
                permit: None,
            })
            .is_err()
        {
            self.replies.fetch_sub(n, Ordering::Relaxed);
        }
    }

    /// Queues input for the program, after the input that came before it.
    /// It never waits: past the input budget it returns `busy`.
    pub fn input(&self, bytes: &[u8]) -> Result<(), RpcError> {
        self.queue(bytes.to_vec(), None)
    }

    /// Queues text, then the Return key `SUBMIT_DELAY` later. No other input comes between them.
    pub fn submit(&self, text: &[u8]) -> Result<(), RpcError> {
        let enter = self.enter_key();
        self.queue(text.to_vec(), Some(enter))
    }

    fn queue(&self, bytes: Vec<u8>, then: Option<Vec<u8>>) -> Result<(), RpcError> {
        if lock(&self.screen).exit.is_some() {
            return Err(RpcError::new(
                ErrorKind::Conflict,
                format!("the program of {} stopped", self.id),
                "Create a new terminal.",
            ));
        }
        let size = bytes.len() + then.as_ref().map_or(0, Vec::len);
        let permit = u32::try_from(size)
            .ok()
            .and_then(|n| self.input_budget.clone().try_acquire_many_owned(n).ok())
            .ok_or_else(|| {
                RpcError::new(
                    ErrorKind::Busy,
                    format!("the program of {} does not read its input", self.id),
                    "Wait until the program reads, or send less than 8 MiB.",
                )
            })?;
        let _ = lock(&self.writer).send(Chunk {
            bytes,
            then,
            permit: Some(permit),
        });
        Ok(())
    }

    /// The bytes of the Return key, with the keyboard modes of the program.
    fn enter_key(&self) -> Vec<u8> {
        let event = KeyEvent {
            action: KeyAction::Press,
            key: Key::Enter,
            mods: Mods::NONE,
            consumed_mods: Mods::NONE,
            text: Some("\r".into()),
            unshifted: None,
            composing: false,
        };
        match lock(&self.screen).vt.encode_key(&event) {
            Ok(bytes) if !bytes.is_empty() => bytes,
            _ => b"\r".to_vec(),
        }
    }

    /// AUTO bytes of a pane. Only the bytes of one pane go to the program.
    pub fn auto(&self, attach: u64, bytes: &[u8]) -> Result<(), RpcError> {
        if lock(&self.screen).auto_owner() != Some(attach) {
            return Ok(());
        }
        self.input(bytes)
    }

    pub fn resize(&self, cols: u16, rows: u16, cell: Option<(u16, u16)>) -> Result<(), RpcError> {
        let size = check_size(cols, rows)?;
        let mut s = lock(&self.screen);
        let cell = cell.unwrap_or(s.cell);
        if size == s.size && cell == s.cell {
            return Ok(());
        }
        s.vt.resize(cols, rows, u32::from(cell.0), u32::from(cell.1))
            .map_err(|e| {
                invalid(
                    format!("the screen refused {cols}x{rows}: {e}"),
                    "Give a smaller size.",
                )
            })?;
        if s.exit.is_none()
            && let Err(e) = self.pty.resize(size)
        {
            eprintln!("core: resize of {}: {e}", self.id);
        }
        let changed = size != s.size;
        s.size = size;
        s.cell = cell;
        if changed {
            for a in &s.attaches {
                a.push_resize(cols, rows);
            }
        }
        Ok(())
    }

    /// A pane got or lost the focus. The program gets a focus report when the
    /// terminal gets its first focused pane, or loses it.
    pub fn focus(&self, attach: u64, focused: bool) {
        let mut s = lock(&self.screen);
        let before = s.focus.is_some();
        if focused {
            s.focus = Some(attach);
        } else if s.focus == Some(attach) {
            s.focus = None;
        }
        self.report_focus(s, before);
    }

    fn report_focus(&self, s: std::sync::MutexGuard<'_, Screen>, before: bool) {
        let after = s.focus.is_some();
        if before == after || s.exit.is_some() || !s.vt.mode(FOCUS_MODE) {
            return;
        }
        drop(s);
        self.reply(if after { b"\x1b[I" } else { b"\x1b[O" }.to_vec());
    }

    /// Adds a pane with its first paint. A pane of a stopped program gets the CLOSE after the paint.
    pub fn attach(&self, attach: Arc<Attach>) {
        let mut s = lock(&self.screen);
        let paint = s.first_paint();
        attach.paint(s.size.cols, s.size.rows, paint);
        if let Some(code) = s.exit {
            attach.push_close(close_reason(&CloseReason::Exit { code }));
            return;
        }
        s.attaches.push(attach);
    }

    /// A new first paint for a pane that lost bytes.
    pub fn resync(&self, attach: &Attach) {
        let mut s = lock(&self.screen);
        if attach.needs_resync() {
            let paint = s.first_paint();
            attach.paint(s.size.cols, s.size.rows, paint);
        }
    }

    pub fn detach(&self, attach: &Attach) {
        attach.detach();
        let mut s = lock(&self.screen);
        s.attaches.retain(|a| a.id != attach.id);
        if s.focus == Some(attach.id) {
            s.focus = None;
            self.report_focus(s, true);
        }
    }

    /// `terminal.kill`: the process group gets SIGHUP and SIGTERM, then
    /// SIGKILL. The terminal leaves the list when its program stopped.
    pub fn kill(self: &Arc<Self>, terminals: &Terminals) {
        let mut s = lock(&self.screen);
        let stopped = s.exit.is_some();
        s.killed = true;
        drop(s);
        if stopped {
            terminals.remove(self);
            self.forget_focus();
            self.signal_rest(libc::SIGHUP);
            self.signal_rest(libc::SIGTERM);
        } else {
            let _ = self.pty.kill();
        }
        let term = self.clone();
        spawn_thread("pty-kill", move || {
            std::thread::sleep(KILL_GRACE);
            if term.pty.signal(libc::SIGKILL).is_err() {
                term.signal_rest(libc::SIGKILL);
            }
        });
    }

    /// Signals the processes of the group that outlive the program and keep
    /// the PTY open. Linux does not give the pid of a group to a new process
    /// while the group has a member, so the id stays safe after the reap. The
    /// end of the output means that no member holds the PTY.
    fn signal_rest(&self, signal: i32) {
        if *lock(&self.output_done.0) {
            return;
        }
        // SAFETY: killpg takes plain integers. See the doc above for the pid.
        unsafe { libc::killpg(self.pid as libc::pid_t, signal) };
    }

    fn read_loop(self: Arc<Self>, mut reader: agentsi_pty::PtyReader) {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.output(&buf[..n], false),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => {
                    eprintln!("core: read of the PTY of {}: {e}", self.id);
                    break;
                }
            }
        }
        self.output(&[], true);
        let (done, cv) = &self.output_done;
        *lock(done) = true;
        cv.notify_all();
    }

    /// Feeds output to the screen and queues it for the panes. `end`: the output ended.
    fn output(&self, bytes: &[u8], end: bool) {
        let mut guard = lock(&self.screen);
        let Screen {
            vt,
            filter,
            attaches,
            title,
            state,
            osc,
            ..
        } = &mut *guard;
        let mut out = Vec::with_capacity(bytes.len());
        let mut feed = |b: &[u8]| {
            vt.write(b);
            let mut answered = false;
            for event in vt.take_events() {
                match event {
                    agentsi_vt::Event::Reply(r) => {
                        answered = true;
                        self.reply(r);
                    }
                    agentsi_vt::Event::Status(r) if r.id.is_empty() => {
                        *osc = true;
                        let next = program_state(&r);
                        self.set_state(state, next);
                    }
                    agentsi_vt::Event::StatusCleared(id) if id.is_empty() => {
                        *osc = true;
                        if state.is_some() {
                            self.set_state(state, idle());
                        }
                    }
                    agentsi_vt::Event::Title(t) if *title != t => {
                        title.clone_from(&t);
                        self.publish::<TerminalTitle>(&TerminalTitleParams {
                            id: self.id.clone(),
                            title: t,
                        });
                    }
                    _ => {}
                }
            }
            answered
        };
        if end {
            filter.finish(&mut feed, &mut out);
        } else {
            filter.push(bytes, &mut feed, &mut out);
        }
        for a in attaches.iter() {
            a.push_bytes(&out);
        }
    }

    /// Sets the state from a Claude Code hook. A program that sent an OSC
    /// 7501 record owns its state, so the hooks do nothing for it. `tool` is the tool of
    /// a permission wait.
    pub fn hook_state(
        &self,
        state: ProgramState,
        kind: Option<String>,
        msg: Option<String>,
        tool: Option<String>,
    ) {
        let mut s = lock(&self.screen);
        if s.osc || s.exit.is_some() {
            return;
        }
        s.wait_tool = tool;
        self.set_state(&mut s.state, State { state, kind, msg });
    }

    /// A terminal that leaves the list is no longer the target of dictation.
    fn forget_focus(&self) {
        if let Some(core) = self.core.upgrade() {
            core.forget_focus(&self.id);
            crate::converse::end(&core, Some(&self.id));
        }
    }

    /// The tool `tool` ran after a hook set `blocked`: the program works again. The
    /// hooks give no event when the owner grants a permission. Another tool that ran in
    /// parallel does not end the wait of a `PermissionRequest`.
    pub fn hook_resume(&self, tool: Option<&str>) {
        let mut s = lock(&self.screen);
        let blocked = s
            .state
            .as_ref()
            .is_some_and(|st| st.state == ProgramState::Blocked);
        let other_tool = s.wait_tool.as_deref().is_some_and(|w| Some(w) != tool);
        if s.osc || s.exit.is_some() || !blocked || other_tool {
            return;
        }
        s.wait_tool = None;
        self.set_state(
            &mut s.state,
            State {
                state: ProgramState::Working,
                kind: None,
                msg: None,
            },
        );
    }

    /// Turns voice mode on or off. `None` only reads it. Gives the state after the call.
    pub fn voice_mode(&self, on: Option<bool>) -> bool {
        let mut v = lock(&self.voice);
        if let Some(on) = on {
            v.on = on;
        }
        v.on
    }

    pub fn voice_on(&self) -> bool {
        lock(&self.voice).on
    }

    pub fn set_prompt(&self, prompt: String) {
        lock(&self.voice).prompt = prompt;
    }

    /// The prompt of the turn that ends. The next turn starts empty.
    pub fn take_prompt(&self) -> String {
        std::mem::take(&mut lock(&self.voice).prompt)
    }

    pub fn publish_event<E: agentsi_proto::Event>(&self, params: &E::Params) {
        self.publish::<E>(params);
    }

    fn set_state(&self, current: &mut Option<State>, next: State) {
        if current.as_ref() == Some(&next) {
            return;
        }
        self.publish::<TerminalState>(&TerminalStateParams {
            id: self.id.clone(),
            state: next.state,
            kind: next.kind.clone(),
            msg: next.msg.clone(),
        });
        *current = Some(next);
    }

    fn wait_loop(self: Arc<Self>) {
        let code = match self.pty.wait() {
            Ok(status) => status.code(),
            Err(e) => {
                eprintln!("core: wait for the program of {}: {e}", self.id);
                None
            }
        };
        let (done, cv) = &self.output_done;
        let _ = cv.wait_timeout_while(lock(done), OUTPUT_GRACE, |done| !*done);

        let mut s = lock(&self.screen);
        s.exit = Some(code);
        // The records of a program that stopped do not hold: it works on nothing and waits for no one.
        let stale = s
            .state
            .as_ref()
            .is_some_and(|st| matches!(st.state, ProgramState::Working | ProgramState::Blocked));
        if stale {
            self.set_state(&mut s.state, idle());
        }
        self.publish::<TerminalExit>(&TerminalExitParams {
            id: self.id.clone(),
            code,
        });
        let reason = close_reason(&CloseReason::Exit { code });
        for a in s.attaches.drain(..) {
            a.push_close(reason.clone());
        }
        let killed = s.killed;
        drop(s);
        // A pane whose program ended is no one to talk to.
        if let Some(core) = self.core.upgrade() {
            crate::converse::end(&core, Some(&self.id));
        }
        if killed && let Some(core) = self.core.upgrade() {
            core.terminals.remove(&self);
            self.forget_focus();
        }
    }
}

fn close_reason(reason: &CloseReason) -> String {
    serde_json::to_string(reason).unwrap_or_default()
}

fn idle() -> State {
    State {
        state: ProgramState::Idle,
        kind: None,
        msg: None,
    }
}

fn program_state(report: &agentsi_vt::StatusReport) -> State {
    let text = |s: &str| (!s.is_empty()).then(|| s.to_string());
    let (state, kind, msg) = match &report.state {
        AgentState::Idle => (ProgramState::Idle, None, None),
        AgentState::Working => (ProgramState::Working, None, None),
        AgentState::Done => (ProgramState::Done, None, None),
        AgentState::Error => (ProgramState::Error, None, None),
        AgentState::Blocked { kind, msg } => (
            ProgramState::Blocked,
            match kind {
                BlockedKind::Permission => Some("permission".into()),
                BlockedKind::Question => Some("question".into()),
                BlockedKind::Auth => Some("auth".into()),
                BlockedKind::Unspecified => None,
            },
            text(msg),
        ),
    };
    State {
        state,
        kind,
        msg: msg.or_else(|| text(&report.message)),
    }
}
