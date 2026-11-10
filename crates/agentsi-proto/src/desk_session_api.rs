//! The `desk` namespace: the desk conversation of a product with its desk session, the one
//! `claude -p` process of the core for the front desk. Spec:
//! `docs/specs/agentsi-desktop-conversation.md`.

use crate::method::{Catalog, Event, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub fn register(c: &mut Catalog) {
    c.method::<DeskHistory>()
        .method::<DeskSend>()
        .method::<DeskStop>()
        .method::<DeskReset>()
        .method::<DeskShow>()
        .method::<DeskAuth>()
        .method::<DeskCheck>()
        .method::<DeskPage>()
        .event::<DeskStateEvent>()
        .event::<DeskDelta>()
        .event::<DeskItemEvent>();
}

/// The catalog of views. Claude shows one with the MCP tool `show_<kind>`. The doc comment of
/// each variant is the description of its tool: it says when Claude calls it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum View {
    /// Shows pending decisions of the product with Approve and Reject buttons. Call it when
    /// the owner asks what waits for them, or when your reply names decisions. The ids come
    /// from pending_decisions.
    Decisions { ids: Vec<i64> },
    /// Shows the priorities of one minutes with their marks, and Accept, Accept with changes
    /// and Reject buttons for each unmarked priority. Call it when the owner asks about the
    /// priorities of a meeting. The minutes id comes from list_minutes.
    Priorities { minutes_id: String },
    /// Shows one story with its state and its acceptance criteria, and a button that opens
    /// the story screen. Call it when your reply is about one story. The id looks like S014
    /// or T003.
    Story { id: String },
    /// Shows the diff of the branch of a story, and a button that opens the story screen.
    /// Call it when the owner asks what a story changed in the code.
    Diff { story_id: String },
    /// Shows a small table. Call it when your answer compares a few items on a few fields.
    /// 1 to 12 columns, at most 200 rows, each row as long as columns, at most 200 characters
    /// in each cell. The data is a copy: it does not change later.
    Table {
        title: String,
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    /// Shows a chart of one series: bar, line, area or pie. Call it when numbers read better
    /// as a picture, such as cost by week. 1 to 50 points, labels as long as values, each
    /// value finite. A pie takes no negative value.
    Chart {
        chart: ChartKind,
        title: String,
        labels: Vec<String>,
        values: Vec<f64>,
    },
    /// Shows a checklist that the owner can check. Call it for steps that the owner does
    /// by hand, such as launch steps. 1 to 30 items, at most 200 characters in each. The next
    /// owner message tells you what the owner checked.
    Checklist { title: String, items: Vec<String> },
    /// Shows one HTML page as a title and an Open in browser button. The app never draws the
    /// page. Call it only for a rich report that Markdown cannot hold. At most 120 characters
    /// in title and 512000 characters in html. Put all styles and scripts inline.
    Page { title: String, html: String },
}

/// The kind of a chart view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    Bar,
    Line,
    Area,
    Pie,
}

impl View {
    /// The `kind` of the view, for example `decisions`.
    pub fn kind(&self) -> &'static str {
        match self {
            View::Decisions { .. } => "decisions",
            View::Priorities { .. } => "priorities",
            View::Story { .. } => "story",
            View::Diff { .. } => "diff",
            View::Table { .. } => "table",
            View::Chart { .. } => "chart",
            View::Checklist { .. } => "checklist",
            View::Page { .. } => "page",
        }
    }
}

/// One `show_<kind>` tool of the MCP server, from one variant of `View`.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewTool {
    pub kind: String,
    /// `show_<kind>`.
    pub name: String,
    pub description: String,
    /// The JSON Schema of the arguments: the fields of the variant, without `kind`.
    pub input_schema: Value,
}

/// One tool for each variant of `View`, in the order of the enum.
pub fn view_tools() -> Vec<ViewTool> {
    let generator = schemars::generate::SchemaSettings::draft2020_12()
        .with(|s| s.inline_subschemas = true)
        .into_generator();
    let root = generator.into_root_schema_for::<View>().to_value();
    let variants = match root.get("oneOf").and_then(Value::as_array) {
        Some(all) => all.clone(),
        None => vec![root],
    };
    variants.into_iter().map(view_tool).collect()
}

