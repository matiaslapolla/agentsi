//! libghostty-vt behind a small safe API: a terminal that parses bytes, answers the queries of the
//! program, reports the agent state of OSC 7501, copies the grid, moves the viewport and encodes keys,
//! mouse events, pastes and focus changes.
//!
//! See `README.md` for the two ways to build the library.

mod error;
mod grid;
mod input;
mod key;
mod status;
mod sys;
mod terminal;

pub use error::{Error, Result};
pub use grid::{
    Cell, CellStyle, Colors, Cursor, CursorShape, Grid, Palette, Rgb, Scroll, Scrollbar, Underline,
    Width,
};
pub use input::{MouseAction, MouseButton, MouseEvent, MouseGeometry};
pub use key::{Key, KeyAction, KeyEvent, Mods};
pub use status::{AgentState, BlockedKind, StatusRecords, StatusReport};
pub use terminal::{Damage, Event, Options, RowId, Terminal, mode};

/// Where the linked library came from: `zig <commit>` or `file <path>`.
pub const LIBRARY_SOURCE: &str = env!("AGENTSI_GHOSTTY_VT_SOURCE");
