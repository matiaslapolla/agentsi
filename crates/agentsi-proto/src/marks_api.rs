//! The methods of `agentsi mark`, `decide`, `decision add`, `archive` and
//! `restore` (U20): `mark.pending`, `mark.set`, `decision.decide`,
//! `decision.add`, `archive.archive` and `archive.restore`. `decision.archived` lists
//! the archived decisions of a product.
//!
//! Each write runs in one transaction of the store and sends `data.changed`
//! for each product that it changed. The field order of `PendingItem` is the
//! key order of `agentsi mark --pending --json`.

use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub fn register(c: &mut Catalog) {
    c.method::<MarkPending>()
        .method::<MarkSet>()
        .method::<DecisionDecide>()
        .method::<DecisionAdd>()
        .method::<DecisionArchived>()
        .method::<DecisionGet>()
        .method::<ArchiveArchive>()
        .method::<ArchiveRestore>();
}

/// The verdict of the owner on one priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Accepted,
    Changed,
    Rejected,
}

impl Mark {
    pub fn as_str(self) -> &'static str {
        match self {
            Mark::Accepted => "accepted",
            Mark::Changed => "changed",
            Mark::Rejected => "rejected",
        }
    }
}

/// A priority that has no mark yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PendingPriority {
    pub minutes_id: String,
    pub idx: u32,
    pub title: String,
    pub why: String,
    pub owner_role: String,
    pub objective: Option<String>,
    /// The sources of the priority, as the minutes JSON holds them.
    pub sources: Vec<Value>,
}

/// A decision that waits for the owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PendingDecisionItem {
    pub id: i64,
    pub minutes_id: Option<String>,
    pub text: String,
    pub why: Option<String>,
    pub action: Option<String>,
    /// The unmarked priorities that the decision links in its minutes.
    pub priorities: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingItem {
    Priority(PendingPriority),
    Decision(PendingDecisionItem),
}

pub struct MarkPending;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MarkPendingParams {
    pub product: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MarkPendingResult {
    /// The unmarked priorities of the valid minutes, oldest minutes first, each followed by the
    /// pending decisions of that minutes. The decisions of no minutes come last.
    pub items: Vec<PendingItem>,
    /// Set when the decisions could not be read. `items` then holds what was read before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decisions_error: Option<String>,
}

impl Method for MarkPending {
    const NAME: &'static str = "mark.pending";
    const DESCRIPTION: &'static str = "Lists what waits for the owner in a product: the unmarked \
        priorities and the pending decisions that are not archived. It is what \
        `agentsi mark --pending --json` prints. It never writes.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = MarkPendingParams;
    type Result = MarkPendingResult;
}

pub struct MarkSet;

/// One mark to set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MarkEntry {
    pub minutes_id: String,
    /// The position of the priority in its minutes, from 0.
    pub idx: u32,
    pub mark: Mark,
    /// true: the owner did not have this priority in mind.
    #[serde(default)]
    pub is_new: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MarkSetParams {
    pub entries: Vec<MarkEntry>,
    /// true: refuse a priority that already has another mark. The same mark again passes.
    #[serde(default)]
    pub unmarked_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MarkSetResult {
    /// The number of entries that were set.
    pub marked: u32,
}

impl Method for MarkSet {
    const NAME: &'static str = "mark.set";
    const DESCRIPTION: &'static str = "Sets the marks of some priorities in one transaction. \
        The `priorities` rows and the minutes JSON change together, or none changes. A repeated \
        priority, a missing minutes or a missing priority is an error. Sends `data.changed` for \
        each product.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Invalid,
        ErrorKind::Conflict,
        ErrorKind::Unavailable,
    ];
    type Params = MarkSetParams;
    type Result = MarkSetResult;
}

/// What the owner decides about a decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Approved,
    Rejected,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Approved => "approved",
            Verdict::Rejected => "rejected",
        }
    }
}

/// The state of a decision after `decision.decide`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Approved,
    Rejected,
    /// The target of a reflection changed since the proposal.
    Stale,
}

pub struct DecisionDecide;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DecideParams {
    pub decision: i64,
    pub status: Verdict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DecideResult {
    pub product: String,
    pub minutes_id: Option<String>,
    pub status: Outcome,
    /// What to run once the decision is approved.
    pub action: Option<String>,
    /// Why the commit of a global role file failed. The approval stands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_error: Option<String>,
    /// Why a reflection decision can never be applied. The decision is rejected, and
    /// a client reports this text as an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
}

impl Method for DecisionDecide {
    const NAME: &'static str = "decision.decide";
    const DESCRIPTION: &'static str = "Approves or rejects a decision. The `decisions` row and \
        the decision inside the minutes JSON change in one transaction. An archived decision is \
        refused. A reflection decision settles its reflection. Sends `data.changed`.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Conflict,
        ErrorKind::Unavailable,
    ];
    type Params = DecideParams;
    type Result = DecideResult;
}

pub struct DecisionAdd;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DecisionAddParams {
    pub product: String,
    pub text: String,
    pub why: String,
    /// What to run once the decision is approved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DecisionAddResult {
    pub id: i64,
}

