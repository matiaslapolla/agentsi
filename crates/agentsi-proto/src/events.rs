//! The events of the protocol spec. U3 sends only `core.lagged`. Later units
//! send the others.

use crate::method::{Catalog, Event};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.event::<Lagged>()
        .event::<TerminalState>()
        .event::<TerminalExit>()
        .event::<TerminalTitle>()
        .event::<DataChanged>()
        .event::<MeetingEvent>()
        .event::<VoiceSpeak>()
        .event::<VoiceStop>()
        .event::<DictationTarget>()
        .event::<DictationStart>()
        .event::<DictationStop>()
        .event::<ConverseState>()
        .event::<ModelsProgress>()
        .event::<ModelsDone>()
        .event::<ModelsError>();
}

pub struct Lagged;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LaggedParams {
    /// The number of events that this connection did not get.
    pub missed: u64,
}

impl Event for Lagged {
    const NAME: &'static str = "core.lagged";
    const DESCRIPTION: &'static str = "The connection read too slowly, and the core dropped events. \
        Read the state again.";
    type Params = LaggedParams;
}

/// The state that a program reports with OSC 7501.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProgramState {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
}

pub struct TerminalState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalStateParams {
    pub id: String,
    pub state: ProgramState,
    /// For `blocked`: `permission`, `question`, `auth` or another kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The text that the program gives with the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

impl Event for TerminalState {
    const NAME: &'static str = "terminal.state";
    const DESCRIPTION: &'static str = "The program state of a terminal changed.";
    type Params = TerminalStateParams;
}

pub struct TerminalExit;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalExitParams {
    pub id: String,
    /// The exit code, or null when a signal stopped the program.
    pub code: Option<i32>,
}

impl Event for TerminalExit {
    const NAME: &'static str = "terminal.exit";
    const DESCRIPTION: &'static str = "The program of a terminal stopped.";
    type Params = TerminalExitParams;
}

pub struct TerminalTitle;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TerminalTitleParams {
    pub id: String,
    pub title: String,
}

impl Event for TerminalTitle {
    const NAME: &'static str = "terminal.title";
    const DESCRIPTION: &'static str = "The program of a terminal set its title.";
    type Params = TerminalTitleParams;
}

pub struct DataChanged;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DataChangedParams {
    pub product: String,
    /// The kind of record that changed, for example `minutes`, `git` or `tickets`.
    pub kind: String,
}

impl Event for DataChanged {
    const NAME: &'static str = "data.changed";
    const DESCRIPTION: &'static str = "The core wrote data of a product.";
    type Params = DataChangedParams;
}

pub struct MeetingEvent;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MeetingEventParams {
    pub meeting: String,
    pub role: String,
    /// One stream-json line of the Claude run of the role.
    pub line: String,
}

impl Event for MeetingEvent {
    const NAME: &'static str = "meeting.event";
    const DESCRIPTION: &'static str = "One stream-json line of one role in a meeting.";
    type Params = MeetingEventParams;
}

pub struct VoiceSpeak;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VoiceSpeakParams {
    pub terminal: String,
    pub role: String,
    pub text: String,
}

impl Event for VoiceSpeak {
    const NAME: &'static str = "voice.speak";
    const DESCRIPTION: &'static str = "The pill speaks this text with the voice of the role.";
    type Params = VoiceSpeakParams;
}

pub struct VoiceStop;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VoiceStopParams {
    pub terminal: String,
}

impl Event for VoiceStop {
    const NAME: &'static str = "voice.stop";
    const DESCRIPTION: &'static str = "The pill stops the speech of this terminal.";
    type Params = VoiceStopParams;
}

pub struct DictationTarget;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DictationTargetParams {
    /// The terminal that gets dictated text, or null for none.
    pub terminal: Option<String>,
}

impl Event for DictationTarget {
    const NAME: &'static str = "dictation.target";
    const DESCRIPTION: &'static str = "The terminal that gets dictated text changed.";
    type Params = DictationTargetParams;
}

pub struct DictationStart;

/// When the key was pressed or released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DictationEdgeParams {
    /// Microseconds since the Unix epoch. A `start` that is older than a `stop` the pill saw
    /// already is stale: the bind runs the two commands as two processes, and they can arrive
    /// out of order.
    pub at_us: u64,
}

impl Event for DictationStart {
    const NAME: &'static str = "dictation.start";
    const DESCRIPTION: &'static str =
        "The pill starts the microphone and shows the listening state.";
    type Params = DictationEdgeParams;
}

pub struct DictationStop;

impl Event for DictationStop {
    const NAME: &'static str = "dictation.stop";
    const DESCRIPTION: &'static str =
        "The pill ends the recording, transcribes it and sends voice.transcript.";
    type Params = DictationEdgeParams;
}

pub struct ConverseState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConverseStateKind {
    /// Conversation mode started. The pill listens.
    Started,
    /// Conversation mode ended, for any reason. The pill closes the microphone and hides.
    Stopped,
    /// The target pane finished a reply with nothing to speak. The pill listens again.
    SilentReply,
    /// A start found no pane to talk to. The pill shows "Focus a pane first".
    NoPane,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConverseStateParams {
    pub state: ConverseStateKind,
    /// The target pane. Null for `no_pane`.
    pub terminal: Option<String>,
}

impl Event for ConverseState {
    const NAME: &'static str = "converse.state";
    const DESCRIPTION: &'static str = "Conversation mode started, ended, got a reply with \
        nothing to speak, or found no pane to talk to. It goes to the subscribers of the voice \
        topic.";
    type Params = ConverseStateParams;
}

pub struct ModelsProgress;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelsProgressParams {
    pub id: String,
    /// Bytes of the model on disk so far.
    pub bytes: u64,
    /// Bytes of all the files of the model.
    pub total: u64,
    /// The core checks the hash of a file.
    pub verifying: bool,
}

impl Event for ModelsProgress {
    const NAME: &'static str = "models.progress";
    const DESCRIPTION: &'static str =
        "A download of a model moved on. The core sends it at most 10 times each second.";
    type Params = ModelsProgressParams;
}

pub struct ModelsDone;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelsDoneParams {
    pub id: String,
}

impl Event for ModelsDone {
    const NAME: &'static str = "models.done";
    const DESCRIPTION: &'static str = "Each file of the model passed its hash check.";
    type Params = ModelsDoneParams;
}

pub struct ModelsError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelsErrorParams {
    pub id: String,
    /// The kind of the failure: `hash_mismatch`, `conflict`, `network`, `bad_response`,
    /// `busy`, `canceled` or `io`.
    pub error: String,
    /// The failure, in a person's words.
    pub message: String,
    /// The file of a `conflict`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

impl Event for ModelsError {
    const NAME: &'static str = "models.error";
    const DESCRIPTION: &'static str = "A download of a model failed. A file with a wrong hash is \
        deleted. A conflict leaves the foreign file in place.";
    type Params = ModelsErrorParams;
}
