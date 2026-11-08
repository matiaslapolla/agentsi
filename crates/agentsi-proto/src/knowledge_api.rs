//! The `knowledge` namespace: the index of a product and the reads that the
//! MCP server of `agentsi mcp` needs. `knowledge.index` writes. The others
//! only read.
//!
//! `knowledge.recent_minutes`, `knowledge.pending_decisions` and
//! `knowledge.role_memory` return what the tools `list_minutes`,
//! `pending_decisions` and `role_memory` print. They live here, next to
//! `knowledge.search` and `knowledge.get_doc`, because the MCP server is their
//! only client. A screen reads the same data through `minutes.*` and `decision.*`.

use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<KnowledgeIndex>()
        .method::<KnowledgeSearch>()
        .method::<KnowledgeGetDoc>()
        .method::<KnowledgeRecentMinutes>()
        .method::<KnowledgePendingDecisions>()
        .method::<KnowledgeRoleMemory>();
}

pub struct KnowledgeIndex;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbIndexParams {
    pub product: String,
    /// true: index each repo, also when its HEAD and its index time are fresh.
    #[serde(default)]
    pub force: bool,
}

/// What the index run did to one repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbRepoIndexed {
    pub path: String,
    /// true: HEAD did not change and the last run is less than 10 minutes old.
    pub skipped: bool,
    pub docs: u64,
    pub commits: u64,
    /// null when the repo has no remote or `gh` failed.
    pub issues: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbIndexResult {
    pub product: String,
    pub repos: Vec<KbRepoIndexed>,
    /// The minutes in the index.
    pub minutes: u64,
    /// The agents with a memory document in the index.
    pub memory: u64,
    /// The time of the run, in milliseconds.
    pub ms: u64,
}

impl Method for KnowledgeIndex {
    const NAME: &'static str = "knowledge.index";
    const DESCRIPTION: &'static str = "Updates the knowledge base of a product: the docs, the \
        journal, the commits and the open issues of its repos, then its minutes and its memory. \
        It is what `agentsi index` does.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = KbIndexParams;
    type Result = KbIndexResult;
}

pub struct KnowledgeSearch;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbSearchParams {
    pub product: String,
    /// Terms. Quotes for phrases, `or` for alternatives, `-` to exclude.
    pub query: String,
    /// Only this kind: `doc`, `journal`, `commit`, `issue`, `minutes` or `memory`.
    /// Without it, each kind except `memory`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// 5 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KbSearchHit {
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub scope: Option<String>,
    pub title: Option<String>,
    pub rank: f64,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KbSearchResult {
    /// The best hits first.
    pub hits: Vec<KbSearchHit>,
    /// true: no document has all the terms, so the hits match any term.
    pub any_term: bool,
}

impl Method for KnowledgeSearch {
    const NAME: &'static str = "knowledge.search";
    const DESCRIPTION: &'static str = "Full-text search over the docs, journal, commits, issues \
        and minutes of a product. When no document has all the terms, it returns the documents \
        that have any term, and sets any_term.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = KbSearchParams;
    type Result = KbSearchResult;
}

pub struct KnowledgeGetDoc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbGetDocParams {
    pub product: String,
    pub kind: String,
    /// The ref of a search hit. A hex prefix of 7 to 40 characters finds a commit.
    #[serde(rename = "ref")]
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbDoc {
    pub product: String,
    pub scope: Option<String>,
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub title: Option<String>,
    pub body: Option<String>,
    /// ISO 8601 UTC.
    pub updated_at: Option<String>,
}

impl Method for KnowledgeGetDoc {
    const NAME: &'static str = "knowledge.get_doc";
    const DESCRIPTION: &'static str = "Reads one indexed document by kind and ref.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = KbGetDocParams;
    type Result = KbDoc;
}

pub struct KnowledgeRecentMinutes;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbRecentMinutesParams {
    pub product: String,
    pub limit: u32,
    /// true: also list the archived minutes.
    #[serde(default)]
    pub include_archived: bool,
}

/// How many priorities of one minutes have a mark. A priority with no mark has the mark `unmarked`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbMarkCount {
    pub mark: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbRecentMinutes {
    pub id: String,
    pub status: Option<String>,
    /// ISO 8601 UTC.
    pub created_at: Option<String>,
    pub question: Option<String>,
    /// In the order of `agentsi status`: shorter mark names first, then by bytes.
    pub marks: Vec<KbMarkCount>,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbRecentMinutesResult {
    /// Newest first.
    pub minutes: Vec<KbRecentMinutes>,
}

impl Method for KnowledgeRecentMinutes {
    const NAME: &'static str = "knowledge.recent_minutes";
    const DESCRIPTION: &'static str = "Lists the newest minutes of a product with their marks. \
        Archived minutes are left out unless include_archived is true.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = KbRecentMinutesParams;
    type Result = KbRecentMinutesResult;
}

pub struct KnowledgePendingDecisions;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbPendingParams {
    pub product: String,
    pub limit: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbPendingRow {
    pub id: i64,
    pub minutes_id: Option<String>,
    pub text: String,
    pub why: Option<String>,
    pub action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbPendingResult {
    /// By minutes id (decisions of no minutes last), then by id.
    pub decisions: Vec<KbPendingRow>,
    /// The pending decisions that `limit` cut.
    pub hidden: u64,
}

impl Method for KnowledgePendingDecisions {
    const NAME: &'static str = "knowledge.pending_decisions";
    const DESCRIPTION: &'static str = "Lists the decisions that wait for the owner, without the \
        archived ones.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = KbPendingParams;
    type Result = KbPendingResult;
}

pub struct KnowledgeRoleMemory;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbRoleMemoryParams {
    pub product: String,
    /// An agent id such as `pm` or `dev.backend`.
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KbRoleMemoryResult {
    /// The active items, newest first, as `- [id] text`, in 6000 characters or less.
    /// Empty when the role has no memory.
    pub text: String,
}

impl Method for KnowledgeRoleMemory {
    const NAME: &'static str = "knowledge.role_memory";
    const DESCRIPTION: &'static str = "Reads what one role learned about a product, as the \
        meeting prompt shows it.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Invalid,
        ErrorKind::Unavailable,
    ];
    type Params = KbRoleMemoryParams;
    type Result = KbRoleMemoryResult;
}
