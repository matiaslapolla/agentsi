//! Mouse events for the mouse encoder of libghostty-vt.

use crate::key::Mods;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseAction {
    #[default]
    Press,
    Release,
    /// The pointer moved. `button` is the button held, or `None`.
    Motion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

/// One mouse event. The position is in pixels from the top left corner of the pane.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MouseEvent {
    pub action: MouseAction,
    pub button: Option<MouseButton>,
    pub mods: Mods,
    pub x: f32,
    pub y: f32,
    /// True while any button is down. The encoder uses it for drag reports.
    pub any_button_down: bool,
}

/// The pixel geometry of the pane, so the encoder can turn pixels into cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseGeometry {
    pub width_px: u32,
    pub height_px: u32,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
    /// The space between the edge of the pane and the first cell, on each side.
    pub padding_px: u32,
}
