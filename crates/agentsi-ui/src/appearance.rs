//! The three choices of the Appearance setting: density, corners and text size.
//!
//! The choice lives in the gpui global [`Appearance`]. The token functions `space`,
//! `size::*`, `radius::*` and `control::*` read it through [`current`], with no `cx`, so a
//! helper that builds an element needs no extra argument. [`Appearance::apply`] is the only
//! writer. It sets the global and the copy that `current` reads in the same call.
//! GPUI draws all views on the main thread, so the copy is thread-local. Each test thread
//! then has its own.
use std::cell::Cell;

use gpui_kit::component::ThemeMode;
use gpui_kit::{App, Global, WindowAppearance};

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
    /// The size of the text and of the icons of controls. A terminal pane keeps its own.
    TextSize {
        Small = "small", "Small";
        Medium = "medium", "Medium";
        Large = "large", "Large";
    } default Medium
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

/// The three choices together. The default gives the look that the app had before the setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Appearance {
    pub density: Density,
    pub corners: Corners,
    pub text: TextSize,
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
            text: TextSize::Medium,
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
    /// The 27 combinations.
    pub fn all() -> impl Iterator<Item = Appearance> {
        Density::ALL.into_iter().flat_map(|density| {
            Corners::ALL.into_iter().flat_map(move |corners| {
                TextSize::ALL.into_iter().map(move |text| Appearance {
                    density,
                    corners,
                    text,
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
            text: TextSize::Large,
            ..Appearance::default()
        };
        cx.update(|cx| large.apply(cx));
        cx.update(|cx| large.apply(cx));
        assert_eq!(seen.get(), 1);
        assert_eq!(current(), large);
        assert_eq!(cx.read(Appearance::get), large);
    }
}
