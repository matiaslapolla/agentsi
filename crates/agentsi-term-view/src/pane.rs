//! The terminal pane: a GPUI view of one terminal. Bytes from the program go in with
//! [`TerminalPane::feed`]. Typed keys, screen replies, sizes and focus come out as [`PaneEvent`]s.
//!
//! The pane has three views. The root takes input. The cells view draws the screen and is cached,
//! so a frame that only moves the cursor does not draw the cells again. The cursor view draws the
//! cursor and its blink.

use std::cell::{Cell as StdCell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use agentsi_ui::bench::Bench;
use agentsi_ui::tokens::{font, motion, size, terminal};
use agentsi_vt::{
    CursorShape, Grid, Key, KeyAction, KeyEvent, MouseAction, MouseButton as VtButton,
    MouseEvent as VtMouse, MouseGeometry, Options, Palette, Rgb, Scroll, Terminal, mode,
};
use gpui_kit::{
    AnyView, App, AppContext as _, Bounds, ClipboardItem, Context, DispatchPhase, Entity,
    EventEmitter, FocusHandle, Focusable, Font, FontStyle, FontWeight, Hsla, InteractiveElement,
    IntoElement, KeyDownEvent, KeyUpEvent, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, ParentElement, Path, PathBuilder, Pixels, Point, Render, ScrollDelta,
    ScrollEvent, ShapedLine, SharedString, StrikethroughStyle, StyleRefinement, Styled,
    Subscription, TextAlign, TextRun, UnderlineStyle, WeakEntity, Window, canvas, div, fill,
    outline, point, px, size as gsize,
};

use crate::frame::{CursorCell, Frame, FrameCache, Run};
use crate::keys::{self, Shortcut};
use crate::selection::{Point as CellPoint, Selection, Unit};
use crate::shapes::{Shape, line_width, shapes};

/// A wheel notch moves this many rows.
const WHEEL_ROWS: f32 = 3.;
/// While the program holds a synchronized update (mode 2026), the pane waits at most this long
/// before it draws. Ghostty uses the same limit.
const SYNC_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct PaneOptions {
    /// The most memory that the scrollback of the screen may use.
    pub scrollback_bytes: usize,
    /// With a terminal bench, the pane records each read-to-paint time.
    pub bench: Option<Bench>,
}

impl Default for PaneOptions {
    fn default() -> Self {
        PaneOptions {
            scrollback_bytes: 10_000_000,
            bench: None,
        }
    }
}

/// What the pane sends toward the program. The kinds match the frames of the core protocol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneOutput {
    /// Typed keys, pastes and mouse reports.
    Input(Vec<u8>),
    /// Replies that the screen of the pane made to queries of the program (protocol frame AUTO).
    Auto(Vec<u8>),
    /// The pane has a new size in cells (protocol frame RESIZE).
    Resize { cols: u16, rows: u16 },
    /// The pane gained or lost the keyboard focus (protocol frame FOCUS).
    Focus(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneEvent {
    Output(PaneOutput),
    Title(String),
    Bell,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Metrics {
    font_size: Pixels,
    cell_w: Pixels,
    cell_h: Pixels,
    pad: Pixels,
}

impl Metrics {
    fn measure(window: &Window) -> Metrics {
        let font_size = size::MONO;
        let ts = window.text_system();
        let id = ts.resolve_font(&mono(false, false));
        let fallback = font_size * 0.6;
        let cell_w = ts
            .advance(id, font_size, 'm')
            .map(|s| s.width)
            .unwrap_or(fallback)
            .round();
        Metrics {
            font_size,
            cell_w,
            cell_h: (font_size * terminal::LINE_HEIGHT).round(),
            pad: terminal::PADDING,
        }
    }

    /// The grid that fits in `bounds`.
    fn fit(&self, bounds: Bounds<Pixels>) -> (u16, u16) {
        let inner = |len: Pixels, cell: Pixels| {
            (f32::from(len - self.pad * 2.) / f32::from(cell))
                .floor()
                .clamp(2., f32::from(u16::MAX)) as u16
        };
        (
            inner(bounds.size.width, self.cell_w),
            inner(bounds.size.height, self.cell_h),
        )
    }
}

/// Whole pixels, for the library: cell sizes, size reports and mouse geometry.
fn px_u32(p: Pixels) -> u32 {
    f32::from(p).round().max(0.) as u32
}

fn mono(bold: bool, italic: bool) -> Font {
    let mut f = gpui_kit::font(font::MONO);
    if bold {
        f.weight = FontWeight::BOLD;
    }
    if italic {
        f.style = FontStyle::Italic;
    }
    f
}

fn rgb(c: agentsi_ui::tokens::Color) -> Rgb {
    Rgb {
        r: (c.0 >> 16) as u8,
        g: (c.0 >> 8) as u8,
        b: c.0 as u8,
    }
}

fn palette() -> Palette {
    Palette {
        foreground: rgb(terminal::FOREGROUND),
        background: rgb(terminal::BACKGROUND),
        cursor: Some(rgb(terminal::CURSOR)),
        ansi: terminal::ANSI.map(rgb),
    }
}

/// The state that the three views share.
struct Screen {
    term: Terminal,
    metrics: Metrics,
    cols: u16,
    rows: u16,
    /// The bounds of the cells view in the last frame.
    bounds: Bounds<Pixels>,
    /// The grid and the frame, kept from one refresh to the next.
    cache: FrameCache,
    /// The absolute row of the first row of the grid.
    top: u64,
    dirty: bool,
    /// The time of the first read that the screen has not painted yet.
    pending_read: Option<Instant>,
    selection: Option<Selection>,
    selecting: bool,
    /// The part of a row that the wheel moved, not yet applied.
    wheel_rest: f32,
    /// False until the pane reported its first size from its layout.
    sized: bool,
    /// The size of the last RESIZE from the core, when it is not the size of the pane.
    source: Option<(u16, u16)>,
    /// True while a first paint at `source` arrives. It ends at the next RESIZE.
    painting: bool,
}

impl Screen {
    /// Reads the rows that changed and builds them again, if bytes, a scroll or the selection
    /// changed the screen.
    fn refresh(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let top = self.term.scrollbar().map(|b| b.offset).unwrap_or(0);
        // On an error the frame stays, so `top` must stay with it.
        if self
            .cache
            .update(&mut self.term, top, self.selection.as_ref())
            .is_ok()
        {
            self.top = top;
        }
    }

    fn grid(&self) -> &Grid {
        self.cache.grid()
    }

    fn frame(&self) -> &Rc<Frame> {
        self.cache.frame()
    }

    /// The cell under a window position, clamped to the grid.
    fn cell_at(&self, pos: Point<Pixels>) -> CellPoint {
        let m = &self.metrics;
        let x = f32::from(pos.x - self.bounds.origin.x - m.pad) / f32::from(m.cell_w);
        let y = f32::from(pos.y - self.bounds.origin.y - m.pad) / f32::from(m.cell_h);
        let col = x.floor().clamp(0., f32::from(self.cols.saturating_sub(1))) as u16;
        let row = y.floor().clamp(0., f32::from(self.rows.saturating_sub(1))) as u64;
        CellPoint {
            row: self.top + row,
            col,
        }
    }

    /// The cells of the absolute row `row`, when it is in view.
    fn row_cells(&self, row: u64) -> &[agentsi_vt::Cell] {
        row.checked_sub(self.top)
            .and_then(|r| self.grid().rows.get(r as usize))
            .map_or(&[], Vec::as_slice)
    }

    fn selected_text(&self) -> Option<String> {
        let s = self.selection.as_ref()?;
        let rows = self
            .grid()
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| (self.top + i as u64, r.as_slice()));
        Some(s.text(rows)).filter(|t| !t.is_empty())
    }

    fn geometry(&self) -> MouseGeometry {
        MouseGeometry {
            width_px: px_u32(self.bounds.size.width),
            height_px: px_u32(self.bounds.size.height),
            cell_width_px: px_u32(self.metrics.cell_w),
            cell_height_px: px_u32(self.metrics.cell_h),
            padding_px: px_u32(self.metrics.pad),
        }
    }
}

pub struct TerminalPane {
    screen: Rc<RefCell<Screen>>,
    focus: FocusHandle,
    cells: Entity<CellsView>,
    cursor: Entity<CursorView>,
    bench: Option<Bench>,
    /// The last focus that the pane reported: its focus handle is focused and the window is active.
    focused: bool,
    blink: Rc<Blink>,
    blink_running: bool,
    last_activity: Instant,
    sync_wait: bool,
    /// Counts the waits, so the timer of an old wait does not end a newer one.
    sync_generation: u64,
    /// The button of a press that the pane took: a press sent to the program, or the start of a
    /// selection. Its release and drag can happen outside the pane.
    pressed: Option<VtButton>,
    _subscriptions: Vec<Subscription>,
}

/// What the cursor view reads.
struct Blink {
    /// False in the off phase of a blink.
    on: StdCell<bool>,
    focused: StdCell<bool>,
}

impl EventEmitter<PaneEvent> for TerminalPane {}

impl Focusable for TerminalPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TerminalPane {
    pub fn new(options: PaneOptions, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let metrics = Metrics::measure(window);
        let (cols, rows) = (80, 24);
        let mut term = Terminal::new(Options {
            cols,
            rows,
            scrollback_bytes: options.scrollback_bytes,
        })
        .expect("libghostty-vt makes a terminal");
        // Both calls fail only on an allocation failure or a value out of range, and the values are fixed.
        let _ = term.resize(cols, rows, px_u32(metrics.cell_w), px_u32(metrics.cell_h));
        let _ = term.set_palette(&palette());
        let screen = Rc::new(RefCell::new(Screen {
            term,
            metrics,
            cols,
            rows,
            bounds: Bounds::default(),
            cache: FrameCache::default(),
            top: 0,
            dirty: true,
            pending_read: None,
            selection: None,
            selecting: false,
            wheel_rest: 0.,
            sized: false,
            source: None,
            painting: false,
        }));
        let blink = Rc::new(Blink {
            on: StdCell::new(true),
            focused: StdCell::new(false),
        });
        let focus = cx.focus_handle();
        let pane = cx.weak_entity();
        let cells = cx.new(|_| CellsView {
            screen: screen.clone(),
            pane: pane.clone(),
            bench: options.bench.clone(),
        });
        let cursor = cx.new(|_| CursorView {
            screen: screen.clone(),
            blink: blink.clone(),
        });
        let subscriptions = vec![
            cx.on_focus_in(&focus, window, |p, window, cx| p.update_focus(window, cx)),
            cx.on_focus_out(&focus, window, |p, _, window, cx| {
                p.update_focus(window, cx)
            }),
            cx.observe_window_activation(window, |p, window, cx| p.update_focus(window, cx)),
        ];
        TerminalPane {
            screen,
            focus,
            cells,
            cursor,
            bench: options.bench,
            focused: false,
            blink,
            blink_running: false,
            last_activity: Instant::now(),
            sync_wait: false,
            sync_generation: 0,
            pressed: None,
            _subscriptions: subscriptions,
        }
    }

    /// Feeds bytes from the program. `read_at` is when they were read, for the read-to-paint time.
    /// Replies of the screen leave as [`PaneOutput::Auto`] in the order of the stream.
    pub fn feed(&mut self, bytes: &[u8], read_at: Instant, cx: &mut Context<Self>) {
        let (events, hold) = {
            let mut s = self.screen.borrow_mut();
            let s = &mut *s;
            let (cw, ch) = (px_u32(s.metrics.cell_w), px_u32(s.metrics.cell_h));
            // A first paint of the core starts with a reset (`ESC c`), and the core sends it in
            // frames of 64 KiB, so one paint can be many feeds.
            if s.source.is_some() && !s.painting {
                s.painting = bytes.starts_with(b"\x1bc");
                if !s.painting {
                    s.source = None;
                }
            }
            let source = s
                .source
                .filter(|&(c, r)| s.term.resize(c, r, cw, ch).is_ok());
            s.term.write(bytes);
            if let Some(size) = source {
                s.selection = None;
                if !s.sized || s.term.resize(s.cols, s.rows, cw, ch).is_err() {
                    // The first layout resizes the terminal from this size.
                    (s.cols, s.rows) = size;
                    s.source = None;
                    s.painting = false;
                }
            }
            s.dirty = true;
            s.pending_read.get_or_insert(read_at);
            (s.term.take_events(), s.term.mode(mode::SYNC_OUTPUT))
        };
        for event in events {
            match event {
                agentsi_vt::Event::Reply(b) => cx.emit(PaneEvent::Output(PaneOutput::Auto(b))),
                agentsi_vt::Event::Title(t) => cx.emit(PaneEvent::Title(t)),
                agentsi_vt::Event::Bell => cx.emit(PaneEvent::Bell),
                _ => {}
            }
        }
        if hold {
            self.wait_for_sync(cx);
        } else {
            self.sync_wait = false;
            self.redraw(cx);
        }
    }

    /// The size of a RESIZE from the core: the size of the screen of the core for the bytes after it.
    /// A first paint after it is written at that size until the next RESIZE, and after each feed the
    /// pane goes back to its own size, as a terminal does on a resize. A paint for more rows would
    /// otherwise scroll its text out of view. Live bytes keep the size of the pane.
    pub fn source_size(&mut self, cols: u16, rows: u16) {
        let mut s = self.screen.borrow_mut();
        s.source = ((cols, rows) != (s.cols, s.rows)).then_some((cols, rows));
        s.painting = false;
    }

    /// The focus report for the program (`CSI I` or `CSI O`), when it asked for focus events.
    /// A host without a core writes it on [`PaneOutput::Focus`]. The core writes its own.
    pub fn focus_report(&self, focused: bool) -> Option<Vec<u8>> {
        self.screen.borrow().term.encode_focus(focused)
    }

    /// The size of the grid in cells.
    pub fn grid_size(&self) -> (u16, u16) {
        let s = self.screen.borrow();
        (s.cols, s.rows)
    }

    /// The text of each visible row, without trailing blanks.
    pub fn text_lines(&self) -> Vec<String> {
        let mut s = self.screen.borrow_mut();
        s.term.grid().map(|g| g.text_lines()).unwrap_or_default()
    }

    fn redraw(&mut self, cx: &mut Context<Self>) {
        self.cells.update(cx, |_, cx| cx.notify());
        self.cursor.update(cx, |_, cx| cx.notify());
    }

    fn wait_for_sync(&mut self, cx: &mut Context<Self>) {
        if self.sync_wait {
            return;
        }
        self.sync_wait = true;
        self.sync_generation += 1;
        let generation = self.sync_generation;
        cx.spawn(async move |pane, cx| {
            cx.background_executor().timer(SYNC_TIMEOUT).await;
            let _ = pane.update(cx, |p, cx| {
                if p.sync_wait && p.sync_generation == generation {
                    p.sync_wait = false;
                    p.redraw(cx);
                }
            });
        })
        .detach();
    }

    fn emit_input(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if !bytes.is_empty() {
            cx.emit(PaneEvent::Output(PaneOutput::Input(bytes)));
        }
    }

    /// Called from the cells view after the first layout, and when its bounds give a new grid size.
    /// The first call always reports the size, so the program learns it even when it is 80 × 24.
    fn resize(&mut self, cols: u16, rows: u16, cx: &mut Context<Self>) {
        {
            let mut s = self.screen.borrow_mut();
            let same = (s.cols, s.rows) == (cols, rows);
            if same && s.sized {
                return;
            }
            let m = s.metrics;
            if !same
                && s.term
                    .resize(cols, rows, px_u32(m.cell_w), px_u32(m.cell_h))
                    .is_err()
            {
                return;
            }
            s.sized = true;
            s.cols = cols;
            s.rows = rows;
            s.selection = None;
            s.dirty = true;
        }
        cx.emit(PaneEvent::Output(PaneOutput::Resize { cols, rows }));
        self.redraw(cx);
    }

    fn update_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focused = self.focus.is_focused(window) && window.is_window_active();
        if focused == self.focused {
            return;
        }
        self.focused = focused;
        self.blink.focused.set(focused);
        self.blink.on.set(true);
        cx.emit(PaneEvent::Output(PaneOutput::Focus(focused)));
        self.cursor.update(cx, |_, cx| cx.notify());
        self.activity(cx);
    }

    /// Typing or a focus change shows the cursor and restarts the blink.
    fn activity(&mut self, cx: &mut Context<Self>) {
        self.last_activity = Instant::now();
        if !self.blink.on.get() {
            self.blink.on.set(true);
            self.cursor.update(cx, |_, cx| cx.notify());
        }
        self.ensure_blink(cx);
    }

    /// Starts the blink when the cursor should blink and no blink runs. The cells view also calls
    /// it, because the frame that tells whether the program asked for a blinking cursor is built
    /// after the bytes arrive.
    fn ensure_blink(&mut self, cx: &mut Context<Self>) {
        if self.blink_running
            || !self.wants_blink()
            || self.last_activity.elapsed() >= motion::CARET_IDLE
        {
            return;
        }
        self.blink_running = true;
        cx.spawn(async move |pane, cx| {
            loop {
                cx.background_executor().timer(motion::CARET_BLINK).await;
                let more = pane.update(cx, |p, cx| p.blink_tick(cx)).unwrap_or(false);
                if !more {
                    break;
                }
            }
        })
        .detach();
    }

    fn wants_blink(&self) -> bool {
        self.focused
            && self
                .screen
                .borrow()
                .frame()
                .cursor
                .as_ref()
                .is_some_and(|c| c.blinking)
    }

    /// One phase of the blink. Returns false when the blink stops: no focus, no blinking cursor, or
    /// no typing and no output for `motion::CARET_IDLE`.
    fn blink_tick(&mut self, cx: &mut Context<Self>) -> bool {
        let stop = !self.wants_blink() || self.last_activity.elapsed() >= motion::CARET_IDLE;
        if stop {
            self.blink_running = false;
            if !self.blink.on.get() {
                self.blink.on.set(true);
                self.cursor.update(cx, |_, cx| cx.notify());
            }
            return false;
        }
        self.blink.on.set(!self.blink.on.get());
        self.cursor.update(cx, |_, cx| cx.notify());
        true
    }

    fn scroll_view(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        {
            let mut s = self.screen.borrow_mut();
            s.term.scroll(scroll);
            s.dirty = true;
        }
        self.redraw(cx);
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.last_activity = Instant::now();
        if let Some(sc) = keys::shortcut(&ev.keystroke) {
            let page =
                isize::try_from(self.screen.borrow().rows.saturating_sub(1).max(1)).unwrap_or(1);
            match sc {
                Shortcut::Copy => self.copy(cx),
                Shortcut::Paste => {
                    let text = cx.read_from_clipboard().and_then(|i| i.text());
                    self.paste(text, cx);
                }
                Shortcut::PageUp => self.scroll_view(Scroll::Lines(-page), cx),
                Shortcut::PageDown => self.scroll_view(Scroll::Lines(page), cx),
                Shortcut::Top => self.scroll_view(Scroll::Top, cx),
                Shortcut::Bottom => self.scroll_view(Scroll::Bottom, cx),
            }
            return;
        }
        let action = if ev.is_held {
            KeyAction::Repeat
        } else {
            KeyAction::Press
        };
        self.send_key(&keys::key_event(&ev.keystroke, action), cx);
    }

    fn on_key_up(&mut self, ev: &KeyUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if keys::shortcut(&ev.keystroke).is_none() {
            let event = keys::key_event(&ev.keystroke, KeyAction::Release);
            let bytes = self.screen.borrow_mut().term.encode_key(&event);
            if let Ok(b) = bytes {
                self.emit_input(b, cx);
            }
        }
    }

    fn send_key(&mut self, event: &KeyEvent, cx: &mut Context<Self>) {
        let bytes = {
            let mut s = self.screen.borrow_mut();
            let bytes = s.term.encode_key(event).unwrap_or_default();
            if !bytes.is_empty() && !s.term.scrollbar().is_ok_and(|b| b.at_bottom()) {
                s.term.scroll(Scroll::Bottom);
                s.dirty = true;
            }
            bytes
        };
        if !bytes.is_empty() {
            self.emit_input(bytes, cx);
            self.redraw(cx);
            self.activity(cx);
        }
    }

    fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.screen.borrow().selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, text: Option<String>, cx: &mut Context<Self>) {
        let Some(text) = text.filter(|t| !t.is_empty()) else {
            return;
        };
        let bytes = self.screen.borrow().term.encode_paste(&text);
        if let Ok(b) = bytes {
            self.emit_input(b, cx);
        }
    }

    /// True when mouse events go to the program: it asked for them, and Shift is not held.
    fn reports_mouse(&self, mods: &Modifiers) -> bool {
        !mods.shift && self.screen.borrow().term.mouse_tracking()
    }

    fn report_mouse(
        &mut self,
        action: MouseAction,
        button: Option<VtButton>,
        pos: Point<Pixels>,
        mods: &Modifiers,
        any_button_down: bool,
        cx: &mut Context<Self>,
    ) {
        let bytes = {
            let mut s = self.screen.borrow_mut();
            let event = VtMouse {
                action,
                button,
                mods: keys::mods(mods),
                x: f32::from(pos.x - s.bounds.origin.x),
                y: f32::from(pos.y - s.bounds.origin.y),
                any_button_down,
            };
            let geometry = s.geometry();
            s.term.encode_mouse(&event, &geometry).unwrap_or_default()
        };
        self.emit_input(bytes, cx);
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some(button) = vt_button(ev.button) else {
            return;
        };
        if self.reports_mouse(&ev.modifiers) {
            self.pressed = Some(button);
            let pos = ev.position;
            self.report_mouse(
                MouseAction::Press,
                Some(button),
                pos,
                &ev.modifiers,
                true,
                cx,
            );
            return;
        }
        match button {
            VtButton::Left => {
                {
                    let mut s = self.screen.borrow_mut();
                    let at = s.cell_at(ev.position);
                    let sel =
                        Selection::new(at, Unit::from_clicks(ev.click_count), s.row_cells(at.row));
                    s.selection = Some(sel);
                    s.selecting = true;
                    s.dirty = true;
                }
                self.pressed = Some(button);
                self.cells.update(cx, |_, cx| cx.notify());
            }
            VtButton::Middle => {
                let text = cx.read_from_primary().and_then(|i| i.text());
                self.paste(text, cx);
            }
            _ => {}
        }
    }

    /// Runs for each release in the window. Only the release of a press that the pane took counts.
    fn on_mouse_up(&mut self, ev: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(button) = vt_button(ev.button) else {
            return;
        };
        if self.pressed != Some(button) {
            return;
        }
        self.pressed = None;
        let selecting = std::mem::take(&mut self.screen.borrow_mut().selecting);
        if selecting && button == VtButton::Left {
            let text = {
                let mut s = self.screen.borrow_mut();
                if s.selection.as_ref().is_some_and(Selection::is_single_cell) {
                    s.selection = None;
                    s.dirty = true;
                }
                s.selected_text()
            };
            if let Some(t) = text {
                cx.write_to_primary(ClipboardItem::new_string(t));
            }
            self.cells.update(cx, |_, cx| cx.notify());
            return;
        }
        if self.reports_mouse(&ev.modifiers) {
            self.report_mouse(
                MouseAction::Release,
                Some(button),
                ev.position,
                &ev.modifiers,
                false,
                cx,
            );
        }
    }

    /// Runs for each move in the window. A move outside the pane counts only during a press that the
    /// pane took.
    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.pressed.is_none() && !self.screen.borrow().bounds.contains(&ev.position) {
            return;
        }
        if self.screen.borrow().selecting {
            if ev.pressed_button == Some(MouseButton::Left) {
                {
                    let mut s = self.screen.borrow_mut();
                    let to = s.cell_at(ev.position);
                    let row = s.row_cells(to.row).to_vec();
                    if let Some(sel) = s.selection.as_mut() {
                        sel.extend(to, &row);
                    }
                    s.dirty = true;
                }
                self.cells.update(cx, |_, cx| cx.notify());
            }
            return;
        }
        if self.reports_mouse(&ev.modifiers) {
            let button = ev.pressed_button.and_then(vt_button);
            self.report_mouse(
                MouseAction::Motion,
                button,
                ev.position,
                &ev.modifiers,
                button.is_some(),
                cx,
            );
        }
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let rows = {
            let mut s = self.screen.borrow_mut();
            let cell_h = f32::from(s.metrics.cell_h);
            let delta = match ev.delta {
                ScrollDelta::Lines(l) => l.y * WHEEL_ROWS,
                ScrollDelta::Pixels(p) => f32::from(p.y) / cell_h,
            };
            s.wheel_rest += delta;
            let rows = s.wheel_rest.trunc();
            s.wheel_rest -= rows;
            rows as isize
        };
        if rows == 0 {
            return;
        }
        // A positive delta moves toward the top: into the history.
        let up = rows > 0;
        let n = rows.unsigned_abs();
        if self.reports_mouse(&ev.modifiers) {
            let button = if up {
                VtButton::WheelUp
            } else {
                VtButton::WheelDown
            };
            for _ in 0..n {
                self.report_mouse(
                    MouseAction::Press,
                    Some(button),
                    ev.position,
                    &ev.modifiers,
                    false,
                    cx,
                );
            }
            return;
        }
        let alt_scroll = {
            let s = self.screen.borrow();
            s.term.alternate_screen() && s.term.mode(mode::ALT_SCROLL)
        };
        if alt_scroll {
            let key = KeyEvent {
                key: if up { Key::ArrowUp } else { Key::ArrowDown },
                ..Default::default()
            };
            for _ in 0..n {
                self.send_key(&key, cx);
            }
            return;
        }
        self.scroll_view(Scroll::Lines(-rows), cx);
    }
}

