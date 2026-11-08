//! Open test 3 of docs/ideas/agentsi-desktop.md: does a Claude Code session render and take input
//! correctly in a libghostty-vt pane in GPUI? Throwaway prototype.
//!
//! One window, one pane. The PTY is opened here with `libc::openpty` (the pattern of herdr-core,
//! Apache-2.0, `platform/unix.rs`). A reader thread feeds the bytes to `ghostty_vt::Terminal` on the
//! GPUI thread. The screen grid is drawn with GPUI quads (cell backgrounds) and shaped text (glyphs).
mod pty;
mod stats;

use std::{cell::RefCell, rc::Rc, time::{Duration, Instant}};

use ghostty_vt::{
    Terminal, TerminalOptions, RenderState,
    key,
    render::{CellIterator, CursorVisualStyle, RowIterator},
    style::Underline,
    terminal::Event as TermEvent,
};
use gpui::{
    App, Bounds, Context, FocusHandle, Font, FontStyle, FontWeight, Hsla, KeyDownEvent, Pixels,
    Render, SharedString, StrikethroughStyle, TextAlign, TextRun, UnderlineStyle, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, canvas, div, fill, font, point,
    prelude::*, px, rgb, size,
};
use gpui_platform::application;

use pty::Pty;
use stats::Stats;

const FONT_FAMILY: &str = "JetBrainsMono Nerd Font";
const FONT_SIZE: f32 = 15.0;
const PAD: f32 = 6.0;

