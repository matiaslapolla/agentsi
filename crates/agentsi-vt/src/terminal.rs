//! The safe wrapper over libghostty-vt. All `unsafe` code of the crate that calls the library is in this file.
//!
//! Threading: a [`Terminal`] is `Send` and not `Sync`. One thread at a time may call it. The library keeps no
//! thread-local state, and every callback runs inside [`Terminal::write`] on the calling thread.

use crate::{
    error::{Error, Result},
    grid::{
        Cell, CellStyle, Colors, Cursor, CursorShape, Grid, Palette, Rgb, Scroll, Scrollbar,
        Underline, Width,
    },
    input::{MouseAction, MouseButton, MouseEvent, MouseGeometry},
    key::{KeyAction, KeyEvent},
    status::{AgentState, BlockedKind, StatusRecords, StatusReport},
    sys,
};
use std::{
    ffi::c_void,
    mem::MaybeUninit,
    ptr::{self, NonNull},
};

/// Something the terminal produced while it parsed bytes. Events keep the order of the stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// Bytes to write back to the PTY: the replies to the queries of the program (DA1, size reports,
    /// and the answer to `OSC 7501 ; ?`). Write them in this order.
    Reply(Vec<u8>),
    /// A valid program status report (OSC 7501).
    Status(StatusReport),
    /// The program (or a reset) removed the records at `id` and beneath. An empty id means all.
    StatusCleared(String),
    /// The store was full, so the record `id` went. Records beneath it stay.
    StatusEvicted(String),
    Title(String),
    Pwd(String),
    Bell,
}

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub cols: u16,
    pub rows: u16,
    /// The most memory that the scrollback may use.
    pub scrollback_bytes: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            cols: 80,
            rows: 24,
            scrollback_bytes: 2_000_000,
        }
    }
}

/// The state that callbacks write to. It lives in a `Box` so its address does not change.
struct Shared {
    events: Vec<Event>,
    records: StatusRecords,
    size: Option<sys::SizeReportSize>,
}

pub struct Terminal {
    raw: sys::Terminal,
    shared: NonNull<Shared>,
    render: sys::RenderState,
    row_iter: sys::RenderStateRowIterator,
    cell_iter: sys::RenderStateRowCells,
    encoder: sys::KeyEncoder,
    key_event: sys::KeyEvent,
    mouse_encoder: sys::MouseEncoder,
    mouse_event: sys::MouseEvent,
}

/// DEC private modes that the pane reads with [`Terminal::mode`].
pub mod mode {
    pub const FOCUS_EVENTS: u16 = 1004;
    pub const ALT_SCROLL: u16 = 1007;
    pub const BRACKETED_PASTE: u16 = 2004;
    pub const SYNC_OUTPUT: u16 = 2026;
}

// SAFETY: the handles are plain heap objects with no thread affinity, and `Shared` holds owned data.
// `Terminal` is not `Sync` (it holds raw pointers): the library is not safe to call from two threads at once.
unsafe impl Send for Terminal {}

fn check(code: sys::Result::Type) -> Result<()> {
    match code {
        sys::Result::SUCCESS => Ok(()),
        sys::Result::OUT_OF_MEMORY => Err(Error::OutOfMemory),
        sys::Result::INVALID_VALUE => Err(Error::InvalidValue),
        sys::Result::OUT_OF_SPACE => Err(Error::OutOfSpace { required: 0 }),
        other => Err(Error::Code(other)),
    }
}

fn lossy(s: sys::String) -> String {
    if s.ptr.is_null() || s.len == 0 {
        return String::new();
    }
    // SAFETY: the library gives a valid pointer and length for the life of the callback. The caller
    // copies the text out before the callback returns.
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(s.ptr, s.len) }).into_owned()
}

impl Terminal {
    pub fn new(options: Options) -> Result<Terminal> {
        let shared = NonNull::from(Box::leak(Box::new(Shared {
            events: Vec::new(),
            records: StatusRecords::default(),
            // Cell sizes in pixels are unknown until the first `resize`.
            size: Some(sys::SizeReportSize {
                rows: options.rows,
                columns: options.cols,
                cell_width: 0,
                cell_height: 0,
            }),
        })));
        // From here on `Drop` frees whatever is set. The null handles are skipped.
        let mut terminal = Terminal {
            raw: ptr::null_mut(),
            shared,
            render: ptr::null_mut(),
            row_iter: ptr::null_mut(),
            cell_iter: ptr::null_mut(),
            encoder: ptr::null_mut(),
            key_event: ptr::null_mut(),
            mouse_encoder: ptr::null_mut(),
            mouse_event: ptr::null_mut(),
        };
        // SAFETY: each `new` call gets a valid out-pointer to a field of `terminal` and the default
        // allocator (null).
        unsafe {
            check(sys::ghostty_terminal_new(
                ptr::null(),
                &mut terminal.raw,
                options.cols,
                options.rows,
            ))?;
            check(sys::ghostty_render_state_new(
                ptr::null(),
                &mut terminal.render,
            ))?;
            check(sys::ghostty_render_state_row_iterator_new(
                ptr::null(),
                &mut terminal.row_iter,
            ))?;
            check(sys::ghostty_render_state_row_cells_new(
                ptr::null(),
                &mut terminal.cell_iter,
            ))?;
            check(sys::ghostty_key_encoder_new(
                ptr::null(),
                &mut terminal.encoder,
            ))?;
            check(sys::ghostty_key_event_new(
                ptr::null(),
                &mut terminal.key_event,
            ))?;
            check(sys::ghostty_mouse_encoder_new(
                ptr::null(),
                &mut terminal.mouse_encoder,
            ))?;
            check(sys::ghostty_mouse_event_new(
                ptr::null(),
                &mut terminal.mouse_event,
            ))?;
        }
        terminal.install_options(options)?;
        Ok(terminal)
    }