/// Releases and moves reach the pane from the whole window. A div gets them only while the
/// pointer is over it, and a program that took a press needs its release and its drag.
fn window_mouse_listeners(pane: WeakEntity<TerminalPane>) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |_, _, window, _| {
            let p = pane.clone();
            window.on_mouse_event(move |ev: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble {
                    let _ = p.update(cx, |p, cx| p.on_mouse_up(ev, window, cx));
                }
            });
            let p = pane.clone();
            window.on_mouse_event(move |ev: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Bubble {
                    let _ = p.update(cx, |p, cx| p.on_mouse_move(ev, window, cx));
                }
            });
        },
    )
    .size_0()
}

fn vt_button(b: MouseButton) -> Option<VtButton> {
    match b {
        MouseButton::Left => Some(VtButton::Left),
        MouseButton::Middle => Some(VtButton::Middle),
        MouseButton::Right => Some(VtButton::Right),
        MouseButton::Navigate(_) => None,
    }
}

impl Render for TerminalPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layer = || {
            StyleRefinement::default()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
        };
        div()
            .size_full()
            .relative()
            .bg(terminal::BACKGROUND.hsla())
            .track_focus(&self.focus)
            .key_context("Terminal")
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_scroll(cx.listener(Self::on_scroll))
            .child(window_mouse_listeners(cx.weak_entity()))
            .child(AnyView::from(self.cells.clone()).cached(layer()))
            .child(AnyView::from(self.cursor.clone()).cached(layer()))
            .children(self.bench.as_ref().map(Bench::marker))
    }
}

