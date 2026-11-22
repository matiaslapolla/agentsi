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
        .method::<ModelsKeep>()
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

/// How long a voice model stays in memory after its last use. `pill.toml` keeps it as
/// `[models] keep`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum KeepTime {
    #[serde(rename = "1m")]
    OneMinute,
    #[default]
    #[serde(rename = "5m")]
    FiveMinutes,
    #[serde(rename = "15m")]
    FifteenMinutes,
    #[serde(rename = "1h")]
    OneHour,
    /// A model stays once it loaded.
    #[serde(rename = "always")]
    Always,
    /// The models load when the pill starts, and stay.
    #[serde(rename = "start")]
    Start,
}

impl KeepTime {
    pub const ALL: [KeepTime; 6] = [
        KeepTime::OneMinute,
        KeepTime::FiveMinutes,
        KeepTime::FifteenMinutes,
        KeepTime::OneHour,
        KeepTime::Always,
        KeepTime::Start,
    ];

    /// The word in `pill.toml`.
    pub const fn key(self) -> &'static str {
        match self {
            KeepTime::OneMinute => "1m",
            KeepTime::FiveMinutes => "5m",
            KeepTime::FifteenMinutes => "15m",
            KeepTime::OneHour => "1h",
            KeepTime::Always => "always",
            KeepTime::Start => "start",
        }
    }

    pub fn from_key(key: &str) -> Option<KeepTime> {
        KeepTime::ALL.into_iter().find(|k| k.key() == key)
    }

    /// The time with no use after which a model drops. `None` keeps it.
    pub const fn limit(self) -> Option<std::time::Duration> {
        let minutes = match self {
            KeepTime::OneMinute => 1,
            KeepTime::FiveMinutes => 5,
            KeepTime::FifteenMinutes => 15,
            KeepTime::OneHour => 60,
            KeepTime::Always | KeepTime::Start => return None,
        };
        Some(std::time::Duration::from_secs(minutes * 60))
    }

    pub const fn loads_at_start(self) -> bool {
        matches!(self, KeepTime::Start)
    }
}

/// The `VmRSS` value of a `/proc/<pid>/status` text, in KB: the resident memory of a process.
pub fn rss_kb(status: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

pub struct ModelsKeep;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelsKeepParams {
    /// The new keep time. Omit it to read the keep time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<KeepTime>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelsKeepResult {
    /// The keep time after the call. A missing or bad value in the file reads as `5m`.
    pub keep: KeepTime,
}

impl Method for ModelsKeep {
    const NAME: &'static str = "voice.models.keep";
    const DESCRIPTION: &'static str = "Sets or reads how long the voice models stay in memory \
        after their last use. The core writes `[models] keep` in pill.toml, keeps the other keys \
        and comments, and sends voice.models, so the pill applies it with no restart.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::Unavailable];
    type Params = ModelsKeepParams;
    type Result = ModelsKeepResult;
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
    fn the_status_file_gives_the_resident_memory() {
        let status = "Name:\tagentsi-pill\nVmPeak:\t 9000 kB\nVmRSS:\t  143360 kB\nThreads:\t9\n";
        assert_eq!(rss_kb(status), Some(143_360));
        assert_eq!(rss_kb("Name:\tx\n"), None);
    }

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

    #[test]
    fn each_keep_time_reads_back_from_its_key() {
        for keep in KeepTime::ALL {
            assert_eq!(KeepTime::from_key(keep.key()), Some(keep));
            assert_eq!(
                serde_json::to_value(keep).unwrap(),
                serde_json::Value::String(keep.key().into())
            );
        }
        assert_eq!(KeepTime::from_key("2m"), None);
        assert_eq!(KeepTime::default(), KeepTime::FiveMinutes);
        assert_eq!(
            KeepTime::OneMinute.limit(),
            Some(std::time::Duration::from_secs(60))
        );
        assert_eq!(KeepTime::Always.limit(), None);
        assert!(KeepTime::Start.loads_at_start());
    }
}