    fn set(&self, option: sys::TerminalOption::Type, value: *const c_void) -> Result<()> {
        // SAFETY: `raw` is a live terminal. For the options set here, `value` is either the pointer
        // value itself (userdata and callbacks) or points to a value of the type the option expects.
        check(unsafe { sys::ghostty_terminal_set(self.raw, option, value) })
    }

    fn install_options(&self, options: Options) -> Result<()> {
        use sys::TerminalOption as Opt;
        self.set(Opt::USERDATA, self.shared.as_ptr().cast_const().cast())?;
        // The callbacks are passed as plain function pointers.
        self.set(Opt::WRITE_PTY, cb_write_pty as *const c_void)?;
        self.set(Opt::BELL, cb_bell as *const c_void)?;
        self.set(Opt::TITLE_CHANGED, cb_title_changed as *const c_void)?;
        self.set(Opt::PWD_CHANGED, cb_pwd_changed as *const c_void)?;
        self.set(
            Opt::DEVICE_ATTRIBUTES,
            cb_device_attributes as *const c_void,
        )?;
        self.set(Opt::SIZE, cb_size as *const c_void)?;
        // With this option set, the library parses OSC 7501, answers `OSC 7501 ; ?` itself
        // and calls back for each valid report.
        self.set(Opt::PROGRAM_STATUS, cb_program_status as *const c_void)?;
        let scrollback = options.scrollback_bytes;
        self.set(Opt::SCROLLBACK_MAX_BYTES, ptr::from_ref(&scrollback).cast())?;
        // `replay` needs the unfinished sequence at the end of the input.
        let continuation = CONTINUATION_MAX_BYTES;
        self.set(
            Opt::CONTINUATION_MAX_BYTES,
            ptr::from_ref(&continuation).cast(),
        )?;
        // Mode 2027 (grapheme clustering) is on by default in Ghostty.
        let grapheme = sys::TerminalModeConfig {
            mode: 2027,
            value: true,
        };
        self.set(Opt::MODE_DEFAULT, ptr::from_ref(&grapheme).cast())
    }

    /// Feeds bytes from the PTY. Replies and changes queue as [`Event`]s. This never fails.
    pub fn write(&mut self, bytes: &[u8]) {
        // SAFETY: `raw` is live, and the pointer and length describe `bytes`. Callbacks run inside this
        // call and touch only `Shared`, which no reference to Rust code holds at this moment.
        unsafe { sys::ghostty_terminal_vt_write(self.raw, bytes.as_ptr(), bytes.len()) }
    }

    /// Takes the queued events in the order that the stream made them.
    pub fn take_events(&mut self) -> Vec<Event> {
        // SAFETY: `shared` is the leaked Box of this terminal. No callback runs while `&mut self` is held.
        std::mem::take(&mut unsafe { self.shared.as_mut() }.events)
    }

    /// The records of the program status protocol, as the reports left them.
    pub fn status(&self) -> &StatusRecords {
        // SAFETY: as in `take_events`. A shared borrow of `self` rules out a running `write`.
        &unsafe { self.shared.as_ref() }.records
    }

    /// The state of the program itself (the record with the empty id).
    pub fn agent_state(&self) -> Option<&AgentState> {
        self.status().root().map(|r| &r.state)
    }

