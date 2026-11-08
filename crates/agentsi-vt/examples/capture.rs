//! Runs a program on a PTY behind an `agentsi_vt::Terminal`, as the core will, and records the bytes
//! that the program wrote. Used to make `tests/fixtures/claude-status.raw`.
//!
//! ```text
//! capture --out FILE [--cwd DIR] [--seconds N] [--script "ms:text@@ms:text"] -- claude --model haiku
//! ```
//!
//! The script writes to the program after each delay. Escapes: `\r \n \t \e \xNN \\`.
//! The terminal answers the queries of the program, so the program sees a terminal that takes OSC 7501.

use agentsi_pty::{Pty, PtySize, SpawnOptions};
use agentsi_vt::{Event, Options, Terminal};
use std::{
    io::{Read, Write},
    sync::mpsc,
    time::{Duration, Instant},
};

fn unescape(s: &str) -> Vec<u8> {
    let (mut out, mut it) = (Vec::new(), s.bytes().peekable());
    while let Some(b) = it.next() {
        if b != b'\\' {
            out.push(b);
            continue;
        }
        match it.next() {
            Some(b'r') => out.push(b'\r'),
            Some(b'n') => out.push(b'\n'),
            Some(b't') => out.push(b'\t'),
            Some(b'e') => out.push(0x1b),
            Some(b'\\') => out.push(b'\\'),
            Some(b'x') => {
                let hex: String = [it.next(), it.next()]
                    .into_iter()
                    .flatten()
                    .map(char::from)
                    .collect();
                out.push(u8::from_str_radix(&hex, 16).expect("two hex digits after \\x"));
            }
            other => panic!("bad escape \\{:?}", other.map(char::from)),
        }
    }
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut out, mut cwd, mut seconds, mut script) = (None, None, 60u64, Vec::new());
    let mut program = Vec::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = args.next(),
            "--cwd" => cwd = args.next(),
            "--seconds" => {
                seconds = args
                    .next()
                    .and_then(|s| s.parse().ok())
                    .expect("--seconds N")
            }
            "--script" => {
                for step in args.next().expect("--script TEXT").split("@@") {
                    let (ms, text) = step.split_once(':').expect("ms:text");
                    script.push((
                        Duration::from_millis(ms.parse().expect("ms")),
                        unescape(text),
                    ));
                }
            }
            "--" => {
                program = args.by_ref().collect();
            }
            other => panic!("unknown flag {other}"),
        }
    }
    let out = out.expect("--out FILE");
    let mut options = SpawnOptions::new(
        program,
        PtySize {
            cols: 120,
            rows: 40,
        },
    );
    options.cwd = cwd.map(Into::into);
    // The child is not a tool call of the session that runs this program.
    options.env_remove = std::env::vars_os()
        .map(|(k, _)| k)
        .filter(|k| {
            let k = k.to_string_lossy();
            ["CLAUDECODE", "CLAUDE_CODE", "CLAUDE_JOB", "CLAUDE_BG"]
                .iter()
                .any(|p| k.starts_with(p))
        })
        .collect();
    let pty = Pty::spawn(&options).expect("spawn");
    let mut terminal = Terminal::new(Options {
        cols: 120,
        rows: 40,
        ..Options::default()
    })
    .expect("terminal");
    terminal.resize(120, 40, 9, 18).expect("resize");

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let mut reader = pty.reader().expect("reader");
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        while let Ok(n @ 1..) = reader.read(&mut buf) {
            if tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let mut raw = std::fs::File::create(&out).expect("create the output file");
    let start = Instant::now();
    let mut script = script.into_iter().peekable();
    while start.elapsed() < Duration::from_secs(seconds) {
        if let Some((delay, _)) = script.peek()
            && start.elapsed() >= *delay
        {
            let (_, bytes) = script.next().unwrap();
            let _ = pty.write(&bytes);
            eprintln!(
                "[{:6.1}s] sent {} bytes",
                start.elapsed().as_secs_f32(),
                bytes.len()
            );
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(bytes) => {
                raw.write_all(&bytes).expect("write the raw bytes");
                terminal.write(&bytes);
                for event in terminal.take_events() {
                    match event {
                        Event::Reply(reply) => {
                            let _ = pty.write(&reply);
                        }
                        Event::Status(r) => eprintln!(
                            "[{:6.1}s] status {} {:?}",
                            start.elapsed().as_secs_f32(),
                            r.state.name(),
                            r.message
                        ),
                        other => eprintln!("[{:6.1}s] {other:?}", start.elapsed().as_secs_f32()),
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let _ = pty.kill();
    eprintln!("exit: {:?}", pty.wait().map(agentsi_pty::exit_code));
    let screen = terminal.grid().expect("grid").text_lines().join("\n");
    eprintln!("---- last screen ----\n{screen}");
}
