//! The choices of the Appearance setting: density, corners, the text offset and the font
//! families.
//!
//! The choice lives in the gpui global [`Appearance`]. The token functions `space`,
//! `size::*`, `radius::*` and `control::*` read it through [`current`], with no `cx`, so a
//! helper that builds an element needs no extra argument. [`Appearance::apply`] is the only
//! writer. It sets the global and the copy that `current` reads in the same call.
//! GPUI draws all views on the main thread, so the copy is thread-local. Each test thread
//! then has its own.
use std::cell::{Cell, RefCell};

use gpui_kit::component::ThemeMode;
use gpui_kit::{App, Global, SharedString, WindowAppearance};

use crate::theme;
use crate::tokens::{Palette, current_palette, set_palette};

macro_rules! choice {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = $key:literal, $label:literal;)+ } default $default:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name {
            $($(#[$vmeta])* $variant,)+
        }

        impl Default for $name {
            fn default() -> Self {
                $name::$default
            }
        }

        impl $name {
            pub const ALL: [$name; 3] = [$($name::$variant,)+];

            /// The word in `appearance.toml`.
            pub const fn key(self) -> &'static str {
                match self {
                    $($name::$variant => $key,)+
                }
            }

            /// The word on the button of the settings screen.
            pub const fn label(self) -> &'static str {
                match self {
                    $($name::$variant => $label,)+
                }
            }

            pub fn from_key(key: &str) -> Option<$name> {
                match key {
                    $($key => Some($name::$variant),)+
                    _ => None,
                }
            }
        }
    };
}

choice! {
    /// How much room rows, controls and gaps take.
    Density {
        Compact = "compact", "Compact";
        Normal = "normal", "Normal";
        Comfortable = "comfortable", "Comfortable";
    } default Normal
}

choice! {
    /// How round the corners of buttons, cards and panels are.
    Corners {
        Square = "square", "Square";
        Subtle = "subtle", "Subtle";
        Round = "round", "Round";
    } default Subtle
}

choice! {
    /// The colors of the app. System follows the window appearance of the system.
    ColorTheme {
        System = "system", "System";
        Dark = "dark", "Dark";
        Light = "light", "Light";
    } default Dark
}

choice! {
    /// Whether animations run. System follows the reduced-motion setting of the system.
    Motion {
        System = "system", "System";
        Reduced = "reduced", "Reduced";
        Full = "full", "Full";
    } default System
}

impl Global for ColorTheme {}
impl Global for Motion {}

impl ColorTheme {
    pub fn get(cx: &App) -> ColorTheme {
        cx.try_global::<ColorTheme>().copied().unwrap_or_default()
    }

    /// The palette that this choice gives now. System reads the window appearance.
    pub fn palette(self, cx: &App) -> Palette {
        match self {
            ColorTheme::Dark => Palette::DARK,
            ColorTheme::Light => Palette::LIGHT,
            ColorTheme::System => match cx.window_appearance() {
                WindowAppearance::Light | WindowAppearance::VibrantLight => Palette::LIGHT,
                WindowAppearance::Dark | WindowAppearance::VibrantDark => Palette::DARK,
            },
        }
    }

    /// Makes `self` the color theme of the app. It asks one frame of each window, and only when
    /// the palette or the choice changes, so the idle app draws nothing.
    pub fn apply(self, cx: &mut App) {
        let palette = self.palette(cx);
        let same_choice = ColorTheme::get(cx) == self;
        if same_choice && current_palette() == palette {
            return;
        }
        cx.set_global(self);
        if current_palette() == palette {
            return;
        }
        set_palette(palette);
        theme::apply(
            cx,
            if palette == Palette::LIGHT {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            },
        );
        theme::apply_metrics(cx);
        cx.refresh_windows();
    }
}

impl Motion {
    pub fn get(cx: &App) -> Motion {
        cx.try_global::<Motion>().copied().unwrap_or_default()
    }

    pub fn apply(self, cx: &mut App) {
        cx.set_global(self);
        match self {
            Motion::Reduced => cx.set_reduce_motion(true),
            Motion::Full => cx.set_reduce_motion(false),
            Motion::System => gpui_kit::base::apply_system_reduce_motion(cx),
        }
    }
}

/// The steps of the text offset, in px. Each step adds to each size of `size::xs` to `xl`.
pub const TEXT_OFFSETS: std::ops::RangeInclusive<i8> = -2..=4;

/// The text offset of a word of the old `text` key: small, medium or large.
pub fn text_offset_of_size(word: &str) -> Option<i8> {
    match word {
        "small" => Some(-1),
        "medium" => Some(0),
        "large" => Some(2),
        _ => None,
    }
}

/// The three choices together. The default gives the look that the app had before the setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Appearance {
    pub density: Density,
    pub corners: Corners,
    /// One of [`TEXT_OFFSETS`]: the px added to each text size, and that moves the icons.
    pub text_offset: i8,
}

impl Global for Appearance {}

