//! A child process on a PTY that this code opens with `libc::openpty`.
//!
//! The shape follows `spawn_pty` of herdr-core, `platform/unix.rs` (Apache-2.0, see NOTICE):
//! `openpty`, then `setsid` and `TIOCSCTTY` in the child, and `TIOCSWINSZ` on the master to resize.
//!
//! Threading model: [`Pty`] is `Send + Sync`, and every method takes `&self`.
//! - One thread owns the [`PtyReader`] from [`Pty::reader`] and blocks in `read`.
//! - Any thread may call [`Pty::write`], [`Pty::resize`] and [`Pty::kill`].
//! - One thread may block in [`Pty::wait`]. [`Pty::kill`] does not wait for it.

use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::{CommandExt, ExitStatusExt},
    },
    path::PathBuf,
    process::{Command, ExitStatus, Stdio},
    sync::{
        Mutex, TryLockError,
        atomic::{AtomicBool, Ordering},
    },
};

/// A terminal size in cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PtySize {
    pub cols: u16,
    pub rows: u16,
}

impl PtySize {
    fn winsize(self) -> libc::winsize {
        libc::winsize {
            ws_row: self.rows,
            ws_col: self.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }
    }
}

/// What to run. `argv[0]` is looked up in `PATH`.
///
/// The child gets `TERM=xterm-256color`, `COLORTERM=truecolor` and `TERM_PROGRAM=agentsi-terminal`
/// unless `env` sets the same name.
#[derive(Clone, Debug)]
pub struct SpawnOptions {
    pub argv: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(OsString, OsString)>,
    /// Names removed from the environment that the child inherits. The list is empty by default.
    /// A core that runs inside a Claude Code session should remove the `CLAUDECODE*`, `CLAUDE_CODE*`,
    /// `CLAUDE_JOB*` and `CLAUDE_BG*` names, so the child does not act as a tool call of that session.
    pub env_remove: Vec<OsString>,
    pub size: PtySize,
}

impl SpawnOptions {
    pub fn new(argv: impl IntoIterator<Item = impl Into<String>>, size: PtySize) -> Self {
        Self {
            argv: argv.into_iter().map(Into::into).collect(),
            cwd: None,
            env: Vec::new(),
            env_remove: Vec::new(),
            size,
        }
    }
}

/// The master side of a PTY and the process group of its child.
pub struct Pty {
    master: File,
    pid: libc::pid_t,
    /// `Some` after the child is reaped. The lock also serialises callers of `wait`.
    status: Mutex<Option<ExitStatus>>,
    /// Set once the pid is reaped. After that the pid may belong to another process, so no signal goes to it.
    reaped: AtomicBool,
}