    /// Resizes the grid. The pixel size of a cell answers the size reports of the program.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_w_px: u32, cell_h_px: u32) -> Result<()> {
        // SAFETY: `raw` is a live terminal.
        check(unsafe { sys::ghostty_terminal_resize(self.raw, cols, rows, cell_w_px, cell_h_px) })?;
        // SAFETY: `shared` as in `take_events`.
        unsafe { self.shared.as_mut() }.size = Some(sys::SizeReportSize {
            rows,
            columns: cols,
            cell_width: cell_w_px,
            cell_height: cell_h_px,
        });
        Ok(())
    }

    pub fn cols(&self) -> Result<u16> {
        self.get_u16(sys::TerminalData::COLS)
    }

    pub fn rows(&self) -> Result<u16> {
        self.get_u16(sys::TerminalData::ROWS)
    }

    fn get_u16(&self, data: sys::TerminalData::Type) -> Result<u16> {
        let mut value = 0u16;
        // SAFETY: `COLS` and `ROWS` write a `u16` to the out-pointer.
        check(unsafe {
            sys::ghostty_terminal_get(self.raw, data, ptr::from_mut(&mut value).cast())
        })?;
        Ok(value)
    }

    /// Bytes that bring a new terminal of the same size to the state of this one: the screen and its
    /// scrollback, the modes, the styles, the cursor, and the sequence that the input left unfinished.
    /// The palette is not in it. A terminal that is not new needs a reset (`ESC c`) first.
    pub fn replay(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let writer = sys::Writer {
            write: Some(cb_append),
            userdata: ptr::from_mut(&mut out).cast(),
        };
        let options = sys::FormatterTerminalOptions {
            size: std::mem::size_of::<sys::FormatterTerminalOptions>(),
            emit: sys::FormatterFormat::VT,
            unwrap: false,
            trim: false,
            extra: sys::FormatterTerminalExtra {
                size: std::mem::size_of::<sys::FormatterTerminalExtra>(),
                // A pane keeps its own theme. A palette from the core would replace it.
                palette: false,
                modes: true,
                scrolling_region: true,
                tabstops: true,
                pwd: true,
                keyboard: true,
                screen: sys::FormatterScreenExtra {
                    size: std::mem::size_of::<sys::FormatterScreenExtra>(),
                    cursor: true,
                    style: true,
                    hyperlink: true,
                    protection: true,
                    kitty_keyboard: true,
                    charsets: true,
                },
            },
            selection: ptr::null(),
        };
        let mut formatter: sys::Formatter = ptr::null_mut();
        // SAFETY: `raw` is live and outlives the formatter, which is freed before this function returns.
        // `writer.userdata` points to `out`, which outlives both calls, and `cb_append` only appends to it.
        // No other call touches the terminal meanwhile, because `&mut self` is held.
        unsafe {
            check(sys::ghostty_formatter_terminal_new(
                ptr::null(),
                &mut formatter,
                self.raw,
                options,
            ))?;
            let formatted = check(sys::ghostty_formatter_format(formatter, writer));
            sys::ghostty_formatter_free(formatter);
            formatted?;
            // An unfinished sequence longer than the limit is not available. The replay then lacks it.
            let _ = sys::ghostty_terminal_continuation_write(self.raw, writer);
        }
        Ok(out)
    }

    /// The value of a DEC private mode, such as 1004 (focus reports). An unknown mode is off.
    pub fn mode(&self, mode: u16) -> bool {
        let mut config = sys::TerminalModeConfig {
            mode: mode & 0x7fff,
            value: false,
        };
        // SAFETY: `MODE` reads `mode` and writes `value` of a `TerminalModeConfig`.
        let code = unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::TerminalData::MODE,
                ptr::from_mut(&mut config).cast(),
            )
        };
        code == sys::Result::SUCCESS && config.value
    }

    /// The window title that the program set.
    pub fn title(&self) -> String {
        self.get_string(sys::TerminalData::TITLE)
    }

    fn get_string(&self, data: sys::TerminalData::Type) -> String {
        // SAFETY: `raw` is live. See `terminal_string`.
        unsafe { terminal_string(self.raw, data) }
    }

    /// Encodes a key event with the modes that the program set (cursor keys, Kitty flags).
    /// The result is empty for a key that makes no bytes.
    pub fn encode_key(&mut self, event: &KeyEvent) -> Result<Vec<u8>> {
        let text = event.text.as_deref().unwrap_or("");
        // SAFETY: `key_event` and `encoder` are live handles. `text` is borrowed from `event`, outlives
        // the encode call, and is cleared from the handle before this function returns, so the library
        // never holds a dangling pointer. The library only reads the buffer.
        unsafe {
            sys::ghostty_key_event_set_action(
                self.key_event,
                match event.action {
                    KeyAction::Release => sys::KeyAction::RELEASE,
                    KeyAction::Press => sys::KeyAction::PRESS,
                    KeyAction::Repeat => sys::KeyAction::REPEAT,
                },
            );
            sys::ghostty_key_event_set_key(self.key_event, event.key.raw());
            sys::ghostty_key_event_set_mods(self.key_event, event.mods.0);
            sys::ghostty_key_event_set_consumed_mods(self.key_event, event.consumed_mods.0);
            sys::ghostty_key_event_set_composing(self.key_event, event.composing);
            sys::ghostty_key_event_set_unshifted_codepoint(
                self.key_event,
                event.unshifted.map_or(0, u32::from),
            );
            if text.is_empty() {
                sys::ghostty_key_event_set_utf8(self.key_event, ptr::null(), 0);
            } else {
                sys::ghostty_key_event_set_utf8(self.key_event, text.as_ptr().cast(), text.len());
            }
            sys::ghostty_key_encoder_setopt_from_terminal(self.encoder, self.raw);
        }
        let mut out = vec![0u8; 64];
        let mut required = 0usize;
        for _ in 0..2 {
            // SAFETY: the buffer pointer and size describe `out`, and `required` is a valid out-pointer.
            let code = unsafe {
                sys::ghostty_key_encoder_encode(
                    self.encoder,
                    self.key_event,
                    out.as_mut_ptr().cast(),
                    out.len(),
                    &mut required,
                )
            };
            match code {
                sys::Result::SUCCESS => {
                    out.truncate(required);
                    self.clear_key_text();
                    return Ok(out);
                }
                sys::Result::OUT_OF_SPACE => out.resize(required, 0),
                other => {
                    self.clear_key_text();
                    return check(other).map(|()| Vec::new());
                }
            }
        }
        self.clear_key_text();
        Err(Error::OutOfSpace { required })
    }

    fn clear_key_text(&mut self) {
        // SAFETY: `key_event` is a live handle. A null pointer with length 0 clears the text.
        unsafe { sys::ghostty_key_event_set_utf8(self.key_event, ptr::null(), 0) }
    }

    /// Sets the default colors. Colors 16 to 255 stay the standard ones.
    pub fn set_palette(&mut self, palette: &Palette) -> Result<()> {
        use sys::TerminalOption as Opt;
        let raw = |c: Rgb| sys::ColorRgb {
            r: c.r,
            g: c.g,
            b: c.b,
        };
        let mut table = [sys::ColorRgb::default(); 256];
        // SAFETY: the function fills a table of 256 colors.
        unsafe { sys::ghostty_color_palette_default(table.as_mut_ptr()) };
        for (slot, c) in table.iter_mut().zip(palette.ansi) {
            *slot = raw(c);
        }
        let (fg, bg) = (raw(palette.foreground), raw(palette.background));
        self.set(Opt::COLOR_PALETTE, table.as_ptr().cast())?;
        self.set(Opt::COLOR_FOREGROUND, ptr::from_ref(&fg).cast())?;
        self.set(Opt::COLOR_BACKGROUND, ptr::from_ref(&bg).cast())?;
        match palette.cursor.map(raw) {
            Some(c) => self.set(Opt::COLOR_CURSOR, ptr::from_ref(&c).cast()),
            None => self.set(Opt::COLOR_CURSOR, ptr::null()),
        }
    }

    /// Moves the viewport over the scrollback.
    pub fn scroll(&mut self, scroll: Scroll) {
        let mut behavior = sys::TerminalScrollViewport::default();
        match scroll {
            Scroll::Top => behavior.tag = sys::TerminalScrollViewportTag::TOP,
            Scroll::Bottom => behavior.tag = sys::TerminalScrollViewportTag::BOTTOM,
            Scroll::Lines(delta) => {
                behavior.tag = sys::TerminalScrollViewportTag::DELTA;
                behavior.value.delta = delta;
            }
        }
        // SAFETY: `raw` is live, and `behavior` is a plain value with a valid tag.
        unsafe { sys::ghostty_terminal_scroll_viewport(self.raw, behavior) }
    }

    pub fn scrollbar(&self) -> Result<Scrollbar> {
        let mut bar = sys::TerminalScrollbar::default();
        // SAFETY: `SCROLLBAR` writes a `TerminalScrollbar` to the out-pointer.
        check(unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::TerminalData::SCROLLBAR,
                ptr::from_mut(&mut bar).cast(),
            )
        })?;
        Ok(Scrollbar {
            total: bar.total,
            offset: bar.offset,
            len: bar.len,
        })
    }

    /// True when the program asked for mouse reports.
    pub fn mouse_tracking(&self) -> bool {
        let mut on = false;
        // SAFETY: `MOUSE_TRACKING` writes a `bool`.
        let code = unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::TerminalData::MOUSE_TRACKING,
                ptr::from_mut(&mut on).cast(),
            )
        };
        code == sys::Result::SUCCESS && on
    }

    /// True when the alternate screen is active (full-screen programs such as `btop`).
    pub fn alternate_screen(&self) -> bool {
        let mut screen = sys::TerminalScreen::PRIMARY;
        // SAFETY: `ACTIVE_SCREEN` writes a `TerminalScreen`.
        let code = unsafe {
            sys::ghostty_terminal_get(
                self.raw,
                sys::TerminalData::ACTIVE_SCREEN,
                ptr::from_mut(&mut screen).cast(),
            )
        };
        code == sys::Result::SUCCESS && screen == sys::TerminalScreen::ALTERNATE
    }

    /// Encodes a mouse event with the tracking mode and format that the program set.
    /// The result is empty when the program asked for no report of this event.
    pub fn encode_mouse(
        &mut self,
        event: &MouseEvent,
        geometry: &MouseGeometry,
    ) -> Result<Vec<u8>> {
        let size = sys::MouseEncoderSize {
            size: std::mem::size_of::<sys::MouseEncoderSize>(),
            screen_width: geometry.width_px,
            screen_height: geometry.height_px,
            cell_width: geometry.cell_width_px.max(1),
            cell_height: geometry.cell_height_px.max(1),
            padding_top: geometry.padding_px,
            padding_bottom: geometry.padding_px,
            padding_right: geometry.padding_px,
            padding_left: geometry.padding_px,
        };
        let any = event.any_button_down;
        // SAFETY: `mouse_encoder` and `mouse_event` are live handles. Each option gets a pointer to a
        // value of the type it documents, which outlives the call.
        unsafe {
            sys::ghostty_mouse_encoder_setopt_from_terminal(self.mouse_encoder, self.raw);
            sys::ghostty_mouse_encoder_setopt(
                self.mouse_encoder,
                sys::MouseEncoderOption::SIZE,
                ptr::from_ref(&size).cast(),
            );
            sys::ghostty_mouse_encoder_setopt(
                self.mouse_encoder,
                sys::MouseEncoderOption::ANY_BUTTON_PRESSED,
                ptr::from_ref(&any).cast(),
            );
            sys::ghostty_mouse_event_set_action(
                self.mouse_event,
                match event.action {
                    MouseAction::Press => sys::MouseAction::PRESS,
                    MouseAction::Release => sys::MouseAction::RELEASE,
                    MouseAction::Motion => sys::MouseAction::MOTION,
                },
            );
            match event.button {
                Some(b) => sys::ghostty_mouse_event_set_button(
                    self.mouse_event,
                    match b {
                        MouseButton::Left => sys::MouseButton::LEFT,
                        MouseButton::Right => sys::MouseButton::RIGHT,
                        MouseButton::Middle => sys::MouseButton::MIDDLE,
                        MouseButton::WheelUp => sys::MouseButton::FOUR,
                        MouseButton::WheelDown => sys::MouseButton::FIVE,
                        MouseButton::WheelLeft => sys::MouseButton::SIX,
                        MouseButton::WheelRight => sys::MouseButton::SEVEN,
                    },
                ),
                None => sys::ghostty_mouse_event_clear_button(self.mouse_event),
            }
            sys::ghostty_mouse_event_set_mods(self.mouse_event, event.mods.0);
            sys::ghostty_mouse_event_set_position(
                self.mouse_event,
                sys::MousePosition {
                    x: event.x,
                    y: event.y,
                },
            );
        }
        let (encoder, mouse_event) = (self.mouse_encoder, self.mouse_event);
        encode_into(|buf, len, out| {
            // SAFETY: the handles are live and `buf` has room for `len` bytes.
            unsafe { sys::ghostty_mouse_encoder_encode(encoder, mouse_event, buf, len, out) }
        })
    }

    /// Encodes pasted text: control bytes become spaces, and the text goes inside bracketed paste
    /// marks when the program set mode 2004. Without the marks, line ends become carriage returns.
    pub fn encode_paste(&self, text: &str) -> Result<Vec<u8>> {
        let bracketed = self.mode(mode::BRACKETED_PASTE);
        let mut data = text.as_bytes().to_vec();
        encode_into(|buf, len, out| {
            // SAFETY: the library edits `data` in place within its length, and writes at most `len`
            // bytes to `buf`. A second call sees the already cleaned text, which gives the same result.
            unsafe {
                sys::ghostty_paste_encode(
                    data.as_mut_ptr().cast(),
                    data.len(),
                    bracketed,
                    buf,
                    len,
                    out,
                )
            }
        })
    }

    /// The focus report (`CSI I` or `CSI O`) when the program set mode 1004, else `None`.
    pub fn encode_focus(&self, focused: bool) -> Option<Vec<u8>> {
        if !self.mode(mode::FOCUS_EVENTS) {
            return None;
        }
        let event = if focused {
            sys::FocusEvent::GAINED
        } else {
            sys::FocusEvent::LOST
        };
        encode_into(|buf, len, out| {
            // SAFETY: the library writes at most `len` bytes to `buf`.
            unsafe { sys::ghostty_focus_encode(event, buf, len, out) }
        })
        .ok()
    }

    /// Copies the visible rows, the default colors and the cursor. It leaves the damage for the next
    /// [`Terminal::update_grid`].
    pub fn grid(&mut self) -> Result<Grid> {
        let mut grid = Grid::default();
        self.read_grid(&mut grid, false)?;
        Ok(grid)
    }

    /// Brings `grid` up to the visible screen. It reads again only the rows that changed since the
    /// last call, and every row of a grid of another size. The first call reads every row.
    pub fn update_grid(&mut self, grid: &mut Grid) -> Result<Damage> {
        self.read_grid(grid, true)
    }

    fn read_grid(&mut self, grid: &mut Grid, incremental: bool) -> Result<Damage> {
        // SAFETY: all handles are live. Every `get` call below writes the type that its data tag documents
        // to an out-pointer of that type. The iterator handles are filled in by the library and used only
        // until the next `update`. Strings from the library are copied before the next call.
        unsafe {
            check(sys::ghostty_render_state_update(self.render, self.raw))?;
            let mut cols = 0u16;
            check(sys::ghostty_render_state_get(
                self.render,
                sys::RenderStateData::COLS,
                ptr::from_mut(&mut cols).cast(),
            ))?;
            let mut rows = 0u16;
            check(sys::ghostty_render_state_get(
                self.render,
                sys::RenderStateData::ROWS,
                ptr::from_mut(&mut rows).cast(),
            ))?;
            let mut dirty: sys::RenderStateDirty::Type = sys::RenderStateDirty::FULL;
            check(sys::ghostty_render_state_get(
                self.render,
                sys::RenderStateData::DIRTY,
                ptr::from_mut(&mut dirty).cast(),
            ))?;
            let mut raw_colors = sized_colors();
            check(sys::ghostty_render_state_get(
                self.render,
                sys::RenderStateData::COLORS,
                ptr::from_mut(&mut raw_colors).cast(),
            ))?;
            let colors = Colors {
                background: rgb(raw_colors.background),
                foreground: rgb(raw_colors.foreground),
                cursor: raw_colors.cursor_has_value.then(|| rgb(raw_colors.cursor)),
            };
            let mut cursor = sized_cursor();
            check(sys::ghostty_render_state_get(
                self.render,
                sys::RenderStateData::CURSOR,
                ptr::from_mut(&mut cursor).cast(),
            ))?;
            // A change of the default colors (OSC 10 and 11) leaves the rows clean, but a cell of
            // the default colors stores no color of its own.
            let all = !incremental
                || dirty == sys::RenderStateDirty::FULL
                || grid.cols != cols
                || grid.rows.len() != usize::from(rows)
                || grid.colors != colors;
            let mut damage = Damage {
                all,
                rows: Vec::new(),
                ids: Vec::with_capacity(usize::from(rows)),
            };
            grid.cols = cols;
            grid.colors = colors;
            grid.rows.resize_with(usize::from(rows), Vec::new);
            check(sys::ghostty_render_state_get(
                self.render,
                sys::RenderStateData::ROW_ITERATOR,
                ptr::from_mut(&mut self.row_iter).cast(),
            ))?;
            let mut y = 0u16;
            while sys::ghostty_render_state_row_iterator_next(self.row_iter) && y < rows {
                let mut id = sys::RenderStateRowId::default();
                check(sys::ghostty_render_state_row_get(
                    self.row_iter,
                    sys::RenderStateRowData::ID,
                    ptr::from_mut(&mut id).cast(),
                ))?;
                damage.ids.push(RowId(id.bits));
                let mut row_dirty = true;
                if !all {
                    check(sys::ghostty_render_state_row_get(
                        self.row_iter,
                        sys::RenderStateRowData::DIRTY,
                        ptr::from_mut(&mut row_dirty).cast(),
                    ))?;
                }
                if all || row_dirty {
                    self.read_row(&mut grid.rows[usize::from(y)], cols)?;
                    damage.rows.push(y);
                }
                y += 1;
            }
            grid.rows.truncate(usize::from(y));
            if incremental {
                // The next `update` reports only what changed after this copy.
                check(sys::ghostty_render_state_clean(self.render))?;
            }
            grid.cursor = cursor.viewport_has_value.then_some(Cursor {
                x: cursor.viewport_x,
                y: cursor.viewport_y,
                on_wide_tail: cursor.wide_tail,
                visible: cursor.visible,
                blinking: cursor.blinking,
                shape: match cursor.visual_style {
                    sys::RenderStateCursorVisualStyle::BAR => CursorShape::Bar,
                    sys::RenderStateCursorVisualStyle::UNDERLINE => CursorShape::Underline,
                    sys::RenderStateCursorVisualStyle::BLOCK_HOLLOW => CursorShape::BlockHollow,
                    _ => CursorShape::Block,
                },
            });
            Ok(damage)
        }
    }

    /// Reads the cells of the row under the row iterator into `row`.
    ///
    /// # Safety
    /// `row_iter` is a live row iterator that `row_iterator_next` just advanced.
    unsafe fn read_row(&mut self, row: &mut Vec<Cell>, cols: u16) -> Result<()> {
        row.clear();
        row.reserve(usize::from(cols));
        // SAFETY: see the function contract. `CELLS_RAW` writes a view of `len` raw cells that stays
        // valid until the next `update`. `CELLS` fills the live row-cells handle.
        unsafe {
            let mut view = sys::CellsView::default();
            check(sys::ghostty_render_state_row_get(
                self.row_iter,
                sys::RenderStateRowData::CELLS_RAW,
                ptr::from_mut(&mut view).cast(),
            ))?;
            check(sys::ghostty_render_state_row_get(
                self.row_iter,
                sys::RenderStateRowData::CELLS,
                ptr::from_mut(&mut self.cell_iter).cast(),
            ))?;
            let raw = if view.ptr.is_null() {
                &[][..]
            } else {
                std::slice::from_raw_parts(view.ptr, view.len)
            };
            for (x, &cell) in raw.iter().enumerate() {
                if is_blank(cell)? {
                    row.push(Cell::default());
                    continue;
                }
                let x = u16::try_from(x).map_err(|_| Error::InvalidValue)?;
                check(sys::ghostty_render_state_row_cells_select(
                    self.cell_iter,
                    x,
                ))?;
                row.push(read_cell(self.cell_iter)?);
            }
        }
        Ok(())
    }
}

