//! GPUI keystrokes to key events for the encoder of libghostty-vt, and the keys that the pane keeps.

use agentsi_vt::{Key, KeyAction, KeyEvent, Mods};
use gpui_kit::Keystroke;

/// A key that the pane handles itself, so the program never sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Copy,
    Paste,
    PageUp,
    PageDown,
    Top,
    Bottom,
}

/// `Ctrl+Shift+C` and `Ctrl+Shift+V` copy and paste. `Shift` with Page Up, Page Down, Home and End
/// moves the viewport.
pub fn shortcut(ks: &Keystroke) -> Option<Shortcut> {
    let m = &ks.modifiers;
    let only_shift = m.shift && !m.control && !m.alt && !m.platform;
    let ctrl_shift = m.shift && m.control && !m.alt && !m.platform;
    match ks.key.as_str() {
        "c" if ctrl_shift => Some(Shortcut::Copy),
        "v" if ctrl_shift => Some(Shortcut::Paste),
        "pageup" if only_shift => Some(Shortcut::PageUp),
        "pagedown" if only_shift => Some(Shortcut::PageDown),
        "home" if only_shift => Some(Shortcut::Top),
        "end" if only_shift => Some(Shortcut::Bottom),
        _ => None,
    }
}

pub fn mods(m: &gpui_kit::Modifiers) -> Mods {
    let mut mods = Mods::NONE;
    if m.shift {
        mods |= Mods::SHIFT;
    }
    if m.alt {
        mods |= Mods::ALT;
    }
    if m.control {
        mods |= Mods::CTRL;
    }
    if m.platform {
        mods |= Mods::SUPER;
    }
    mods
}

/// The key event of a GPUI keystroke, as a press, a repeat or a release.
pub fn key_event(ks: &Keystroke, action: KeyAction) -> KeyEvent {
    let m = &ks.modifiers;
    let name = ks.key.as_str();
    // With Ctrl the encoder makes the control code from the key. Text with a control character is
    // not text (GPUI gives "\r" for Enter, for example).
    let text = ks
        .key_char
        .clone()
        .filter(|s| !s.is_empty() && !s.chars().any(char::is_control) && !m.control);
    let mut chars = name.chars();
    let unshifted = match (chars.next(), chars.next()) {
        (Some(c), None) => c.to_lowercase().next(),
        _ if name == "space" => Some(' '),
        _ => None,
    };
    KeyEvent {
        action,
        key: Key::from_name(name),
        mods: mods(m),
        consumed_mods: if text.is_some() && m.shift {
            Mods::SHIFT
        } else {
            Mods::NONE
        },
        text,
        unshifted,
        composing: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsi_vt::{Options, Terminal};

    fn ks(spec: &'static str, key_char: Option<&str>) -> Keystroke {
        let mut k = Keystroke::parse(spec).unwrap_or_else(|_| panic!("{spec} is a keystroke"));
        k.key_char = key_char.map(str::to_string);
        k
    }

    fn encode(t: &mut Terminal, spec: &'static str, key_char: Option<&str>) -> Vec<u8> {
        t.encode_key(&key_event(&ks(spec, key_char), KeyAction::Press))
            .unwrap()
    }

    #[test]
    fn keys_encode_as_a_terminal_sends_them() {
        let mut t = Terminal::new(Options::default()).unwrap();
        assert_eq!(encode(&mut t, "a", Some("a")), b"a");
        assert_eq!(encode(&mut t, "shift-a", Some("A")), b"A");
        assert_eq!(encode(&mut t, "enter", Some("\r")), b"\r");
        assert_eq!(encode(&mut t, "backspace", None), b"\x7f");
        assert_eq!(encode(&mut t, "tab", Some("\t")), b"\t");
        assert_eq!(encode(&mut t, "escape", None), b"\x1b");
        assert_eq!(encode(&mut t, "ctrl-c", None), b"\x03");
        assert_eq!(encode(&mut t, "ctrl-d", None), b"\x04");
        assert_eq!(encode(&mut t, "alt-x", Some("x")), b"\x1bx");
        assert_eq!(encode(&mut t, "up", None), b"\x1b[A");
        assert_eq!(encode(&mut t, "f5", None), b"\x1b[15~");
        assert_eq!(encode(&mut t, "space", Some(" ")), b" ");
        assert_eq!(encode(&mut t, "ctrl-left", None), b"\x1b[1;5D");
        assert_eq!(encode(&mut t, "ä", Some("ä")), "ä".as_bytes());
    }

    #[test]
    fn the_program_modes_change_the_arrow_keys() {
        let mut t = Terminal::new(Options::default()).unwrap();
        t.write(b"\x1b[?1h");
        assert_eq!(encode(&mut t, "up", None), b"\x1bOA");
    }

    #[test]
    fn the_pane_keeps_its_own_shortcuts() {
        assert_eq!(shortcut(&ks("ctrl-shift-c", None)), Some(Shortcut::Copy));
        assert_eq!(shortcut(&ks("ctrl-shift-v", None)), Some(Shortcut::Paste));
        assert_eq!(shortcut(&ks("shift-pageup", None)), Some(Shortcut::PageUp));
        assert_eq!(shortcut(&ks("shift-end", None)), Some(Shortcut::Bottom));
        assert_eq!(shortcut(&ks("ctrl-c", None)), None);
        assert_eq!(shortcut(&ks("pageup", None)), None);
    }
}
