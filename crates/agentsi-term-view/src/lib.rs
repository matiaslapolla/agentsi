//! The terminal pane of the AgentSI app: a GPUI view that draws one libghostty-vt screen and turns
//! keys, the mouse, pastes and focus into bytes for the program.
//!
//! - [`TerminalPane::feed`] takes the bytes of the program.
//! - The pane emits [`PaneEvent`]s: [`PaneOutput`] for the program, the title, the bell, a
//!   [`Link`] to open and a request for the context menu.
//! - `examples/pane.rs` runs a local program on a PTY. In the app, a core stream feeds the pane.
//!
//! See `README.md` for the interface and the traps.

mod frame;
mod keys;
mod links;
mod pane;
mod selection;
mod shapes;

pub use keys::shortcut;
pub use links::Link;
pub use pane::{PaneEvent, PaneOptions, PaneOutput, TerminalPane};
