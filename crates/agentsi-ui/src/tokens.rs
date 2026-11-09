//! Each token of the design: color, type, space, radius and motion.
//!
//! Screens read these values and never write a color or a size by hand.
//! The colors are sRGB hex values, so the contrast test can read them.
use std::cell::Cell;

use gpui_kit::{Hsla, Pixels, Rgba, px};

use crate::appearance::{Corners, Density, TextSize, current};

/// A color token, `0xRRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    pub const fn rgba(self) -> Rgba {
        let c = self.0;
        Rgba {
            r: ((c >> 16) & 0xff) as f32 / 255.0,
            g: ((c >> 8) & 0xff) as f32 / 255.0,
            b: (c & 0xff) as f32 / 255.0,
            a: 1.0,
        }
    }

    pub fn hsla(self) -> Hsla {
        self.rgba().into()
    }

    /// This color at `alpha` (0 to 1).
    pub fn alpha(self, alpha: f32) -> Hsla {
        let mut c = self.hsla();
        c.a = alpha;
        c
    }

    /// The opaque color that `self` at `alpha` gives over `below`.
    pub const fn over(self, below: Color, alpha: f32) -> Color {
        const fn channel(top: u32, below: u32, shift: u32, alpha: f32) -> u32 {
            let a = ((top >> shift) & 0xff) as f32;
            let b = ((below >> shift) & 0xff) as f32;
            let v = a * alpha + b * (1.0 - alpha) + 0.5;
            (v as u32) << shift
        }
        Color(
            channel(self.0, below.0, 16, alpha)
                | channel(self.0, below.0, 8, alpha)
                | channel(self.0, below.0, 0, alpha),
        )
    }
}

impl From<Color> for Hsla {
    fn from(c: Color) -> Self {
        c.hsla()
    }
}

/// So a screen writes `.bg(surface::panel())` with no conversion.
impl From<Color> for gpui_kit::Fill {
    fn from(c: Color) -> Self {
        c.hsla().into()
    }
}

/// The color values of one color theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub surface_base: Color,
    pub surface_panel: Color,
    pub surface_raised: Color,
    pub surface_overlay: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub line_border: Color,
    pub line_strong: Color,
    pub accent_base: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    pub accent_on: Color,
    /// Opacity of `accent::subtle()` over the panel.
    pub accent_subtle_alpha: f32,
    pub status_ok: Color,
    pub status_warn: Color,
    pub status_danger: Color,
    pub status_info: Color,
    pub status_off: Color,
    pub switch_thumb: Color,
    /// The label colors, in the order of `label::Hue::ALL`.
    pub hues: [Color; 8],
}

impl Palette {
    /// The colors of the app before the color theme existed.
    pub const DARK: Palette = Palette {
        surface_base: Color(0x0b0c0f),
        surface_panel: Color(0x101115),
        surface_raised: Color(0x16171c),
        surface_overlay: Color(0x1e2027),
        text_primary: Color(0xe8e9ed),
        text_secondary: Color(0xa4a8b2),
        text_tertiary: Color(0x8a8f9a),
        line_border: Color(0x23252c),
        line_strong: Color(0x31343d),
        accent_base: Color(0x8b7cf6),
        accent_hover: Color(0x9d90f8),
        accent_pressed: Color(0x7a6af0),
        accent_on: Color(0x0b0c0f),
        accent_subtle_alpha: 0.16,
        status_ok: Color(0x4ade80),
        status_warn: Color(0xf2b84b),
        status_danger: Color(0xf87171),
        status_info: Color(0x60a5fa),
        status_off: Color(0x9a9faa),
        switch_thumb: Color(0xe8e9ed),
        hues: [
            Color(0xfb7185),
            Color(0xfb923c),
            Color(0xfbbf24),
            Color(0xa3e635),
            Color(0x2dd4bf),
            Color(0x38bdf8),
            Color(0xc084fc),
            Color(0xf472b6),
        ],
    };

    /// The light color theme. Each value passes the contrast tests of this file.
    pub const LIGHT: Palette = Palette {
        surface_base: Color(0xeef0f3),
        surface_panel: Color(0xf7f8fa),
        surface_raised: Color(0xffffff),
        surface_overlay: Color(0xe8eaee),
        text_primary: Color(0x15171c),
        text_secondary: Color(0x484d59),
        text_tertiary: Color(0x5c6270),
        line_border: Color(0xdcdfe5),
        line_strong: Color(0xc3c7cf),
        accent_base: Color(0x6248d8),
        accent_hover: Color(0x5539cc),
        accent_pressed: Color(0x4a2fb8),
        accent_on: Color(0xffffff),
        accent_subtle_alpha: 0.12,
        status_ok: Color(0x146c33),
        status_warn: Color(0x8a4f00),
        status_danger: Color(0xb01f26),
        status_info: Color(0x1a54ad),
        status_off: Color(0x575d69),
        switch_thumb: Color(0xffffff),
        hues: [
            Color(0xa50f34),
            Color(0x9c370a),
            Color(0x7a5000),
            Color(0x3f6110),
            Color(0x0c6159),
            Color(0x075a8b),
            Color(0x7c2bc4),
            Color(0xa0164f),
        ],
    };
}

