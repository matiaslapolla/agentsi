//! The methods of the roles (U21): `roles.list`, `roles.validate`,
//! `roles.apply` and `roles.reset`. They are what `agentsi roles` and
//! `agentsi reset` call. `roles.apply` and `roles.reset` write only when
//! `yes` is true. The field order of each struct is the key order of the
//! TypeScript output where the CLI prints the struct as it is.

use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub fn register(c: &mut Catalog) {
    c.method::<RolesList>()
        .method::<RolesBrief>()
        .method::<RolesValidate>()
        .method::<RolesApply>()
        .method::<RolesReset>();
}

/// Where the definition of an agent comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RoleSource {
    /// A global role file, unchanged.
    Global,
    /// A global role file with the overlay of the product on top.
    Overlay,
    /// An overlay with no `extends`: a role of the product alone.
    Custom,
}

/// One agent of the roster. An agent that does not resolve has `error` and null fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RosterEntry {
    pub id: String,
    pub role: Option<String>,
    pub source: Option<RoleSource>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub error: Option<String>,
}

/// The resolved definition of one agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoleDef {
    pub id: String,
    pub role: String,
    pub source: RoleSource,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The built-in tools that a meeting grants. Absent when the file lists none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<String>>,
    pub body: String,
}

pub struct RolesList;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RolesListParams {
    pub product: String,
    /// One agent id: the result holds its definition in `agent`. Without it, the result holds the roster.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RolesListResult {
    pub product: String,
    /// Each agent of the roster, in roster order. Absent when `agent` was asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roster: Option<Vec<RosterEntry>>,
    /// The agent that was asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<RoleDef>,
    /// The agent ids that have an overlay in this product, sorted.
    #[serde(default)]
    pub overlays: Vec<String>,
    /// The names of the global roles, in roster order.
    #[serde(default)]
    pub globals: Vec<String>,
    /// Warnings that the TypeScript code prints on stderr: tools that a meeting does not allow.
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Method for RolesList {
    const NAME: &'static str = "roles.list";
    const DESCRIPTION: &'static str = "Gives the roster of a product with the source, model and \
        effort of each agent, or the resolved definition of one agent. It never writes. It is what \
        `agentsi roles` prints.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = RolesListParams;
    type Result = RolesListResult;
}

pub struct RolesBrief;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RolesBriefParams {
    pub product: String,
}

/// The brief of a product, as `/agentsi setup` stores it. A field that the stored brief lacks reads as empty.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct BriefView {
    /// What the product is.
    #[serde(default)]
    pub what: String,
    #[serde(default)]
    pub audience: String,
    /// `idea`, `building`, `launched`, `growing` or `maintenance`.
    #[serde(default)]
    pub stage: String,
    #[serde(default)]
    pub objectives: Vec<crate::desk_api::Objective>,
    #[serde(default)]
    pub north_stars: Vec<String>,
    #[serde(default)]
    pub out_of_scope: Vec<String>,
    /// What each agent is for, by agent id.
    #[serde(default)]
    pub roles: std::collections::BTreeMap<String, String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RolesBriefResult {
    pub product: String,
    /// Null when the product has no brief.
    pub brief: Option<BriefView>,
}

impl Method for RolesBrief {
    const NAME: &'static str = "roles.brief";
    const DESCRIPTION: &'static str = "Gives the brief of a product, or null when it has none. \
        It never writes. The Roles screen shows it.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = RolesBriefParams;
    type Result = RolesBriefResult;
}

pub struct RolesValidate;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RolesValidateParams {
    pub product: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RolesValidateResult {
    pub product: String,
    pub valid: bool,
    pub failures: Vec<String>,
    /// The number of agents in the roster.
    pub agents: u64,
    /// Warnings that the TypeScript code prints on stderr: tools that a meeting does not allow.
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Method for RolesValidate {
    const NAME: &'static str = "roles.validate";
    const DESCRIPTION: &'static str = "Checks the roster, the overlays and the brief of a \
        product. It never writes. It is what `agentsi roles validate` prints.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = RolesValidateParams;
    type Result = RolesValidateResult;
}

/// How an `apply` registers a product that the registry does not hold yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Registration {
    pub name: String,
    /// An existing absolute path.
    pub path: String,
}

pub struct RolesApply;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RolesApplyParams {
    pub product: String,
    /// The folder of the stage: `brief.json`, `roster.json`, `agents/<id>.md` and `stories.json`. An absolute path.
    /// Send `stage_dir` or `stage`, not both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_dir: Option<String>,
    /// The inline stage: the name of each stage file to its text. The names are `brief.json`,
    /// `roster.json`, `stories.json` and `agents/<id>.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<BTreeMap<String, String>>,
    /// true: write the plan. false: only plan.
    #[serde(default)]
    pub yes: bool,
    /// Set to register the product, which the registry does not hold yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub register: Option<Registration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ChangeStatus {
    New,
    Changed,
    Removed,
}

/// A file of the stage that differs from what the database holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FileChange {
    /// `brief.json` or `agents/<id>.md`.
    pub path: String,
    pub status: ChangeStatus,
    /// A unified diff of a changed file. Empty for the other statuses.
    pub diff: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ApplyPlan {
    pub product: String,
    /// true when the registry does not hold the product yet.
    pub register: bool,
    pub roster_before: Vec<String>,
    pub roster_after: Vec<String>,
    pub changes: Vec<FileChange>,
    /// The stories block of the registry before, or null.
    pub stories_before: Option<Value>,
    pub stories_after: Option<Value>,
    pub stories_changed: bool,
}

/// What the commit of the registry did. `error` is set when git failed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CommitInfo {
    pub committed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RolesApplyResult {
    /// The plan. Null when the stage is not valid.
    pub plan: Option<ApplyPlan>,
    /// Why the stage is not valid. Empty when there is a plan.
    pub failures: Vec<String>,
    /// true when the core wrote the plan.
    pub applied: bool,
    pub commit: Option<CommitInfo>,
    /// Warnings that the TypeScript code prints on stderr: tools that a meeting does not allow.
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Method for RolesApply {
    const NAME: &'static str = "roles.apply";
    const DESCRIPTION: &'static str = "Checks a staged brief, roster and overlays and compares \
        them with the database. The stage is a folder (`stage_dir`) or inline text (`stage`). \
        With `yes` it writes the brief, the overlays and the `products` row into the store and \
        the roster into `agentsi.config.json`, commits that file and sends `data.changed` of \
        kind `product`. Without `yes` it writes nothing. It is what `agentsi roles apply` does.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Invalid,
        ErrorKind::Unavailable,
    ];
    type Params = RolesApplyParams;
    type Result = RolesApplyResult;
}

pub struct RolesReset;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RolesResetParams {
    pub product: String,
    /// One agent id to return to its global role. Absent: reset the whole product.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// true: also delete the memory.
    #[serde(default)]
    pub memory: bool,
    /// true: write the plan. false: only plan.
    #[serde(default)]
    pub yes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResetPlan {
    pub product: String,
    /// Null: the whole product.
    pub agent: Option<String>,
    /// What the reset deletes: `brief.json`, `agents/<id>.md` and `memory/<id>.json`.
    pub removals: Vec<String>,
    /// What the reset edits: `brief.json` when an entry of its roles leaves.
    pub edits: Vec<String>,
    pub roster_before: Vec<String>,
    pub roster_after: Vec<String>,
    /// true when the reset would change nothing.
    pub noop: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RolesResetResult {
    pub plan: ResetPlan,
    pub applied: bool,
    pub commit: Option<CommitInfo>,
}

impl Method for RolesReset {
    const NAME: &'static str = "roles.reset";
    const DESCRIPTION: &'static str = "Plans the return of an agent, or of the whole product, \
        to the global roles. With `yes` it deletes the overlays (and the brief and the memory \
        as the plan lists), edits the roster in `agentsi.config.json` and commits that file. \
        Without `yes` it writes nothing. It is what `agentsi reset` does.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = RolesResetParams;
    type Result = RolesResetResult;
}
