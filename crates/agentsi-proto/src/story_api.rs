//! The methods of stories mode (U23): `story.*`. They are what `agentsi story`
//! calls, and what the stories screen of the app shows. A method that writes
//! sends `data.changed {product, kind: "stories"}`.
//!
//! `Story` keeps the shape of the story JSON of `packages/core/src/story.ts`.
//! The enum-like fields stay strings, because the store can hold an old or a
//! bad value, and the validation names it.

use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<StoryConfigGet>()
        .method::<StoryList>()
        .method::<StoryGet>()
        .method::<StoryNew>()
        .method::<StorySuggest>()
        .method::<StoryApprove>()
        .method::<StoryDiscard>()
        .method::<StoryChanges>()
        .method::<StoryRun>()
        .method::<StoryExec>()
        .method::<StoryReview>()
        .method::<StoryAccept>()
        .method::<StoryReject>()
        .method::<StoryResultSet>();
}

/// A field that can be absent, null or a value. Absent stays absent when the
/// story is written again.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Criterion {
    #[serde(default)]
    pub given: String,
    #[serde(default)]
    pub when: String,
    #[serde(default)]
    pub then: String,
    /// The repo-relative path of the test file. Null only in a task.
    #[serde(default)]
    pub test: Option<String>,
    /// One plain sentence of what the test proves.
    #[serde(default)]
    pub checks: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Source {
    /// `commit`, `issue`, `path`, `url` or `story`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
}

/// Why Claude Code stopped a session: today only `budget`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StoryStop {
    pub reason: String,
    pub cap_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StorySession {
    /// `queued`, `tests`, `work`, `checks`, `ready` or `failed`.
    pub state: String,
    /// `story/<id>-<slug>`
    pub branch: String,
    /// `<repo>/.worktrees/<id>-<slug>`
    pub worktree: String,
    pub base: String,
    #[serde(default)]
    pub tests_commit: Option<String>,
    #[serde(default)]
    pub final_commit: Option<String>,
    #[serde(default)]
    pub attempts: i64,
    #[serde(default)]
    pub failure: Option<String>,
    /// Where to read the log: `story_logs <product>/<id>`.
    pub log: String,
    pub started_at: String,
    /// The process that runs the session. Absent until the session starts.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(with = "Option<i64>")]
    pub pid: Option<Option<i64>>,
    /// Set when Claude Code stopped the session at its cap.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    #[schemars(with = "Option<StoryStop>")]
    pub stop: Option<Option<StoryStop>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoryResult {
    /// `merged_unchanged`, `merged_changed` or `discarded`.
    pub result: String,
    pub at: String,
    pub commit: Option<String>,
}

/// One story. `S014` is a story, `T015` a task. The fields are in the order of
/// the TypeScript code that makes a story.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Story {
    pub id: String,
    /// `story` or `task`.
    #[serde(rename = "type")]
    pub kind: String,
    pub slug: String,
    pub title: String,
    #[serde(default)]
    pub as_a: Option<String>,
    #[serde(default)]
    pub i_want: String,
    #[serde(default)]
    pub so_that: String,
    #[serde(default)]
    pub criteria: Vec<Criterion>,
    pub owner_role: String,
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub sources: Vec<Source>,
    /// `not_implemented`, `implemented` or `discarded`.
    pub status: String,
    /// `pending`, `approved` or `changes_requested`.
    pub approval: String,
    #[serde(default)]
    pub changes_note: Option<String>,
    /// `owner` or `pm`.
    pub proposed_by: String,
    #[serde(default)]
    pub minutes: Option<String>,
    #[serde(default)]
    pub session: Option<StorySession>,
    /// Newest last.
    #[serde(default)]
    pub results: Vec<StoryResult>,
    pub created_at: String,
    pub updated_at: String,
    /// True on a story of the archive record. The store never holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
}

/// The `stories` block of a product in `agentsi.config.json`, with its defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StoryConfig {
    /// The absolute path of the product repo.
    pub repo: String,
    /// The test command. `{files}` gets the repo-relative test paths.
    pub test: String,
    pub gate: Vec<String>,
    pub budget_usd: f64,
}