thread_local! {
    static PALETTE: Cell<Palette> = const { Cell::new(Palette::DARK) };
}

/// The palette in effect on this thread. Before the first [`set_palette`], the dark one.
pub fn current_palette() -> Palette {
    PALETTE.with(Cell::get)
}

/// Makes `palette` the one that the color functions read. GPUI draws on the main thread, so
/// the copy is thread-local, as `appearance::current` is.
pub fn set_palette(palette: Palette) {
    PALETTE.with(|c| c.set(palette));
}

/// Sets the palette of this thread with no app, for the tests of the tokens.
#[cfg(test)]
pub(crate) fn set_palette_for_test(palette: Palette) {
    set_palette(palette);
}

/// Near-black surfaces, from the deepest to the highest.
pub mod surface {
    use super::{Color, current_palette};
    /// Window behind the panels, sidebar, terminal well.
    /// Main content area.
    /// Cards and raised rows.
    /// Popovers, inputs, the hover fill of a raised element.
    pub fn base() -> Color {
        current_palette().surface_base
    }
    pub fn panel() -> Color {
        current_palette().surface_panel
    }
    pub fn raised() -> Color {
        current_palette().surface_raised
    }
    pub fn overlay() -> Color {
        current_palette().surface_overlay
    }
}

/// State layers. A pressed control is not a surface level: it is its fill with a
/// light layer on top.
pub mod state {
    use super::{Color, text};
    /// Opacity of the press layer.
    pub const PRESS_ALPHA: f32 = 0.04;
    /// Opacity of a disabled component. WCAG 1.4.3 exempts the text of a disabled control.
    pub const DISABLED_ALPHA: f32 = 0.45;
    /// `fill` with the press layer on top.
    pub fn pressed(fill: Color) -> Color {
        text::primary().over(fill, PRESS_ALPHA)
    }
}

/// The colors of a switch: the track when it is off, and the thumb. The track of a switch that is
/// on uses the accent.
pub mod switch {
    use super::{Color, current_palette, line, state, text};
    /// Opacity of the hover layer on the track.
    pub const HOVER_ALPHA: f32 = 0.08;
    pub fn track() -> Color {
        line::strong()
    }
    pub fn track_hover() -> Color {
        text::primary().over(track(), HOVER_ALPHA)
    }
    pub fn track_pressed() -> Color {
        state::pressed(track_hover())
    }
    pub fn thumb() -> Color {
        current_palette().switch_thumb
    }
}

/// Text, from the strongest to the weakest. Each passes 4.5:1 on each surface.
pub mod text {
    use super::{Color, current_palette};
    pub fn primary() -> Color {
        current_palette().text_primary
    }
    pub fn secondary() -> Color {
        current_palette().text_secondary
    }
    pub fn tertiary() -> Color {
        current_palette().text_tertiary
    }
}

/// Lines and the focus ring.
pub mod line {
    use super::{Color, current_palette};
    /// Hairline between areas.
    /// Border of controls and of hovered cards.
    /// The 2 px focus ring.
    pub fn border() -> Color {
        current_palette().line_border
    }
    pub fn strong() -> Color {
        current_palette().line_strong
    }
    pub fn focus() -> Color {
        current_palette().accent_base
    }
}

/// The one violet accent.
pub mod accent {
    use super::{Color, current_palette};
    /// Text and icons on an accent fill.
    /// A selected row or tab: the accent at 16 % over the panel.
    pub fn subtle() -> Color {
        base().over(
            super::surface::panel(),
            current_palette().accent_subtle_alpha,
        )
    }
    /// Opacity of the accent border of a hovered card.
    pub const HOVER_BORDER_ALPHA: f32 = 0.6;
    pub fn base() -> Color {
        current_palette().accent_base
    }
    pub fn hover() -> Color {
        current_palette().accent_hover
    }
    pub fn pressed() -> Color {
        current_palette().accent_pressed
    }
    pub fn on() -> Color {
        current_palette().accent_on
    }
}

/// The status colors. Each is a text color, and each has a tint fill.
pub mod status {
    use super::{Color, current_palette};
    /// Opacity of a status tint over a raised surface.
    pub const TINT_ALPHA: f32 = 0.14;
    /// Opacity of the status border of an attention marker.
    pub const BORDER_ALPHA: f32 = 0.35;
    /// Opacity of the danger tint of a hovered danger button.
    pub const DANGER_HOVER_ALPHA: f32 = 0.22;
    /// Opacity of the danger tint of a pressed danger button. It is less than the
    /// hover tint, because danger text on a 30 % tint fails 4.5:1.
    pub const DANGER_PRESS_ALPHA: f32 = 0.17;
    pub fn ok() -> Color {
        current_palette().status_ok
    }
    pub fn warn() -> Color {
        current_palette().status_warn
    }
    pub fn danger() -> Color {
        current_palette().status_danger
    }
    pub fn info() -> Color {
        current_palette().status_info
    }
    pub fn off() -> Color {
        current_palette().status_off
    }
}

