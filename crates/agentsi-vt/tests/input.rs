//! The viewport, the modes, and the mouse, paste and focus encoders.
use agentsi_vt::{
    Mods, MouseAction, MouseButton, MouseEvent, MouseGeometry, Options, Palette, Rgb, Scroll,
    Terminal, mode,
};

fn term(cols: u16, rows: u16) -> Terminal {
    let mut t = Terminal::new(Options {
        cols,
        rows,
        scrollback_bytes: 1_000_000,
    })
    .expect("terminal");
    t.resize(cols, rows, 10, 20).expect("resize");
    t
}

#[test]
fn the_viewport_moves_over_the_scrollback() {
    let mut t = term(20, 4);
    for i in 0..30 {
        t.write(format!("line {i}\r\n").as_bytes());
    }
    let bar = t.scrollbar().unwrap();
    assert_eq!(bar.len, 4);
    assert!(bar.total >= 31);
    assert!(bar.at_bottom());
    let live = t.grid().unwrap().text_lines();
    assert_eq!(live[2], "line 29");

    t.scroll(Scroll::Lines(-5));
    let up = t.scrollbar().unwrap();
    assert_eq!(up.offset + 5, bar.offset);
    assert!(!up.at_bottom());
    assert_eq!(t.grid().unwrap().text_lines()[2], "line 24");

    // New output does not move a viewport that the user scrolled.
    t.write(b"line 30\r\n");
    assert_eq!(t.grid().unwrap().text_lines()[2], "line 24");

    t.scroll(Scroll::Top);
    assert_eq!(t.scrollbar().unwrap().offset, 0);
    assert_eq!(t.grid().unwrap().text_lines()[0], "line 0");
    t.scroll(Scroll::Bottom);
    assert!(t.scrollbar().unwrap().at_bottom());
}

#[test]
fn modes_follow_the_program() {
    let mut t = term(20, 4);
    assert!(!t.mode(mode::BRACKETED_PASTE));
    t.write(b"\x1b[?2004h\x1b[?1004h");
    assert!(t.mode(mode::BRACKETED_PASTE));
    assert!(t.mode(mode::FOCUS_EVENTS));
    t.write(b"\x1b[?2004l");
    assert!(!t.mode(mode::BRACKETED_PASTE));
    assert!(!t.alternate_screen());
    t.write(b"\x1b[?1049h");
    assert!(t.alternate_screen());
}

#[test]
fn pastes_are_bracketed_when_the_program_asks() {
    let mut t = term(20, 4);
    assert_eq!(t.encode_paste("a\nb").unwrap(), b"a\rb");
    t.write(b"\x1b[?2004h");
    assert_eq!(t.encode_paste("a\nb").unwrap(), b"\x1b[200~a\nb\x1b[201~");
    // An escape byte inside the text cannot end the paste early.
    let evil = t.encode_paste("x\x1b[201~rm").unwrap();
    assert!(!evil[6..evil.len() - 6].contains(&0x1b), "{evil:?}");
    let long = "y".repeat(10_000);
    assert_eq!(t.encode_paste(&long).unwrap().len(), 10_000 + 12);
}

#[test]
fn focus_reports_only_in_mode_1004() {
    let mut t = term(20, 4);
    assert_eq!(t.encode_focus(true), None);
    t.write(b"\x1b[?1004h");
    assert_eq!(t.encode_focus(true).as_deref(), Some(&b"\x1b[I"[..]));
    assert_eq!(t.encode_focus(false).as_deref(), Some(&b"\x1b[O"[..]));
}

fn geometry() -> MouseGeometry {
    MouseGeometry {
        width_px: 200,
        height_px: 80,
        cell_width_px: 10,
        cell_height_px: 20,
        padding_px: 0,
    }
}

fn click(action: MouseAction, x: f32, y: f32) -> MouseEvent {
    MouseEvent {
        action,
        button: Some(MouseButton::Left),
        mods: Mods::NONE,
        x,
        y,
        any_button_down: action == MouseAction::Press,
    }
}

#[test]
fn mouse_reports_follow_the_tracking_mode() {
    let mut t = term(20, 4);
    assert!(!t.mouse_tracking());
    assert!(
        t.encode_mouse(&click(MouseAction::Press, 15., 25.), &geometry())
            .unwrap()
            .is_empty()
    );
    t.write(b"\x1b[?1000h\x1b[?1006h");
    assert!(t.mouse_tracking());
    // Cell (2, 2), 1-based, in the SGR format.
    assert_eq!(
        t.encode_mouse(&click(MouseAction::Press, 15., 25.), &geometry())
            .unwrap(),
        b"\x1b[<0;2;2M"
    );
    assert_eq!(
        t.encode_mouse(&click(MouseAction::Release, 15., 25.), &geometry())
            .unwrap(),
        b"\x1b[<0;2;2m"
    );
    let wheel = MouseEvent {
        button: Some(MouseButton::WheelUp),
        ..click(MouseAction::Press, 0., 0.)
    };
    assert_eq!(
        t.encode_mouse(&wheel, &geometry()).unwrap(),
        b"\x1b[<64;1;1M"
    );
}

#[test]
fn the_palette_sets_the_default_and_ansi_colors() {
    let mut t = term(20, 4);
    let mut ansi = [Rgb::default(); 16];
    ansi[1] = Rgb { r: 1, g: 2, b: 3 };
    t.set_palette(&Palette {
        foreground: Rgb {
            r: 200,
            g: 201,
            b: 202,
        },
        background: Rgb {
            r: 10,
            g: 11,
            b: 12,
        },
        cursor: Some(Rgb { r: 9, g: 9, b: 9 }),
        ansi,
    })
    .unwrap();
    t.write(b"\x1b[31mR\x1b[0mD");
    let g = t.grid().unwrap();
    assert_eq!(
        g.colors.foreground,
        Rgb {
            r: 200,
            g: 201,
            b: 202
        }
    );
    assert_eq!(
        g.colors.background,
        Rgb {
            r: 10,
            g: 11,
            b: 12
        }
    );
    assert_eq!(g.colors.cursor, Some(Rgb { r: 9, g: 9, b: 9 }));
    assert_eq!(g.rows[0][0].fg, Some(Rgb { r: 1, g: 2, b: 3 }));
    assert_eq!(g.rows[0][1].fg, None);
}