fn view_tool(mut variant: Value) -> ViewTool {
    let kind = variant["properties"]["kind"]["const"]
        .as_str()
        .expect("each variant of View has a const kind")
        .to_string();
    let description = variant
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let object = variant.as_object_mut().expect("a variant is an object");
    object.remove("description");
    if let Some(p) = object.get_mut("properties").and_then(Value::as_object_mut) {
        p.remove("kind");
    }
    if let Some(r) = object.get_mut("required").and_then(Value::as_array_mut) {
        r.retain(|k| k != "kind");
    }
    ViewTool {
        name: format!("show_{kind}"),
        kind,
        description,
        input_schema: variant,
    }
}

/// The state of the desk session of a product.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeskState {
    /// No process runs. The next owner message starts one.
    Stopped,
    /// The process starts and has not answered yet.
    Starting,
    /// The process waits for an owner message.
    Idle,
    /// A reply runs.
    Working,
    /// The login of Claude Code is not usable. `message` says what to do.
    Auth,
    /// The process failed. `message` says why. The next owner message starts a new process.
    Failed,
}

/// The result of the login check of `claude`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    /// `loggedIn` is true.
    Ready,
    /// `loggedIn` is false.
    SignedOut,
    /// No `claude` program on the PATH.
    Missing,
    /// A reply failed with an auth error: Claude Code did not use the login.
    NotRead,
    /// Other output. The message is its first line.
    Unknown,
}

impl AuthState {
    /// The text that the desk conversation shows in place of the input, or `None` for `ready`.
    /// For `unknown`, the caller gives the first line of the output.
    pub fn message(self) -> Option<&'static str> {
        match self {
            AuthState::Ready | AuthState::Unknown => None,
            AuthState::SignedOut => Some(
                "Claude Code is not signed in. Run `claude auth login` in a terminal, then \
                 select Check again.",
            ),
            AuthState::Missing => Some("AgentSI cannot find the `claude` program on the PATH."),
            AuthState::NotRead => Some(
                "Claude Code did not use your login. A newer version can run `claude -p` in \
                 bare mode.",
            ),
        }
    }
}

/// One stored item of a desk conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeskItem {
    /// A message of the owner.
    Owner { text: String },
    /// The text of one assistant message of Claude, in Markdown.
    Reply { text: String },
    /// One tool use of Claude, as one line such as `Read: docs/a.md`.
    Tool { line: String },
    /// A view that Claude showed.
    View {
        view: View,
        /// The items of a checklist that the owner checked, from 0, in order.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        checked: Vec<u32>,
        /// The characters of the HTML of a page. `desk.item` and `desk.history` give a page
        /// with an empty `html` and this length. `desk.page` gives the HTML.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        html_len: Option<u64>,
    },
    /// A desk note: one line about a write of product data. The next owner message carries it.
    Note { text: String },
    /// The end of a reply.
    End {
        cost_usd: f64,
        duration_ms: u64,
        /// The error of the reply, if it failed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// A failure of the desk session, such as a crash or a time limit.
    Error { message: String },
}

impl DeskItem {
    /// The `kind` of the item, as stored.
    pub fn kind(&self) -> &'static str {
        match self {
            DeskItem::Owner { .. } => "owner",
            DeskItem::Reply { .. } => "reply",
            DeskItem::Tool { .. } => "tool",
            DeskItem::View { .. } => "view",
            DeskItem::Note { .. } => "note",
            DeskItem::End { .. } => "end",
            DeskItem::Error { .. } => "error",
        }
    }
}

/// One item with its place in the desk conversation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeskEntry {
    /// The number of the item in the conversation of its product. It only grows.
    pub seq: i64,
    /// ISO 8601 UTC.
    pub at: String,
    pub item: DeskItem,
}

pub struct DeskHistory;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskHistoryParams {
    pub product: String,
    /// Only the items with a smaller `seq`. Leave it out for the newest items.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<i64>,
    /// The most items to give. The default is 200.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeskHistoryResult {
    /// The uuid of the desk session, or null when the product has none yet.
    pub session: Option<String>,
    pub state: DeskState,
    /// What the state needs: the auth message, or the failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Oldest first.
    pub items: Vec<DeskEntry>,
}