/// Draws the cells. It is cached: it draws again only after a `notify`.
struct CellsView {
    screen: Rc<RefCell<Screen>>,
    pane: WeakEntity<TerminalPane>,
    bench: Option<Bench>,
}

impl Render for CellsView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (frame, metrics) = {
            let mut s = self.screen.borrow_mut();
            s.refresh();
            (s.frame().clone(), s.metrics)
        };
        let blinking = frame.cursor.as_ref().is_some_and(|c| c.blinking);
        let screen = self.screen.clone();
        let pane = self.pane.clone();
        let bench = self.bench.clone();
        canvas(
            move |bounds, _, cx| {
                let (cols, rows, sized) = {
                    let mut s = screen.borrow_mut();
                    s.bounds = bounds;
                    (s.cols, s.rows, s.sized)
                };
                let fit = metrics.fit(bounds);
                if fit != (cols, rows) || !sized || blinking {
                    cx.defer(move |cx| {
                        let _ = pane.update(cx, |p, cx| {
                            p.resize(fit.0, fit.1, cx);
                            if blinking {
                                p.ensure_blink(cx);
                            }
                        });
                    });
                }
                screen
            },
            move |bounds, screen, window, cx| {
                paint_cells(&frame, &metrics, bounds, window, cx);
                let read = screen.borrow_mut().pending_read.take();
                if let (Some(b), Some(t)) = (&bench, read) {
                    b.record_latency(t.elapsed().as_secs_f64() * 1000.);
                }
            },
        )
        .size_full()
    }
}

