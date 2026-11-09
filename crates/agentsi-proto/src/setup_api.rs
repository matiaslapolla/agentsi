//! The methods of the setup screen: `setup.scan` reads a product folder with code only,
//! `setup.draft` drafts the forms with one Claude call, and `setup.outcome` records what
//! the owner kept of a draft.

use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub fn register(c: &mut Catalog) {
    c.method::<SetupScan>()
        .method::<SetupDraft>()
        .method::<SetupOutcome>();
}

pub struct SetupScan;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SetupScanParams {
    /// An existing absolute folder on the host of the core.
    pub path: String,
}

/// The commands that the manifest of the folder proposes for Stories mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Manifest {
    /// `package.json`, `Cargo.toml`, `pyproject.toml` or `composer.json`: the first one found.
    pub file: String,
    /// A test command with `{files}`. Null when the manifest names no runner that takes files.
    pub test: Option<String>,
    /// The typecheck and lint commands that the manifest has.
    pub gate: Vec<String>,
}

/// The tools of the host that the forms need.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ScanTools {
    /// True when `bwrap` is on the `PATH`. Without it, Stories mode cannot run.
    pub bwrap: bool,
    /// True when `codebase-memory-mcp` is on the `PATH`.
    pub codebase_memory_mcp: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SetupScanResult {
    pub path: String,
    /// The folder name in lowercase, with `-` for each character that an id does not permit.
    pub id: String,
    /// The folder name.
    pub name: String,
    /// The product of the registry that holds the folder, as `product.whoami` finds it.
    pub registered: Option<String>,
    pub manifest: Option<Manifest>,
    /// The north-star candidates that exist, relative to the folder: `docs/product.md`,
    /// `docs/vision*` and `ROADMAP*`.
    pub north_stars: Vec<String>,
    pub tools: ScanTools,
}

impl Method for SetupScan {
    const NAME: &'static str = "setup.scan";
    const DESCRIPTION: &'static str = "Reads a product folder for the setup screen: a proposed \
        id and name, the registered product, the commands of the manifest, the north-star docs \
        and the tools on the PATH. It writes nothing and calls no model.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::Invalid];
    type Params = SetupScanParams;
    type Result = SetupScanResult;
}

pub struct SetupDraft;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SetupDraftParams {
    /// An existing absolute folder on the host of the core.
    pub path: String,
    /// An earlier draft to repair: the `fields` of its result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Value>,
    /// Why `roles.apply` refused the stage of the earlier draft.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SetupDraftResult {
    /// The id of the draft, for `setup.outcome`.
    pub draft: String,
    /// The values of the five forms: `product`, `objectives`, `roster`, `models` and `stories`.
    pub fields: Value,
    /// The paths of the fields that the files do not answer, for example `product.audience`.
    pub no_data: Vec<String>,
    pub cost_usd: f64,
    pub seconds: f64,
}

impl Method for SetupDraft {
    const NAME: &'static str = "setup.draft";
    const DESCRIPTION: &'static str = "Drafts the forms of the setup screen with one Claude call: \
        model sonnet, no tools, a cap of 0.50 USD. It reads at most 15 files and 60 KiB of the \
        folder and `git log --oneline -30`. `previous` and `failures` ask for a repair. It \
        appends one line to `logs/setup.jsonl` of the data root. It writes nothing else.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::Invalid,
        ErrorKind::Unavailable,
        ErrorKind::Internal,
    ];
    type Params = SetupDraftParams;
    type Result = SetupDraftResult;
}

/// What the owner did with a drafted field before the apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FieldOutcome {
    Kept,
    Edited,
    Cleared,
}

pub struct SetupOutcome;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SetupOutcomeParams {
    /// The id of the draft.
    pub draft: String,
    /// The product that the apply wrote.
    pub product: String,
    /// The path of each drafted field, for example `product.what`, to its outcome.
    pub fields: BTreeMap<String, FieldOutcome>,
}

impl Method for SetupOutcome {
    const NAME: &'static str = "setup.outcome";
    const DESCRIPTION: &'static str = "Records what the owner did with each drafted field: \
        kept, edited or cleared. It appends one line to `logs/setup.jsonl` of the data root. \
        The app calls it after an apply.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::Invalid, ErrorKind::Unavailable];
    type Params = SetupOutcomeParams;
    type Result = crate::core_api::Empty;
}
