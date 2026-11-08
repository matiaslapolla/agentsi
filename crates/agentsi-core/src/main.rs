//! Daemon.

use agentsi_agency::meeting::running as meetings;
use agentsi_agency::story::run as stories;
use agentsi_core::server::{self, BindError};
use agentsi_core::state::Core;
use agentsi_proto::client::{CORE_STOP_WAIT as STOP_WAIT, EXIT_ALREADY_RUNNING};
use agentsi_proto::socket::{SocketPath, socket_path};
use std::process::ExitCode;
use std::time::{Duration, Instant};

const STOP_POLL: Duration = Duration::from_millis(20);
/// The time that a writer needs to send the last reply to a client, after the grace.
const LINK_FLUSH: Duration = Duration::from_millis(500);

const USAGE: &str = "usage: agentsi-core serve [--detach] | connect | hook <event> | dictate <start|stop|toggle> | converse <start|stop|toggle> | --version\n       agentsi-core import-postgres <url> [--fresh] [--data-root <dir>]\n       agentsi-core backup [--keep <n>] [--data-root <dir>]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["--version"] => {
            println!("agentsi-core {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        ["serve"] => serve(false),
        ["serve", "--detach"] => serve(true),
        ["connect"] => connect(),
        ["dictate", rest @ ..] => agentsi_core::dictate::run(rest),
        ["converse", rest @ ..] => agentsi_core::converse::run(rest),
        ["hook", rest @ ..] => agentsi_core::hook::run(rest),
        ["import-postgres", rest @ ..] => agentsi_core::import::run(rest),
        ["backup", rest @ ..] => agentsi_core::backup::run(rest),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn socket() -> Result<SocketPath, ExitCode> {
    socket_path().map_err(|e| {
        eprintln!("agentsi-core: {e}");
        ExitCode::FAILURE
    })
}

/// With `--detach`, as a client starts it: a new session, and stderr to
/// the log next to the socket.
fn detach(socket: &SocketPath) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    server::prepare_folder(socket)?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(socket.log())?;
    // SAFETY: dup2 and setsid take plain values. The log fd stays open
    // until dup2 has copied it.
    unsafe {
        libc::dup2(log.as_raw_fd(), libc::STDERR_FILENO);
        libc::setsid();
    }
    Ok(())
}

fn serve(detached: bool) -> ExitCode {
    let socket = match socket() {
        Ok(s) => s,
        Err(code) => return code,
    };
    if detached && let Err(e) = detach(&socket) {
        eprintln!("agentsi-core: {e}");
        return ExitCode::FAILURE;
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("agentsi-core: {e}");
            return ExitCode::FAILURE;
        }
    };
    // `held` keeps the lock of the socket, so no second core binds while the work stops.
    let (code, held) = runtime.block_on(run(&socket));
    drop(runtime);
    drop(held);
    code
}

async fn run(socket: &SocketPath) -> (ExitCode, Option<server::HeldLock>) {
    // The handlers come before the socket: a SIGTERM after the bind must
    // remove the socket.
    let stop = match signals() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("agentsi-core: {e}");
            return (ExitCode::FAILURE, None);
        }
    };
    let bound = match server::bind(socket) {
        Ok(b) => b,
        Err(BindError::AlreadyRunning) => {
            eprintln!(
                "agentsi-core: a core already runs on {}",
                socket.path.display()
            );
            return (ExitCode::from(EXIT_ALREADY_RUNNING as u8), None);
        }
        Err(e) => {
            eprintln!("agentsi-core: {}: {e}", socket.path.display());
            return (ExitCode::FAILURE, None);
        }
    };
    eprintln!("core: listening on {}", socket.path.display());
    let core = Core::new();
    tokio::spawn(agentsi_core::backup::daily(core.clone()));
    let served = bound.serve(core.clone(), server::euid(), stop).await;
    let stopped_at = Instant::now();
    // A request in flight gets its grace to answer, while the work stops.
    let (work_ended, ()) = tokio::join!(
        stop_work(stopped_at),
        core.links_ended(server::STOP_GRACE + LINK_FLUSH)
    );
    if !work_ended {
        let panes: Vec<u32> = core.terminals.list().iter().map(|t| t.pid).collect();
        exit_with_work_running(&panes);
    }
    match served {
        Ok(lock) => (ExitCode::SUCCESS, Some(lock)),
        Err(e) => {
            eprintln!("agentsi-core: {e}");
            (ExitCode::FAILURE, None)
        }
    }
}

/// Cancels each meeting and story session, and waits for them and for the blocking threads
/// of the requests, which no cancel reaches. The client links are closed already, so a
/// client does not wait for this. Returns false when work still runs after `STOP_WAIT`.
async fn stop_work(stopped_at: Instant) -> bool {
    meetings::stop_all();
    stories::stop_all();
    while !(meetings::idle() && stories::idle() && agentsi_core::blocking::running() == 0) {
        if stopped_at.elapsed() >= STOP_WAIT {
            return false;
        }
        tokio::time::sleep(STOP_POLL).await;
    }
    true
}

/// A thread that works past the wait cannot be stopped, and the lock must not outlive a
/// writer. So the core stops the child processes of that work, then exits. The drop of the
/// runtime would wait for the work. Its transactions roll back. A pane is the owner's program,
/// not work of a request, so it keeps the end of a normal stop: the hangup of its PTY.
fn exit_with_work_running(panes: &[u32]) -> ! {
    eprintln!("agentsi-core: work still ran after {STOP_WAIT:?}. Exiting.");
    kill_descendants(panes);
    std::process::exit(1)
}

/// Sends SIGKILL to each process below this one, except `spared` and the processes below them,
/// until none is left. A scan takes the whole tree before the first kill, so the child of a
/// process that dies is not lost.
fn kill_descendants(spared: &[u32]) {
    let me = std::process::id();
    for _ in 0..50 {
        let mut parents = std::collections::HashMap::new();
        for entry in std::fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            let stat = std::fs::read_to_string(entry.path().join("stat")).unwrap_or_default();
            // The name in the first field can hold spaces and parentheses.
            let mut fields = stat.rsplit(") ").next().unwrap_or("").split(' ');
            let (state, ppid) = (
                fields.next(),
                fields.next().and_then(|p| p.parse::<u32>().ok()),
            );
            if let (Some(state), Some(ppid)) = (state, ppid) {
                parents.insert(pid, (ppid, state == "Z"));
            }
        }
        let mut found = Vec::new();
        let mut frontier = vec![me];
        while let Some(parent) = frontier.pop() {
            for (&pid, &(ppid, zombie)) in &parents {
                if ppid == parent && !spared.contains(&pid) {
                    frontier.push(pid);
                    if !zombie {
                        found.push(pid);
                    }
                }
            }
        }
        if found.is_empty() {
            return;
        }
        for pid in found {
            // SAFETY: a pid below this process. A reused pid at worst ends a process of this user.
            unsafe { libc::kill(pid as i32, libc::SIGKILL) };
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// SIGTERM and SIGINT stop the core. Then it removes its socket.
fn signals() -> std::io::Result<impl std::future::Future<Output = ()>> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    Ok(async move {
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
    })
}

fn connect() -> ExitCode {
    let socket = match socket() {
        Ok(s) => s,
        Err(code) => return code,
    };
    let result = std::env::current_exe().and_then(|bin| agentsi_core::relay::relay(&socket, &bin));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("agentsi-core connect: {e}");
            ExitCode::FAILURE
        }
    }
}
