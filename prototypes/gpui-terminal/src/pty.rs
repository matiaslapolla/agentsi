//! A child process on a PTY that this code opens with `libc::openpty`.
//! The shape follows herdr-core `platform/unix.rs` `spawn_pty` (Apache-2.0): openpty, then
//! `setsid` + `TIOCSCTTY` in the child, and `TIOCSWINSZ` on the master to resize.
use std::{
    fs::File,
    io::{self, Read, Write},
    os::{fd::{AsRawFd, FromRawFd}, unix::process::CommandExt},
    process::{Child, Command, Stdio},
    time::Instant,
};

pub struct Pty {
    master: File,
    child: Child,
}

impl Pty {
    pub fn spawn(argv: &[String], cwd: &str, cols: u16, rows: u16) -> io::Result<Pty> {
        let (mut master, mut slave) = (0, 0);
        let size = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
        if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &size) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
        for fd in [master.as_raw_fd(), slave.as_raw_fd()] {
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]).current_dir(cwd);
        // The child is not a Claude Code tool call: drop the markers of the session that runs this program.
        for (k, _) in std::env::vars() {
            if k.starts_with("CLAUDECODE") || k.starts_with("CLAUDE_CODE") || k.starts_with("CLAUDE_JOB") || k.starts_with("CLAUDE_BG") {
                cmd.env_remove(k);
            }
        }
        cmd.env("TERM", "xterm-256color").env("COLORTERM", "truecolor").env("TERM_PROGRAM", "agentsi-terminal");
        cmd.stdin(Stdio::from(slave.try_clone()?)).stdout(Stdio::from(slave.try_clone()?)).stderr(Stdio::from(slave));
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                // A parent started in the background (`cmd &` in a script) has SIGINT and SIGQUIT ignored,
                // and `exec` keeps that. Without this reset, Ctrl-C in the pane would not stop the child.
                for sig in [libc::SIGINT, libc::SIGQUIT, libc::SIGTERM, libc::SIGHUP, libc::SIGPIPE, libc::SIGCHLD, libc::SIGTSTP, libc::SIGTTIN, libc::SIGTTOU] {
                    libc::signal(sig, libc::SIG_DFL);
                }
                let mut set: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut set);
                libc::sigprocmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
                Ok(())
            });
        }
        let child = cmd.spawn()?;
        Ok(Pty { master, child })
    }

    /// Reads the master on a thread. Each chunk carries the time of its `read` for the latency stat.
    pub fn start_reader(&self) -> async_channel::Receiver<(Instant, Vec<u8>)> {
        let (tx, rx) = async_channel::unbounded();
        let mut f = self.master.try_clone().expect("clone master");
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match f.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if tx.send_blocking((Instant::now(), buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        rx
    }

    pub fn write(&self, bytes: &[u8]) {
        let mut f = &self.master;
        let _ = f.write_all(bytes);
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let size = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
        unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &size) };
    }

    pub fn kill(&self) {
        let pid = self.child.id() as i32;
        unsafe {
            libc::killpg(pid, libc::SIGHUP);
            libc::killpg(pid, libc::SIGTERM);
        }
    }
}