/// The identity of a row across updates. A row keeps its id when the screen or the viewport
/// moves it. A row that a program changes in place may get a new id. An id tells a renderer where
/// to look for the old copy of a row. The renderer must still compare the cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RowId([u64; 2]);

/// What [`Terminal::update_grid`] read again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    /// The update read every row. The first update, a scroll of the screen or the viewport, a
    /// resize, new colors and the other screen do this.
    pub all: bool,
    /// The rows that the update read, top to bottom.
    pub rows: Vec<u16>,
    /// The id of each row of the grid.
    pub ids: Vec<RowId>,
}

/// True for a cell with no text, no style, no color and one column: the cell of [`Cell::default`].
///
/// # Safety
/// `cell` is a raw cell value from the library.
unsafe fn is_blank(cell: sys::Cell) -> Result<bool> {
    let mut tag: sys::CellContentTag::Type = sys::CellContentTag::CODEPOINT;
    let mut has_text = true;
    let mut has_styling = true;
    let mut wide: sys::CellWide::Type = sys::CellWide::NARROW;
    let keys = [
        sys::CellData::CONTENT_TAG,
        sys::CellData::HAS_TEXT,
        sys::CellData::HAS_STYLING,
        sys::CellData::WIDE,
    ];
    let mut values: [*mut c_void; 4] = [
        ptr::from_mut(&mut tag).cast(),
        ptr::from_mut(&mut has_text).cast(),
        ptr::from_mut(&mut has_styling).cast(),
        ptr::from_mut(&mut wide).cast(),
    ];
    let mut written = 0usize;
    // SAFETY: each key writes the type of its out-pointer: a tag enum, two bools and a wide enum.
    check(unsafe {
        sys::ghostty_cell_get_multi(
            cell,
            keys.len(),
            keys.as_ptr(),
            values.as_mut_ptr(),
            &mut written,
        )
    })?;
    Ok(tag == sys::CellContentTag::CODEPOINT
        && !has_text
        && !has_styling
        && wide == sys::CellWide::NARROW)
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: each handle was made by the matching `new` call and is freed once. The terminal is freed
        // last of the handles, and `shared` after it, because the terminal calls back into `shared` until it is freed. `shared` is the leaked Box from `new`.
        unsafe {
            if !self.mouse_event.is_null() {
                sys::ghostty_mouse_event_free(self.mouse_event);
            }
            if !self.mouse_encoder.is_null() {
                sys::ghostty_mouse_encoder_free(self.mouse_encoder);
            }
            if !self.key_event.is_null() {
                sys::ghostty_key_event_free(self.key_event);
            }
            if !self.encoder.is_null() {
                sys::ghostty_key_encoder_free(self.encoder);
            }
            if !self.cell_iter.is_null() {
                sys::ghostty_render_state_row_cells_free(self.cell_iter);
            }
            if !self.row_iter.is_null() {
                sys::ghostty_render_state_row_iterator_free(self.row_iter);
            }
            if !self.render.is_null() {
                sys::ghostty_render_state_free(self.render);
            }
            if !self.raw.is_null() {
                sys::ghostty_terminal_free(self.raw);
            }
            drop(Box::from_raw(self.shared.as_ptr()));
        }
    }
}