/// The height of the ink of one line of text, in em: from the top of an ascender to the bottom
/// of a descender. Inter measures 0.94 em from "l" to "g", and 1.16 em with an accented capital.
/// 1.1 em keeps a margin for the plain letters. The fit tests use it,
/// because the test platform of GPUI has no real font to measure.
pub const INK_EM: f32 = 1.1;

thread_local! {
    static CURRENT: Cell<Appearance> = const {
        Cell::new(Appearance {
            density: Density::Normal,
            corners: Corners::Subtle,
            text_offset: 0,
        })
    };
}

/// The appearance in effect on this thread. Before the first [`Appearance::apply`], the default.
pub fn current() -> Appearance {
    CURRENT.with(Cell::get)
}

/// Sets the look of this thread with no app, for the tests of the tokens.
#[cfg(test)]
pub(crate) fn set_for_test(a: Appearance) {
    CURRENT.with(|c| c.set(a));
}

impl Appearance {
    /// The 63 combinations: 3 densities, 3 corners and 7 text offsets.
    pub fn all() -> impl Iterator<Item = Appearance> {
        Density::ALL.into_iter().flat_map(|density| {
            Corners::ALL.into_iter().flat_map(move |corners| {
                TEXT_OFFSETS.map(move |text_offset| Appearance {
                    density,
                    corners,
                    text_offset,
                })
            })
        })
    }

    /// The appearance in the global of `cx`.
    pub fn get(cx: &App) -> Appearance {
        cx.try_global::<Appearance>().copied().unwrap_or_default()
    }

    /// Makes `self` the look of the app. It asks one frame of each window, and only when
    /// the look changes, so the idle app draws nothing.
    pub fn apply(self, cx: &mut App) {
        if Appearance::get(cx) == self && current() == self {
            return;
        }
        CURRENT.with(|c| c.set(self));
        cx.set_global(self);
        theme::apply_metrics(cx);
        cx.refresh_windows();
    }
}

/// The family of the text of the app, and the family of the terminals and the ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fonts {
    pub ui: SharedString,
    pub mono: SharedString,
}

impl Default for Fonts {
    fn default() -> Self {
        Fonts {
            ui: crate::tokens::font::UI_DEFAULT.into(),
            mono: crate::tokens::font::MONO_DEFAULT.into(),
        }
    }
}

impl Global for Fonts {}

thread_local! {
    static FONTS: RefCell<Option<Fonts>> = const { RefCell::new(None) };
}

/// The fonts in effect on this thread. Before the first [`Fonts::apply`], the default.
pub fn current_fonts() -> Fonts {
    FONTS.with_borrow(|f| f.clone()).unwrap_or_default()
}

impl Fonts {
    pub fn get(cx: &App) -> Fonts {
        cx.try_global::<Fonts>().cloned().unwrap_or_default()
    }

    /// Makes `self` the fonts of the app. Each window draws again, and only when a family
    /// changes, so the idle app draws nothing. The caller checks that the text system knows
    /// each family: GPUI stops at the first line in a family that it cannot find.
    pub fn apply(self, cx: &mut App) {
        if Fonts::get(cx) == self && current_fonts() == self {
            return;
        }
        FONTS.with_borrow_mut(|f| *f = Some(self.clone()));
        cx.set_global(self);
        theme::apply_fonts(cx);
        cx.refresh_windows();
    }
}

choice! {
    /// Whether the cursor of a terminal pane blinks. Program follows what the program asks.
    CursorBlink {
        Program = "program", "As the program asks";
        On = "on", "Blink";
        Off = "off", "Steady";
    } default Program
}

choice! {
    /// The 16 colors of the terminal panes.
    TerminalPalette {
        AgentsiDark = "agentsi-dark", "AgentSI dark";
        AgentsiLight = "agentsi-light", "AgentSI light";
        FollowTheme = "follow-theme", "Follow the color theme";
    } default FollowTheme
}

/// The shape of the cursor of a terminal pane. Program follows what the program asks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorStyle {
    #[default]
    Program,
    Block,
    Bar,
    Underline,
}

impl CursorStyle {
    pub const ALL: [CursorStyle; 4] = [
        CursorStyle::Program,
        CursorStyle::Block,
        CursorStyle::Bar,
        CursorStyle::Underline,
    ];

    /// The word in `terminal.toml`.
    pub const fn key(self) -> &'static str {
        match self {
            CursorStyle::Program => "program",
            CursorStyle::Block => "block",
            CursorStyle::Bar => "bar",
            CursorStyle::Underline => "underline",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            CursorStyle::Program => "As the program asks",
            CursorStyle::Block => "Block",
            CursorStyle::Bar => "Bar",
            CursorStyle::Underline => "Underline",
        }
    }

    pub fn from_key(key: &str) -> Option<CursorStyle> {
        CursorStyle::ALL.into_iter().find(|c| c.key() == key)
    }
}

