use agentsi_vt::{
    AgentState, BlockedKind, Damage, Event, Grid, Key, KeyAction, KeyEvent, Mods, Options, Rgb,
    Scroll, Terminal, Width,
};

fn term() -> Terminal {
    Terminal::new(Options::default()).expect("terminal")
}

fn replies(events: &[Event]) -> Vec<u8> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Reply(b) => Some(b.as_slice()),
            _ => None,
        })
        .flatten()
        .copied()
        .collect()
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn b64(s: &str) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in s.as_bytes().chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[test]
fn prints_text_into_the_grid() {
    let mut t = term();
    t.write(b"hello\r\nworld");
    let grid = t.grid().unwrap();
    let lines = grid.text_lines();
    assert_eq!(grid.cols, 80);
    assert_eq!(grid.rows.len(), 24);
    assert_eq!(&lines[..2], ["hello", "world"]);
    let cursor = grid.cursor.expect("cursor in the viewport");
    assert_eq!((cursor.x, cursor.y), (5, 1));
    assert!(cursor.visible);
}

#[test]
fn reads_colors_styles_and_wide_cells() {
    let mut t = term();
    t.write("\x1b[1;31mR\x1b[0m \x1b[4mu\x1b[0m日".as_bytes());
    let grid = t.grid().unwrap();
    let row = &grid.rows[0];
    assert_eq!(row[0].text, "R");
    assert!(row[0].style.bold);
    assert!(row[0].fg.is_some(), "an explicit foreground");
    assert!(row[1].fg.is_none(), "the default foreground");
    assert_eq!(row[2].style.underline, agentsi_vt::Underline::Single);
    assert_eq!(row[3].text, "日");
    assert_eq!(row[3].width, Width::Wide);
    assert_eq!(row[4].width, Width::SpacerTail);
    assert_eq!(grid.text_lines()[0], "R u日");
}

#[test]
fn resize_changes_the_grid_and_answers_size_reports() {
    let mut t = term();
    t.resize(100, 30, 9, 18).unwrap();
    assert_eq!((t.cols().unwrap(), t.rows().unwrap()), (100, 30));
    let grid = t.grid().unwrap();
    assert_eq!((grid.cols, grid.rows.len()), (100, 30));
    // XTWINOPS 18: the size of the text area in characters.
    t.write(b"\x1b[18t");
    assert_eq!(replies(&t.take_events()), b"\x1b[8;30;100t");
    // XTWINOPS 16: the size of a cell in pixels.
    t.write(b"\x1b[16t");
    assert_eq!(replies(&t.take_events()), b"\x1b[6;18;9t");
}

#[test]
fn answers_the_device_attributes_query() {
    let mut t = term();
    t.write(b"\x1b[c");
    assert_eq!(replies(&t.take_events()), b"\x1b[?62;22c");
}

#[test]
fn reports_title_and_bell() {
    let mut t = term();
    t.write(b"\x1b]2;my title\x07\x07");
    let events = t.take_events();
    assert!(
        events.contains(&Event::Title("my title".into())),
        "{events:?}"
    );
    assert!(events.contains(&Event::Bell), "{events:?}");
    assert_eq!(t.title(), "my title");
    assert!(
        t.take_events().is_empty(),
        "the queue is empty after a take"
    );
}

#[test]
fn answers_the_program_status_query() {
    let mut t = term();
    t.write(b"\x1b]7501;?\x1b\\");
    let reply = replies(&t.take_events());
    let text = String::from_utf8_lossy(&reply);
    assert!(
        text.starts_with("\x1b]7501;?") && text.contains("states="),
        "{text:?}"
    );
    for state in ["idle", "working", "blocked"] {
        assert!(text.contains(state), "{text:?} lacks {state}");
    }
}

/// Claude Code sends the status query, then DA1. It counts a query without a reply by the time DA1
/// is answered as unanswered, so the replies must keep the order of the queries.
#[test]
fn the_status_reply_comes_before_the_device_attributes_reply() {
    let stream = b"\x1b]7501;?\x1b\\\x1b[c";
    let da1 = b"\x1b[?62;22c";
    // In one write, and byte by byte.
    for chunked in [false, true] {
        let mut t = term();
        if chunked {
            for byte in stream {
                t.write(&[*byte]);
            }
        } else {
            t.write(stream);
        }
        let all = replies(&t.take_events());
        let status = find(&all, b"\x1b]7501;?").expect("a status reply");
        let attrs = find(&all, da1).expect("a DA1 reply");
        assert!(status < attrs, "chunked={chunked}: {all:?}");
    }
    // The other order of the queries gives the other order of the replies.
    let mut t = term();
    t.write(b"\x1b[c\x1b]7501;?\x1b\\");
    let all = replies(&t.take_events());
    assert!(find(&all, da1).unwrap() < find(&all, b"\x1b]7501;?").unwrap());
}

