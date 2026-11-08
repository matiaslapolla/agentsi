use super::*;
use std::{
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

const SIZE: PtySize = PtySize { cols: 80, rows: 24 };
const LIMIT: Duration = Duration::from_secs(20);

fn sh(script: &str) -> SpawnOptions {
    SpawnOptions::new(["sh", "-c", script], SIZE)
}

/// Collects the output on a thread until the end of the stream.
fn collect(pty: &Pty) -> mpsc::Receiver<Vec<u8>> {
    let mut reader = pty.reader().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    rx
}

/// Reads until `needle` appears in the output, or fails after `LIMIT`.
fn read_until(rx: &mpsc::Receiver<Vec<u8>>, needle: &str) -> String {
    let start = Instant::now();
    let mut text = String::new();
    while !text.contains(needle) {
        let left = LIMIT.saturating_sub(start.elapsed());
        match rx.recv_timeout(left) {
            Ok(chunk) => text.push_str(&String::from_utf8_lossy(&chunk)),
            Err(_) => panic!("no {needle:?} in the output: {text:?}"),
        }
    }
    text
}

fn read_to_end(rx: &mpsc::Receiver<Vec<u8>>) -> String {
    let mut text = String::new();
    while let Ok(chunk) = rx.recv_timeout(LIMIT) {
        text.push_str(&String::from_utf8_lossy(&chunk));
    }
    text
}

#[test]
fn runs_argv_and_reports_the_exit_code() {
    let pty = Pty::spawn(&sh("echo out; exit 7")).unwrap();
    let rx = collect(&pty);
    let status = pty.wait().unwrap();
    assert_eq!(status.code(), Some(7));
    assert_eq!(exit_code(status), 7);
    assert!(read_to_end(&rx).contains("out"));
    // A second wait returns the same status.
    assert_eq!(pty.wait().unwrap().code(), Some(7));
    assert_eq!(pty.try_wait().unwrap().unwrap().code(), Some(7));
}

#[test]
fn the_reader_sees_the_end_after_the_child_exits() {
    let pty = Pty::spawn(&sh("exit 0")).unwrap();
    let rx = collect(&pty);
    // The sender drops when the reader reaches the end, so `recv` reports a disconnect.
    assert_eq!(
        rx.recv_timeout(LIMIT).err(),
        Some(mpsc::RecvTimeoutError::Disconnected)
    );
}

#[test]
fn an_empty_argv_is_an_error() {
    let options = SpawnOptions::new(Vec::<String>::new(), SIZE);
    let err = Pty::spawn(&options).err().unwrap();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn a_missing_program_is_an_error() {
    let options = SpawnOptions::new(["/nonexistent/agentsi-pty-test"], SIZE);
    assert!(Pty::spawn(&options).is_err());
}

#[test]
fn passes_cwd_and_env() {
    let dir = std::env::temp_dir().canonicalize().unwrap();
    let mut options = sh(
        "pwd; echo v=$AGENTSI_PTY_TEST; echo t=$TERM c=$COLORTERM; echo r=${AGENTSI_PTY_REMOVED-unset}",
    );
    options.cwd = Some(dir.clone());
    options.env = vec![("AGENTSI_PTY_TEST".into(), "yes".into())];
    options.env_remove = vec!["AGENTSI_PTY_REMOVED".into()];
    let pty = Pty::spawn(&options).unwrap();
    let rx = collect(&pty);
    pty.wait().unwrap();
    let out = read_to_end(&rx);
    assert!(out.contains(dir.to_str().unwrap()), "{out:?}");
    assert!(out.contains("v=yes"), "{out:?}");
    assert!(out.contains("t=xterm-256color c=truecolor"), "{out:?}");
    assert!(out.contains("r=unset"), "{out:?}");
}

#[test]
fn env_overrides_the_default_term() {
    let mut options = sh("echo t=$TERM");
    options.env = vec![("TERM".into(), "dumb".into())];
    let pty = Pty::spawn(&options).unwrap();
    let rx = collect(&pty);
    pty.wait().unwrap();
    assert!(read_to_end(&rx).contains("t=dumb"));
}

#[test]
fn writes_reach_the_child() {
    let pty = Pty::spawn(&SpawnOptions::new(["cat"], SIZE)).unwrap();
    let rx = collect(&pty);
    pty.write(b"ping\n").unwrap();
    read_until(&rx, "ping");
    // Ctrl-D ends `cat` at the start of a line.
    pty.write(b"\x04").unwrap();
    assert_eq!(pty.wait().unwrap().code(), Some(0));
}

#[test]
fn resize_changes_the_size_the_child_sees() {
    let pty = Pty::spawn(&sh("stty size; read x; stty size")).unwrap();
    let rx = collect(&pty);
    read_until(&rx, "24 80");
    pty.resize(PtySize {
        cols: 100,
        rows: 30,
    })
    .unwrap();
    pty.write(b"\n").unwrap();
    read_until(&rx, "30 100");
    pty.wait().unwrap();
}

#[test]
fn the_child_has_a_controlling_terminal_and_gets_ctrl_c() {
    // `setsid` and `TIOCSCTTY` make the child leader of a foreground group that owns the terminal.
    // Ctrl-C is a byte on the master, and the line discipline turns it into SIGINT for that group.
    let pty = Pty::spawn(&sh("echo ready; sleep 60")).unwrap();
    let rx = collect(&pty);
    read_until(&rx, "ready");
    pty.write(b"\x03").unwrap();
    let status = pty.wait().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGINT), "{status:?}");
}