fn rgb(c: sys::ColorRgb) -> Rgb {
    Rgb {
        r: c.r,
        g: c.g,
        b: c.b,
    }
}

/// Calls an encoder of the library with a buffer, and once more with a larger one when it says so.
fn encode_into(
    mut encode: impl FnMut(*mut std::ffi::c_char, usize, *mut usize) -> sys::Result::Type,
) -> Result<Vec<u8>> {
    let mut out = vec![0u8; 128];
    for _ in 0..2 {
        let mut written = 0usize;
        match encode(out.as_mut_ptr().cast(), out.len(), &mut written) {
            sys::Result::SUCCESS => {
                out.truncate(written);
                return Ok(out);
            }
            sys::Result::OUT_OF_SPACE if written > out.len() => out.resize(written, 0),
            other => return check(other).map(|()| Vec::new()),
        }
    }
    Err(Error::OutOfSpace {
        required: out.len(),
    })
}

fn sized_colors() -> sys::RenderStateColors {
    sys::RenderStateColors {
        size: std::mem::size_of::<sys::RenderStateColors>(),
        ..Default::default()
    }
}

fn sized_cursor() -> sys::RenderStateCursor {
    sys::RenderStateCursor {
        size: std::mem::size_of::<sys::RenderStateCursor>(),
        ..Default::default()
    }
}

