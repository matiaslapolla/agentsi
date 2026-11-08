//! The `terminal` namespace: a PTY and its program, owned by the core.
//!
//! A pane calls `terminal.attach` on a framed connection. The core answers
//! with a channel. On that channel it sends a RESIZE frame and the first
//! paint, then the live bytes of the program, and a CLOSE frame at the end.

use crate::core_api::Empty;
use crate::events::ProgramState;
use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub fn register(c: &mut Catalog) {
    c.method::<Create>()
        .method::<List>()
        .method::<Attach>()
        .method::<Input>()
        .method::<Resize>()
        .method::<Kill>();
}

/// The name in the child environment that holds the id of its terminal.
pub const TERMINAL_ENV: &str = "AGENTSI_TERMINAL";

pub struct Create;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CreateParams {
    /// The folder where the program starts. It must exist.
    pub cwd: String,
    /// The program and its arguments. The core looks up `argv[0]` in the `PATH`.
    pub argv: Vec<String>,
    /// Names and values to add to the environment of the program.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// The product that the terminal belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// The width in cells. The default is 80.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u16>,
    /// The height in cells. The default is 24.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u16>,
}

/// One terminal of the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalInfo {
    pub id: String,
    pub argv: Vec<String>,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// The process id of the program.
    pub pid: u32,
    pub cols: u16,
    pub rows: u16,
    /// The window title that the program set.
    pub title: String,
    /// The last program state, or null if the program never reported one.
    pub state: Option<ProgramState>,
    /// For `blocked`: `permission`, `question`, `auth` or another kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The text that the program gave with the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
    /// false after the program stopped. The terminal stays in the list until `terminal.kill`.
    pub running: bool,
    /// The exit code after the program stopped, or null for a signal or a program that runs.
    pub code: Option<i32>,
}

impl Method for Create {
    const NAME: &'static str = "terminal.create";
    const DESCRIPTION: &'static str = "Starts a program on a new PTY. The child gets AGENTSI_TERMINAL \
        with the id and AGENTSI_CORE_SOCKET.";
    const ERRORS: &'static [ErrorKind] = &[];
    type Params = CreateParams;
    type Result = TerminalInfo;
}

pub struct List;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListResult {
    pub terminals: Vec<TerminalInfo>,
}

impl Method for List {
    const NAME: &'static str = "terminal.list";
    const DESCRIPTION: &'static str = "Gives each terminal of the core, in the order of creation.";
    const ERRORS: &'static [ErrorKind] = &[];
    type Params = Empty;
    type Result = ListResult;
}

/// The params of a method that names one terminal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalId {
    pub id: String,
}

pub struct Attach;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AttachParams {
    pub id: String,
    /// true: the pane only watches. The core ignores its DATA, AUTO, RESIZE and FOCUS frames.
    #[serde(default)]
    pub observe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AttachResult {
    /// The stream channel of this pane on this connection.
    pub channel: u32,
}

impl Method for Attach {
    const NAME: &'static str = "terminal.attach";
    const DESCRIPTION: &'static str = "Opens a stream channel to a terminal, on a framed connection. \
        The channel starts with a RESIZE frame and the first paint, then the live bytes follow. \
        The pane returns a CREDIT for each byte that it read. A CLOSE frame ends the channel.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound];
    type Params = AttachParams;
    type Result = AttachResult;
}

pub struct Input;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InputParams {
    pub id: String,
    /// The bytes to write to the program, as text.
    pub text: String,
    /// true: the core writes the Return key 100 ms after the text.
    #[serde(default)]
    pub submit: bool,
}

impl Method for Input {
    const NAME: &'static str = "terminal.input";
    const DESCRIPTION: &'static str = "Writes text to the program of a terminal. With submit, the \
        Return key follows 100 ms later.";
    const ERRORS: &'static [ErrorKind] =
        &[ErrorKind::NotFound, ErrorKind::Conflict, ErrorKind::Busy];
    type Params = InputParams;
    type Result = Empty;
}

pub struct Resize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResizeParams {
    pub id: String,
    pub cols: u16,
    pub rows: u16,
    /// The width of a cell in pixels, for the size reports of the program.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_width: Option<u16>,
    /// The height of a cell in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell_height: Option<u16>,
}

impl Method for Resize {
    const NAME: &'static str = "terminal.resize";
    const DESCRIPTION: &'static str = "Changes the size of a terminal. Each attached pane gets a \
        RESIZE frame before the bytes at the new size.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound];
    type Params = ResizeParams;
    type Result = Empty;
}

pub struct Kill;

impl Method for Kill {
    const NAME: &'static str = "terminal.kill";
    const DESCRIPTION: &'static str = "Stops the program of a terminal: SIGHUP and SIGTERM, then \
        SIGKILL after 2 seconds. The core removes the terminal from the list when the program stopped.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound];
    type Params = TerminalId;
    type Result = Empty;
}

/// The JSON payload of a CLOSE frame from the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum CloseReason {
    /// The program stopped. `code` is null for a signal.
    Exit { code: Option<i32> },
}