/// A program state or a host state, with its color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    Warn,
    Danger,
    Info,
    Off,
}

impl Status {
    pub const ALL: [Status; 5] = [
        Status::Ok,
        Status::Warn,
        Status::Danger,
        Status::Info,
        Status::Off,
    ];

    pub fn color(self) -> Color {
        match self {
            Status::Ok => status::ok(),
            Status::Warn => status::warn(),
            Status::Danger => status::danger(),
            Status::Info => status::info(),
            Status::Off => status::off(),
        }
    }

    /// A short label for the gallery and the logs.
    pub const fn label(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Warn => "warn",
            Status::Danger => "danger",
            Status::Info => "info",
            Status::Off => "off",
        }
    }

    /// The opaque tint fill of this status over a raised surface.
    pub fn tint(self) -> Color {
        self.color().over(surface::raised(), status::TINT_ALPHA)
    }
}

/// The one state that the app shows for a pane, a meeting role, a story session or an agent.
/// `rank()` keeps the order of the state map: when two states apply, the lower rank wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WorkState {
    Blocked,
    Failed,
    Review,
    Repairing,
    Speaking,
    Listening,
    Working,
    Done,
    Off,
}

impl WorkState {
    pub const ALL: [WorkState; 9] = [
        WorkState::Blocked,
        WorkState::Failed,
        WorkState::Review,
        WorkState::Repairing,
        WorkState::Speaking,
        WorkState::Listening,
        WorkState::Working,
        WorkState::Done,
        WorkState::Off,
    ];

    /// 0 for Blocked, up to 8 for Off.
    pub const fn rank(self) -> u8 {
        self as u8
    }

    /// The edge, the icon and the text color of this state.
    pub fn color(self) -> Color {
        match self {
            WorkState::Blocked | WorkState::Repairing => status::warn(),
            WorkState::Failed => status::danger(),
            WorkState::Review | WorkState::Listening => accent::base(),
            WorkState::Speaking | WorkState::Done => status::ok(),
            WorkState::Working => status::info(),
            WorkState::Off => status::off(),
        }
    }

    /// The opaque fill of this state over a raised surface.
    pub fn tint(self) -> Color {
        self.color().over(surface::raised(), status::TINT_ALPHA)
    }

    /// The word of this state. A screen may add a reason after it.
    pub const fn word(self) -> &'static str {
        match self {
            WorkState::Blocked => "Blocked",
            WorkState::Failed => "Failed",
            WorkState::Review => "Review",
            WorkState::Repairing => "Repairing the minutes",
            WorkState::Speaking => "Speaking",
            WorkState::Listening => "Listening",
            WorkState::Working => "Working",
            WorkState::Done => "Done",
            WorkState::Off => "Off",
        }
    }

    /// The status of the plain dot for this state.
    pub const fn status(self) -> Status {
        match self {
            WorkState::Blocked | WorkState::Repairing => Status::Warn,
            WorkState::Failed => Status::Danger,
            WorkState::Review | WorkState::Listening | WorkState::Working => Status::Info,
            WorkState::Speaking | WorkState::Done => Status::Ok,
            WorkState::Off => Status::Off,
        }
    }

    /// How many times the glow pulses after it fades in. Zero means this state has no glow.
    pub const fn pulses(self) -> usize {
        match self {
            WorkState::Blocked => motion::PULSES,
            WorkState::Failed | WorkState::Review => 1,
            _ => 0,
        }
    }
}

/// The fill of a danger button when hovered.
pub fn danger_fill_hover() -> Color {
    status::danger().over(surface::raised(), status::DANGER_HOVER_ALPHA)
}

/// The fill of a danger button when pressed.
pub fn danger_fill_press() -> Color {
    status::danger().over(surface::raised(), status::DANGER_PRESS_ALPHA)
}

/// Colors of a terminal pane. `ANSI` holds the 16 colors, normal then bright.
pub mod terminal {
    use super::Color;
    use gpui_kit::{Pixels, px};
    pub const BACKGROUND: Color = Color(0x0b0c0f);
    pub const FOREGROUND: Color = Color(0xdcdde2);
    pub const CURSOR: Color = Color(0x8b7cf6);
    pub const SELECTION: Color = Color(0x2f2a52);
    pub const ANSI: [Color; 16] = [
        Color(0x1e2027),
        Color(0xf87171),
        Color(0x4ade80),
        Color(0xf2b84b),
        Color(0x60a5fa),
        Color(0xb69cf8),
        Color(0x5ccfe6),
        Color(0xc8cad0),
        Color(0x5c616d),
        Color(0xfca5a5),
        Color(0x86efac),
        Color(0xfcd38d),
        Color(0x93c5fd),
        Color(0xd2c2fb),
        Color(0x99e3f0),
        Color(0xf4f5f7),
    ];
    /// The height of a row of cells, as a factor of `size::MONO`.
    pub const LINE_HEIGHT: f32 = 1.25;
    /// The space between the edge of a pane and its cells. It does not follow the density.
    pub const PADDING: Pixels = super::grid(2);
    /// The width of a bar cursor, and the height of an underline cursor.
    pub const CURSOR_BAR: Pixels = px(2.);
    /// The opacity of faint text (SGR 2).
    pub const FAINT_ALPHA: f32 = 0.6;
}

