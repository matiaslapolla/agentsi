//! The design system of the AgentSI app, on `gpui-component`.
//!
//! - `init`: initializes `gpui-component`, loads the bundled fonts and sets the theme.
//! - `tokens`: each color, size, space, radius and duration.
//! - `appearance`: the density, corners and text size that the owner picks.
//! - `components`: the components, own and from `gpui-component`.
//! - `style`: style helpers that take tokens, for example `pad_x`.
//! - `bench`: the bench mode, for the gallery and `agentsi-app`.
//!
//! Screens use this crate and never set a color or a size by hand.
pub mod appearance;
pub mod bench;
pub mod components;
pub mod style;
mod theme;
pub mod tokens;

/// Initializes `gpui-component`, the fonts and the theme. Call it first in `Application::run`.
pub fn init(cx: &mut gpui_kit::App) {
    gpui_kit::init(cx);
    theme::init(cx);
}