impl Method for DeskHistory {
    const NAME: &'static str = "desk.history";
    const DESCRIPTION: &'static str = "Gives the stored items of the desk conversation of a \
        product, oldest first, and the state of its desk session.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = DeskHistoryParams;
    type Result = DeskHistoryResult;
}

pub struct DeskSend;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskSendParams {
    pub product: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskSendResult {
    /// The `seq` of the owner item.
    pub seq: i64,
}

impl Method for DeskSend {
    const NAME: &'static str = "desk.send";
    const DESCRIPTION: &'static str = "Sends an owner message to the desk session of a product, \
        with the unsent desk notes before it. It starts the process when none runs. The reply \
        comes as `desk.delta` and `desk.item` events. `busy` while a reply runs. `unavailable` \
        when the login of Claude Code is not usable: then no process starts.";
    const ERRORS: &'static [ErrorKind] =
        &[ErrorKind::NotFound, ErrorKind::Busy, ErrorKind::Unavailable];
    type Params = DeskSendParams;
    type Result = DeskSendResult;
}

pub struct DeskStop;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskProductParams {
    pub product: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskStopResult {
    /// False when no process ran.
    pub stopped: bool,
}

impl Method for DeskStop {
    const NAME: &'static str = "desk.stop";
    const DESCRIPTION: &'static str = "Stops the process of the desk session of a product. The \
        items and the session id stay, so the next owner message resumes the session.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound];
    type Params = DeskProductParams;
    type Result = DeskStopResult;
}

pub struct DeskReset;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskResetResult {
    /// The uuid of the new desk session.
    pub session: String,
}

impl Method for DeskReset {
    const NAME: &'static str = "desk.reset";
    const DESCRIPTION: &'static str = "Starts a new desk conversation: stops the process, \
        deletes the items and gives the product a new session uuid.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = DeskProductParams;
    type Result = DeskResetResult;
}

pub struct DeskShow;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeskShowParams {
    /// The uuid of the desk session, from `AGENTSI_DESK_SESSION`.
    pub session: String,
    pub view: View,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskShowResult {
    /// The `seq` of the stored `view` item.
    pub view_id: i64,
}

impl Method for DeskShow {
    const NAME: &'static str = "desk.show";
    const DESCRIPTION: &'static str = "Shows a view in the desk conversation of a session. The \
        `show_*` MCP tools call it. `invalid` for a view that does not match the catalog, \
        `not_found` for an unknown session or an id that the product does not have. After the \
        check, the core stores a `view` item and sends `desk.item`.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = DeskShowParams;
    type Result = DeskShowResult;
}

pub struct DeskCheck;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskCheckParams {
    pub product: String,
    /// The `seq` of the `view` item of a checklist.
    pub seq: i64,
    /// All the items that are checked now, from 0. An item that is not here is not checked.
    pub checked: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskSeqResult {
    pub seq: i64,
}

impl Method for DeskCheck {
    const NAME: &'static str = "desk.check";
    const DESCRIPTION: &'static str = "Stores the checked items of a checklist view and a desk \
        note that the next owner message carries. It writes no product data. `not_found` for \
        an unknown item, `invalid` for an item that is not a checklist or an index out of range.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = DeskCheckParams;
    type Result = DeskSeqResult;
}

pub struct DeskPage;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskPageParams {
    pub product: String,
    /// The `seq` of the `view` item of a page.
    pub seq: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskPageResult {
    pub title: String,
    pub html: String,
}

impl Method for DeskPage {
    const NAME: &'static str = "desk.page";
    const DESCRIPTION: &'static str = "Gives the title and the HTML of a page view. The app \
        writes it to a file and opens the file in the browser. `not_found` for an unknown \
        item, `invalid` for an item that is not a page.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = DeskPageParams;
    type Result = DeskPageResult;
}

pub struct DeskAuth;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskAuthParams {
    /// Run `claude auth status --json` again. Else the core gives the last result.
    #[serde(default)]
    pub recheck: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskAuthResult {
    pub state: AuthState,
    /// What the owner must do. Null for `ready`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Method for DeskAuth {
    const NAME: &'static str = "desk.auth";
    const DESCRIPTION: &'static str = "Gives the state of the login of Claude Code, from \
        `claude auth status --json` with a limit of 5 s. The core never reads the credentials.";
    const ERRORS: &'static [ErrorKind] = &[];
    type Params = DeskAuthParams;
    type Result = DeskAuthResult;
}

pub struct DeskStateEvent;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskStateParams {
    pub product: String,
    pub state: DeskState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Event for DeskStateEvent {
    const NAME: &'static str = "desk.state";
    const DESCRIPTION: &'static str = "The state of the desk session of a product changed.";
    type Params = DeskStateParams;
}

pub struct DeskDelta;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeskDeltaParams {
    pub product: String,
    /// The next text of the reply that streams. The `reply` item holds the whole text.
    pub text: String,
}

impl Event for DeskDelta {
    const NAME: &'static str = "desk.delta";
    const DESCRIPTION: &'static str = "A part of the text of a reply, while it streams.";
    type Params = DeskDeltaParams;
}

pub struct DeskItemEvent;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeskItemParams {
    pub product: String,
    pub seq: i64,
    /// ISO 8601 UTC.
    pub at: String,
    pub item: DeskItem,
}

impl Event for DeskItemEvent {
    const NAME: &'static str = "desk.item";
    const DESCRIPTION: &'static str = "The core stored an item of the desk conversation of a \
        product.";
    type Params = DeskItemParams;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn each_view_gives_a_show_tool_with_a_description_and_no_kind() {
        let tools = view_tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "show_decisions",
                "show_priorities",
                "show_story",
                "show_diff",
                "show_table",
                "show_chart",
                "show_checklist",
                "show_page"
            ]
        );
        let chart = &tools[5];
        assert_eq!(
            chart.input_schema["properties"]["chart"]["enum"],
            json!(["bar", "line", "area", "pie"])
        );
        let story = &tools[2];
        assert_eq!(story.input_schema["required"], json!(["id"]));
        assert!(story.description.contains("story screen"), "{story:?}");
        assert_eq!(tools[3].input_schema["required"], json!(["story_id"]));
        let t = &tools[0];
        assert!(
            t.description.starts_with("Shows pending decisions"),
            "{t:?}"
        );
        assert!(!t.description.contains('\n'), "{t:?}");
        assert_eq!(t.input_schema["type"], "object");
        assert_eq!(t.input_schema["required"], json!(["ids"]));
        assert!(t.input_schema["properties"].get("kind").is_none());
        assert_eq!(t.input_schema["properties"]["ids"]["type"], "array");
    }