fn origin(bounds: Bounds<Pixels>, m: &Metrics) -> Point<Pixels> {
    bounds.origin + point(m.pad, m.pad)
}

fn cell_bounds(o: Point<Pixels>, m: &Metrics, row: u16, col: u16, cols: u16) -> Bounds<Pixels> {
    Bounds {
        origin: point(
            o.x + m.cell_w * f32::from(col),
            o.y + m.cell_h * f32::from(row),
        ),
        size: gsize(m.cell_w * f32::from(cols), m.cell_h),
    }
}

fn paint_cells(f: &Frame, m: &Metrics, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    if f.cols == 0 {
        return;
    }
    window.paint_quad(fill(bounds, f.background));
    let o = origin(bounds, m);
    for (y, row) in f.rows.iter().enumerate() {
        for b in &row.backs {
            let at = cell_bounds(o, m, y as u16, b.col, b.cols);
            window.paint_quad(fill(at, b.color));
        }
    }
    for (y, row) in f.rows.iter().enumerate() {
        for s in &row.shapes {
            paint_shape(s.ch, s.color, cell_bounds(o, m, y as u16, s.col, 1), window);
        }
    }
    for (y, row) in f.rows.iter().enumerate() {
        let mut lines = row.lines.borrow_mut();
        let key = (m.font_size, m.cell_w);
        if lines.as_ref().is_none_or(|(k, _)| *k != key) {
            let shaped = row.runs.iter().map(|r| shape_run(r, m, window)).collect();
            *lines = Some((key, shaped));
        }
        let Some((_, shaped)) = lines.as_ref() else {
            continue;
        };
        for (r, line) in row.runs.iter().zip(shaped) {
            let at = cell_bounds(o, m, y as u16, r.col, 1).origin;
            let _ = line.paint(at, m.cell_h, TextAlign::Left, None, window, cx);
        }
    }
}