impl Method for DecisionAdd {
    const NAME: &'static str = "decision.add";
    const DESCRIPTION: &'static str = "Records a pending decision outside any meeting \
        (`kind` product). Sends `data.changed`.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = DecisionAddParams;
    type Result = DecisionAddResult;
}

pub struct DecisionArchived;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DecisionArchivedParams {
    pub product: String,
}

/// A decision that the archive record holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArchivedDecision {
    pub id: i64,
    /// `minutes`, `reflection` or `product`.
    pub kind: String,
    pub minutes_id: Option<String>,
    pub text: Option<String>,
    pub why: Option<String>,
    /// What to run once the decision is approved.
    pub action: Option<String>,
    /// `pending`, `approved`, `rejected` or `stale`.
    pub status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DecisionArchivedResult {
    /// Newest id first.
    pub decisions: Vec<ArchivedDecision>,
}

impl Method for DecisionArchived {
    const NAME: &'static str = "decision.archived";
    const DESCRIPTION: &'static str = "Lists the decisions of a product that are archived, \
        newest id first. `archive.restore` brings one back. It only reads.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = DecisionArchivedParams;
    type Result = DecisionArchivedResult;
}

pub struct DecisionGet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DecisionGetParams {
    pub product: String,
    pub ids: Vec<i64>,
}

/// One stored decision, as a view of the desk conversation reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DecisionInfo {
    pub id: i64,
    /// `minutes`, `reflection` or `product`.
    pub kind: String,
    pub minutes_id: Option<String>,
    pub text: String,
    pub why: Option<String>,
    /// What to run once the decision is approved.
    pub action: Option<String>,
    /// `pending`, `approved`, `rejected` or `stale`.
    pub status: Option<String>,
    /// True when the archive record holds the decision.
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DecisionGetResult {
    /// In the order of `ids`. An id that the product does not have is left out.
    pub decisions: Vec<DecisionInfo>,
}

impl Method for DecisionGet {
    const NAME: &'static str = "decision.get";
    const DESCRIPTION: &'static str = "Gives the decisions of a product with these ids, \
        archived or not, with their status. A view of the desk conversation reads it. It only \
        reads.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = DecisionGetParams;
    type Result = DecisionGetResult;
}

/// The kind of record that an archive record names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveKind {
    Minutes,
    Decision,
    Story,
    Product,
}

impl ArchiveKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ArchiveKind::Minutes => "minutes",
            ArchiveKind::Decision => "decision",
            ArchiveKind::Story => "story",
            ArchiveKind::Product => "product",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveParams {
    pub product: String,
    pub kind: ArchiveKind,
    /// The minutes id, the decision number or the story id. Left out for the whole product.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveResult {
    pub product: String,
    pub kind: ArchiveKind,
    pub id: Option<String>,
}

pub struct ArchiveArchive;

impl Method for ArchiveArchive {
    const NAME: &'static str = "archive.archive";
    const DESCRIPTION: &'static str = "Hides a minutes, a pending decision, a story or a whole \
        product from the lists and the counts. It writes one `archive_records` row and deletes \
        nothing. An archived record is a conflict. Sends `data.changed`.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Invalid,
        ErrorKind::Conflict,
        ErrorKind::Unavailable,
    ];
    type Params = ArchiveParams;
    type Result = ArchiveResult;
}

pub struct ArchiveRestore;

impl Method for ArchiveRestore {
    const NAME: &'static str = "archive.restore";
    const DESCRIPTION: &'static str = "Brings an archived minutes, decision, story or product \
        back to the lists. A record that is not archived is a conflict. Sends `data.changed`.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Invalid,
        ErrorKind::Conflict,
        ErrorKind::Unavailable,
    ];
    type Params = ArchiveParams;
    type Result = ArchiveResult;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_item_prints_its_kind_first_and_keeps_the_typescript_key_order() {
        let item = PendingItem::Decision(PendingDecisionItem {
            id: 2,
            minutes_id: None,
            text: "t".into(),
            why: None,
            action: None,
            priorities: vec![],
        });
        assert_eq!(
            serde_json::to_string(&item).unwrap(),
            r#"{"kind":"decision","id":2,"minutes_id":null,"text":"t","why":null,"action":null,"priorities":[]}"#
        );
        let item = PendingItem::Priority(PendingPriority {
            minutes_id: "m".into(),
            idx: 0,
            title: "t".into(),
            why: "w".into(),
            owner_role: "pm".into(),
            objective: None,
            sources: vec![],
        });
        assert_eq!(
            serde_json::to_string(&item).unwrap(),
            r#"{"kind":"priority","minutes_id":"m","idx":0,"title":"t","why":"w","owner_role":"pm","objective":null,"sources":[]}"#
        );
    }

    #[test]
    fn is_new_defaults_to_false() {
        let e: MarkEntry =
            serde_json::from_str(r#"{"minutes_id":"m","idx":1,"mark":"changed"}"#).unwrap();
        assert!(!e.is_new);
    }
}