impl Pty {
    pub fn spawn(options: &SpawnOptions) -> io::Result<Pty> {
        let (program, args) = options
            .argv
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "argv is empty"))?;
        let (mut master, mut slave) = (0, 0);
        let size = options.size.winsize();
        // SAFETY: `master` and `slave` are valid out-pointers, the name and termios are null (not wanted),
        // and `size` outlives the call.
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &size,
            )
        };
        if rc != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `openpty` returned two new descriptors that nothing else owns.
        let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
        for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
            // SAFETY: `fd` is open and owned by a `File` above.
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }

        let mut cmd = Command::new(program);
        cmd.args(args);
        if let Some(cwd) = &options.cwd {
            cmd.current_dir(cwd);
        }
        for name in &options.env_remove {
            cmd.env_remove(name);
        }
        for (name, value) in [
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("TERM_PROGRAM", "agentsi-terminal"),
        ] {
            if !options.env.iter().any(|(k, _)| k == name) {
                cmd.env(name, value);
            }
        }
        for (name, value) in &options.env {
            cmd.env(name, value);
        }
        cmd.stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        // SAFETY: the closure runs between `fork` and `exec`. It calls only async-signal-safe functions
        // (`setsid`, `ioctl`, `signal`, `sigemptyset`, `sigprocmask`) and allocates nothing.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                // A parent started in the background (`cmd &` in a script) has SIGINT and SIGQUIT ignored,
                // and `exec` keeps that. Without this reset, Ctrl-C in the pane would not stop the child.
                for sig in [
                    libc::SIGINT,
                    libc::SIGQUIT,
                    libc::SIGTERM,
                    libc::SIGHUP,
                    libc::SIGPIPE,
                    libc::SIGCHLD,
                    libc::SIGTSTP,
                    libc::SIGTTIN,
                    libc::SIGTTOU,
                ] {
                    libc::signal(sig, libc::SIG_DFL);
                }
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::sigprocmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
                Ok(())
            });
        }
        let child = cmd.spawn()?;
        // `cmd` holds copies of the slave. Drop it now: while the parent keeps the slave open, the
        // reader never sees the end of the output after the child exits.
        drop(cmd);
        let pid = child.id() as libc::pid_t;
        // `Child` has no `Drop` that kills or reaps, so `wait` reaps the pid itself.
        drop(child);
        Ok(Pty {
            master,
            pid,
            status: Mutex::new(None),
            reaped: AtomicBool::new(false),
        })
    }

    pub fn pid(&self) -> u32 {
        self.pid as u32
    }

    /// A blocking reader on the master. Each call returns an independent handle to the same stream.
    /// It reads the end of the output (`EIO` on Linux) as `Ok(0)`.
    pub fn reader(&self) -> io::Result<PtyReader> {
        Ok(PtyReader(self.master.try_clone()?))
    }

    /// Writes all bytes to the child's input.
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let mut master = &self.master;
        master.write_all(bytes)
    }

    pub fn resize(&self, size: PtySize) -> io::Result<()> {
        let size = size.winsize();
        // SAFETY: the master is open for the life of `self` and `size` outlives the call.
        let rc = unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &size) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Sends a signal to the process group of the child.
    pub fn signal(&self, signal: i32) -> io::Result<()> {
        if self.reaped.load(Ordering::Acquire) {
            return Err(io::Error::from_raw_os_error(libc::ESRCH));
        }
        // SAFETY: `killpg` takes plain integers. The group id is the child's pid because the child
        // called `setsid`. The check above skips a pid that `wait` already reaped.
        let rc = unsafe { libc::killpg(self.pid, signal) };
        if rc < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Sends SIGHUP and then SIGTERM to the process group of the child.
    /// A child that ignores both stays alive. Call `signal(libc::SIGKILL)` for it.
    pub fn kill(&self) -> io::Result<()> {
        let hup = self.signal(libc::SIGHUP);
        let term = self.signal(libc::SIGTERM);
        hup.or(term)
    }

    /// Blocks until the child exits and returns its status. A second call returns the same status.
    pub fn wait(&self) -> io::Result<ExitStatus> {
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(done) = *status {
            return Ok(done);
        }
        let raw = loop {
            let mut raw = 0;
            // SAFETY: `raw` is a valid out-pointer and the pid is a child that this process has not reaped.
            let rc = unsafe { libc::waitpid(self.pid, &mut raw, 0) };
            if rc == self.pid {
                break raw;
            }
            let err = io::Error::last_os_error();
            if rc < 0 && err.kind() != io::ErrorKind::Interrupted {
                return Err(err);
            }
        };
        let done = ExitStatus::from_raw(raw);
        *status = Some(done);
        self.reaped.store(true, Ordering::Release);
        Ok(done)
    }

    /// Returns the status if the child has exited, and `None` if it runs.
    pub fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        let mut status = match self.status.try_lock() {
            Ok(status) => status,
            Err(TryLockError::Poisoned(e)) => e.into_inner(),
            // Another thread blocks in `wait`: the child has not exited yet.
            Err(TryLockError::WouldBlock) => return Ok(None),
        };
        if let Some(done) = *status {
            return Ok(Some(done));
        }
        let mut raw = 0;
        // SAFETY: as in `wait`, with WNOHANG.
        let rc = unsafe { libc::waitpid(self.pid, &mut raw, libc::WNOHANG) };
        match rc {
            0 => Ok(None),
            rc if rc == self.pid => {
                let done = ExitStatus::from_raw(raw);
                *status = Some(done);
                self.reaped.store(true, Ordering::Release);
                Ok(Some(done))
            }
            _ => Err(io::Error::last_os_error()),
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if self.reaped.load(Ordering::Acquire) {
            return;
        }
        // Closing the master hangs up the session. A thread reaps the child, so no zombie stays.
        // A child that ignores the hang-up gets SIGKILL after a grace period.
        let pid = self.pid;
        let _ = self.signal(libc::SIGHUP);
        std::thread::spawn(move || {
            let mut raw = 0;
            for _ in 0..DROP_GRACE_STEPS {
                // SAFETY: the pid is a child of this process that nothing else waits for any more.
                if unsafe { libc::waitpid(pid, &mut raw, libc::WNOHANG) } != 0 {
                    return;
                }
                std::thread::sleep(DROP_GRACE_STEP);
            }
            // SAFETY: as above. The child is not reaped, so the pid still names it.
            unsafe {
                libc::killpg(pid, libc::SIGKILL);
                libc::waitpid(pid, &mut raw, 0);
            }
        });
    }
}

const DROP_GRACE_STEP: std::time::Duration = std::time::Duration::from_millis(100);
const DROP_GRACE_STEPS: u32 = 10;

/// The blocking read side of a [`Pty`].
pub struct PtyReader(File);

impl Read for PtyReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buf) {
            // Linux reports the hang-up of the slave as EIO.
            Err(e) if e.raw_os_error() == Some(libc::EIO) => Ok(0),
            other => other,
        }
    }
}

/// The exit code of a normal exit, or `128 + signal` after a signal, as a shell reports it.
pub fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .or_else(|| status.signal().map(|s| 128 + s))
        .unwrap_or(-1)
}

#[cfg(test)]
mod tests;
