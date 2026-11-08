//! Keys for the encoder of libghostty-vt.

use crate::sys;
use std::ops::{BitOr, BitOrAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods(pub u16);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const SHIFT: Mods = Mods(sys::MODS_SHIFT);
    pub const CTRL: Mods = Mods(sys::MODS_CTRL);
    pub const ALT: Mods = Mods(sys::MODS_ALT);
    pub const SUPER: Mods = Mods(sys::MODS_SUPER);

    pub fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for Mods {
    type Output = Mods;
    fn bitor(self, rhs: Mods) -> Mods {
        Mods(self.0 | rhs.0)
    }
}

impl BitOrAssign for Mods {
    fn bitor_assign(&mut self, rhs: Mods) {
        self.0 |= rhs.0;
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyAction {
    Release,
    #[default]
    Press,
    Repeat,
}

/// A physical key. The letters and digits are the keys of a US layout.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Key {
    #[default]
    Unidentified,
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Space,
    /// `F1` to `F12`.
    F(u8),
    /// A letter, `'a'` to `'z'`.
    Letter(char),
    /// A digit, `'0'` to `'9'`.
    Digit(char),
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Semicolon,
    Quote,
    Comma,
    Period,
    Slash,
    Backquote,
}

impl Key {
    /// Maps the key names of GPUI (`"enter"`, `"up"`, `"f5"`, `"a"`, `"["`) to a key.
    pub fn from_name(name: &str) -> Key {
        let mut chars = name.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return match c.to_ascii_lowercase() {
                c @ 'a'..='z' => Key::Letter(c),
                c @ '0'..='9' => Key::Digit(c),
                '-' => Key::Minus,
                '=' => Key::Equal,
                '[' => Key::BracketLeft,
                ']' => Key::BracketRight,
                '\\' => Key::Backslash,
                ';' => Key::Semicolon,
                '\'' => Key::Quote,
                ',' => Key::Comma,
                '.' => Key::Period,
                '/' => Key::Slash,
                '`' => Key::Backquote,
                _ => Key::Unidentified,
            };
        }
        match name {
            "enter" => Key::Enter,
            "tab" => Key::Tab,
            "escape" => Key::Escape,
            "backspace" => Key::Backspace,
            "delete" => Key::Delete,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" => Key::PageUp,
            "pagedown" => Key::PageDown,
            "insert" => Key::Insert,
            "up" => Key::ArrowUp,
            "down" => Key::ArrowDown,
            "left" => Key::ArrowLeft,
            "right" => Key::ArrowRight,
            "space" => Key::Space,
            _ => match name.strip_prefix('f') {
                Some(n) if n.bytes().all(|b| b.is_ascii_digit()) && !n.starts_with('0') => {
                    match n.parse::<u8>() {
                        Ok(n @ 1..=12) => Key::F(n),
                        _ => Key::Unidentified,
                    }
                }
                _ => Key::Unidentified,
            },
        }
    }

    pub(crate) fn raw(self) -> sys::Key::Type {
        use sys::Key as K;
        match self {
            Key::Unidentified => K::UNIDENTIFIED,
            Key::Enter => K::ENTER,
            Key::Tab => K::TAB,
            Key::Escape => K::ESCAPE,
            Key::Backspace => K::BACKSPACE,
            Key::Delete => K::DELETE,
            Key::Home => K::HOME,
            Key::End => K::END,
            Key::PageUp => K::PAGE_UP,
            Key::PageDown => K::PAGE_DOWN,
            Key::Insert => K::INSERT,
            Key::ArrowUp => K::ARROW_UP,
            Key::ArrowDown => K::ARROW_DOWN,
            Key::ArrowLeft => K::ARROW_LEFT,
            Key::ArrowRight => K::ARROW_RIGHT,
            Key::Space => K::SPACE,
            Key::F(n @ 1..=12) => K::F1 + (n as sys::Key::Type - 1),
            Key::F(_) => K::UNIDENTIFIED,
            Key::Letter(c @ 'a'..='z') => K::A + (c as sys::Key::Type - 'a' as sys::Key::Type),
            Key::Letter(_) => K::UNIDENTIFIED,
            Key::Digit(c @ '0'..='9') => K::DIGIT_0 + (c as sys::Key::Type - '0' as sys::Key::Type),
            Key::Digit(_) => K::UNIDENTIFIED,
            Key::Minus => K::MINUS,
            Key::Equal => K::EQUAL,
            Key::BracketLeft => K::BRACKET_LEFT,
            Key::BracketRight => K::BRACKET_RIGHT,
            Key::Backslash => K::BACKSLASH,
            Key::Semicolon => K::SEMICOLON,
            Key::Quote => K::QUOTE,
            Key::Comma => K::COMMA,
            Key::Period => K::PERIOD,
            Key::Slash => K::SLASH,
            Key::Backquote => K::BACKQUOTE,
        }
    }
}

/// One key event for [`crate::Terminal::encode_key`].
#[derive(Clone, Debug, Default)]
pub struct KeyEvent {
    pub action: KeyAction,
    pub key: Key,
    pub mods: Mods,
    /// Modifiers that the layout used to make `text`. For `Shift+a` that gives `"A"`, set `SHIFT` here.
    pub consumed_mods: Mods,
    /// The text that the key makes, without control characters.
    pub text: Option<String>,
    /// The codepoint of the key without Shift.
    pub unshifted: Option<char>,
    pub composing: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_map_to_keys() {
        assert_eq!(Key::from_name("enter"), Key::Enter);
        assert_eq!(Key::from_name("up"), Key::ArrowUp);
        assert_eq!(Key::from_name("f5"), Key::F(5));
        assert_eq!(Key::from_name("f13"), Key::Unidentified);
        assert_eq!(Key::from_name("f+5"), Key::Unidentified);
        assert_eq!(Key::from_name("f05"), Key::Unidentified);
        assert_eq!(Key::from_name("A"), Key::Letter('a'));
        assert_eq!(Key::from_name("7"), Key::Digit('7'));
        assert_eq!(Key::from_name("["), Key::BracketLeft);
        assert_eq!(Key::from_name("nonsense"), Key::Unidentified);
    }

    #[test]
    fn raw_keys_follow_the_library_order() {
        assert_eq!(Key::Letter('a').raw(), sys::Key::A);
        assert_eq!(Key::Letter('z').raw(), sys::Key::Z);
        assert_eq!(Key::Digit('0').raw(), sys::Key::DIGIT_0);
        assert_eq!(Key::Digit('9').raw(), sys::Key::DIGIT_9);
        assert_eq!(Key::F(1).raw(), sys::Key::F1);
        assert_eq!(Key::F(12).raw(), sys::Key::F12);
    }
}