/// Font families. `theme::load_fonts` registers the bundled files.
pub mod font {
    pub const UI: &str = "Inter";
    pub const MONO: &str = "JetBrains Mono";
}

/// Type sizes, in px. The text size of the appearance moves `XS` to `XL`, and `MONO` stays.
pub mod size {
    use super::{TextSize, current};
    use gpui_kit::{Pixels, px};

    /// The size of the step `base` px at medium, for the text size in effect.
    fn step(base: f32) -> Pixels {
        px(match current().text {
            TextSize::Small => base - 1.,
            TextSize::Medium => base,
            TextSize::Large => base + 2.,
        })
    }
    /// Meta text, key hints, tags.
    pub fn xs() -> Pixels {
        step(12.)
    }
    /// Dense lists and secondary body.
    pub fn sm() -> Pixels {
        step(13.)
    }
    /// Body text.
    pub fn base() -> Pixels {
        step(14.)
    }
    /// Card titles.
    pub fn md() -> Pixels {
        step(16.)
    }
    /// Section titles.
    pub fn lg() -> Pixels {
        step(20.)
    }
    /// Page titles.
    pub fn xl() -> Pixels {
        step(28.)
    }
    /// Terminal cells.
    pub const MONO: Pixels = px(13.);
}

/// Font weights.
pub mod weight {
    use gpui_kit::FontWeight;
    pub const REGULAR: FontWeight = FontWeight::NORMAL;
    pub const MEDIUM: FontWeight = FontWeight::MEDIUM;
    pub const SEMIBOLD: FontWeight = FontWeight::SEMIBOLD;
}

/// One step of the grid: `space(3)` is 12 px at normal density, 9 px at compact and 15 px at
/// comfortable.
pub fn space(steps: u16) -> Pixels {
    px(match current().density {
        Density::Compact => 3.,
        Density::Normal => 4.,
        Density::Comfortable => 5.,
    } * steps as f32)
}

/// One step of the fixed 4 px grid, for what does not follow the density: a dot, a switch,
/// the pill and the padding of a terminal.
pub const fn grid(steps: u16) -> Pixels {
    px(4. * steps as f32)
}

/// Corner radii. The corners of the appearance move `SM` to `LG`.
pub mod radius {
    use super::{Corners, current};
    use gpui_kit::{Pixels, px};

    fn step(square: f32, subtle: f32, round: f32) -> Pixels {
        px(match current().corners {
            Corners::Square => square,
            Corners::Subtle => subtle,
            Corners::Round => round,
        })
    }
    /// Tags, key hints, small controls.
    pub fn sm() -> Pixels {
        step(0., 6., 8.)
    }
    /// Buttons, inputs, rows.
    pub fn md() -> Pixels {
        step(2., 8., 12.)
    }
    /// Cards, panels, dialogs.
    pub fn lg() -> Pixels {
        step(2., 12., 16.)
    }
    pub const FULL: Pixels = px(9999.);
}

/// The 8 colors that the owner gives to a product, a pane or a product group. Each one reaches
/// 4.5:1 as text on every surface and on its own tint, so a label can use it as a text color.
pub mod label {
    use super::{Color, current_palette, status, surface};

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub enum Hue {
        Rose,
        Orange,
        Amber,
        Lime,
        Teal,
        Sky,
        Violet,
        Pink,
    }

    impl Hue {
        pub const ALL: [Hue; 8] = [
            Hue::Rose,
            Hue::Orange,
            Hue::Amber,
            Hue::Lime,
            Hue::Teal,
            Hue::Sky,
            Hue::Violet,
            Hue::Pink,
        ];

        pub fn color(self) -> Color {
            current_palette().hues[self as usize]
        }

        /// The word in a file and in a list: `red`, `teal`.
        pub const fn key(self) -> &'static str {
            match self {
                Hue::Rose => "rose",
                Hue::Orange => "orange",
                Hue::Amber => "amber",
                Hue::Lime => "lime",
                Hue::Teal => "teal",
                Hue::Sky => "sky",
                Hue::Violet => "violet",
                Hue::Pink => "pink",
            }
        }

