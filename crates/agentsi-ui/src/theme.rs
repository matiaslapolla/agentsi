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

        t.background = c(surface::PANEL);
        t.foreground = c(text::PRIMARY);
        t.muted = c(surface::OVERLAY);
        t.muted_foreground = c(text::SECONDARY);
        t.border = c(line::BORDER);
        t.input = c(line::STRONG);
        t.ring = c(line::FOCUS);
        t.caret = c(accent::BASE);
        t.selection = accent::BASE.alpha(0.32);
        t.accent = c(surface::OVERLAY);
        t.accent_foreground = c(text::PRIMARY);
        t.popover = c(surface::RAISED);
        t.popover_foreground = c(text::PRIMARY);
        t.overlay = surface::BASE.alpha(0.6);
        t.window_border = c(line::BORDER);

        t.primary = c(accent::BASE);
        t.primary_hover = c(accent::HOVER);
        t.primary_active = c(accent::PRESSED);
        t.primary_foreground = c(accent::ON);
        t.secondary = c(surface::RAISED);
        t.secondary_hover = c(surface::OVERLAY);
        t.secondary_active = c(state::pressed(surface::OVERLAY));
        t.secondary_foreground = c(text::PRIMARY);
        t.button_primary = t.primary;
        t.button_primary_hover = t.primary_hover;
        t.button_primary_active = t.primary_active;
        t.button_primary_foreground = t.primary_foreground;
        t.link = c(accent::BASE);
        t.link_hover = c(accent::HOVER);
        t.link_active = c(accent::PRESSED);

        t.success = c(status::OK);
        t.success_hover = c(status::OK);
        t.success_active = c(status::OK);
        t.success_foreground = c(surface::BASE);
        t.warning = c(status::WARN);
        t.warning_hover = c(status::WARN);
        t.warning_active = c(status::WARN);
        t.warning_foreground = c(surface::BASE);
        t.danger = c(status::DANGER);
        t.danger_hover = c(status::DANGER);
        t.danger_active = c(status::DANGER);
        t.danger_foreground = c(surface::BASE);
        t.info = c(status::INFO);
        t.info_hover = c(status::INFO);
        t.info_active = c(status::INFO);
        t.info_foreground = c(surface::BASE);

        t.sidebar = c(surface::BASE);
        t.sidebar_border = c(line::BORDER);
        t.sidebar_foreground = c(text::PRIMARY);
        t.sidebar_accent = c(surface::OVERLAY);
        t.sidebar_accent_foreground = c(text::PRIMARY);
        t.sidebar_primary = c(accent::BASE);
        t.sidebar_primary_foreground = c(accent::ON);

        t.list_hover = c(surface::OVERLAY);
        t.list_active = c(accent::SUBTLE);
        t.list_active_border = c(accent::BASE);
        t.list_even = c(surface::PANEL);
        t.list_head = c(surface::RAISED);
        t.table = c(surface::PANEL);
        t.table_hover = c(surface::OVERLAY);
        t.table_active = c(accent::SUBTLE);
        t.table_active_border = c(accent::BASE);
        t.table_even = c(surface::PANEL);
        t.table_head = c(surface::RAISED);
        t.table_head_foreground = c(text::SECONDARY);
        t.table_row_border = c(line::BORDER);

        t.tab = c(surface::PANEL);
        t.tab_active = c(surface::RAISED);
        t.tab_active_foreground = c(text::PRIMARY);
        t.tab_bar = c(surface::BASE);
        t.tab_foreground = c(text::SECONDARY);
        t.title_bar = c(surface::BASE);
        t.title_bar_border = c(line::BORDER);
        t.status_bar = c(surface::BASE);
        t.status_bar_border = c(line::BORDER);

        t.scrollbar = surface::PANEL.alpha(0.0);
        t.scrollbar_thumb = line::STRONG.alpha(0.9);
        t.scrollbar_thumb_hover = c(text::TERTIARY);
        t.switch = c(switch::TRACK);
        t.switch_thumb = c(switch::THUMB);
        t.slider_bar = c(accent::BASE);
        t.slider_thumb = c(text::PRIMARY);
        t.progress_bar = c(accent::BASE);
        t.skeleton = c(surface::OVERLAY);
        t.drag_border = c(accent::BASE);
        t.drop_target = accent::BASE.alpha(0.12);
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