/// The line of one run, with a fixed advance of one cell, or two for a wide character.
fn shape_run(r: &Run, m: &Metrics, window: &Window) -> ShapedLine {
    let line = px(line_width(f32::from(m.cell_w)));
    let advance = if r.is_wide() { m.cell_w * 2. } else { m.cell_w };
    let run = TextRun {
        len: r.text.len(),
        font: mono(r.bold, r.italic),
        color: r.color,
        background_color: None,
        underline: (r.underline != agentsi_vt::Underline::None).then_some(UnderlineStyle {
            thickness: line,
            color: Some(r.color),
            wavy: r.underline == agentsi_vt::Underline::Curly,
        }),
        strikethrough: r.strike.then_some(StrikethroughStyle {
            thickness: line,
            color: Some(r.color),
        }),
    };
    window.text_system().shape_line(
        SharedString::from(r.text.clone()),
        m.font_size,
        &[run],
        Some(advance),
    )
}

/// One part of a shape cell, placed in a cell whose origin is at zero.
enum Part {
    Quad { bounds: Bounds<Pixels>, alpha: f32 },
    Path(Path<Pixels>),
}

/// A shape character and the bits of the cell width and height.
type ShapeKey = (char, u32, u32);

/// The most entries of the cache of shape parts. A full cache starts again.
const SHAPE_CACHE_MAX: usize = 4096;

