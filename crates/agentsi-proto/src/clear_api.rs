//! `data.clear_plan` and `data.clear`: what the "Data" section of the settings screen
//! and `agentsi clear` delete. Spec: `docs/specs/agentsi-desktop-clear-data.md`.

use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<DataClearPlan>().method::<DataClear>();
}

/// What the owner selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClearKind {
    /// One product: its rows, its `workspace/<product>/` folder and its registry entry.
    Product,
    /// The minutes of one product, with what hangs on them, all or older than a date.
    Meetings,
    /// The done and rejected stories of one product.
    Stories,
    /// The memory, overlays, brief and reflections of one product.
    RoleLearning,
    /// The `audio/` folders of one product or of all, and their listens.
    CallAudio,
    /// The files of `voice/transcripts/`, all or older than a date.
    VoiceTranscripts,
    /// `logs/hook.log`.
    Logs,
    /// Each backup but the newest, with the registry copies.
    Backups,
    /// Each item above for each product, and the products of the registry.
    Everything,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClearItem {
    pub item: ClearKind,
    /// The product. Required for `product`, `meetings`, `stories` and `role_learning`. Optional for `call_audio`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// `YYYY-MM-DD`, in UTC. For `meetings` and `voice_transcripts`: only what is older than this day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClearTable {
    pub table: String,
    pub rows: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClearFile {
    /// Relative to the data root.
    pub path: String,
    pub bytes: u64,
    /// True for a folder. The delete removes it with what it holds.
    pub dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClearItemPlan {
    #[serde(flatten)]
    pub item: ClearItem,
    /// The rows per table. A table with no row to delete is left out.
    pub tables: Vec<ClearTable>,
    pub files: Vec<ClearFile>,
    /// The sum of the rows of `tables`.
    pub rows: u64,
    /// The sum of the bytes of `files`.
    pub bytes: u64,
    /// True when the delete removes a product from the registry `agentsi.config.json`.
    pub registry: bool,
}

pub struct DataClearPlan;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DataClearPlanParams {
    /// The items to delete. An empty list gives only `data_root` and `root_bytes`.
    pub items: Vec<ClearItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DataClearPlanResult {
    /// Pass it to `data.clear`. It holds for 5 minutes.
    pub plan_id: String,
    pub data_root: String,
    /// The size of the data root now, without the database file shrunk by the delete.
    pub root_bytes: u64,
    pub items: Vec<ClearItemPlan>,
    pub rows: u64,
    /// The files and folders to remove.
    pub files: u64,
    pub bytes: u64,
}

impl Method for DataClearPlan {
    const NAME: &'static str = "data.clear_plan";
    const DESCRIPTION: &'static str = "Gives the clear plan of the items: the rows per table, the files and the bytes \
        that `data.clear` removes. It writes nothing.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::Invalid,
        ErrorKind::NotFound,
        ErrorKind::Unavailable,
    ];
    type Params = DataClearPlanParams;
    type Result = DataClearPlanResult;
}

pub struct DataClear;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DataClearParams {
    pub items: Vec<ClearItem>,
    /// The `plan_id` of a `data.clear_plan` of the last 5 minutes with the same items.
    pub plan_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClearFailure {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DataClearResult {
    /// The backup that ran before the delete.
    pub backup: String,
    /// The copy of the registry in the backup folder.
    pub registry_backup: Option<String>,
    /// The rows deleted, equal to `rows` of the plan.
    pub rows: u64,
    /// The files and folders removed.
    pub files: u64,
    /// The bytes the plan listed.
    pub bytes: u64,
    /// The products removed from the registry.
    pub registry_removed: Vec<String>,
    /// The files that could not go. The rows stay deleted.
    pub failed: Vec<ClearFailure>,
}

impl Method for DataClear {
    const NAME: &'static str = "data.clear";
    const DESCRIPTION: &'static str = "Deletes the items of a plan. It runs `data.backup` first, and deletes nothing if the backup fails. \
        The rows go in one transaction, then the files. The core refuses with `plan_changed` when the plan is older than \
        5 minutes or the data differs from the plan, and with `busy` when a product has a running pane, meeting, story session or reflection.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::Invalid,
        ErrorKind::Busy,
        ErrorKind::PlanChanged,
        ErrorKind::Unavailable,
    ];
    type Params = DataClearParams;
    type Result = DataClearResult;
}
