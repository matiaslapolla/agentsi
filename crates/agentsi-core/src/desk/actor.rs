//! The thread of one desk process. It owns the process, gives it the owner messages, stores
//! what it writes and applies the time limits. The process gets SIGKILL when this thread ends.

use super::{CLAUDE, Cmd, Limits, RUNNING, check_auth, set_auth, set_state, store_item};
use crate::state::{Core, lock};
use agentsi_agency::desk::args::user_line;
use agentsi_agency::desk::auth::Auth;
use agentsi_agency::desk::process::{DeskProcess, ProcEvent, first_line};
use agentsi_agency::desk::session_lost;
use agentsi_agency::desk::stream::{Out, read_line};
use agentsi_proto::desk_session_api::{AuthState, DeskDelta, DeskDeltaParams, DeskItem, DeskState};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(50);

pub(crate) struct Start {
    pub product: String,
    pub generation: u64,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    /// The first owner message, with its desk notes.
    pub first: String,
    pub inbox: Receiver<Cmd>,
    pub limits: Limits,
}

struct Running;

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(crate) fn spawn(core: Arc<Core>, start: Start) {
    RUNNING.fetch_add(1, Ordering::SeqCst);
    let running = Running;
    let spawned = std::thread::Builder::new()
        .name("agentsi-desk".into())
        .spawn(move || {
            let _running = running;
            Actor { core, s: start }.run();
        });
    if let Err(e) = spawned {
        eprintln!("core: the desk thread did not start: {e}");
    }
}

struct Actor {
    core: Arc<Core>,
    s: Start,
}

/// How the process ended.
struct End {
    state: DeskState,
    message: Option<String>,
}

impl Actor {
    fn current(&self) -> bool {
        self.core.desks.current(&self.s.product, self.s.generation)
    }

    fn state(&self, state: DeskState, message: Option<String>) {
        set_state(
            &self.core,
            &self.s.product,
            Some(self.s.generation),
            state,
            message,
        );
    }

    /// Stores an item of this process. A stale process stores nothing.
    fn item(&self, item: DeskItem) {
        if !self.current() {
            return;
        }
        if let Err(e) = store_item(&self.core, &self.s.product, item) {
            eprintln!(
                "core: desk item of {} failed: {}",
                self.s.product, e.message
            );
        }
    }

    fn error(&self, message: &str) {
        self.item(DeskItem::Error {
            message: message.to_string(),
        });
    }

    fn run(self) {
        let end = match DeskProcess::start(CLAUDE, &self.s.args, &self.s.cwd, &[]) {
            Ok(process) => self.serve(process),
            Err(e) => {
                let message = if e.kind() == std::io::ErrorKind::NotFound {
                    set_auth(&self.core, Auth::of(AuthState::Missing));
                    AuthState::Missing.message().unwrap_or_default().to_string()
                } else {
                    format!("AgentSI cannot start claude: {e}")
                };
                self.error(&message);
                End {
                    state: DeskState::Failed,
                    message: Some(message),
                }
            }
        };
        self.finish(end);
    }

    /// Frees the slot, then reports an owner message that came too late for this process.
    fn finish(self, end: End) {
        let current = {
            let mut slots = lock(&self.core.desks.slots);
            match slots.get_mut(&self.s.product) {
                Some(slot) if slot.generation == self.s.generation => {
                    slot.commands = None;
                    true
                }
                _ => false,
            }
        };
        if current {
            while let Ok(cmd) = self.s.inbox.try_recv() {
                if matches!(cmd, Cmd::Send(_)) {
                    self.error(
                        "The desk session stopped before it read your message. Send it again.",
                    );
                }
            }
        }
        self.state(end.state, end.message);
    }

    fn serve(&self, mut p: DeskProcess) -> End {
        if let Err(e) = p.send(&user_line(&self.s.first)) {
            let message = format!("AgentSI cannot write to claude: {e}");
            self.error(&message);
            return End {
                state: DeskState::Failed,
                message: Some(message),
            };
        }
        let mut reply_started = Some(Instant::now());
        let mut last_owner = Instant::now();
        let mut started = false;
        let mut auth_error = false;
        let end = loop {
            match p.next(POLL) {
                Some(ProcEvent::Line(line)) => {
                    if !started {
                        started = true;
                        self.state(DeskState::Working, None);
                    }
                    for out in read_line(&line) {
                        match out {
                            Out::Delta(text) => {
                                if self.current() {
                                    self.core.publish::<DeskDelta>(&DeskDeltaParams {
                                        product: self.s.product.clone(),
                                        text,
                                    });
                                }
                            }
                            Out::AuthError => auth_error = true,
                            Out::Item(item) => {
                                let done = matches!(item, DeskItem::End { .. });
                                self.item(item);
                                if done {
                                    reply_started = None;
                                    if !auth_error {
                                        self.state(DeskState::Idle, None);
                                    }
                                }
                            }
                        }
                    }
                    if auth_error && reply_started.is_none() {
                        break self.auth_failed();
                    }
                }
                Some(ProcEvent::Closed { stderr }) => {
                    let code = p.exit_code();
                    break self.closed(&stderr, code);
                }
                None => {}
            }
            match self.s.inbox.try_recv() {
                Ok(Cmd::Send(text)) => {
                    if let Err(e) = p.send(&user_line(&text)) {
                        let message = format!("AgentSI cannot write to claude: {e}");
                        self.error(&message);
                        break End {
                            state: DeskState::Failed,
                            message: Some(message),
                        };
                    }
                    reply_started = Some(Instant::now());
                    last_owner = Instant::now();
                    self.state(DeskState::Working, None);
                }
                Ok(Cmd::Stop) | Err(TryRecvError::Disconnected) => {
                    break End {
                        state: DeskState::Stopped,
                        message: None,
                    };
                }
                Err(TryRecvError::Empty) => {}
            }
            if reply_started.is_some_and(|t| t.elapsed() >= self.s.limits.reply) {
                self.error(&format!(
                    "The reply ran for more than {} min. The core stopped the desk session.",
                    self.s.limits.reply.as_secs().div_ceil(60)
                ));
                break End {
                    state: DeskState::Stopped,
                    message: None,
                };
            }
            if reply_started.is_none() && last_owner.elapsed() >= self.s.limits.idle {
                break End {
                    state: DeskState::Stopped,
                    message: None,
                };
            }
        };
        p.kill();
        end
    }

    /// A reply failed with an auth error. A login that still reads as `ready` is one that
    /// `claude -p` did not use.
    fn auth_failed(&self) -> End {
        let mut result = check_auth(&self.core);
        if result.state == AuthState::Ready {
            result = Auth::of(AuthState::NotRead);
            set_auth(&self.core, result.clone());
        }
        End {
            state: DeskState::Auth,
            message: result.message,
        }
    }

    fn closed(&self, stderr: &str, code: Option<i32>) -> End {
        let message = if session_lost(stderr) {
            "Claude Code lost the stored desk session. Select New conversation to start a new one."
                .to_string()
        } else {
            let why = first_line(stderr).unwrap_or_else(|| match code {
                Some(c) => format!("exit code {c}"),
                None => "no exit code".into(),
            });
            format!("The desk session stopped: {why}")
        };
        self.error(&message);
        End {
            state: DeskState::Failed,
            message: Some(message),
        }
    }
}
