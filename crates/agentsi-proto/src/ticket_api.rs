//! The tracker: the `ticket.*` methods and the type `Ticket`. Spec:
//! `docs/specs/agentsi-desktop-tracker.md`. The field names of `Ticket` are the
//! contract of each client that shows tickets.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TicketKind {
    Note,
    Todo,
    Question,
}

impl TicketKind {
    pub const ALL: [TicketKind; 3] = [TicketKind::Note, TicketKind::Todo, TicketKind::Question];

    pub fn as_str(self) -> &'static str {
        match self {
            TicketKind::Note => "note",
            TicketKind::Todo => "todo",
            TicketKind::Question => "question",
        }
    }

    pub fn parse(text: &str) -> Option<TicketKind> {
        TicketKind::ALL.into_iter().find(|k| k.as_str() == text)
    }
}

/// `open` and `waiting` are the open states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TicketState {
    Open,
    Waiting,
    Done,
    Dropped,
}

impl TicketState {
    /// In the order of `ticket.list`.
    pub const ALL: [TicketState; 4] = [
        TicketState::Open,
        TicketState::Waiting,
        TicketState::Done,
        TicketState::Dropped,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TicketState::Open => "open",
            TicketState::Waiting => "waiting",
            TicketState::Done => "done",
            TicketState::Dropped => "dropped",
        }
    }

    pub fn parse(text: &str) -> Option<TicketState> {
        TicketState::ALL.into_iter().find(|s| s.as_str() == text)
    }

    pub fn is_open(self) -> bool {
        matches!(self, TicketState::Open | TicketState::Waiting)
    }
}

/// Who wrote a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthorBy {
    Owner,
    Agent,
    Session,
}

impl AuthorBy {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthorBy::Owner => "owner",
            AuthorBy::Agent => "agent",
            AuthorBy::Session => "session",
        }
    }
}

/// The author or the last writer of a ticket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Author {
    pub by: AuthorBy,
    /// The name of the owner, the agent id, or the session id. A client of the owner leaves
    /// it out, and the core writes the name of the owner block.
    #[serde(default)]
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkType {
    Story,
    Decision,
    Minutes,
    Priority,
    Ticket,
    Url,
    Commit,
    Path,
    Issue,
}

impl LinkType {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkType::Story => "story",
            LinkType::Decision => "decision",
            LinkType::Minutes => "minutes",
            LinkType::Priority => "priority",
            LinkType::Ticket => "ticket",
            LinkType::Url => "url",
            LinkType::Commit => "commit",
            LinkType::Path => "path",
            LinkType::Issue => "issue",
        }
    }
}

/// A link of a ticket. `story`, `decision`, `minutes`, `priority` (`<minutes id>#<idx>`) and
/// `ticket` must exist. `url` is an http(s) URL. `commit`, `path` and `issue` are free text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Link {
    #[serde(rename = "type")]
    pub kind: LinkType,
    #[serde(rename = "ref")]
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Ticket {
    pub product: String,
    /// `C` and 3 or more digits, for example `C007`.
    pub id: String,
    pub scope: Option<String>,
    pub kind: TicketKind,
    pub state: TicketState,
    pub title: String,
    /// Markdown.
    pub body: String,
    /// Set only in the state `waiting`.
    pub waiting_on: Option<String>,
    /// `owner`, `claude`, an agent id of the roster, or none.
    pub assignee: Option<String>,
    pub author: Author,
    pub updated_by: Author,
    pub links: Vec<Link>,
    /// An objective id of the brief.
    pub objective: Option<String>,
    /// ISO 8601 UTC with 3 fraction digits.
    pub created_at: String,
    pub updated_at: String,
    /// Set only in the states `done` and `dropped`.
    pub closed_at: Option<String>,
}

impl Ticket {
    /// `<id> [<kind>[, waiting on <waiting_on>]] <title>[ (for <assignee>)]`
    pub fn line(&self) -> String {
        let waiting = self
            .waiting_on
            .as_ref()
            .map(|w| format!(", waiting on {w}"))
            .unwrap_or_default();
        let assignee = self
            .assignee
            .as_ref()
            .map(|a| format!(" (for {a})"))
            .unwrap_or_default();
        format!(
            "{} [{}{waiting}] {}{assignee}",
            self.id,
            self.kind.as_str(),
            self.title
        )
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TicketListParams {
    pub product: String,
    /// `[open, waiting, done]` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub states: Option<Vec<TicketState>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<TicketKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Only the tickets with this author kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<AuthorBy>,
    /// Only the tickets that have this link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<Link>,
    /// 1 to 200. 50 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TicketListResult {
    /// By state (`open`, `waiting`, `done`, `dropped`), then the newest `updated_at`, then the
    /// highest id.
    pub tickets: Vec<Ticket>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TicketGetParams {
    pub product: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TicketGetResult {
    pub ticket: Ticket,
    /// The ticket as one Markdown text.
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TicketAddParams {
    pub product: String,
    pub kind: TicketKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<Link>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    pub author: Author,
}

/// Changes only the given fields. An empty `assignee`, `scope` or `objective` clears the
/// field. `links` replaces the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TicketUpdateParams {
    pub product: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<TicketKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<Link>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    pub by: Author,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TicketMoveParams {
    pub product: String,
    pub id: String,
    pub state: TicketState,
    /// Necessary for a move from `open` to `waiting`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<String>,
    pub by: Author,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TicketResult {
    pub ticket: Ticket,
}
