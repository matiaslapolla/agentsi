//! Applies the tokens to the theme of `gpui-component`.
use std::borrow::Cow;

use gpui_kit::App;
use gpui_kit::component::input::CursorBlink;
use gpui_kit::component::{Theme, ThemeMode};

use gpui_kit::base::motion::Easing;

use crate::appearance::Appearance;
use crate::tokens::{
    Color, accent, font, line, motion, radius, size, state, status, surface, switch, text,
};

/// The bundled font files (SIL Open Font License 1.1, see `fonts/`).
const FONTS: [&[u8]; 5] = [
    include_bytes!("../fonts/Inter-Regular.ttf"),
    include_bytes!("../fonts/Inter-Medium.ttf"),
    include_bytes!("../fonts/Inter-SemiBold.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Bold.ttf"),
];

/// Registers the bundled fonts with the text system.
fn load_fonts(cx: &mut App) -> Result<(), String> {
    cx.text_system()
        .add_fonts(FONTS.iter().map(|f| Cow::Borrowed(*f)).collect())
        .map_err(|e| e.to_string())
}

/// Sets the sizes of the theme that follow the appearance: the font size and the radii.
pub(crate) fn apply_metrics(cx: &mut App) {
    Theme::update(cx, |t| {
        t.font_size = size::base();
        t.radius = radius::md();
        t.radius_lg = radius::lg();
    });
}

/// Sets the dark theme of `gpui-component` from the tokens.
fn apply(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |t| {
        let c = |c: Color| c.hsla();
        t.font_family = font::UI.into();
        t.mono_font_family = font::MONO.into();
        t.mono_font_size = size::MONO;
        t.shadow = false;
        t.focus_ring = true;
        t.cursor_blink =
            CursorBlink::blinking(motion::CARET_BLINK).with_idle_timeout(motion::CARET_IDLE);
        t.motion.duration_fast = motion::HOVER;
        t.motion.duration_normal = motion::PANEL;
        t.motion.duration_slow = motion::DIALOG;
        let (x1, y1, x2, y2) = motion::EASE_OUT;
        let ease_out = Easing::cubic_bezier(x1, y1, x2, y2).expect("a valid ease-out curve");
        t.motion.easing_enter = ease_out.clone();
        t.motion.easing_exit = ease_out.clone();
        t.motion.easing_move = ease_out;

        t.background = c(surface::panel());
        t.foreground = c(text::primary());
        t.muted = c(surface::overlay());
        t.muted_foreground = c(text::secondary());
        t.border = c(line::border());
        t.input = c(line::strong());
        t.ring = c(line::focus());
        t.caret = c(accent::base());
        t.selection = accent::base().alpha(0.32);
        t.accent = c(surface::overlay());
        t.accent_foreground = c(text::primary());
        t.popover = c(surface::raised());
        t.popover_foreground = c(text::primary());
        t.overlay = surface::base().alpha(0.6);
        t.window_border = c(line::border());

        t.primary = c(accent::base());
        t.primary_hover = c(accent::hover());
        t.primary_active = c(accent::pressed());
        t.primary_foreground = c(accent::on());
        t.secondary = c(surface::raised());
        t.secondary_hover = c(surface::overlay());
        t.secondary_active = c(state::pressed(surface::overlay()));
        t.secondary_foreground = c(text::primary());
        t.button_primary = t.primary;
        t.button_primary_hover = t.primary_hover;
        t.button_primary_active = t.primary_active;
        t.button_primary_foreground = t.primary_foreground;
        t.link = c(accent::base());
        t.link_hover = c(accent::hover());
        t.link_active = c(accent::pressed());

        t.success = c(status::ok());
        t.success_hover = c(status::ok());
        t.success_active = c(status::ok());
        t.success_foreground = c(surface::base());
        t.warning = c(status::warn());
        t.warning_hover = c(status::warn());
        t.warning_active = c(status::warn());
        t.warning_foreground = c(surface::base());
        t.danger = c(status::danger());
        t.danger_hover = c(status::danger());
        t.danger_active = c(status::danger());
        t.danger_foreground = c(surface::base());
        t.info = c(status::info());
        t.info_hover = c(status::info());
        t.info_active = c(status::info());
        t.info_foreground = c(surface::base());

        t.sidebar = c(surface::base());
        t.sidebar_border = c(line::border());
        t.sidebar_foreground = c(text::primary());
        t.sidebar_accent = c(surface::overlay());
        t.sidebar_accent_foreground = c(text::primary());
        t.sidebar_primary = c(accent::base());
        t.sidebar_primary_foreground = c(accent::on());

        t.list_hover = c(surface::overlay());
        t.list_active = c(accent::subtle());
        t.list_active_border = c(accent::base());
        t.list_even = c(surface::panel());
        t.list_head = c(surface::raised());
        t.table = c(surface::panel());
        t.table_hover = c(surface::overlay());
        t.table_active = c(accent::subtle());
        t.table_active_border = c(accent::base());
        t.table_even = c(surface::panel());
        t.table_head = c(surface::raised());
        t.table_head_foreground = c(text::secondary());
        t.table_row_border = c(line::border());

        t.tab = c(surface::panel());
        t.tab_active = c(surface::raised());
        t.tab_active_foreground = c(text::primary());
        t.tab_bar = c(surface::base());
        t.tab_foreground = c(text::secondary());
        t.title_bar = c(surface::base());
        t.title_bar_border = c(line::border());
        t.status_bar = c(surface::base());
        t.status_bar_border = c(line::border());

        t.scrollbar = surface::panel().alpha(0.0);
        t.scrollbar_thumb = line::strong().alpha(0.9);
        t.scrollbar_thumb_hover = c(text::tertiary());
        t.switch = c(switch::track());
        t.switch_thumb = c(switch::thumb());
        t.slider_bar = c(accent::base());
        t.slider_thumb = c(text::primary());
        t.progress_bar = c(accent::base());
        t.skeleton = c(surface::overlay());
        t.drag_border = c(accent::base());
        t.drop_target = accent::base().alpha(0.12);
    });
}

/// Loads the fonts and applies the theme. Call it after `gpui_kit::init`.
pub(crate) fn init(cx: &mut App) {
    if let Err(e) = load_fonts(cx) {
        eprintln!("agentsi-ui: the bundled fonts did not load: {e}");
    }
    apply(cx);
    // The app sets the saved appearance after this. A fresh app starts at the default.
    Appearance::default().apply(cx);
    apply_metrics(cx);
}