/// Reads the cell under the iterator.
///
/// # Safety
/// `cells` is a live row-cells handle that `row_cells_select` just moved to a cell.
unsafe fn read_cell(cells: sys::RenderStateRowCells) -> Result<Cell> {
    // SAFETY: see the function contract. Each tag writes the type of its out-pointer.
    unsafe {
        let mut style = sys::Style {
            size: std::mem::size_of::<sys::Style>(),
            ..Default::default()
        };
        check(sys::ghostty_render_state_row_cells_get(
            cells,
            sys::RenderStateRowCellsData::STYLE,
            ptr::from_mut(&mut style).cast(),
        ))?;
        let mut raw = 0u64;
        check(sys::ghostty_render_state_row_cells_get(
            cells,
            sys::RenderStateRowCellsData::RAW,
            ptr::from_mut(&mut raw).cast(),
        ))?;
        let mut wide = sys::CellWide::NARROW;
        check(sys::ghostty_cell_get(
            raw,
            sys::CellData::WIDE,
            ptr::from_mut(&mut wide).cast(),
        ))?;
        Ok(Cell {
            text: cell_text(cells)?,
            width: match wide {
                sys::CellWide::WIDE => Width::Wide,
                sys::CellWide::SPACER_TAIL => Width::SpacerTail,
                sys::CellWide::SPACER_HEAD => Width::SpacerHead,
                _ => Width::Narrow,
            },
            fg: cell_color(cells, sys::RenderStateRowCellsData::FG_COLOR),
            bg: cell_color(cells, sys::RenderStateRowCellsData::BG_COLOR),
            style: CellStyle {
                bold: style.bold,
                italic: style.italic,
                faint: style.faint,
                blink: style.blink,
                inverse: style.inverse,
                invisible: style.invisible,
                strikethrough: style.strikethrough,
                overline: style.overline,
                underline: match style.underline {
                    sys::SgrUnderline::SINGLE => Underline::Single,
                    sys::SgrUnderline::DOUBLE => Underline::Double,
                    sys::SgrUnderline::CURLY => Underline::Curly,
                    sys::SgrUnderline::DOTTED => Underline::Dotted,
                    sys::SgrUnderline::DASHED => Underline::Dashed,
                    _ => Underline::None,
                },
            },
        })
    }
}