        pub const fn name(self) -> &'static str {
            match self {
                Hue::Rose => "Rose",
                Hue::Orange => "Orange",
                Hue::Amber => "Amber",
                Hue::Lime => "Lime",
                Hue::Teal => "Teal",
                Hue::Sky => "Sky",
                Hue::Violet => "Violet",
                Hue::Pink => "Pink",
            }
        }

        pub fn from_key(key: &str) -> Option<Hue> {
            Hue::ALL.into_iter().find(|h| h.key() == key)
        }

        /// The fill behind a label in this color, over `below`.
        pub fn tint_over(self, below: Color) -> Color {
            self.color().over(below, status::TINT_ALPHA)
        }

        /// The fill of a label on the panel surface.
        pub fn tint(self) -> Color {
            self.tint_over(surface::panel())
        }
    }
}

/// Fixed sizes of controls and the shell.
pub mod control {
    use super::{Density, TextSize, current};
    use gpui_kit::{Pixels, px};

    fn by_density(compact: f32, normal: f32, comfortable: f32) -> Pixels {
        px(match current().density {
            Density::Compact => compact,
            Density::Normal => normal,
            Density::Comfortable => comfortable,
        })
    }
    fn by_text(small: f32, medium: f32, large: f32) -> Pixels {
        px(match current().text {
            TextSize::Small => small,
            TextSize::Medium => medium,
            TextSize::Large => large,
        })
    }
    /// The height of a small button.
    pub fn height_sm() -> Pixels {
        by_density(24., 28., 32.)
    }
    /// The height of a normal button, an input and a tab.
    pub fn height_md() -> Pixels {
        by_density(28., 32., 36.)
    }
    /// The height of a list row.
    pub fn row() -> Pixels {
        by_density(32., 44., 52.)
    }
    /// The vertical padding of a row of the sidebar and of the tree.
    pub fn row_pad() -> Pixels {
        by_density(4., 6., 8.)
    }
    /// The icon next to a small text.
    pub fn icon_sm() -> Pixels {
        by_text(12., 14., 16.)
    }
    /// The icon next to a normal text.
    pub fn icon_md() -> Pixels {
        by_text(14., 16., 18.)
    }
    pub const FOCUS_RING: Pixels = px(2.);
    /// The edge of a glow, and the blur of its shadow.
    pub const GLOW_EDGE: Pixels = px(2.);
    pub const GLOW_BLUR: Pixels = px(8.);
    /// Gap between an icon and its label.
    pub const ICON_GAP: Pixels = px(6.);
    /// Horizontal padding of a small and of a normal button.
    pub const PAD_X_SM: Pixels = px(10.);
    pub const PAD_X_MD: Pixels = px(12.);
    pub const TAG_HEIGHT: Pixels = px(20.);
    pub const KBD_PAD_X: Pixels = px(6.);
    pub const MARKER_HEIGHT: Pixels = px(24.);
    /// The track of a switch, the thumb, and the gap between them.
    pub const SWITCH_WIDTH: Pixels = px(36.);
    pub const SWITCH_HEIGHT: Pixels = px(20.);
    pub const SWITCH_THUMB: Pixels = px(16.);
    pub const SWITCH_INSET: Pixels = px(2.);
    /// The accent bar of a selected list row: width, and inset from the top and the bottom.
    /// The color bar of a product row and of a tab: its width.
    pub const COLOR_BAR: Pixels = px(3.);
    pub const SELECTION_BAR: Pixels = px(2.);
    pub const SELECTION_INSET: Pixels = px(10.);
    /// The widest trailing element of a list row. The title keeps the rest.
    pub const TRAILING_MAX: Pixels = px(160.);
    pub const SIDEBAR_WIDTH: Pixels = px(248.);
    /// The desk conversation: the right column of the front desk.
    pub const DESK_CONVERSATION_WIDTH: Pixels = px(380.);
    /// The widest column of a screen of text: longer lines read badly.
    pub const CONTENT_WIDTH: Pixels = px(880.);
    /// The width of a number column in a table.
    pub const NUM_COLUMN: Pixels = px(96.);
    pub const TOOLBAR_HEIGHT: Pixels = px(48.);
    /// The command palette of the app.
    pub const PALETTE_WIDTH: Pixels = px(560.);
    /// The first size of the app window.
    pub const WINDOW_WIDTH: Pixels = px(1280.);
    pub const WINDOW_HEIGHT: Pixels = px(820.);
    /// The opacity of the base surface behind a palette or a dialog.
    pub const SCRIM_ALPHA: f32 = 0.6;
}

/// Durations. Each transition uses `EASE_OUT`.
pub mod motion {
    use std::time::Duration;
    pub const HOVER: Duration = Duration::from_millis(120);
    pub const PRESS: Duration = Duration::from_millis(120);
    pub const PANEL: Duration = Duration::from_millis(180);
    pub const DIALOG: Duration = Duration::from_millis(260);
    /// Caret blink phase of inputs.
    pub const CARET_BLINK: Duration = Duration::from_millis(530);
    /// After this time with no typing, a caret stops blinking and stays on.
    pub const CARET_IDLE: Duration = Duration::from_secs(5);
    /// The ease-out cubic curve, as cubic-bezier points. It is the default curve
    /// of `gpui_kit::base::motion::Transition`.
    /// One rise and fall of the glow shadow.
    pub const PULSE: Duration = Duration::from_millis(1200);
    /// The pulses of a Blocked glow. The glow of another state pulses once.
    pub const PULSES: usize = 3;
    /// A pulse redraws at most this many times per second.
    pub const PULSE_FPS: f32 = 30.;
    /// The glow fades in during this time.
    pub const GLOW_IN: Duration = Duration::from_millis(180);
    pub const EASE_OUT: (f32, f32, f32, f32) = (0.33, 1.0, 0.68, 1.0);
}

