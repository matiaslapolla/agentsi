//! A terminal pane on a local PTY, to check the pane before the app connects it to the core.
//!
//! `cargo run -p agentsi-term-view --example pane --release -- [options] [-- <argv>…]`
//!
//! - Without `<argv>`, the pane runs `$SHELL`.
//! - `--size <w>x<h>`: the window size in pixels (default 1000x640).
//! - `--script "<ms>:<text>@@<ms>:<text>"`: writes each text to the PTY after its delay, as if typed.
//!   `\r`, `\n`, `\t`, `\e` and `\xNN` work in the text.
//! - `--exit-after <s>`: quits after that many seconds. `--dump <file>` then writes the screen text.
//! - `--bench terminal --seconds 10 --json`: without `<argv>`, a script writes about 60 lines a
//!   second, and the bench reports the time from each PTY read to its paint.
//! - `--bench idle --seconds 10 --json`: counts the frames of the idle pane.
use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

use agentsi_pty::{Pty, PtySize, SpawnOptions};
use agentsi_term_view::{PaneEvent, PaneOptions, PaneOutput, TerminalPane};
use agentsi_ui::bench::{Bench, BenchKind};
use futures::StreamExt;
use futures::channel::mpsc;
use gpui_kit::{AppContext as _, Bounds, Focusable as _, WindowBounds, WindowOptions, px, size};

const BENCH_WRITER: &str = include_str!("../bench/terminal.sh");

struct Args {
    argv: Vec<String>,
    width: f32,
    height: f32,
    script: Vec<(Duration, Vec<u8>)>,
    exit_after: Option<Duration>,
    dump: Option<String>,
}

fn unescape(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            match b[i + 1] {
                b'r' => out.push(b'\r'),
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'e' => out.push(0x1b),
                b'x' if i + 3 < b.len() => {
                    let hex = std::str::from_utf8(&b[i + 2..i + 4]).unwrap_or("");
                    if let Ok(v) = u8::from_str_radix(hex, 16) {
                        out.push(v);
                    }
                    i += 2;
                }
                c => out.push(c),
            }
            i += 2;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    out
}

fn parse_args(bench: Option<BenchKind>) -> Args {
    let mut a = Args {
        argv: Vec::new(),
        width: 1000.,
        height: 640.,
        script: Vec::new(),
        exit_after: None,
        dump: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--size" => {
                if let Some((w, h)) = it.next().as_deref().and_then(|s| s.split_once('x')) {
                    a.width = w.parse().unwrap_or(a.width);
                    a.height = h.parse().unwrap_or(a.height);
                }
            }
            "--script" => {
                for step in it.next().unwrap_or_default().split("@@") {
                    if let Some((ms, text)) = step.split_once(':') {
                        let delay = Duration::from_millis(ms.parse().unwrap_or(0));
                        a.script.push((delay, unescape(text)));
                    }
                }
            }
            "--exit-after" => {
                a.exit_after = it
                    .next()
                    .and_then(|s| s.parse().ok())
                    .map(Duration::from_secs_f32)
            }
            "--dump" => a.dump = it.next(),
            // The bench flags with a value. `Bench::from_env` reads them.
            "--bench" | "--seconds" | "--warmup-ms" | "--speed" | "--runs" => {
                it.next();
            }
            "--" => a.argv = it.by_ref().collect(),
            _ => {}
        }
    }
    if a.argv.is_empty() {
        a.argv = if bench == Some(BenchKind::Terminal) {
            vec!["sh".into(), "-c".into(), BENCH_WRITER.into()]
        } else {
            vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())]
        };
    }
    a
}