#[test]
fn tracks_the_program_status() {
    let mut t = term();
    assert_eq!(t.agent_state(), None);
    t.write(b"\x1b]7501;state=idle:app=claude-code\x07");
    assert_eq!(t.agent_state(), Some(&AgentState::Idle));
    t.write(b"\x1b]7501;state=working:app=claude-code:progress=40\x1b\\");
    assert_eq!(t.agent_state(), Some(&AgentState::Working));
    let msg = "approve Bash: touch /tmp/x";
    t.write(
        format!(
            "\x1b]7501;state=blocked:app=claude-code:kind=permission:msg={}\x1b\\",
            b64(msg)
        )
        .as_bytes(),
    );
    assert_eq!(
        t.agent_state(),
        Some(&AgentState::Blocked {
            kind: BlockedKind::Permission,
            msg: msg.into()
        })
    );
    let reports: Vec<_> = t
        .take_events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Status(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(reports.len(), 3);
    assert_eq!(reports[0].app, "claude-code");
    assert_eq!(reports[0].progress, None);
    assert_eq!(reports[1].progress, Some(40));
    assert_eq!(reports[2].message, msg);
    // Other states and kinds.
    t.write(b"\x1b]7501;state=done\x07");
    assert_eq!(t.agent_state(), Some(&AgentState::Done));
    t.write(b"\x1b]7501;state=error\x07");
    assert_eq!(t.agent_state(), Some(&AgentState::Error));
    t.write(b"\x1b]7501;state=blocked:kind=question\x07");
    assert!(matches!(
        t.agent_state(),
        Some(AgentState::Blocked {
            kind: BlockedKind::Question,
            ..
        })
    ));
    t.write(b"\x1b]7501;state=blocked:kind=auth\x07");
    assert!(matches!(
        t.agent_state(),
        Some(AgentState::Blocked {
            kind: BlockedKind::Auth,
            ..
        })
    ));
    // A report with a bad shape does not reach the callback and leaves the state alone.
    t.write(b"\x1b]7501;state=nonsense\x07");
    assert!(matches!(
        t.agent_state(),
        Some(AgentState::Blocked {
            kind: BlockedKind::Auth,
            ..
        })
    ));
}

#[test]
fn clear_and_reset_remove_the_records() {
    let mut t = term();
    t.write(b"\x1b]7501;state=working:app=cargo\x07");
    t.write(b"\x1b]7501;state=working;id=build\x07");
    assert!(t.agent_state().is_some());
    t.write(b"\x1b]7501;state=clear\x07");
    assert_eq!(t.agent_state(), None);
    assert!(
        t.take_events()
            .iter()
            .any(|e| matches!(e, Event::StatusCleared(id) if id.is_empty()))
    );
    t.write(b"\x1b]7501;state=idle\x07");
    t.write(b"\x1bc"); // RIS
    assert_eq!(t.agent_state(), None);
}

fn press(key: Key, mods: Mods, text: Option<&str>) -> KeyEvent {
    KeyEvent {
        action: KeyAction::Press,
        key,
        mods,
        text: text.map(Into::into),
        unshifted: text.and_then(|t| t.chars().next()),
        ..Default::default()
    }
}

#[test]
fn encodes_keys_with_the_modes_of_the_program() {
    let mut t = term();
    let enc = |t: &mut Terminal, e: KeyEvent| t.encode_key(&e).unwrap();
    assert_eq!(enc(&mut t, press(Key::Enter, Mods::NONE, None)), b"\r");
    assert_eq!(
        enc(&mut t, press(Key::Letter('a'), Mods::NONE, Some("a"))),
        b"a"
    );
    assert_eq!(
        enc(&mut t, press(Key::Letter('c'), Mods::CTRL, None)),
        b"\x03"
    );
    assert_eq!(
        enc(&mut t, press(Key::Letter('x'), Mods::ALT, Some("x"))),
        b"\x1bx"
    );
    assert_eq!(
        enc(&mut t, press(Key::ArrowUp, Mods::NONE, None)),
        b"\x1b[A"
    );
    // The program switches the cursor keys to application mode (DECCKM).
    t.write(b"\x1b[?1h");
    assert_eq!(
        enc(&mut t, press(Key::ArrowUp, Mods::NONE, None)),
        b"\x1bOA"
    );
    assert_eq!(enc(&mut t, press(Key::Tab, Mods::SHIFT, None)), b"\x1b[Z");
    // A release makes no bytes in the legacy encoding.
    let release = KeyEvent {
        action: KeyAction::Release,
        ..press(Key::Letter('a'), Mods::NONE, Some("a"))
    };
    assert!(enc(&mut t, release).is_empty());
    // A text longer than the first buffer still encodes.
    let long = "é".repeat(100);
    assert_eq!(
        enc(&mut t, press(Key::Unidentified, Mods::NONE, Some(&long))),
        long.as_bytes()
    );
}

#[test]
fn the_kitty_keyboard_flags_change_the_encoding() {
    let mut t = term();
    // Without the flags, Ctrl+Enter is the xterm form CSI 27;5;13~. With "disambiguate" it is CSI 13;5u.
    let ctrl_enter = press(Key::Enter, Mods::CTRL, None);
    assert_eq!(t.encode_key(&ctrl_enter).unwrap(), b"\x1b[27;5;13~");
    t.write(b"\x1b[>1u");
    assert_eq!(t.encode_key(&ctrl_enter).unwrap(), b"\x1b[13;5u");
}

#[test]
fn a_terminal_moves_to_another_thread() {
    let mut t = term();
    t.write(b"one");
    let t = std::thread::spawn(move || {
        t.write_and_grid();
        t
    })
    .join()
    .unwrap();
    drop(t);
}

trait WriteAndGrid {
    fn write_and_grid(&mut self);
}

impl WriteAndGrid for Terminal {
    fn write_and_grid(&mut self) {
        self.write(b" two");
        assert_eq!(self.grid().unwrap().text_lines()[0], "one two");
    }
}

#[test]
fn a_flood_of_records_keeps_the_root_and_reports_the_eviction() {
    let mut t = term();
    t.write(b"\x1b]7501;state=working:app=x\x07");
    for i in 0..300 {
        t.write(format!("\x1b]7501;state=working:id=job{i:03}\x07").as_bytes());
    }
    assert_eq!(
        t.agent_state(),
        Some(&AgentState::Working),
        "the root record stays"
    );
    assert_eq!(t.status().iter().count(), 256);
    let evicted = t
        .take_events()
        .iter()
        .filter(|e| matches!(e, Event::StatusEvicted(_)))
        .count();
    assert_eq!(evicted, 301 - 256, "each eviction is an event");
    // The oldest non-root record went first.
    assert!(t.status().get("job000").is_none());
    assert!(t.status().get("job299").is_some());
}

#[test]
fn size_reports_work_before_the_first_resize() {
    let mut t = term();
    t.write(b"\x1b[18t");
    assert_eq!(replies(&t.take_events()), b"\x1b[8;24;80t");
}

#[test]
fn a_rejected_resize_keeps_the_old_size_for_reports() {
    let mut t = term();
    t.resize(100, 30, 9, 18).unwrap();
    assert!(t.resize(0, 0, 9, 18).is_err());
    t.write(b"\x1b[18t");
    assert_eq!(replies(&t.take_events()), b"\x1b[8;30;100t");
}

#[test]
fn a_replay_brings_a_new_terminal_to_the_same_screen_and_modes() {
    let mut a = term();
    for i in 0..40 {
        a.write(format!("line {i}\r\n").as_bytes());
    }
    // Colors, cursor keys, bracketed paste, focus reports, the alternate screen, a cursor move.
    a.write(b"\x1b[31mred\x1b[0m \x1b[?1h\x1b[?2004h\x1b[?1004h");
    a.write(b"\x1b[?1049h\x1b[2J\x1b[H\x1b[1;32malt screen\x1b[0m\x1b[5;7H");
    // An unfinished UTF-8 character at the end of the input.
    a.write(b"\xe2\x82");
    let replay = a.replay().unwrap();

    let mut b = term();
    b.write(&replay);
    assert_eq!(a.grid().unwrap(), b.grid().unwrap());
    for mode in [1, 1004, 1049, 2004] {
        assert!(b.mode(mode), "mode {mode}");
    }
    assert!(!b.mode(1000));
    // Both terminals finish the same character.
    a.write(b"\xac");
    b.write(b"\xac");
    assert_eq!(a.grid().unwrap(), b.grid().unwrap());
    assert!(b.grid().unwrap().text_lines()[4].contains('€'));
}

#[test]
fn a_replay_of_the_main_screen_holds_the_scrollback() {
    let mut a = term();
    for i in 0..40 {
        a.write(format!("line {i}\r\n").as_bytes());
    }
    let replay = a.replay().unwrap();
    assert!(find(&replay, b"line 0\r\n").is_some());
    let mut b = term();
    b.write(&replay);
    assert_eq!(a.grid().unwrap(), b.grid().unwrap());
}

#[test]
fn a_reset_and_a_replay_repaint_a_used_terminal() {
    let mut a = term();
    a.write(b"\x1b[44mfirst\x1b[0m\r\nsecond\x1b[?25l");
    let mut b = term();
    b.write(b"\x1b[?1h\x1b[3;3Hjunk\x1b]0;old\x07");
    b.write(b"\x1bc");
    b.write(&a.replay().unwrap());
    assert_eq!(a.grid().unwrap(), b.grid().unwrap());
    assert!(!b.mode(1));
}

fn small() -> Terminal {
    Terminal::new(Options {
        cols: 10,
        rows: 4,
        scrollback_bytes: 100_000,
    })
    .expect("terminal")
}

/// An update of a kept grid gives the same grid as a full copy.
fn update(t: &mut Terminal, grid: &mut Grid) -> Damage {
    let damage = t.update_grid(grid).unwrap();
    assert_eq!(*grid, t.grid().unwrap());
    damage
}

#[test]
fn the_first_update_reads_every_row_and_a_quiet_one_reads_none() {
    let mut t = small();
    let mut grid = Grid::default();
    t.write(b"a\r\nb");
    let first = update(&mut t, &mut grid);
    assert!(first.all);
    assert_eq!(first.rows, [0, 1, 2, 3]);
    assert_eq!(first.ids.len(), 4);
    let quiet = update(&mut t, &mut grid);
    assert!(!quiet.all);
    assert!(quiet.rows.is_empty());
}

#[test]
fn an_edit_in_place_reads_only_its_rows() {
    let mut t = small();
    let mut grid = Grid::default();
    t.write(b"a\r\nb\r\nc");
    update(&mut t, &mut grid);
    t.write(b"\x1b[2;1HX\x1b[2;1H");
    let d = update(&mut t, &mut grid);
    assert!(!d.all);
    assert!(d.rows.contains(&1), "{:?}", d.rows);
    assert!(!d.rows.contains(&0), "{:?}", d.rows);
    assert_eq!(grid.text_lines()[1], "X");
}

#[test]
fn a_scroll_reads_every_row_and_the_rows_keep_their_ids() {
    let mut t = small();
    let mut grid = Grid::default();
    t.write(b"a\r\nb\r\nc\r\nd");
    let before = update(&mut t, &mut grid);
    t.write(b"\r\ne");
    let after = update(&mut t, &mut grid);
    assert!(after.all);
    assert_eq!(after.ids[..3], before.ids[1..]);
    assert_eq!(grid.text_lines(), ["b", "c", "d", "e"]);
    t.scroll(Scroll::Lines(-1));
    let up = update(&mut t, &mut grid);
    assert!(up.all);
    assert_eq!(up.ids[1..], after.ids[..3]);
}

#[test]
fn new_default_colors_read_every_row() {
    let mut t = small();
    let mut grid = Grid::default();
    t.write(b"plain");
    update(&mut t, &mut grid);
    // The render state takes OSC 11 with the next change of OSC 10.
    t.write(b"\x1b]11;#123456\x07\x1b]10;#ff0000\x07");
    let d = update(&mut t, &mut grid);
    assert!(d.all);
    assert_eq!(
        grid.colors.background,
        Rgb {
            r: 0x12,
            g: 0x34,
            b: 0x56
        }
    );
}

#[test]
fn a_resize_and_the_other_screen_read_every_row() {
    let mut t = small();
    let mut grid = Grid::default();
    update(&mut t, &mut grid);
    t.resize(12, 5, 8, 16).unwrap();
    let d = update(&mut t, &mut grid);
    assert!(d.all);
    assert_eq!((grid.cols, grid.rows.len()), (12, 5));
    t.write(b"\x1b[?1049hALT");
    assert!(update(&mut t, &mut grid).all);
    assert_eq!(grid.text_lines()[0], "ALT");
}

#[test]
fn a_full_copy_leaves_the_damage_for_the_next_update() {
    let mut t = small();
    let mut grid = Grid::default();
    update(&mut t, &mut grid);
    t.write(b"\x1b[3;1Hz");
    let _ = t.grid().unwrap();
    let d = update(&mut t, &mut grid);
    assert!(d.rows.contains(&2), "{:?}", d.rows);
}
