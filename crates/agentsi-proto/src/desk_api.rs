//! The `product` and `minutes` namespaces: the products of the store of a core,
//! their front desk and their minutes. All methods here only read.

use crate::core_api::Empty;
use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<ProductList>()
        .method::<ProductDesk>()
        .method::<MinutesList>()
        .method::<MinutesGet>();
}

pub struct ProductList;

/// One product of the store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProductInfo {
    pub id: String,
    /// The name of the product, or the id when the store has no name.
    pub name: String,
    /// The first folder of the product, or null when it has none.
    pub path: Option<String>,
    /// True when the product is in the archive record. An older core sends no field, and the
    /// field then reads as false. The core leaves the field out when it is false.
    #[serde(default, skip_serializing_if = "is_false")]
    pub archived: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProductListResult {
    /// The products, in the order of their ids.
    pub products: Vec<ProductInfo>,
}

impl Method for ProductList {
    const NAME: &'static str = "product.list";
    const DESCRIPTION: &'static str = "Gives the products of the store of this core. With no \
        database file, the list is empty and the core does not make the file.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::Unavailable];
    type Params = Empty;
    type Result = ProductListResult;
}

pub struct ProductDesk;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProductDeskParams {
    pub product: String,
}

/// What the store holds about the index of a product.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IndexSummary {
    pub docs: u64,
    pub commits: u64,
    pub issues: u64,
    /// When the last index run ended (ISO 8601 UTC), or null when none ran.
    pub indexed_at: Option<String>,
    pub last_commit: Option<LastCommit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LastCommit {
    /// `YYYY-MM-DD`.
    pub date: String,
    pub title: String,
}

/// One objective of the brief of a product.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Objective {
    pub id: String,
    pub text: String,
    pub signal: String,
    pub due: String,
}

/// A decision that waits for the owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PendingDecision {
    pub id: i64,
    /// `minutes`, `reflection` or `product`.
    pub kind: String,
    pub minutes_id: Option<String>,
    pub reflection_id: Option<String>,
    pub text: String,
    pub why: Option<String>,
    pub action: Option<String>,
    /// The unmarked priorities of its minutes that it links, as `agentsi desk` prints them.
    #[serde(default)]
    pub priorities: Vec<u32>,
}

/// How many priorities of one minutes have each mark.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MarkCounts {
    pub accepted: u32,
    pub changed: u32,
    pub rejected: u32,
    pub unmarked: u32,
}

/// One row of a list of minutes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MinutesRow {
    pub id: String,
    pub question: String,
    /// `valid` or `invalid`.
    pub status: Option<String>,
    /// ISO 8601 UTC.
    pub created_at: Option<String>,
    pub scope: Option<String>,
    pub marks: MarkCounts,
    /// The owner listened to the call of this minutes.
    pub heard: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UsageTotals {
    /// The number of usage rows.
    pub rows: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeskResult {
    pub product: ProductInfo,
    /// `active` or `planned`, from the store.
    pub status: Option<String>,
    /// The agent ids of the registry of the data root. Empty with no registry.
    pub roles: Vec<String>,
    pub index: IndexSummary,
    /// The objectives of the brief. Empty with no brief.
    pub objectives: Vec<Objective>,
    /// The pending decisions that are not archived, in the order of `agentsi desk`.
    pub pending_decisions: Vec<PendingDecision>,
    /// The minutes of the product that are not archived.
    pub minutes_total: u64,
    /// The 3 newest minutes.
    pub last_minutes: Vec<MinutesRow>,
    pub usage: UsageTotals,
}

impl Method for ProductDesk {
    const NAME: &'static str = "product.desk";
    const DESCRIPTION: &'static str = "Gives the front desk of a product: its state, the index, \
        the objectives, the pending decisions and the last minutes. It is what `agentsi desk` and \
        `agentsi status` show today. It never writes.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = ProductDeskParams;
    type Result = DeskResult;
}

pub struct MinutesList;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MinutesListParams {
    pub product: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MinutesListResult {
    /// The minutes that are not archived, newest first.
    pub minutes: Vec<MinutesRow>,
}

impl Method for MinutesList {
    const NAME: &'static str = "minutes.list";
    const DESCRIPTION: &'static str = "Gives one row for each minutes of a product that is not \
        archived, newest first. A product with no database or no minutes gives an empty list.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::Unavailable];
    type Params = MinutesListParams;
    type Result = MinutesListResult;
}

pub struct MinutesGet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MinutesGetParams {
    /// The id of the minutes. Ids are global.
    pub id: String,
}

/// A source of a priority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceRef {
    /// `commit`, `issue`, `path`, `url` or `story`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PriorityInfo {
    pub idx: u32,
    pub title: String,
    pub why: String,
    pub owner_role: Option<String>,
    /// The id of the objective of the brief that this priority serves.
    pub objective: Option<String>,
    /// `accepted`, `changed` or `rejected`. Null while unmarked.
    pub mark: Option<String>,
    pub is_new: bool,
    pub sources: Vec<SourceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Position {
    pub role: String,
    pub view: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Disagreement {
    pub topic: String,
    pub positions: Vec<Position>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MinutesDecision {
    /// The id of the decision row. Null when the store has no row with this text.
    pub id: Option<i64>,
    pub text: String,
    pub why: Option<String>,
    pub action: Option<String>,
    /// `pending`, `approved` or `rejected`. The row of the store wins over the minutes file.
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoleAnswerInfo {
    pub role: String,
    pub model: Option<String>,
    pub answer: String,
    pub concerns: Vec<String>,
    pub memory_notes: Vec<String>,
}

/// The result of the check of a minutes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MinutesCheck {
    pub passed: bool,
    pub repaired: bool,
    pub failures: Vec<String>,
}

/// The usage of one role in one meeting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UsageInfo {
    pub role: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MinutesDetail {
    pub id: String,
    pub product: String,
    pub scope: Option<String>,
    pub question: String,
    pub status: Option<String>,
    pub created_at: Option<String>,
    pub archived: bool,
    pub heard: bool,
    pub check: Option<MinutesCheck>,
    pub priorities: Vec<PriorityInfo>,
    pub disagreements: Vec<Disagreement>,
    pub decisions: Vec<MinutesDecision>,
    /// The answer of each role, in the order pm, dev, qa, scout, social, then the others.
    pub roles: Vec<RoleAnswerInfo>,
    pub usage: Vec<UsageInfo>,
    pub usage_totals: UsageTotals,
}

impl Method for MinutesGet {
    const NAME: &'static str = "minutes.get";
    const DESCRIPTION: &'static str = "Gives one minutes with its priorities, sources, \
        disagreements, decisions, check result and usage. The marks and the decision status \
        come from the store tables, as the web app shows them.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = MinutesGetParams;
    type Result = MinutesDetail;
}