thread_local! {
    /// The parts of each shape character at each cell size. A path costs a tessellation, so the
    /// pane makes it once and moves a copy to each cell.
    static SHAPE_PARTS: RefCell<HashMap<ShapeKey, Rc<[Part]>>> = RefCell::default();
}

fn shape_parts(ch: char, w: f32, h: f32) -> Rc<[Part]> {
    let key = (ch, w.to_bits(), h.to_bits());
    if let Some(parts) = SHAPE_PARTS.with_borrow(|c| c.get(&key).cloned()) {
        return parts;
    }
    let at = |(x, y): (f32, f32)| point(px(x), px(y));
    let parts: Rc<[Part]> = shapes(ch, w, h)
        .into_iter()
        .filter_map(|shape| match shape {
            Shape::Rect { x, y, w, h, alpha } => Some(Part::Quad {
                bounds: Bounds {
                    origin: at((x, y)),
                    size: gsize(px(w), px(h)),
                },
                alpha,
            }),
            Shape::Curve {
                from,
                ctrl,
                to,
                width,
            } => {
                let mut p = PathBuilder::stroke(px(width));
                p.move_to(at(from));
                p.curve_to(at(to), at(ctrl));
                p.build().ok().map(Part::Path)
            }
            Shape::Line { from, to, width } => {
                let mut p = PathBuilder::stroke(px(width));
                p.move_to(at(from));
                p.line_to(at(to));
                p.build().ok().map(Part::Path)
            }
        })
        .collect();
    SHAPE_PARTS.with_borrow_mut(|c| {
        if c.len() >= SHAPE_CACHE_MAX {
            c.clear();
        }
        c.insert(key, parts.clone());
    });
    parts
}

fn paint_shape(ch: char, color: Hsla, cell: Bounds<Pixels>, window: &mut Window) {
    let (w, h) = (f32::from(cell.size.width), f32::from(cell.size.height));
    for part in shape_parts(ch, w, h).iter() {
        match part {
            Part::Quad { bounds, alpha } => {
                let mut c = color;
                c.a *= alpha;
                let b = Bounds {
                    origin: cell.origin + bounds.origin,
                    size: bounds.size,
                };
                window.paint_quad(fill(b, c));
            }
            Part::Path(path) => {
                let mut path = path.clone();
                path.bounds.origin += cell.origin;
                for v in &mut path.vertices {
                    v.xy_position += cell.origin;
                }
                window.paint_path(path, color);
            }
        }
    }
}

/// Draws the cursor. A blink frame draws only this view.
struct CursorView {
    screen: Rc<RefCell<Screen>>,
    blink: Rc<Blink>,
}

impl Render for CursorView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (cursor, metrics) = {
            let s = self.screen.borrow();
            (s.frame().cursor.clone(), s.metrics)
        };
        let (on, focused) = (self.blink.on.get(), self.blink.focused.get());
        canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                if let Some(c) = &cursor {
                    paint_cursor(c, &metrics, bounds, on, focused, window, cx);
                }
            },
        )
        .size_full()
    }
}