/// The values of the Terminal section of the settings. The terminal tokens read them.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalSettings {
    /// The px of the text of a pane: [`TerminalSettings::FONT_SIZES`].
    pub font_size: u8,
    /// The height of a row as a factor of the font size: [`TerminalSettings::LINE_HEIGHTS`],
    /// in steps of 0.05.
    pub line_height: f32,
    pub cursor_style: CursorStyle,
    pub cursor_blink: CursorBlink,
    /// The most memory of the scrollback of a new pane, in MB (10^6 bytes).
    pub scrollback_mb: u16,
    /// The program of a new shell. Empty means `$SHELL`.
    pub shell: SharedString,
    pub palette: TerminalPalette,
    /// A selection with the mouse also goes to the clipboard, not only to the primary selection.
    pub copy_on_select: bool,
    /// Option sends Alt (ESC and the key) on macOS. Other systems always send Alt.
    pub option_as_meta: bool,
}

impl TerminalSettings {
    pub const FONT_SIZES: std::ops::RangeInclusive<u8> = 9..=24;
    pub const LINE_HEIGHTS: std::ops::RangeInclusive<f32> = 1.0..=1.6;
    pub const SCROLLBACK_MB: std::ops::RangeInclusive<u16> = 1..=100;
}

impl Default for TerminalSettings {
    fn default() -> Self {
        TerminalSettings {
            font_size: 13,
            line_height: 1.25,
            cursor_style: CursorStyle::Program,
            cursor_blink: CursorBlink::Program,
            scrollback_mb: 10,
            shell: SharedString::default(),
            palette: TerminalPalette::FollowTheme,
            copy_on_select: false,
            option_as_meta: true,
        }
    }
}

impl Global for TerminalSettings {}

thread_local! {
    static TERMINAL: RefCell<Option<TerminalSettings>> = const { RefCell::new(None) };
}

/// Calls `f` with the terminal settings in effect on this thread, with no copy.
pub fn with_terminal<R>(f: impl FnOnce(&TerminalSettings) -> R) -> R {
    TERMINAL.with_borrow(|t| match t {
        Some(t) => f(t),
        None => f(&TerminalSettings::default()),
    })
}

impl TerminalSettings {
    pub fn get(cx: &App) -> TerminalSettings {
        cx.try_global::<TerminalSettings>()
            .cloned()
            .unwrap_or_default()
    }

    /// Makes `self` the terminal settings of the app. Each window draws again, and only when a
    /// value changes, so the idle app draws nothing. A pane reads the new values in that draw.
    pub fn apply(self, cx: &mut App) {
        if TerminalSettings::get(cx) == self && with_terminal(|t| *t == self) {
            return;
        }
        TERMINAL.with_borrow_mut(|t| *t = Some(self.clone()));
        cx.set_global(self);
        theme::apply_metrics(cx);
        cx.refresh_windows();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;

    #[gpui::test]
    fn the_same_appearance_again_asks_for_no_frame_and_a_new_one_asks_once(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(crate::init);
        let seen = Rc::new(Cell::new(0));
        let s = seen.clone();
        let _sub = cx.update(|cx| cx.observe_global::<Appearance>(move |_| s.set(s.get() + 1)));
        cx.update(|cx| Appearance::default().apply(cx));
        assert_eq!(seen.get(), 0, "the default is in effect already");
        let large = Appearance {
            text_offset: 2,
            ..Appearance::default()
        };
        cx.update(|cx| large.apply(cx));
        cx.update(|cx| large.apply(cx));
        assert_eq!(seen.get(), 1);
        assert_eq!(current(), large);
        assert_eq!(cx.read(Appearance::get), large);
    }

    #[gpui::test]
    fn the_terminal_settings_ask_a_frame_only_for_a_change_and_pick_the_palette(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::tokens::{Palette, size, terminal};
        cx.update(crate::init);
        let seen = Rc::new(Cell::new(0));
        let s = seen.clone();
        let _sub =
            cx.update(|cx| cx.observe_global::<TerminalSettings>(move |_| s.set(s.get() + 1)));
        cx.update(|cx| TerminalSettings::default().apply(cx));
        assert_eq!(seen.get(), 0, "the default is in effect already");
        assert_eq!(terminal::colors(), terminal::Colors::DARK, "the dark theme");
        let light = TerminalSettings {
            palette: TerminalPalette::AgentsiLight,
            font_size: 16,
            ..TerminalSettings::default()
        };
        cx.update(|cx| light.clone().apply(cx));
        cx.update(|cx| light.clone().apply(cx));
        assert_eq!(seen.get(), 1);
        assert_eq!(terminal::colors(), terminal::Colors::LIGHT);
        assert_eq!(size::mono(), gpui::px(16.));
        // Follow the theme: the light color theme gives the light palette.
        cx.update(|cx| TerminalSettings::default().apply(cx));
        cx.update(|cx| ColorTheme::Light.apply(cx));
        assert_eq!(terminal::colors(), terminal::Colors::LIGHT);
        cx.update(|cx| ColorTheme::Dark.apply(cx));
        assert_eq!(terminal::colors(), terminal::Colors::DARK);
        assert_eq!(crate::tokens::current_palette(), Palette::DARK);
    }
}