struct Args {
    program: Vec<String>,
    cwd: String,
    script: Vec<(Duration, Vec<u8>)>,
    exit_after: Option<Duration>,
    stats_path: Option<String>,
    event_log: Option<String>,
    dump: Option<String>,
    raw: Option<String>,
    width: f32,
    height: f32,
    cols_hint: Option<(u16, u16)>,
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
                b'\\' => out.push(b'\\'),
                b'x' if i + 3 < b.len() => {
                    if let Ok(v) = u8::from_str_radix(&s[i + 2..i + 4], 16) {
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

fn parse_args() -> Args {
    let mut a = Args {
        program: vec![],
        cwd: std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_else(|_| "/".into()),
        script: vec![],
        exit_after: None,
        stats_path: None,
        event_log: None,
        dump: None,
        raw: None,
        width: 1000.0,
        height: 640.0,
        cols_hint: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            // "delay_ms:text@@delay_ms:text": bytes written to the PTY after each delay, as if typed.
            "--script" => {
                for step in it.next().unwrap_or_default().split("@@") {
                    if let Some((ms, text)) = step.split_once(':') {
                        a.script.push((Duration::from_millis(ms.parse().unwrap_or(0)), unescape(text)));
                    }
                }
            }
            "--exit-after" => a.exit_after = it.next().and_then(|s| s.parse().ok()).map(Duration::from_secs_f32),
            "--stats" => a.stats_path = it.next(),
            "--event-log" => a.event_log = it.next(),
            "--dump" => a.dump = it.next(),
            "--raw" => a.raw = it.next(),
            "--cwd" => a.cwd = it.next().unwrap_or_default(),
            "--size" => {
                if let Some((w, h)) = it.next().and_then(|s| s.split_once('x').map(|(w, h)| (w.to_string(), h.to_string()))) {
                    a.width = w.parse().unwrap_or(a.width);
                    a.height = h.parse().unwrap_or(a.height);
                }
            }
            "--" => {
                a.program = it.by_ref().collect();
            }
            other => eprintln!("unknown argument {other}"),
        }
    }
    if a.program.is_empty() {
        a.program = vec![std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into())];
    }
    let _ = &a.cols_hint;
    a
}

/// One run of cells with the same look, drawn as one shaped line.
struct Seg {
    row: u16,
    col: u16,
    text: String,
    cols: u16,
    fg: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

struct Frame {
    bg: Hsla,
    backs: Vec<(u16, u16, u16, Hsla)>, // row, col, ncols, color
    segs: Vec<Seg>,
    blocks: Vec<(u16, u16, char, Hsla)>, // row, col, glyph U+2580..U+259F, color
    plain: Vec<String>,
    cursor: Option<(u16, u16, bool, CursorVisualStyle, Hsla)>, // col,row,wide,style,color
}

fn hsla(c: ghostty_vt::style::RgbColor) -> Hsla {
    rgb(((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32).into()
}

struct TermView {
    args: Args,
    font_size: Pixels,
    cw: Pixels,
    lh: Pixels,
    grid: (u16, u16),
    term: Option<Terminal>,
    rs: RenderState,
    rows: RowIterator,
    cells: CellIterator,
    encoder: key::Encoder,
    pty: Option<Pty>,
    focus: FocusHandle,
    stats: Rc<RefCell<Stats>>,
    pending_since: Option<Instant>,
    started: bool,
    title: String,
    last_plain: Vec<String>,
    osc_carry: Vec<u8>,
}

impl TermView {
    fn new(args: Args, cx: &mut Context<Self>) -> Self {
        let stats = Rc::new(RefCell::new(Stats::new(args.stats_path.clone())));
        TermView {
            args,
            font_size: px(FONT_SIZE),
            cw: px(9.0),
            lh: px(18.0),
            grid: (0, 0),
            term: None,
            rs: RenderState::new().expect("render state"),
            rows: RowIterator::new().expect("row iterator"),
            cells: CellIterator::new().expect("cell iterator"),
            encoder: key::Encoder::new().expect("key encoder"),
            pty: None,
            focus: cx.focus_handle(),
            stats,
            pending_since: None,
            started: false,
            title: String::new(),
            last_plain: vec![],
            osc_carry: vec![],
        }
    }

    fn the_font(bold: bool, italic: bool) -> Font {
        let mut f = font(FONT_FAMILY);
        if bold {
            f.weight = FontWeight::BOLD;
        }
        if italic {
            f.style = FontStyle::Italic;
        }
        f
    }

    /// Measures the cell and starts the child once the first layout size is known.
    fn ensure_started(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let vp = window.viewport_size();
        let ts = window.text_system();
        let fid = ts.resolve_font(&Self::the_font(false, false));
        self.cw = ts.advance(fid, self.font_size, 'm').map(|s| s.width.round()).unwrap_or(px(9.0));
        self.lh = (self.font_size * 1.25).round();
        let cols = (((vp.width - px(2. * PAD)) / self.cw).floor() as u16).max(2);
        let rows = (((vp.height - px(2. * PAD)) / self.lh).floor() as u16).max(2);
        if !self.started {
            self.started = true;
            self.grid = (cols, rows);
            let mut term = Terminal::new(TerminalOptions { cols, rows, max_scrollback: 2_000_000 }).expect("terminal");
            let _ = term.resize(cols, rows, f32::from(self.cw) as u32, f32::from(self.lh) as u32);
            self.term = Some(term);
            let pty = Pty::spawn(&self.args.program, &self.args.cwd, cols, rows).expect("spawn the child on a PTY");
            let rx = pty.start_reader();
            self.pty = Some(pty);
            cx.spawn(async move |this, cx| {
                while let Ok((t, mut bytes)) = rx.recv().await {
                    while let Ok((_, more)) = rx.try_recv() {
                        bytes.extend(more);
                    }
                    if this.update(cx, |v, cx| v.feed(t, &bytes, cx)).is_err() {
                        break;
                    }
                }
                let _ = this.update(cx, |v, cx| {
                    v.stats.borrow_mut().note("child output closed");
                    cx.notify();
                });
            })
            .detach();
            let script = std::mem::take(&mut self.args.script);
            if !script.is_empty() {
                cx.spawn(async move |this, cx| {
                    for (delay, bytes) in script {
                        cx.background_executor().timer(delay).await;
                        if this.update(cx, |v, _| v.send(&bytes)).is_err() {
                            break;
                        }
                    }
                })
                .detach();
            }
            if let Some(after) = self.args.exit_after {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(after).await;
                    let _ = this.update(cx, |v, cx| {
                        v.shutdown();
                        cx.quit();
                    });
                })
                .detach();
            }
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if this.update(cx, |v, _| v.stats.borrow_mut().sample()).is_err() {
                        break;
                    }
                }
            })
            .detach();
        } else if (cols, rows) != self.grid {
            self.grid = (cols, rows);
            if let Some(t) = self.term.as_mut() {
                let _ = t.resize(cols, rows, f32::from(self.cw) as u32, f32::from(self.lh) as u32);
            }
            if let Some(p) = &self.pty {
                p.resize(cols, rows);
            }
            self.stats.borrow_mut().note(&format!("resize to {cols}x{rows} cells ({:.0}x{:.0}px)", f32::from(vp.width), f32::from(vp.height)));
        }
    }

    fn feed(&mut self, t: Instant, bytes: &[u8], cx: &mut Context<Self>) {
        let Some(term) = self.term.as_mut() else { return };
        term.vt_write(bytes);
        let mut writes: Vec<Vec<u8>> = vec![];
        let mut logs: Vec<String> = vec![];
        for ev in term.events() {
            match ev {
                TermEvent::PtyWrite(b) => writes.push(b),
                TermEvent::TitleChanged(s) => {
                    self.title = s.clone();
                    logs.push(format!("title={s:?}"));
                }
                TermEvent::Bell => logs.push("bell".into()),
                TermEvent::PwdChanged(s) => logs.push(format!("pwd={s}")),
                TermEvent::DesktopNotification { title, body } => logs.push(format!("notification title={title:?} body={body:?}")),
                TermEvent::ProgressReport { state, progress } => logs.push(format!("progress state={state:?} progress={progress:?}")),
                TermEvent::RenderHold(on) => logs.push(format!("render-hold={on}")),
                other => logs.push(format!("{other:?}")),
            }
        }
        // The 7501 reply goes first: Claude Code sends `CSI c` after its probes and counts any query
        // without a reply by then as unanswered, so replies must keep the order of the queries.
        // libghostty-vt (stock) ignores OSC 7501 and 21337, so a side scanner reads them. A reply to
        // `OSC 7501 ; ?` tells Claude Code that this terminal takes program-status reports.
        for (code, payload) in self.scan_osc(bytes) {
            if code == 7501 && payload.starts_with('?') {
                self.send(b"\x1b]7501;?:states=idle,working,done,blocked,error\x1b\\");
                logs.push("osc7501 query answered".into());
            } else {
                logs.push(format!("osc{code} {payload}"));
            }
        }
        for w in writes {
            self.send(&w);
        }
        for l in logs {
            self.stats.borrow_mut().event(&l);
        }
        self.stats.borrow_mut().bytes += bytes.len() as u64;
        if let Some(p) = &self.args.raw
            && let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p)
        {
            use std::io::Write;
            let _ = f.write_all(bytes);
        }
        self.pending_since.get_or_insert(t);
        cx.notify();
    }

    /// Finds complete `ESC ] 7501 ; ...` and `ESC ] 21337 ; ...` sequences, also when a chunk splits one.
    fn scan_osc(&mut self, bytes: &[u8]) -> Vec<(u32, String)> {
        self.osc_carry.extend_from_slice(bytes);
        let buf = std::mem::take(&mut self.osc_carry);
        let mut out = vec![];
        let mut i = 0;
        let mut keep_from = buf.len();
        while let Some(rel) = buf[i..].windows(2).position(|w| w == b"\x1b]") {
            let start = i + rel;
            let body = start + 2;
            let Some(semi) = buf[body..].iter().take(6).position(|b| *b == b';') else {
                if buf.len() - body < 6 {
                    keep_from = start;
                }
                i = body;
                continue;
            };
            let code: u32 = std::str::from_utf8(&buf[body..body + semi]).ok().and_then(|c| c.parse().ok()).unwrap_or(0);
            let data = body + semi + 1;
            let end = buf[data..].iter().position(|b| *b == 0x07 || *b == 0x1b);
            match end {
                None => {
                    if code == 7501 || code == 21337 {
                        keep_from = start;
                    }
                    break;
                }
                Some(e) => {
                    if code == 7501 || code == 21337 {
                        out.push((code, String::from_utf8_lossy(&buf[data..data + e]).into_owned()));
                    }
                    i = data + e;
                }
            }
        }
        if keep_from < buf.len() && buf.len() - keep_from < 4096 {
            self.osc_carry = buf[keep_from..].to_vec();
        }
        out
    }

    fn send(&self, bytes: &[u8]) {
        if let Some(p) = &self.pty {
            p.write(bytes);
        }
    }

    fn shutdown(&mut self) {
        if let Some(p) = &self.args.dump {
            let _ = std::fs::write(p, self.last_plain.join("\n"));
        }
        if let Some(p) = &self.pty {
            p.kill();
        }
        self.stats.borrow_mut().finish();
    }

    /// Reads the terminal screen into owned segments, so the paint callback owns its data.
    fn build_frame(&mut self) -> Option<Frame> {
        let term = self.term.as_ref()?;
        let snap = self.rs.update(term).ok()?;
        let colors = snap.colors().ok()?;
        let cur = snap.cursor().ok()?;
        let mut f = Frame { bg: hsla(colors.background), backs: vec![], segs: vec![], blocks: vec![], plain: vec![], cursor: None };
        let mut row_iter = self.rows.update(&snap).ok()?;
        let mut y: u16 = 0;
        let mut buf = String::new();
        while let Some(row) = row_iter.next() {
            let mut cell_iter = self.cells.update(row).ok()?;
            let mut x: u16 = 0;
            let mut cur_seg: Option<Seg> = None;
            let mut gap: u16 = 0;
            let mut line = String::new();
            let mut cur_back: Option<(u16, u16, Hsla)> = None; // col, ncols, color
            while let Some(cell) = cell_iter.next() {
                let style = cell.style().unwrap_or_default();
                let wide = cell.raw_cell().and_then(|c| c.wide()).map(|w| w == ghostty_vt::screen::CellWide::Wide).unwrap_or(false);
                buf.clear();
                let _ = cell.graphemes_utf8(&mut buf);
                line.push_str(if buf.is_empty() { " " } else { &buf });
                if wide {
                    line.push(' ');
                }
                let block = {
                    let mut it = buf.chars();
                    match (it.next(), it.next()) {
                        (Some(c), None) if ('\u{2580}'..='\u{259F}').contains(&c) => Some(c),
                        _ => None,
                    }
                };
                let mut fg = cell.fg_color().ok().flatten().unwrap_or(colors.foreground);
                let mut bg = cell.bg_color().ok().flatten();
                if style.inverse {
                    let b = bg.unwrap_or(colors.background);
                    bg = Some(fg);
                    fg = b;
                }
                let mut fgc = hsla(fg);
                if style.faint {
                    let b = hsla(bg.unwrap_or(colors.background));
                    fgc = Hsla { h: fgc.h, s: fgc.s * 0.6, l: fgc.l * 0.55 + b.l * 0.45, a: fgc.a };
                }
                // Backgrounds: merge runs of the same color.
                let bgc = bg.map(hsla);
                match (&mut cur_back, bgc) {
                    (Some((_, n, c)), Some(b)) if *c == b => *n += 1,
                    (slot, new) => {
                        if let Some((c0, n, c)) = slot.take() {
                            f.backs.push((y, c0, n, c));
                        }
                        *slot = new.map(|b| (x, 1, b));
                    }
                }
                // Text. Blank cells wait in `gap` and join the segment only if more text of the same look follows.
                let underline = style.underline != Underline::None;
                let has_text = !buf.is_empty() && buf != " " && !style.invisible && block.is_none();
                if let Some(c) = block {
                    f.blocks.push((y, x, c, fgc));
                    if let Some(s) = cur_seg.take() {
                        f.segs.push(s);
                    }
                    gap = 0;
                }
                if has_text {
                    let same = !wide && cur_seg.as_ref().is_some_and(|s| {
                        s.fg == fgc && s.bold == style.bold && s.italic == style.italic && s.underline == underline
                            && s.strike == style.strikethrough && s.col + s.cols + gap == x
                    });
                    if same {
                        let s = cur_seg.as_mut().unwrap();
                        for _ in 0..gap {
                            s.text.push(' ');
                        }
                        s.cols += gap;
                        s.text.push_str(&buf);
                        s.cols += 1;
                    } else {
                        if let Some(s) = cur_seg.take() {
                            f.segs.push(s);
                        }
                        let seg = Seg {
                            row: y, col: x, text: buf.clone(), cols: if wide { 2 } else { 1 }, fg: fgc,
                            bold: style.bold, italic: style.italic, underline, strike: style.strikethrough,
                        };
                        if wide { f.segs.push(seg) } else { cur_seg = Some(seg) }
                    }
                    gap = 0;
                } else if cur_seg.is_some() {
                    gap += 1;
                }
                x += 1;
            }
            if let Some(s) = cur_seg.take() {
                f.segs.push(s);
            }
            if let Some((c0, n, c)) = cur_back.take() {
                f.backs.push((y, c0, n, c));
            }
            f.plain.push(line.trim_end().to_string());
            y += 1;
        }
        if cur.visible
            && let Some(v) = cur.viewport
        {
            f.cursor = Some((v.x, v.y, v.at_wide_tail, cur.visual_style, colors.cursor.map(hsla).unwrap_or(hsla(colors.foreground))));
        }
        let _ = snap.clean();
        self.last_plain = f.plain.clone();
        Some(f)
    }

    fn on_key(&mut self, ev: &KeyDownEvent) {
        let ks = &ev.keystroke;
        let name = ks.key.as_str();
        let k = match name {
            "enter" => key::Key::Enter,
            "tab" => key::Key::Tab,
            "escape" => key::Key::Escape,
            "backspace" => key::Key::Backspace,
            "delete" => key::Key::Delete,
            "home" => key::Key::Home,
            "end" => key::Key::End,
            "pageup" => key::Key::PageUp,
            "pagedown" => key::Key::PageDown,
            "insert" => key::Key::Insert,
            "up" => key::Key::ArrowUp,
            "down" => key::Key::ArrowDown,
            "left" => key::Key::ArrowLeft,
            "right" => key::Key::ArrowRight,
            "space" => key::Key::Space,
            "f1" => key::Key::F1, "f2" => key::Key::F2, "f3" => key::Key::F3, "f4" => key::Key::F4,
            "f5" => key::Key::F5, "f6" => key::Key::F6, "f7" => key::Key::F7, "f8" => key::Key::F8,
            "f9" => key::Key::F9, "f10" => key::Key::F10, "f11" => key::Key::F11, "f12" => key::Key::F12,
            s if s.chars().count() == 1 => match s.chars().next().unwrap().to_ascii_lowercase() {
                'a' => key::Key::A, 'b' => key::Key::B, 'c' => key::Key::C, 'd' => key::Key::D, 'e' => key::Key::E,
                'f' => key::Key::F, 'g' => key::Key::G, 'h' => key::Key::H, 'i' => key::Key::I, 'j' => key::Key::J,
                'k' => key::Key::K, 'l' => key::Key::L, 'm' => key::Key::M, 'n' => key::Key::N, 'o' => key::Key::O,
                'p' => key::Key::P, 'q' => key::Key::Q, 'r' => key::Key::R, 's' => key::Key::S, 't' => key::Key::T,
                'u' => key::Key::U, 'v' => key::Key::V, 'w' => key::Key::W, 'x' => key::Key::X, 'y' => key::Key::Y,
                'z' => key::Key::Z,
                '0' => key::Key::Digit0, '1' => key::Key::Digit1, '2' => key::Key::Digit2, '3' => key::Key::Digit3,
                '4' => key::Key::Digit4, '5' => key::Key::Digit5, '6' => key::Key::Digit6, '7' => key::Key::Digit7,
                '8' => key::Key::Digit8, '9' => key::Key::Digit9,
                '-' => key::Key::Minus, '=' => key::Key::Equal, '[' => key::Key::BracketLeft, ']' => key::Key::BracketRight,
                '\\' => key::Key::Backslash, ';' => key::Key::Semicolon, '\'' => key::Key::Quote, ',' => key::Key::Comma,
                '.' => key::Key::Period, '/' => key::Key::Slash, '`' => key::Key::Backquote,
                _ => key::Key::Unidentified,
            },
            _ => key::Key::Unidentified,
        };
        let m = &ks.modifiers;
        let mut mods = key::Mods::empty();
        if m.shift { mods |= key::Mods::SHIFT }
        if m.alt { mods |= key::Mods::ALT }
        if m.control { mods |= key::Mods::CTRL }
        if m.platform { mods |= key::Mods::SUPER }
        let text = ks.key_char.clone().filter(|s| !s.chars().any(|c| c.is_control()) && !m.control);
        let Some(term) = self.term.as_ref() else { return };
        let (Ok(mut event), true) = (key::Event::new(), true) else { return };
        event.set_action(if ev.is_held { key::Action::Repeat } else { key::Action::Press });
        event.set_key(k);
        event.set_mods(mods);
        if text.is_some() && m.shift {
            event.set_consumed_mods(key::Mods::SHIFT);
        }
        if let Some(c) = name.chars().next().filter(|_| name.chars().count() == 1) {
            event.set_unshifted_codepoint(c.to_lowercase().next().unwrap_or(c));
        }
        event.set_utf8(text);
        self.encoder.set_options_from_terminal(term);
        let mut out = Vec::new();
        if self.encoder.encode_to_vec(&event, &mut out).is_ok() && !out.is_empty() {
            self.send(&out);
        }
    }
}