fn spawn(argv: Vec<String>) -> std::io::Result<Pty> {
    let mut options = SpawnOptions::new(argv, PtySize { cols: 80, rows: 24 });
    options.cwd = std::env::current_dir().ok();
    // Inside a Claude Code session, these variables make a child `claude` act as a tool call of it.
    const PREFIXES: [&str; 4] = ["CLAUDECODE", "CLAUDE_CODE", "CLAUDE_JOB", "CLAUDE_BG"];
    options.env_remove = std::env::vars_os()
        .map(|(k, _)| k)
        .filter(|k| PREFIXES.iter().any(|p| k.to_string_lossy().starts_with(p)))
        .collect();
    Pty::spawn(&options)
}

fn main() {
    let main_start = Instant::now();
    let bench = Bench::from_env(main_start);
    let args = parse_args(bench.as_ref().map(Bench::kind));
    let pty = match spawn(args.argv.clone()) {
        Ok(p) => Arc::new(p),
        Err(e) => {
            eprintln!("pane: cannot start {:?}: {e}", args.argv);
            std::process::exit(1);
        }
    };
    let (tx, mut rx) = mpsc::unbounded::<(Instant, Vec<u8>)>();
    let mut reader = pty.reader().expect("a reader of the PTY");
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx
                        .unbounded_send((Instant::now(), buf[..n].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            agentsi_ui::init(cx);
            let opts = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(args.width), px(args.height)),
                    cx,
                ))),
                app_id: Some("agentsi-term-view".into()),
                ..Default::default()
            };
            let b = bench.clone();
            let opened = gpui_kit::open_window(opts, cx, move |window, cx| {
                let pane = cx.new(|cx| {
                    let options = PaneOptions {
                        bench: b.clone(),
                        ..Default::default()
                    };
                    let pane = TerminalPane::new(options, window, cx);
                    if let Some(b) = &b {
                        b.start(window, cx);
                    }
                    pane
                });
                window.focus(&pane.focus_handle(cx), cx);
                pane
            });
            let (handle, pane) = opened.expect("open the pane window");
            let out = pty.clone();
            cx.subscribe(&pane, move |pane, event, cx| match event {
                PaneEvent::Output(PaneOutput::Input(b) | PaneOutput::Auto(b)) => {
                    let _ = out.write(b);
                }
                PaneEvent::Output(PaneOutput::Resize { cols, rows }) => {
                    let _ = out.resize(PtySize {
                        cols: *cols,
                        rows: *rows,
                    });
                }
                PaneEvent::Output(PaneOutput::Focus(f)) => {
                    if let Some(b) = pane.read(cx).focus_report(*f) {
                        let _ = out.write(&b);
                    }
                }
                PaneEvent::Title(t) => {
                    let t = t.clone();
                    let _ = handle.update(cx, |_, window, _| window.set_window_title(&t));
                }
                PaneEvent::Bell => {}
            })
            .detach();
            let feed = pane.clone();
            cx.spawn(async move |cx| {
                while let Some((t, mut bytes)) = rx.next().await {
                    while let Ok((_, more)) = rx.try_recv() {
                        bytes.extend(more);
                    }
                    feed.update(cx, |p, cx| p.feed(&bytes, t, cx));
                }
                // The program ended. A bench keeps the window until its report.
                if bench.is_none() {
                    cx.update(|cx| cx.quit());
                }
            })
            .detach();
            let typed = pty.clone();
            let script = args.script;
            cx.spawn(async move |cx| {
                for (delay, bytes) in script {
                    cx.background_executor().timer(delay).await;
                    let _ = typed.write(&bytes);
                }
            })
            .detach();
            if let Some(after) = args.exit_after {
                let dump = args.dump.clone();
                let quit_pane = pane.clone();
                let child = pty.clone();
                cx.spawn(async move |cx| {
                    cx.background_executor().timer(after).await;
                    cx.update(|cx| {
                        if let Some(path) = &dump {
                            let text = quit_pane.read(cx).text_lines().join("\n");
                            let _ = std::fs::write(path, text + "\n");
                        }
                        let _ = child.kill();
                        cx.quit();
                    });
                })
                .detach();
            }
            cx.activate(true);
        });
}