/// The usage of one model call, as the TypeScript `UsageRow`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StoryUsage {
    pub role: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: f64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProductParams {
    pub product: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoryIdParams {
    pub product: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StoryOne {
    pub story: Story,
}

pub struct StoryConfigGet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoryConfigParams {
    pub product: String,
    /// Also refuse an archived product, as `agentsi story run` does first.
    #[serde(default)]
    pub run: bool,
}

impl Method for StoryConfigGet {
    const NAME: &'static str = "story.config";
    const DESCRIPTION: &'static str = "Gives the stories block of a product with its defaults. \
        It returns `invalid` when stories mode is off for the product or the block is not \
        valid. `agentsi story` calls it before each subcommand.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = StoryConfigParams;
    type Result = StoryConfig;
}

pub struct StoryList;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoryListParams {
    pub product: String,
    /// Also give the archived stories, with `archived: true`.
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StoryListResult {
    pub product: String,
    /// Oldest id first.
    pub stories: Vec<Story>,
}

impl Method for StoryList {
    const NAME: &'static str = "story.list";
    const DESCRIPTION: &'static str = "Gives the stories of a product, oldest id first. It never \
        writes. It is what `agentsi story list` prints.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = StoryListParams;
    type Result = StoryListResult;
}

pub struct StoryGet;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StoryGetResult {
    pub story: Story,
    /// The Markdown view of `agentsi story show`.
    pub markdown: String,
    /// The actions that are legal now: `approve`, `changes`, `discard`, `run`, `accept`, `reject`.
    pub actions: Vec<String>,
    /// The story log. Empty when the story has none.
    pub log: String,
    /// True when the story is in the archive record.
    pub archived: bool,
}

impl Method for StoryGet {
    const NAME: &'static str = "story.get";
    const DESCRIPTION: &'static str = "Gives one story with its Markdown view, its legal \
        actions and its log. It never writes. It is what `agentsi story show` prints.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = StoryIdParams;
    type Result = StoryGetResult;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StoryProposeResult {
    pub stories: Vec<Story>,
    pub usage: Vec<StoryUsage>,
    pub total_cost_usd: f64,
}

pub struct StoryNew;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoryNewParams {
    pub product: String,
    pub text: String,
    /// `story` or `task`. Absent: the PM picks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "type")]
    pub kind: Option<String>,
    /// The minutes that the story comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minutes: Option<String>,
}

impl Method for StoryNew {
    const NAME: &'static str = "story.new";
    const DESCRIPTION: &'static str = "The PM shapes the owner's text into one story, with one \
        repair round. It saves the story with approval `pending`. It is `agentsi story new`.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = StoryNewParams;
    type Result = StoryProposeResult;
}

pub struct StorySuggest;

impl Method for StorySuggest {
    const NAME: &'static str = "story.suggest";
    const DESCRIPTION: &'static str = "The PM proposes up to 3 stories from the snapshot of the \
        product. It is `agentsi story suggest`.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = ProductParams;
    type Result = StoryProposeResult;
}

pub struct StoryApprove;

impl Method for StoryApprove {
    const NAME: &'static str = "story.approve";
    const DESCRIPTION: &'static str = "Approves a pending story. No model call.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = StoryIdParams;
    type Result = StoryOne;
}

pub struct StoryDiscard;

impl Method for StoryDiscard {
    const NAME: &'static str = "story.discard";
    const DESCRIPTION: &'static str = "Discards a story that has no story session. No model call.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = StoryIdParams;
    type Result = StoryOne;
}

pub struct StoryChanges;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoryChangesParams {
    pub product: String,
    pub id: String,
    pub note: String,
    /// True: the PM rewrites the story with the note (`agentsi story changes`).
    /// False: only the note is recorded, as the changes action of the web does.
    #[serde(default)]
    pub rewrite: bool,
}

impl Method for StoryChanges {
    const NAME: &'static str = "story.changes";
    const DESCRIPTION: &'static str = "Records a change request. With `rewrite` the PM rewrites \
        the story with the note and the approval goes back to `pending`.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = StoryChangesParams;
    type Result = StoryProposeResult;
}

pub struct StoryRun;

impl Method for StoryRun {
    const NAME: &'static str = "story.run";
    const DESCRIPTION: &'static str = "Checks that the story may run, records a `queued` session \
        and starts it in the core. It returns at once. Linux only: the tests and gates run in \
        bubblewrap. It is `agentsi story run`.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = StoryIdParams;
    type Result = StoryOne;
}

pub struct StoryExec;

impl Method for StoryExec {
    const NAME: &'static str = "story.exec";
    const DESCRIPTION: &'static str = "Runs a queued story session and returns when it ends, \
        `ready` or `failed`. It is `agentsi story _exec`.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = StoryIdParams;
    type Result = StoryOne;
}

pub struct StoryReview;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StoryReviewResult {
    pub story: Story,
    /// `git diff --stat base..branch`, empty when the branch is gone.
    pub stat: String,
    /// `git diff base..branch`, empty when the branch is gone.
    pub diff: String,
    /// The product repo of the stories block.
    pub repo: String,
}

impl Method for StoryReview {
    const NAME: &'static str = "story.review";
    const DESCRIPTION: &'static str = "Gives the session of a story and the diff of its branch. \
        It never writes. It is what `agentsi story review` prints.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = StoryIdParams;
    type Result = StoryReviewResult;
}

pub struct StoryAccept;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StoryAcceptResult {
    pub story: Story,
    /// `merged_unchanged` or `merged_changed`.
    pub result: String,
    pub commit: String,
}

impl Method for StoryAccept {
    const NAME: &'static str = "story.accept";
    const DESCRIPTION: &'static str = "Merges the branch of a ready story with `--no-ff` into \
        the main branch of the product repo, records the result and removes the worktree and \
        the branch. It is `agentsi story accept`.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = StoryIdParams;
    type Result = StoryAcceptResult;
}

pub struct StoryReject;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoryRejectParams {
    pub product: String,
    pub id: String,
    /// Stop a session whose process still runs.
    #[serde(default)]
    pub force: bool,
}

impl Method for StoryReject {
    const NAME: &'static str = "story.reject";
    const DESCRIPTION: &'static str = "Records a `discarded` result, removes the worktree and the \
        branch and clears the session, so the story can run again. It is `agentsi story reject`.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = StoryRejectParams;
    type Result = StoryOne;
}

pub struct StoryResultSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoryResultParams {
    pub product: String,
    pub id: String,
    /// `merged_changed` or `merged_unchanged`.
    pub kind: String,
    pub commit: String,
}

impl Method for StoryResultSet {
    const NAME: &'static str = "story.result";
    const DESCRIPTION: &'static str = "Records a merge that happened outside `story accept`: \
        status `implemented` and a result with the full commit hash. It is \
        `agentsi story result`.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = StoryResultParams;
    type Result = StoryOne;
}
