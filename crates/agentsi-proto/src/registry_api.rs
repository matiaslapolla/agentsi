//! `registry.get` and `registry.update`: the owner block and the products of
//! `agentsi.config.json`, for the settings and the setup screen. The registry stays a
//! file (ADR 0003). `registry.update` makes no commit.

use crate::agency_api::Owner;
use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<RegistryGet>().method::<RegistryUpdate>();
}

/// A product of the registry. The roster and the stories block have their own methods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RegistryProduct {
    pub id: String,
    pub name: String,
    /// Absolute folders. The first one is the main folder.
    pub paths: Vec<String>,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RegistryResult {
    /// The owner block with its defaults.
    pub owner: Owner,
    /// The products in registry order.
    pub products: Vec<RegistryProduct>,
}

pub struct RegistryGet;

impl Method for RegistryGet {
    const NAME: &'static str = "registry.get";
    const DESCRIPTION: &'static str =
        "Gives the owner block and the products of `agentsi.config.json`. It only reads.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::Unavailable];
    type Params = crate::core_api::Empty;
    type Result = RegistryResult;
}

/// The owner fields to change. An absent field stays.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OwnerChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// The product fields to change. An absent field stays.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProductChange {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Existing absolute folders, one at least.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paths: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RegistryUpdateParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<OwnerChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<ProductChange>,
}

pub struct RegistryUpdate;

impl Method for RegistryUpdate {
    const NAME: &'static str = "registry.update";
    const DESCRIPTION: &'static str = "Changes the owner block, or the name and the paths of \
        one product, in `agentsi.config.json`. Each other key stays. The file changes in one \
        step, so a failed write leaves it as it was. A product change also writes its \
        `products` row and sends `data.changed` of kind `product`. It makes no commit.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Invalid,
        ErrorKind::Unavailable,
    ];
    type Params = RegistryUpdateParams;
    type Result = RegistryResult;
}