/// # Safety
/// As `read_cell`.
unsafe fn cell_color(
    cells: sys::RenderStateRowCells,
    data: sys::RenderStateRowCellsData::Type,
) -> Option<Rgb> {
    let mut color = sys::ColorRgb::default();
    // SAFETY: `FG_COLOR` and `BG_COLOR` write a `ColorRgb`. Any non-success code means "no explicit color".
    let code = unsafe {
        sys::ghostty_render_state_row_cells_get(cells, data, ptr::from_mut(&mut color).cast())
    };
    (code == sys::Result::SUCCESS).then(|| rgb(color))
}

/// # Safety
/// As `read_cell`.
unsafe fn cell_text(cells: sys::RenderStateRowCells) -> Result<String> {
    let mut scratch = [0u8; 64];
    let mut buffer = sys::Buffer {
        ptr: scratch.as_mut_ptr(),
        cap: scratch.len(),
        len: 0,
    };
    // SAFETY: `GRAPHEMES_UTF8` fills a `Buffer` that points to caller memory of `cap` bytes.
    let code = unsafe {
        sys::ghostty_render_state_row_cells_get(
            cells,
            sys::RenderStateRowCellsData::GRAPHEMES_UTF8,
            ptr::from_mut(&mut buffer).cast(),
        )
    };
    let bytes = match code {
        sys::Result::SUCCESS => scratch[..buffer.len].to_vec(),
        sys::Result::OUT_OF_SPACE => {
            let mut grown = vec![0u8; buffer.len];
            let mut buffer = sys::Buffer {
                ptr: grown.as_mut_ptr(),
                cap: grown.len(),
                len: 0,
            };
            // SAFETY: as above, with a buffer of the size that the library asked for.
            check(unsafe {
                sys::ghostty_render_state_row_cells_get(
                    cells,
                    sys::RenderStateRowCellsData::GRAPHEMES_UTF8,
                    ptr::from_mut(&mut buffer).cast(),
                )
            })?;
            grown.truncate(buffer.len);
            grown
        }
        other => return check(other).map(|()| String::new()),
    };
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// # Safety
/// `terminal` is a live terminal. The library writes a string that is valid until the next call into it.
unsafe fn terminal_string(terminal: sys::Terminal, data: sys::TerminalData::Type) -> String {
    let mut value = MaybeUninit::<sys::String>::zeroed();
    // SAFETY: `TITLE` and `PWD` write a `String` value to the out-pointer.
    let code = unsafe { sys::ghostty_terminal_get(terminal, data, value.as_mut_ptr().cast()) };
    if code != sys::Result::SUCCESS {
        return String::new();
    }
    // SAFETY: the call succeeded, so the value is set.
    lossy(unsafe { value.assume_init() })
}

/// The most bytes of an unfinished sequence that `replay` can give.
const CONTINUATION_MAX_BYTES: usize = 64 * 1024;

unsafe extern "C" fn cb_append(ud: *mut c_void, data: *const u8, len: usize) -> bool {
    if data.is_null() || len == 0 {
        return true;
    }
    // SAFETY: `ud` is the `Vec<u8>` that `replay` passed, and `data` and `len` describe valid bytes.
    unsafe {
        (*ud.cast::<Vec<u8>>()).extend_from_slice(std::slice::from_raw_parts(data, len));
    }
    true
}

/// # Safety
/// `ud` is the `Shared` pointer that `install_options` set as userdata.
unsafe fn shared<'a>(ud: *mut c_void) -> &'a mut Shared {
    // SAFETY: see the contract. Only the thread inside `write` runs callbacks, and it holds no other
    // reference to `Shared`.
    unsafe { &mut *ud.cast::<Shared>() }
}

unsafe extern "C" fn cb_write_pty(_: sys::Terminal, ud: *mut c_void, data: *const u8, len: usize) {
    // SAFETY: library callback contract: `ud` is our userdata, and `data` and `len` describe valid bytes.
    let (shared, bytes) = unsafe {
        (
            shared(ud),
            if data.is_null() || len == 0 {
                &[][..]
            } else {
                std::slice::from_raw_parts(data, len)
            },
        )
    };
    // One reply may arrive as several calls. Join neighbours, so the queue holds as few events as the stream made.
    if let Some(Event::Reply(last)) = shared.events.last_mut() {
        last.extend_from_slice(bytes);
    } else {
        shared.events.push(Event::Reply(bytes.to_vec()));
    }
}

unsafe extern "C" fn cb_bell(_: sys::Terminal, ud: *mut c_void) {
    // SAFETY: library callback contract.
    unsafe { shared(ud) }.events.push(Event::Bell);
}

unsafe extern "C" fn cb_title_changed(t: sys::Terminal, ud: *mut c_void) {
    // SAFETY: library callback contract. `t` is live during the callback.
    let (shared, title) = unsafe { (shared(ud), terminal_string(t, sys::TerminalData::TITLE)) };
    shared.events.push(Event::Title(title));
}

unsafe extern "C" fn cb_pwd_changed(t: sys::Terminal, ud: *mut c_void) {
    // SAFETY: library callback contract. `t` is live during the callback.
    let (shared, pwd) = unsafe { (shared(ud), terminal_string(t, sys::TerminalData::PWD)) };
    shared.events.push(Event::Pwd(pwd));
}

unsafe extern "C" fn cb_device_attributes(
    _: sys::Terminal,
    _: *mut c_void,
    out: *mut sys::DeviceAttributes,
) -> bool {
    // Ghostty's own answer: a level 2 terminal with ANSI color (`CSI ? 62 ; 22 c`), a VT220 with firmware 10.
    let mut features = [0u16; 64];
    features[0] = sys::DA_FEATURE_ANSI_COLOR;
    // SAFETY: `out` is a valid out-pointer for the call.
    unsafe {
        *out = sys::DeviceAttributes {
            primary: sys::DeviceAttributesPrimary {
                conformance_level: sys::DA_CONFORMANCE_LEVEL_2,
                features,
                num_features: 1,
            },
            secondary: sys::DeviceAttributesSecondary {
                device_type: sys::DA_DEVICE_TYPE_VT220,
                firmware_version: 10,
                rom_cartridge: 0,
            },
            tertiary: sys::DeviceAttributesTertiary { unit_id: 0 },
        };
    }
    true
}

unsafe extern "C" fn cb_size(
    _: sys::Terminal,
    ud: *mut c_void,
    out: *mut sys::SizeReportSize,
) -> bool {
    // SAFETY: library callback contract. `out` is a valid out-pointer.
    unsafe {
        match shared(ud).size {
            Some(size) => {
                *out = size;
                true
            }
            None => false,
        }
    }
}

unsafe extern "C" fn cb_program_status(
    _: sys::Terminal,
    ud: *mut c_void,
    report: *const sys::TerminalProgramStatus,
) {
    // SAFETY: library callback contract. `report` is valid for the call, and its strings are copied here.
    let (shared, report) = unsafe { (shared(ud), &*report) };
    let id = lossy(report.id);
    if report.state == sys::ProgramStatusState::CLEAR {
        shared.records.clear(&id);
        shared.events.push(Event::StatusCleared(id));
        return;
    }
    let message = lossy(report.message);
    let state = match report.state {
        sys::ProgramStatusState::WORKING => AgentState::Working,
        sys::ProgramStatusState::DONE => AgentState::Done,
        sys::ProgramStatusState::BLOCKED => AgentState::Blocked {
            kind: match report.kind {
                sys::ProgramStatusKind::PERMISSION => BlockedKind::Permission,
                sys::ProgramStatusKind::QUESTION => BlockedKind::Question,
                sys::ProgramStatusKind::AUTH => BlockedKind::Auth,
                _ => BlockedKind::Unspecified,
            },
            msg: message.clone(),
        },
        sys::ProgramStatusState::ERROR => AgentState::Error,
        _ => AgentState::Idle,
    };
    let report = StatusReport {
        id,
        app: lossy(report.app),
        title: lossy(report.title),
        message,
        progress: u8::try_from(report.progress).ok().filter(|p| *p <= 100),
        state,
    };
    let evicted = shared.records.put(report.clone());
    shared.events.push(Event::Status(report));
    if let Some(id) = evicted {
        shared.events.push(Event::StatusEvicted(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Terminal {
        /// Every cell through the full read, without the blank check.
        fn slow_rows(&mut self) -> Vec<Vec<Cell>> {
            // SAFETY: as in `read_grid`.
            unsafe {
                check(sys::ghostty_render_state_update(self.render, self.raw)).unwrap();
                check(sys::ghostty_render_state_get(
                    self.render,
                    sys::RenderStateData::ROW_ITERATOR,
                    ptr::from_mut(&mut self.row_iter).cast(),
                ))
                .unwrap();
                let mut rows = Vec::new();
                while sys::ghostty_render_state_row_iterator_next(self.row_iter) {
                    check(sys::ghostty_render_state_row_get(
                        self.row_iter,
                        sys::RenderStateRowData::CELLS,
                        ptr::from_mut(&mut self.cell_iter).cast(),
                    ))
                    .unwrap();
                    let mut row = Vec::new();
                    let mut x = 0;
                    while sys::ghostty_render_state_row_cells_select(self.cell_iter, x)
                        == sys::Result::SUCCESS
                    {
                        row.push(read_cell(self.cell_iter).unwrap());
                        x += 1;
                    }
                    rows.push(row);
                }
                rows
            }
        }
    }

    #[test]
    fn the_blank_check_skips_only_cells_that_read_as_default() {
        let mut t = Terminal::new(Options {
            cols: 12,
            rows: 8,
            scrollback_bytes: 10_000,
        })
        .unwrap();
        // Erased cells with a background, styled blanks, inverse and underlined spaces, a hidden
        // character, wide characters (also one that wraps and leaves a spacer head), a grapheme
        // cluster and a 256-color and an RGB background.
        t.write(
            "\x1b[44m\x1b[K\x1b[0m\r\n\x1b[7m  \x1b[0m \x1b[4m \x1b[0m \x1b[8mx\x1b[0m\r\n\
             a中b\u{2764}\u{FE0F}\r\n0123456789A中\r\n\x1b[48;5;123m \x1b[48;2;1;2;3m \x1b[0m\
             \x1b[1m \x1b[3m \x1b[0m\x1b[9m \x1b[2m \x1b[0m\r\ne\u{301}"
                .as_bytes(),
        );
        let slow = t.slow_rows();
        let mut grid = Grid::default();
        t.update_grid(&mut grid).unwrap();
        assert_eq!(grid.rows, slow);
        assert!(
            grid.rows.iter().flatten().any(|c| *c == Cell::default()),
            "some cells are blank"
        );
        assert!(
            grid.rows
                .iter()
                .flatten()
                .any(|c| c.text.is_empty() && c.bg.is_some()),
            "an erased cell keeps its background"
        );
    }
}
