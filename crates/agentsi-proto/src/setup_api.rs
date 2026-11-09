//! The methods of the setup screen: `setup.scan` reads a product folder with code only.

use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<SetupScan>();
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
