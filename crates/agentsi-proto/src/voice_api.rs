//! The `voice`, `hook` and `focus` namespaces: voice mode of a terminal,
//! dictation, the events of the Claude Code hooks and the focused pane.

use crate::core_api::Empty;
use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<Mode>()
        .method::<SessionMode>()
        .method::<Dictate>()
        .method::<Converse>()
        .method::<Transcript>()
        .method::<HookEvent>()
        .method::<FocusReport>();
}

pub struct Mode;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModeParams {
    pub terminal: String,
    /// true turns voice mode on, false turns it off. Omit it to read the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModeResult {
    pub terminal: String,
    /// The state of voice mode after the call.
    pub on: bool,
}

impl Method for Mode {
    const NAME: &'static str = "voice.mode";
    const DESCRIPTION: &'static str = "Turns voice mode on or off for a terminal, or reads it. \
        With voice mode on, the core sends voice.speak after each reply of the Claude session \
        of that terminal and saves a transcript.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound];
    type Params = ModeParams;
    type Result = ModeResult;
}

pub struct SessionMode;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionModeParams {
    /// The id of a Claude Code session that runs outside the app.
    pub session: String,
    /// true turns voice mode on, false turns it off. Omit it to read the state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionModeResult {
    pub session: String,
    /// The state of voice mode after the call.
    pub on: bool,
}

impl Method for SessionMode {
    const NAME: &'static str = "voice.session";
    const DESCRIPTION: &'static str = "Turns voice mode on or off for a Claude Code session \
        that runs outside the app, or reads it. The state is a file in the data root, so it \
        survives a restart of the core. The hook bridge forwards the hooks of such a session \
        when its file exists. The core then sends voice.speak after each reply and saves a \
        transcript.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::Invalid];
    type Params = SessionModeParams;
    type Result = SessionModeResult;
}

pub struct Dictate;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DictateAction {
    Start,
    Stop,
    // Starts the dictation when none runs, else stops it. The core keeps the state.
    Toggle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DictateParams {
    pub action: DictateAction,
    /// When the key edge happened, in microseconds since the Unix epoch. The core uses its own
    /// clock when it is absent. The pill drops a `start` that is older than a `stop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_us: Option<u64>,
}

impl Method for Dictate {
    const NAME: &'static str = "voice.dictate";
    const DESCRIPTION: &'static str = "Starts or stops a dictation. The core sends dictation.start \
        or dictation.stop to the pill, which records and transcribes. A compositor bind runs it \
        with agentsi-core dictate start and stop, or with dictate toggle, which starts the \
        dictation when none runs and stops it otherwise. The core answers unavailable when no \
        pill is subscribed to the dictation topic.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::Unavailable];
    type Params = DictateParams;
    type Result = Empty;
}

pub struct Converse;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConverseAction {
    Start,
    Stop,
    /// Stops conversation mode when it runs, else starts it. A toggle that names another pane
    /// than the target starts the mode on that pane.
    Toggle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConverseParams {
    pub action: ConverseAction,
    /// The pane to talk to. A start uses the focused pane when it is absent. A stop ignores it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConverseResult {
    /// Conversation mode runs after the call.
    pub on: bool,
    /// The target pane of the mode that runs. Null when the mode does not run.
    pub terminal: Option<String>,
}

impl Method for Converse {
    const NAME: &'static str = "voice.converse";
    const DESCRIPTION: &'static str = "Starts or stops conversation mode, in which the pill \
        listens, sends each spoken turn to one pane and speaks the reply. A start turns on voice \
        mode of the target pane, and a stop puts it back as it was. While the mode runs, \
        voice.transcript submits the text to the target pane. A compositor bind runs it with \
        agentsi-core converse toggle. The core sends converse.state. It answers unavailable \
        when no pill is subscribed to the dictation topic or a dictation runs, and invalid when a \
        start has no target pane.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Invalid,
        ErrorKind::Unavailable,
    ];
    type Params = ConverseParams;
    type Result = ConverseResult;
}

pub struct Transcript;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TranscriptParams {
    /// The text that the speech model returned.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TranscriptResult {
    /// The text after the vocabulary and the control-character filter.
    pub text: String,
    /// The terminal that got the text. Null when no pane has the focus: the pill copies the text.
    pub terminal: Option<String>,
}

impl Method for Transcript {
    const NAME: &'static str = "voice.transcript";
    const DESCRIPTION: &'static str = "The pill gives the text of a dictation. The core applies the \
        vocabulary and types the text into the focused pane with no Return. With no focused pane \
        it types nothing, and the pill copies the text.";
    const ERRORS: &'static [ErrorKind] = &[];
    type Params = TranscriptParams;
    type Result = TranscriptResult;
}

/// A Claude Code hook event that the bridge forwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum HookKind {
    SessionStart,
    UserPromptSubmit,
    Notification,
    PermissionRequest,
    /// A tool ran. After a granted permission, the program works again.
    PostToolUse,
    Stop,
}

impl HookKind {
    pub const ALL: [HookKind; 6] = [
        HookKind::SessionStart,
        HookKind::UserPromptSubmit,
        HookKind::Notification,
        HookKind::PermissionRequest,
        HookKind::PostToolUse,
        HookKind::Stop,
    ];

    /// The event of a command-line word, in any case: `stop` or `Stop`.
    pub fn parse(word: &str) -> Option<HookKind> {
        Self::ALL
            .into_iter()
            .find(|k| format!("{k:?}").eq_ignore_ascii_case(word))
    }
}

pub struct HookEvent;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HookEventParams {
    /// The terminal of the hook, or an empty string for a Claude Code session outside the app.
    pub terminal: String,
    /// The Claude Code session id, or an empty string.
    pub session: String,
    pub event: HookKind,
    /// The JSON that Claude Code gave to the hook.
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HookEventResult {
    /// Text that the bridge prints for Claude Code as hook output: the voice
    /// style rules on UserPromptSubmit with voice mode on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

impl Method for HookEvent {
    const NAME: &'static str = "hook.event";
    const DESCRIPTION: &'static str = "A Claude Code hook ran in a terminal. The core takes the \
        state of the terminal from it when the program sends no OSC 7501 state, and runs voice mode.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound];
    type Params = HookEventParams;
    type Result = HookEventResult;
}

pub struct FocusReport;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FocusReportParams {
    /// The terminal of the focused pane, or none when no pane has the focus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
}

impl Method for FocusReport {
    const NAME: &'static str = "focus.report";
    const DESCRIPTION: &'static str = "The app gives the terminal of its focused pane. Dictated \
        text goes to that terminal. A change sends dictation.target.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound];
    type Params = FocusReportParams;
    type Result = Empty;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bridge_reads_an_event_in_any_case() {
        assert_eq!(HookKind::parse("stop"), Some(HookKind::Stop));
        assert_eq!(
            HookKind::parse("UserPromptSubmit"),
            Some(HookKind::UserPromptSubmit)
        );
        assert_eq!(
            HookKind::parse("userpromptsubmit"),
            Some(HookKind::UserPromptSubmit)
        );
        assert_eq!(HookKind::parse("posttooluse"), Some(HookKind::PostToolUse));
        assert_eq!(HookKind::parse("PreToolUse"), None);
    }
}
