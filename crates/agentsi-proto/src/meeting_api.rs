//! The methods of meetings (U22): `meeting.start`, `meeting.ask`,
//! `meeting.cancel` and `minutes.check`. They are what `agentsi meet`, `ask` and
//! `check` call. A role runs `claude -p`, so a meeting costs money and takes
//! minutes. `meeting.event` carries each stream-json line of each role, and
//! `meeting.progress` carries the stages of a meeting.

use crate::method::{Catalog, Event, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub fn register(c: &mut Catalog) {
    c.method::<MeetingStart>()
        .method::<MeetingAsk>()
        .method::<MeetingCancel>()
        .method::<MinutesCheck>()
        .event::<MeetingProgress>();
}

/// The cost of one Claude run. Its `role` is the agent id, `synthesis` or `synthesis-repair`.
pub type MeetingUsage = crate::story_api::StoryUsage;

pub struct MeetingStart;

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct MeetingStartParams {
    pub product: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub question: String,
    /// The agent ids that attend. Absent or empty: the roster of the product.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    /// A name for the events of this meeting, which the client picks. Without it, the core makes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting: Option<String>,
    /// The id of stored minutes of this product. The meeting reads their frozen snapshot, with no index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MeetingStartResult {
    pub meeting: String,
    /// The stored minutes, with the shape of `minutes.get` before the rows add to it.
    pub minutes: Value,
    /// In the order in which the runs ended.
    pub usage: Vec<MeetingUsage>,
    pub total_cost_usd: f64,
}

impl Method for MeetingStart {
    const NAME: &'static str = "meeting.start";
    const DESCRIPTION: &'static str = "Runs a meeting: each agent, the synthesis, the check and \
        one repair. Writes the minutes, the meeting log line and the frozen snapshot. The reply \
        comes at the end.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Conflict,
        ErrorKind::Unavailable,
    ];
    type Params = MeetingStartParams;
    type Result = MeetingStartResult;
}

pub struct MeetingAsk;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MeetingAskParams {
    pub product: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub role: String,
    pub question: String,
    /// A name for the events of this call, as in `meeting.start`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MeetingAskResult {
    pub answer: String,
    pub usage: MeetingUsage,
    /// Lines for stderr: tools of an overlay that a meeting does not allow.
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Method for MeetingAsk {
    const NAME: &'static str = "meeting.ask";
    const DESCRIPTION: &'static str =
        "Asks one agent one question in plain text. Writes no minutes.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Conflict,
        ErrorKind::Unavailable,
    ];
    type Params = MeetingAskParams;
    type Result = MeetingAskResult;
}

pub struct MeetingCancel;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MeetingCancelParams {
    pub meeting: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MeetingCancelResult {
    /// False when no meeting of that name runs.
    pub canceled: bool,
}

impl Method for MeetingCancel {
    const NAME: &'static str = "meeting.cancel";
    const DESCRIPTION: &'static str = "Stops the Claude runs of a meeting. The meeting writes \
        nothing, and its `meeting.start` returns `conflict`.";
    const ERRORS: &'static [ErrorKind] = &[];
    type Params = MeetingCancelParams;
    type Result = MeetingCancelResult;
}

pub struct MinutesCheck;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MinutesCheckParams {
    /// The minutes as JSON, as `agentsi check` reads them from a file.
    pub minutes: Value,
    /// The agent ids whose roles must answer. Absent: the roster of the product.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MinutesCheckResult {
    pub passed: bool,
    pub failures: Vec<String>,
}

impl Method for MinutesCheck {
    const NAME: &'static str = "minutes.check";
    const DESCRIPTION: &'static str = "The pass or fail check of minutes. Only reads.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = MinutesCheckParams;
    type Result = MinutesCheckResult;
}

pub struct MeetingProgress;

/// One stage of a meeting. `stage` is one of these:
///
/// - `start`: the meeting began. `note` holds the question.
/// - `index`, `role`, `synthesis`, `check`, with `phase` `start` or `end`, as the TypeScript `MeetingEvent`.
/// - `warning`: `note` holds a line for the owner.
/// - `done`: the last event. `note` holds the minutes id, or `error` the reason of the end.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct MeetingProgressParams {
    pub meeting: String,
    pub product: String,
    pub stage: String,
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Event for MeetingProgress {
    const NAME: &'static str = "meeting.progress";
    const DESCRIPTION: &'static str =
        "One stage of a meeting, for the log of the CLI and the meeting screen.";
    type Params = MeetingProgressParams;
}