    #[test]
    fn a_view_reads_its_kind_and_refuses_unknown_input() {
        let v: View = serde_json::from_value(json!({"kind": "decisions", "ids": [3]})).unwrap();
        assert_eq!(v.kind(), "decisions");
        assert_eq!(
            serde_json::to_value(&v).unwrap(),
            json!({"kind": "decisions", "ids": [3]})
        );
        for bad in [
            json!({"kind": "nope"}),
            json!({"kind": "decisions"}),
            json!({"kind": "decisions", "ids": ["x"]}),
            json!({"kind": "decisions", "ids": [1], "extra": 1}),
        ] {
            assert!(
                serde_json::from_value::<View>(bad.clone()).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn an_item_is_tagged_by_kind() {
        let item = DeskItem::View {
            view: View::Decisions { ids: vec![1] },
            checked: Vec::new(),
            html_len: None,
        };
        assert_eq!(
            serde_json::to_value(&item).unwrap(),
            json!({"kind": "view", "view": {"kind": "decisions", "ids": [1]}})
        );
        assert_eq!(item.kind(), "view");
        let end: DeskItem =
            serde_json::from_value(json!({"kind": "end", "cost_usd": 0.1, "duration_ms": 5}))
                .unwrap();
        assert_eq!(
            end,
            DeskItem::End {
                cost_usd: 0.1,
                duration_ms: 5,
                error: None
            }
        );
    }

    #[test]
    fn each_auth_state_but_ready_and_unknown_has_a_message() {
        assert_eq!(AuthState::Ready.message(), None);
        assert!(
            AuthState::SignedOut
                .message()
                .unwrap()
                .contains("claude auth login")
        );
        assert!(AuthState::Missing.message().is_some());
        assert!(AuthState::NotRead.message().is_some());
    }
}