fn paint_cursor(
    c: &CursorCell,
    m: &Metrics,
    bounds: Bounds<Pixels>,
    on: bool,
    focused: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let cell = cell_bounds(
        origin(bounds, m),
        m,
        c.row,
        c.col,
        if c.wide { 2 } else { 1 },
    );
    let bar = terminal::CURSOR_BAR;
    if !focused || c.shape == CursorShape::BlockHollow {
        window.paint_quad(
            outline(cell, c.color, gpui_kit::BorderStyle::Solid).border_widths(bar / 2.),
        );
        return;
    }
    if !on {
        return;
    }
    match c.shape {
        CursorShape::Bar => {
            let b = Bounds {
                origin: cell.origin,
                size: gsize(bar, cell.size.height),
            };
            window.paint_quad(fill(b, c.color));
        }
        CursorShape::Underline => {
            let b = Bounds {
                origin: point(cell.origin.x, cell.origin.y + cell.size.height - bar),
                size: gsize(cell.size.width, bar),
            };
            window.paint_quad(fill(b, c.color));
        }
        _ => {
            window.paint_quad(fill(cell, c.color));
            if !c.text.is_empty() && !crate::shapes::is_shape(c.text.chars().next().unwrap_or(' '))
            {
                let run = TextRun {
                    len: c.text.len(),
                    font: mono(c.bold, c.italic),
                    color: c.text_color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = window.text_system().shape_line(
                    SharedString::from(c.text.clone()),
                    m.font_size,
                    &[run],
                    Some(cell.size.width),
                );
                let _ = line.paint(cell.origin, m.cell_h, TextAlign::Left, None, window, cx);
            } else if let Some(ch) = c.text.chars().next() {
                paint_shape(ch, c.text_color, cell, window);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, VisualTestContext};

    /// A window that holds one pane in a box of a fixed size, and keeps the events of the pane.
    struct Host {
        pane: Entity<TerminalPane>,
        size: (Pixels, Pixels),
        events: Vec<PaneEvent>,
        _events: Subscription,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(self.size.0).h(self.size.1).child(self.pane.clone())
        }
    }

    /// A host whose box fits `cols` × `rows` cells, with half a cell to spare.
    fn host(
        cx: &mut TestAppContext,
        cols: u16,
        rows: u16,
    ) -> (Entity<Host>, &mut VisualTestContext) {
        cx.update(agentsi_ui::init);
        let (host, cx) = cx.add_window_view(|window, cx| {
            let pane = cx.new(|cx| TerminalPane::new(PaneOptions::default(), window, cx));
            let m = pane.read(cx).screen.borrow().metrics;
            let fit = |n: u16, cell: Pixels| m.pad * 2. + cell * (f32::from(n) + 0.5);
            let events = cx.subscribe(&pane, |h: &mut Host, _, e: &PaneEvent, _| {
                h.events.push(e.clone())
            });
            Host {
                pane,
                size: (fit(cols, m.cell_w), fit(rows, m.cell_h)),
                events: Vec::new(),
                _events: events,
            }
        });
        draw(cx);
        (host, cx)
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }

    fn resizes(host: &Entity<Host>, cx: &mut VisualTestContext) -> Vec<(u16, u16)> {
        host.read_with(cx, |h, _| {
            h.events
                .iter()
                .filter_map(|e| match e {
                    PaneEvent::Output(PaneOutput::Resize { cols, rows }) => Some((*cols, *rows)),
                    _ => None,
                })
                .collect()
        })
    }

    #[gpui::test]
    fn a_pane_reports_its_first_size_even_when_it_is_the_default(cx: &mut TestAppContext) {
        let (host, cx) = host(cx, 80, 24);
        assert_eq!(resizes(&host, cx), vec![(80, 24)]);
        // Another frame at the same size reports nothing more.
        draw(cx);
        assert_eq!(resizes(&host, cx), vec![(80, 24)]);
    }

    #[gpui::test]
    fn a_pane_reports_a_new_size_once(cx: &mut TestAppContext) {
        let (host, cx) = host(cx, 100, 30);
        assert_eq!(resizes(&host, cx), vec![(100, 30)]);
        let pane = host.read_with(cx, |h, _| h.pane.clone());
        assert_eq!(pane.read_with(cx, |p, _| p.grid_size()), (100, 30));
    }

    #[gpui::test]
    fn the_blink_starts_when_the_last_output_asks_for_a_blinking_cursor(cx: &mut TestAppContext) {
        let (host, cx) = host(cx, 40, 10);
        let pane = host.read_with(cx, |h, _| h.pane.clone());
        cx.update(|window, cx| {
            window.activate_window();
            let handle = pane.read(cx).focus.clone();
            window.focus(&handle, cx);
        });
        draw(cx);
        pane.update(cx, |p, cx| p.activity(cx));
        assert!(pane.read_with(cx, |p, _| p.focused));
        assert!(
            !pane.read_with(cx, |p, _| p.blink_running),
            "a steady cursor does not blink"
        );
        // The chunk that turns on a blinking cursor is the last output.
        pane.update(cx, |p, cx| p.feed(b"\x1b[1 q", Instant::now(), cx));
        draw(cx);
        assert!(pane.read_with(cx, |p, _| p.blink_running));
    }

    #[gpui::test]
    fn a_blink_keeps_the_frame_of_the_cells(cx: &mut TestAppContext) {
        let (host, cx) = host(cx, 40, 10);
        let pane = host.read_with(cx, |h, _| h.pane.clone());
        cx.update(|window, cx| {
            window.activate_window();
            let handle = pane.read(cx).focus.clone();
            window.focus(&handle, cx);
        });
        pane.update(cx, |p, cx| p.feed(b"text\x1b[1 q", Instant::now(), cx));
        draw(cx);
        let frame = |cx: &mut VisualTestContext| {
            pane.read_with(cx, |p, _| p.screen.borrow().frame().clone())
        };
        let before = frame(cx);
        for _ in 0..2 {
            pane.update(cx, |p, cx| p.blink_tick(cx));
            draw(cx);
        }
        assert!(Rc::ptr_eq(&before, &frame(cx)), "a blink built a frame");
        assert!(pane.read_with(cx, |p, _| p.blink_running));
    }

    fn inputs(host: &Entity<Host>, cx: &mut VisualTestContext) -> Vec<u8> {
        host.read_with(cx, |h, _| {
            h.events
                .iter()
                .filter_map(|e| match e {
                    PaneEvent::Output(PaneOutput::Input(b)) => Some(b.clone()),
                    _ => None,
                })
                .flatten()
                .collect()
        })
    }

    #[gpui::test]
    fn a_release_outside_the_pane_reaches_the_program(cx: &mut TestAppContext) {
        let (host, cx) = host(cx, 20, 5);
        let pane = host.read_with(cx, |h, _| h.pane.clone());
        pane.update(cx, |p, cx| {
            p.feed(b"\x1b[?1002h\x1b[?1006h", Instant::now(), cx)
        });
        draw(cx);
        let inside = gpui_kit::point(px(30.), px(30.));
        let outside = gpui_kit::point(px(900.), px(700.));
        cx.simulate_mouse_down(inside, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(outside, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
        let sent = String::from_utf8_lossy(&inputs(&host, cx)).into_owned();
        assert!(sent.contains("M"), "the press: {sent:?}");
        assert!(sent.contains("\x1b[<32;"), "the drag outside: {sent:?}");
        assert!(sent.ends_with('m'), "the release: {sent:?}");
    }

    #[gpui::test]
    fn a_selection_drag_that_ends_outside_the_pane_finishes(cx: &mut TestAppContext) {
        let (host, cx) = host(cx, 20, 5);
        let pane = host.read_with(cx, |h, _| h.pane.clone());
        pane.update(cx, |p, cx| p.feed(b"hello world", Instant::now(), cx));
        draw(cx);
        let inside = gpui_kit::point(px(12.), px(12.));
        let outside = gpui_kit::point(px(900.), px(700.));
        cx.simulate_mouse_down(inside, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
        assert!(!pane.read_with(cx, |p, _| p.screen.borrow().selecting));
    }

    #[gpui::test]
    fn an_old_sync_timer_does_not_draw_inside_a_later_update(cx: &mut TestAppContext) {
        let (host, cx) = host(cx, 20, 5);
        let pane = host.read_with(cx, |h, _| h.pane.clone());
        pane.update(cx, |p, cx| p.feed(b"\x1b[?2026h", Instant::now(), cx));
        cx.executor().advance_clock(SYNC_TIMEOUT * 9 / 10);
        // The first hold ends in one read, and a second hold starts in the next.
        pane.update(cx, |p, cx| p.feed(b"\x1b[?2026l", Instant::now(), cx));
        pane.update(cx, |p, cx| {
            p.feed(b"\x1b[?2026hpartial", Instant::now(), cx)
        });
        draw(cx);
        // The timer of the first hold fires here. The second hold is still on.
        cx.executor().advance_clock(SYNC_TIMEOUT / 5);
        cx.run_until_parked();
        assert!(
            pane.read_with(cx, |p, _| p.sync_wait),
            "the second hold still waits"
        );
        cx.executor().advance_clock(SYNC_TIMEOUT);
        cx.run_until_parked();
        assert!(
            !pane.read_with(cx, |p, _| p.sync_wait),
            "its own timer ends it"
        );
    }

    /// The first paint of a core screen of `cols` × `rows`: a reset and the replay of a shell.
    fn core_paint(cols: u16, rows: u16) -> Vec<u8> {
        let mut core = agentsi_vt::Terminal::new(agentsi_vt::Options {
            cols,
            rows,
            scrollback_bytes: 100_000,
        })
        .unwrap();
        core.write(b"$ echo hello\r\nhello\r\n$ ls\r\nfile1  file2\r\n$ ");
        let mut paint = b"\x1bc".to_vec();
        paint.extend(core.replay().unwrap());
        paint
    }

    fn visible(pane: &Entity<TerminalPane>, cx: &mut VisualTestContext) -> Vec<String> {
        pane.read_with(cx, |p, _| p.text_lines())
            .into_iter()
            .filter(|l| !l.is_empty())
            .collect()
    }

    const SHELL: [&str; 5] = ["$ echo hello", "hello", "$ ls", "file1  file2", "$"];

    #[gpui::test]
    fn a_paint_for_a_taller_screen_keeps_the_earlier_output_in_view(cx: &mut TestAppContext) {
        let (host, cx) = host(cx, 80, 24);
        let pane = host.read_with(cx, |h, _| h.pane.clone());
        pane.update(cx, |p, cx| {
            p.source_size(100, 30);
            p.feed(&core_paint(100, 30), Instant::now(), cx);
        });
        draw(cx);
        assert_eq!(visible(&pane, cx), SHELL);
        assert_eq!(pane.read_with(cx, |p, _| p.grid_size()), (80, 24));
        // The next feed is at the size of the pane again.
        pane.update(cx, |p, cx| p.feed(b"x", Instant::now(), cx));
        assert_eq!(visible(&pane, cx).last().unwrap(), "$ x");
    }

    /// A core screen of 100 × 30 with more than 64 KiB of scrollback and a cleared screen, its
    /// first paint, and the text that a terminal of 80 × 24 shows after the whole paint and a resize.
    fn long_paint() -> (Vec<u8>, Vec<String>) {
        let options = |cols, rows| agentsi_vt::Options {
            cols,
            rows,
            scrollback_bytes: 20_000_000,
        };
        let mut core = agentsi_vt::Terminal::new(options(100, 30)).unwrap();
        for i in 0..8000 {
            core.write(format!("line {i} of the earlier output\r\n").as_bytes());
        }
        // A clear leaves blank rows under the prompt.
        core.write(b"\x1b[H\x1b[2J$ echo hello\r\nhello\r\n$ ls\r\nfile1  file2\r\n$ ");
        let mut paint = b"\x1bc".to_vec();
        paint.extend(core.replay().unwrap());
        let mut whole = agentsi_vt::Terminal::new(options(100, 30)).unwrap();
        whole.write(&paint);
        whole.resize(80, 24, 9, 18).unwrap();
        (paint, whole.grid().unwrap().text_lines())
    }

    #[gpui::test]
    fn a_paint_in_many_feeds_keeps_the_earlier_output_in_view(cx: &mut TestAppContext) {
        let (host, cx) = host(cx, 80, 24);
        let pane = host.read_with(cx, |h, _| h.pane.clone());
        let (paint, expected) = long_paint();
        assert!(paint.len() > 3 * 64 * 1024, "{}", paint.len());
        pane.update(cx, |p, cx| {
            p.source_size(100, 30);
            // The core sends a paint in frames of 64 KiB, and the app feeds the frames that wait.
            for chunk in paint.chunks(64 * 1024 + 17) {
                p.feed(chunk, Instant::now(), cx);
            }
        });
        draw(cx);
        assert_eq!(pane.read_with(cx, |p, _| p.text_lines()), expected);
        let shown: Vec<&String> = expected.iter().filter(|l| !l.is_empty()).collect();
        assert_eq!(shown, SHELL);
        // The size of the pane comes back at its next RESIZE, and live bytes use it.
        pane.update(cx, |p, cx| {
            p.source_size(80, 24);
            p.feed(b"x", Instant::now(), cx);
        });
        assert_eq!(visible(&pane, cx).last().unwrap(), "$ x");
    }

    #[gpui::test]
    fn live_bytes_after_a_resize_of_another_pane_keep_the_screen_of_this_pane(
        cx: &mut TestAppContext,
    ) {
        let (host, cx) = host(cx, 100, 30);
        let pane = host.read_with(cx, |h, _| h.pane.clone());
        // A full-screen program draws near the corner of the alternate screen.
        pane.update(cx, |p, cx| {
            p.feed(b"\x1b[?1049h\x1b[28;90Hfar", Instant::now(), cx)
        });
        // Another pane of 80 x 24 resized the terminal. Live bytes follow.
        pane.update(cx, |p, cx| {
            p.source_size(80, 24);
            p.feed(b"\x1b[1;1Hlive", Instant::now(), cx);
        });
        assert_eq!(pane.read_with(cx, |p, _| p.grid_size()), (100, 30));
        let lines = pane.read_with(cx, |p, _| p.text_lines());
        assert_eq!(lines[0], "live");
        assert_eq!(lines[27].trim(), "far");
    }

    #[gpui::test]
    fn a_paint_before_the_first_layout_keeps_the_earlier_output(cx: &mut TestAppContext) {
        cx.update(agentsi_ui::init);
        let (pane, cx) =
            cx.add_window_view(|window, cx| TerminalPane::new(PaneOptions::default(), window, cx));
        pane.update(cx, |p, cx| {
            p.source_size(120, 40);
            p.feed(&core_paint(120, 40), Instant::now(), cx);
        });
        assert_eq!(visible(&pane, cx), SHELL);
    }

    #[gpui::test]
    fn output_does_not_keep_the_cursor_blinking(cx: &mut TestAppContext) {
        let (host, cx) = host(cx, 40, 10);
        let pane = host.read_with(cx, |h, _| h.pane.clone());
        let before = pane.read_with(cx, |p, _| p.last_activity);
        pane.update(cx, |p, cx| p.feed(b"output", Instant::now(), cx));
        assert_eq!(pane.read_with(cx, |p, _| p.last_activity), before);
    }
}