fn proc_status_mask(pid: u32, field: &str) -> u64 {
    let text = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let line = text
        .lines()
        .find_map(|l| l.strip_prefix(field))
        .unwrap_or_else(|| panic!("no {field} in /proc/{pid}/status"));
    u64::from_str_radix(line.trim_start_matches(':').trim(), 16).unwrap()
}

#[cfg(target_os = "linux")]
#[test]
fn the_child_starts_with_default_signals_and_an_empty_mask() {
    fn bit(sig: i32) -> u64 {
        1 << (sig - 1)
    }
    // Ignore SIGQUIT in this process and block SIGINT in this thread. Both would pass to the child
    // through `exec` without the reset in `pre_exec`.
    let pty = thread::spawn(|| {
        // SAFETY: plain signal calls on this thread. SIGQUIT goes back to its default below.
        let old = unsafe { libc::signal(libc::SIGQUIT, libc::SIG_IGN) };
        // SAFETY: `sigset_t` is plain data that `sigemptyset` initialises right after.
        let mut set: libc::sigset_t = unsafe { std::mem::zeroed() };
        // SAFETY: `set` is a valid sigset and the old mask is not wanted (null).
        unsafe {
            libc::sigemptyset(&mut set);
            libc::sigaddset(&mut set, libc::SIGINT);
            libc::pthread_sigmask(libc::SIG_BLOCK, &set, std::ptr::null_mut());
        }
        let pty = Pty::spawn(&sh("sleep 60")).unwrap();
        // SAFETY: puts back the handler that the call above returned.
        unsafe { libc::signal(libc::SIGQUIT, old) };
        pty
    })
    .join()
    .unwrap();
    // The `sleep` child may not have called `exec` yet. Wait until its image is not the test binary.
    let start = Instant::now();
    loop {
        let comm = std::fs::read_to_string(format!("/proc/{}/comm", pty.pid())).unwrap_or_default();
        if comm.trim() == "sleep" || start.elapsed() > LIMIT {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let ignored = proc_status_mask(pty.pid(), "SigIgn");
    let blocked = proc_status_mask(pty.pid(), "SigBlk");
    assert_eq!(ignored & bit(libc::SIGQUIT), 0, "SigIgn {ignored:x}");
    assert_eq!(blocked & bit(libc::SIGINT), 0, "SigBlk {blocked:x}");
    pty.kill().unwrap();
    pty.wait().unwrap();
}

#[test]
fn kill_ends_the_process_group() {
    // The grandchild is in the same group, and `kill` reaches it too.
    let pty = Pty::spawn(&sh("sleep 60 & echo $!; wait")).unwrap();
    let rx = collect(&pty);
    let text = read_until(&rx, "\n");
    let grandchild: i32 = text.trim().parse().unwrap();
    assert!(pty.try_wait().unwrap().is_none());
    pty.kill().unwrap();
    let status = pty.wait().unwrap();
    assert!(status.code().is_some() || status.signal().is_some());
    let start = Instant::now();
    // SAFETY: signal 0 only tests that the process exists.
    while unsafe { libc::kill(grandchild, 0) } == 0 {
        // A zombie still answers. `sleep` is not our child, so init reaps it soon.
        let state = std::fs::read_to_string(format!("/proc/{grandchild}/stat")).unwrap_or_default();
        if state.contains(") Z") || start.elapsed() > LIMIT {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(start.elapsed() <= LIMIT, "the grandchild is still alive");
}

#[test]
fn drop_hangs_up_and_reaps_the_child() {
    let pty = Pty::spawn(&sh("sleep 60")).unwrap();
    let pid = pty.pid() as i32;
    drop(pty);
    let start = Instant::now();
    // SAFETY: signal 0 only tests that the process exists. A reaped pid gives ESRCH.
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(start.elapsed() < LIMIT, "the child was not reaped");
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn signal_after_the_child_is_reaped_does_not_touch_the_pid() {
    let pty = Pty::spawn(&sh("exit 0")).unwrap();
    pty.wait().unwrap();
    let err = pty.kill().unwrap_err();
    assert_eq!(err.raw_os_error(), Some(libc::ESRCH));
    assert!(pty.signal(libc::SIGKILL).is_err());
}

#[test]
fn drop_kills_a_child_that_ignores_hangup_and_terminate() {
    let pty = Pty::spawn(&sh("trap '' HUP TERM; echo up; sleep 300")).unwrap();
    let rx = collect(&pty);
    read_until(&rx, "up");
    let pid = pty.pid() as i32;
    drop(pty);
    let start = Instant::now();
    // SAFETY: signal 0 only tests that the process exists. A reaped pid gives ESRCH.
    while unsafe { libc::kill(pid, 0) } == 0 {
        assert!(start.elapsed() < LIMIT, "the child survived the drop");
        thread::sleep(Duration::from_millis(50));
    }
}