impl Render for TermView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_started(window, cx);
        if !self.focus.is_focused(window) {
            window.focus(&self.focus, cx);
        }
        let t_build = Instant::now();
        let frame = self.build_frame();
        self.stats.borrow_mut().build_ms.push(t_build.elapsed().as_secs_f32() * 1000.0);
        let pending = self.pending_since.take();
        let stats = self.stats.clone();
        let (cw, lh, fs) = (self.cw, self.lh, self.font_size);
        let bg = frame.as_ref().map(|f| f.bg).unwrap_or(gpui::black());
        if !self.title.is_empty() {
            window.set_window_title(&format!("{} - gpui-terminal", self.title));
        }
        div()
            .size_full()
            .bg(bg)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, _| this.on_key(ev)))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds: Bounds<Pixels>, _, window, cx| {
                        let t_paint = Instant::now();
                        if let Some(f) = &frame {
                            let o = bounds.origin + point(px(PAD), px(PAD));
                            for (row, col, n, c) in &f.backs {
                                let b = Bounds { origin: point(o.x + cw * *col as f32, o.y + lh * *row as f32), size: size(cw * *n as f32, lh) };
                                window.paint_quad(fill(b, *c));
                            }
                            for (row, col, c, color) in &f.blocks {
                                let (cx0, cy0) = (o.x + cw * *col as f32, o.y + lh * *row as f32);
                                let rects: &[(f32, f32, f32, f32)] = match *c as u32 {
                                    0x2580 => &[(0., 0., 1., 0.5)],
                                    0x2581 => &[(0., 0.875, 1., 1.)], 0x2582 => &[(0., 0.75, 1., 1.)], 0x2583 => &[(0., 0.625, 1., 1.)],
                                    0x2584 => &[(0., 0.5, 1., 1.)], 0x2585 => &[(0., 0.375, 1., 1.)], 0x2586 => &[(0., 0.25, 1., 1.)],
                                    0x2587 => &[(0., 0.125, 1., 1.)], 0x2588 | 0x2591..=0x2593 => &[(0., 0., 1., 1.)],
                                    0x2589 => &[(0., 0., 0.875, 1.)], 0x258A => &[(0., 0., 0.75, 1.)], 0x258B => &[(0., 0., 0.625, 1.)],
                                    0x258C => &[(0., 0., 0.5, 1.)], 0x258D => &[(0., 0., 0.375, 1.)], 0x258E => &[(0., 0., 0.25, 1.)],
                                    0x258F => &[(0., 0., 0.125, 1.)], 0x2590 => &[(0.5, 0., 1., 1.)],
                                    0x2594 => &[(0., 0., 1., 0.125)], 0x2595 => &[(0.875, 0., 1., 1.)],
                                    0x2596 => &[(0., 0.5, 0.5, 1.)], 0x2597 => &[(0.5, 0.5, 1., 1.)], 0x2598 => &[(0., 0., 0.5, 0.5)],
                                    0x2599 => &[(0., 0., 0.5, 1.), (0.5, 0.5, 1., 1.)], 0x259A => &[(0., 0., 0.5, 0.5), (0.5, 0.5, 1., 1.)],
                                    0x259B => &[(0., 0., 1., 0.5), (0., 0.5, 0.5, 1.)], 0x259C => &[(0., 0., 1., 0.5), (0.5, 0.5, 1., 1.)],
                                    0x259D => &[(0.5, 0., 1., 0.5)], 0x259E => &[(0.5, 0., 1., 0.5), (0., 0.5, 0.5, 1.)],
                                    _ => &[(0., 0.5, 1., 1.), (0.5, 0., 1., 1.)], // 0x259F
                                };
                                let mut col = *color;
                                col.a = match *c as u32 { 0x2591 => 0.25, 0x2592 => 0.5, 0x2593 => 0.75, _ => 1.0 };
                                for (x0, y0, x1, y1) in rects {
                                    let b = Bounds { origin: point(cx0 + cw * *x0, cy0 + (lh * *y0).round()), size: size(cw * (*x1 - *x0), (lh * *y1).round() - (lh * *y0).round()) };
                                    window.paint_quad(fill(b, col));
                                }
                            }
                            for s in &f.segs {
                                let run = TextRun {
                                    len: s.text.len(),
                                    font: TermView::the_font(s.bold, s.italic),
                                    color: s.fg,
                                    background_color: None,
                                    underline: s.underline.then_some(UnderlineStyle { thickness: px(1.), color: Some(s.fg), wavy: false }),
                                    strikethrough: s.strike.then_some(StrikethroughStyle { thickness: px(1.), color: Some(s.fg) }),
                                };
                                let fw = if s.cols == 2 && s.text.chars().count() == 1 { cw * 2.0 } else { cw };
                                let line = window.text_system().shape_line(SharedString::from(s.text.clone()), fs, &[run], Some(fw));
                                let _ = line.paint(point(o.x + cw * s.col as f32, o.y + lh * s.row as f32), lh, TextAlign::Left, None, window, cx);
                            }
                            if let Some((col, row, wide, style, c)) = f.cursor {
                                let w = if wide { cw * 2.0 } else { cw };
                                let origin = point(o.x + cw * col as f32, o.y + lh * row as f32);
                                let b = match style {
                                    CursorVisualStyle::Bar => Bounds { origin, size: size(px(2.), lh) },
                                    CursorVisualStyle::Underline => Bounds { origin: point(origin.x, origin.y + lh - px(2.)), size: size(w, px(2.)) },
                                    _ => Bounds { origin, size: size(w, lh) },
                                };
                                let mut cc = c;
                                if matches!(style, CursorVisualStyle::Block) {
                                    cc.a = 0.55;
                                }
                                window.paint_quad(fill(b, cc));
                            }
                        }
                        let mut st = stats.borrow_mut();
                        st.paint_ms.push(t_paint.elapsed().as_secs_f32() * 1000.0);
                        st.frames += 1;
                        if let Some(t) = pending {
                            st.latency_ms.push(t.elapsed().as_secs_f32() * 1000.0);
                        }
                    },
                )
                .size_full(),
            )
    }
}

fn main() {
    let args = parse_args();
    let (w, h) = (args.width, args.height);
    application().run(move |cx: &mut App| {
        cx.open_window(
            WindowOptions {
                titlebar: None,
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
                    size: size(px(w), px(h)),
                })),
                app_id: Some("agentsi-terminal".to_string()),
                window_background: WindowBackgroundAppearance::Opaque,
                focus: true,
                ..Default::default()
            },
            |_, cx| cx.new(|cx| TermView::new(args, cx)),
        )
        .expect("open the terminal window");
    });
}