/// WCAG 2.x contrast ratio of two opaque colors.
pub fn contrast(a: Color, b: Color) -> f32 {
    fn luminance(c: Color) -> f32 {
        let ch = |shift: u32| {
            let v = ((c.0 >> shift) & 0xff) as f32 / 255.;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
    }
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Each text color on each fill where the components draw it, in `palette`.
pub fn text_pairs(palette: Palette) -> Vec<(String, Color, Color)> {
    let before = current_palette();
    set_palette(palette);
    let pairs = text_pairs_in_effect();
    set_palette(before);
    pairs
}

fn text_pairs_in_effect() -> Vec<(String, Color, Color)> {
    let surfaces = [
        ("surface::base()", surface::base()),
        ("surface::panel()", surface::panel()),
        ("surface::raised()", surface::raised()),
        ("surface::overlay()", surface::overlay()),
        ("accent::subtle()", accent::subtle()),
    ];
    let mut texts = vec![
        ("text::primary()", text::primary()),
        ("text::secondary()", text::secondary()),
        ("text::tertiary()", text::tertiary()),
        ("accent::base()", accent::base()),
    ];
    for s in Status::ALL {
        texts.push((s.label(), s.color()));
    }
    let mut pairs = Vec::new();
    for (sn, s) in surfaces {
        for &(tn, t) in &texts {
            pairs.push((format!("{tn} on {sn}"), t, s));
        }
    }
    for h in label::Hue::ALL {
        for (sn, s) in [
            ("BASE", surface::base()),
            ("PANEL", surface::panel()),
            ("RAISED", surface::raised()),
            ("OVERLAY", surface::overlay()),
        ] {
            pairs.push((format!("label {} on {sn}", h.key()), h.color(), s));
            pairs.push((
                format!("label {} on its tint over {sn}", h.key()),
                h.color(),
                h.tint_over(s),
            ));
        }
    }
    // Controls draw only the 3 text levels on a pressed fill.
    for (fname, fill) in [
        ("RAISED", surface::raised()),
        ("OVERLAY", surface::overlay()),
    ] {
        for (tn, t) in [
            ("text::primary()", text::primary()),
            ("text::secondary()", text::secondary()),
            ("text::tertiary()", text::tertiary()),
        ] {
            pairs.push((format!("{tn} on pressed {fname}"), t, state::pressed(fill)));
        }
    }
    for fill in [accent::base(), accent::hover(), accent::pressed()] {
        pairs.push(("accent::on() on an accent fill".into(), accent::on(), fill));
    }
    for s in Status::ALL {
        pairs.push((format!("{s:?} on its tint"), s.color(), s.tint()));
        pairs.push((
            format!("text::primary() on the {s:?} tint"),
            text::primary(),
            s.tint(),
        ));
    }
    pairs.push((
        "status::danger() on danger_fill_hover".into(),
        status::danger(),
        danger_fill_hover(),
    ));
    pairs.push((
        "status::danger() on danger_fill_press".into(),
        status::danger(),
        danger_fill_press(),
    ));
    // The terminal keeps its dark colors in both color themes.
    pairs.push((
        "terminal::FOREGROUND".into(),
        terminal::FOREGROUND,
        terminal::BACKGROUND,
    ));
    pairs.push((
        "terminal::FOREGROUND on terminal::SELECTION".into(),
        terminal::FOREGROUND,
        terminal::SELECTION,
    ));
    for (i, &c) in terminal::ANSI.iter().enumerate().skip(1) {
        // Color 0 is black and color 8 is dark grey. Programs use them as fills.
        if i != 8 {
            pairs.push((format!("terminal::ANSI[{i}]"), c, terminal::BACKGROUND));
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_text_and_fill_pair_reaches_4_5_to_1() {
        for (name, palette) in [("dark", Palette::DARK), ("light", Palette::LIGHT)] {
            let failures: Vec<String> = text_pairs(palette)
                .into_iter()
                .filter(|(_, t, f)| contrast(*t, *f) < 4.5)
                .map(|(n, t, f)| {
                    format!("{n}: #{:06x} on #{:06x} is {:.2}", t.0, f.0, contrast(t, f))
                })
                .collect();
            assert!(
                failures.is_empty(),
                "{name} pairs under 4.5:1:\n{}",
                failures.join("\n")
            );
        }
    }

    #[test]
    fn the_8_label_colors_are_distinct_and_reach_4_5_to_1_on_each_surface_and_on_their_tint() {
        for palette in [Palette::DARK, Palette::LIGHT] {
            set_palette_for_test(palette);
            labels_pass();
        }
        set_palette_for_test(Palette::DARK);
    }

    fn labels_pass() {
        use label::Hue;
        let colors: std::collections::BTreeSet<_> = Hue::ALL.iter().map(|h| h.color().0).collect();
        assert_eq!(colors.len(), 8);
        for h in Hue::ALL {
            assert_eq!(Hue::from_key(h.key()), Some(h));
            assert!(
                !status_colors().contains(&h.color().0),
                "{} is a status color",
                h.key()
            );
            for (name, s) in [
                ("BASE", surface::base()),
                ("PANEL", surface::panel()),
                ("RAISED", surface::raised()),
                ("OVERLAY", surface::overlay()),
            ] {
                assert!(contrast(h.color(), s) >= 4.5, "{} on {name}", h.key());
                assert!(
                    contrast(h.color(), h.tint_over(s)) >= 4.5,
                    "{} on its tint over {name}",
                    h.key()
                );
            }
        }
        assert_eq!(Hue::from_key("mauve"), None);
    }

    fn status_colors() -> Vec<u32> {
        Status::ALL.iter().map(|s| s.color().0).collect()
    }

    #[test]
    fn each_work_state_color_reaches_3_to_1_on_each_surface_of_both_palettes() {
        for (name, palette) in [("dark", Palette::DARK), ("light", Palette::LIGHT)] {
            set_palette_for_test(palette);
            for w in WorkState::ALL {
                for (sn, s) in [
                    ("base", surface::base()),
                    ("panel", surface::panel()),
                    ("raised", surface::raised()),
                    ("overlay", surface::overlay()),
                ] {
                    assert!(
                        contrast(w.color(), s) >= 3.0,
                        "{name}: {w:?} on {sn} is {:.2}",
                        contrast(w.color(), s)
                    );
                }
            }
        }
        set_palette_for_test(Palette::DARK);
    }

    #[test]
    fn each_work_state_has_a_word_a_rank_and_the_glow_of_the_state_map() {
        let words: std::collections::BTreeSet<_> =
            WorkState::ALL.iter().map(|w| w.word()).collect();
        assert_eq!(words.len(), 9);
        for (i, w) in WorkState::ALL.into_iter().enumerate() {
            assert_eq!(w.rank() as usize, i);
        }
        assert_eq!(WorkState::Blocked.pulses(), 3);
        assert_eq!(WorkState::Failed.pulses(), 1);
        assert_eq!(WorkState::Review.pulses(), 1);
        assert_eq!(WorkState::Working.pulses(), 0);
    }

    #[test]
    fn contrast_matches_known_values() {
        assert!((contrast(Color(0x000000), Color(0xffffff)) - 21.0).abs() < 0.01);
        assert!((contrast(Color(0x777777), Color(0xffffff)) - 4.48).abs() < 0.01);
    }

    #[test]
    fn over_mixes_in_srgb() {
        assert_eq!(Color(0xffffff).over(Color(0x000000), 0.5), Color(0x808080));
        assert_eq!(accent::base().over(surface::panel(), 1.0), accent::base());
    }

    #[test]
    fn the_color_functions_read_the_palette() {
        assert_eq!(surface::panel(), Color(0x101115));
        assert_eq!(
            accent::subtle(),
            Color(0x8b7cf6).over(Color(0x101115), 0.16)
        );
        let light = Palette {
            surface_panel: Color(0xf7f8fa),
            ..Palette::DARK
        };
        set_palette_for_test(light);
        assert_eq!(surface::panel(), Color(0xf7f8fa));
        set_palette_for_test(Palette::DARK);
        assert_eq!(surface::panel(), Color(0x101115));
    }

    #[test]
    fn the_grid_is_4_px() {
        assert_eq!(space(0), px(0.));
        assert_eq!(space(3), px(12.));
        assert_eq!(grid(3), px(12.));
    }

    use crate::appearance::{Appearance, set_for_test};

    /// The sizes that the Appearance tokens in `size`, `space`, `radius` and `control` give.
    #[derive(Debug, PartialEq)]
    struct Measures {
        sizes: [Pixels; 6],
        space: Pixels,
        radii: [Pixels; 3],
        heights: [Pixels; 3],
        row_pad: Pixels,
        icons: [Pixels; 2],
    }

    fn measures() -> Measures {
        Measures {
            sizes: [
                size::xs(),
                size::sm(),
                size::base(),
                size::md(),
                size::lg(),
                size::xl(),
            ],
            space: space(1),
            radii: [radius::sm(), radius::md(), radius::lg()],
            heights: [control::height_sm(), control::height_md(), control::row()],
            row_pad: control::row_pad(),
            icons: [control::icon_sm(), control::icon_md()],
        }
    }

    fn at(density: Density, corners: Corners, text: TextSize) -> Measures {
        set_for_test(Appearance {
            density,
            corners,
            text,
        });
        let m = measures();
        set_for_test(Appearance::default());
        m
    }

    #[test]
    fn the_default_appearance_gives_the_constants_of_the_app_before_the_setting() {
        // The values of `tokens.rs` at the commit before the Appearance setting.
        assert_eq!(
            measures(),
            Measures {
                sizes: [px(12.), px(13.), px(14.), px(16.), px(20.), px(28.)],
                space: px(4.),
                radii: [px(6.), px(8.), px(12.)],
                heights: [px(28.), px(32.), px(44.)],
                row_pad: px(6.),
                icons: [px(14.), px(16.)],
            }
        );
        assert_eq!(Appearance::default(), Appearance::all().nth(13).unwrap());
    }

    #[test]
    fn each_choice_gives_the_values_of_the_spec() {
        let (n, s, m) = (Density::Normal, Corners::Subtle, TextSize::Medium);
        let d = |d| at(d, s, m);
        assert_eq!(d(Density::Compact).space, px(3.));
        assert_eq!(d(Density::Comfortable).space, px(5.));
        assert_eq!(d(Density::Compact).heights, [px(24.), px(28.), px(32.)]);
        assert_eq!(d(Density::Comfortable).heights, [px(32.), px(36.), px(52.)]);
        assert_eq!(
            (d(Density::Compact).row_pad, d(Density::Comfortable).row_pad),
            (px(4.), px(8.))
        );
        let c = |c| at(n, c, m).radii;
        assert_eq!(c(Corners::Square), [px(0.), px(2.), px(2.)]);
        assert_eq!(c(Corners::Round), [px(8.), px(12.), px(16.)]);
        let t = |t| at(n, s, t);
        assert_eq!(
            t(TextSize::Small).sizes,
            [px(11.), px(12.), px(13.), px(15.), px(19.), px(27.)]
        );
        assert_eq!(
            t(TextSize::Large).sizes,
            [px(14.), px(15.), px(16.), px(18.), px(22.), px(30.)]
        );
        assert_eq!(t(TextSize::Small).icons, [px(12.), px(14.)]);
        assert_eq!(t(TextSize::Large).icons, [px(16.), px(18.)]);
    }

    #[test]
    fn only_the_listed_groups_change_with_a_choice() {
        let base = at(Density::Normal, Corners::Subtle, TextSize::Medium);
        let denser = at(Density::Compact, Corners::Subtle, TextSize::Medium);
        let rounder = at(Density::Normal, Corners::Round, TextSize::Medium);
        let larger = at(Density::Normal, Corners::Subtle, TextSize::Large);
        assert_eq!(
            (denser.sizes, denser.radii, denser.icons),
            (base.sizes, base.radii, base.icons)
        );
        assert_eq!(
            (rounder.sizes, rounder.space, rounder.heights, rounder.icons),
            (base.sizes, base.space, base.heights, base.icons)
        );
        assert_eq!(
            (larger.space, larger.radii, larger.heights, larger.row_pad),
            (base.space, base.radii, base.heights, base.row_pad)
        );
    }

    #[test]
    fn the_fixed_heights_hold_their_text_in_each_of_the_27_combinations() {
        use crate::appearance::INK_EM;
        let ink = |p: Pixels| f32::from(p) * INK_EM;
        let mut failures = Vec::new();
        for a in Appearance::all() {
            set_for_test(a);
            let mut need = |what: &str, text: f32, height: Pixels| {
                if text > f32::from(height) {
                    failures.push(format!("{a:?}: {what} needs {text:.1} px in {height:?}"));
                }
            };
            need("small button", ink(size::sm()), control::height_sm());
            need("normal button", ink(size::base()), control::height_md());
            need(
                "icon of a small button",
                f32::from(control::icon_sm()),
                control::height_sm(),
            );
            need(
                "icon of a normal button",
                f32::from(control::icon_md()),
                control::height_md(),
            );
            need(
                "row with a meta line",
                ink(size::sm()) + ink(size::xs()),
                control::row(),
            );
            need(
                "row with padding",
                ink(size::sm()) + 2. * f32::from(control::row_pad()),
                control::row(),
            );
            need("tag", ink(size::xs()), control::TAG_HEIGHT);
            need("attention marker", ink(size::xs()), control::MARKER_HEIGHT);
            need("toolbar title", ink(size::md()), control::TOOLBAR_HEIGHT);
        }
        set_for_test(Appearance::default());
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn the_text_and_fill_colors_do_not_depend_on_the_appearance() {
        // Colors are constants, so each combination keeps the 4.5:1 of the test above.
        let pairs = text_pairs(Palette::DARK);
        for a in Appearance::all() {
            set_for_test(a);
            assert_eq!(text_pairs(Palette::DARK), pairs, "{a:?}");
        }
        set_for_test(Appearance::default());
        assert_eq!(Appearance::all().count(), 27);
    }
}
