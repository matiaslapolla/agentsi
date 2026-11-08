//! The methods that `agentsi whoami`, `status`, `desk`, `snapshot` and `audit`
//! call (U16): `product.whoami`, `product.status`, `data.snapshot` and
//! `data.audit`. `agentsi desk` also calls `product.desk` of `desk_api`.
//!
//! The results hold what the TypeScript commands print. The field order of
//! each struct is the key order of the TypeScript output, because the CLI
//! prints these types as they are.

use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

pub fn register(c: &mut Catalog) {
    c.method::<ProductWhoami>()
        .method::<ProductStatus>()
        .method::<DataSnapshot>()
        .method::<DataAudit>();
}

/// A JSON object whose keys keep the order in which they were added, as a
/// JavaScript object does. A `BTreeMap` sorts its keys, so a typed result
/// that must keep the order uses this type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ordered<V>(pub Vec<(String, V)>);

impl<V: Serialize> Serialize for Ordered<V> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for Ordered<V> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Ordered<V>, D::Error> {
        struct Walk<V>(PhantomData<V>);
        impl<'de, V: Deserialize<'de>> Visitor<'de> for Walk<V> {
            type Value = Ordered<V>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Ordered<V>, A::Error> {
                let mut out = Vec::new();
                while let Some(entry) = map.next_entry()? {
                    out.push(entry);
                }
                Ok(Ordered(out))
            }
        }
        d.deserialize_map(Walk(PhantomData))
    }
}

impl<V: JsonSchema> JsonSchema for Ordered<V> {
    fn schema_name() -> Cow<'static, str> {
        format!("Ordered_{}", V::schema_name()).into()
    }
    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        BTreeMap::<String, V>::json_schema(generator)
    }
}

/// Who approves the work of the agency. The prompts name them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Owner {
    pub name: String,
    /// Free text for the prompts, for example "English".
    pub language: String,
}

/// The product of a folder, as `agentsi whoami` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProductIdentity {
    pub product: String,
    pub name: String,
    /// Always null today.
    pub scope: Option<String>,
    pub roles: Vec<String>,
    pub paths: Vec<String>,
    pub owner: Owner,
}

/// One product of the registry, as `agentsi whoami --list` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ListedProduct {
    pub id: String,
    pub name: String,
    pub status: String,
    pub paths: Vec<String>,
    pub archived: bool,
}

/// The folder of the registry and the data, and where the core found it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DataRoot {
    pub root: String,
    /// `AGENTSI_HOME` or `home file`.
    pub from: String,
}

pub struct ProductWhoami;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WhoamiParams {
    /// The folder to look up. It must be an absolute path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// A product id. It takes the place of the lookup by folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// true: give the whole registry in `products`.
    #[serde(default)]
    pub list: bool,
    /// true: give the data root in `home`.
    #[serde(default)]
    pub home: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WhoamiResult {
    /// The product of `product` or of the folder. Null when the folder is in no product.
    pub product: Option<ProductIdentity>,
    /// With `list`: each product of the registry, in registry order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub products: Option<Vec<ListedProduct>>,
    /// With `home`: the data root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<DataRoot>,
}

impl Method for ProductWhoami {
    const NAME: &'static str = "product.whoami";
    const DESCRIPTION: &'static str = "Finds the product of a folder in the registry \
        (`agentsi.config.json`), or the product with the given id. With `list` it gives the \
        registry, and with `home` the data root. It is what `agentsi whoami` prints.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = WhoamiParams;
    type Result = WhoamiResult;
}

pub struct ProductStatus;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatusParams {
    pub product: String,
}

/// What the store holds about the index of a product, as one line of `agentsi desk` shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct IndexCounts {
    /// The repos of the last index run.
    pub repos: u64,
    /// The documents and the journals.
    pub docs: u64,
    pub commits: u64,
    pub issues: u64,
    /// The minutes that have a stored body.
    pub minutes: u64,
    /// The agents that have an active memory item.
    pub memory: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StatusResult {
    /// The text that `agentsi status` prints.
    pub markdown: String,
    pub index: IndexCounts,
}

impl Method for ProductStatus {
    const NAME: &'static str = "product.status";
    const DESCRIPTION: &'static str = "Gives the state of a product as the Markdown text of \
        `agentsi status`: the index, the pending decisions and the last minutes. It never writes.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = StatusParams;
    type Result = StatusResult;
}

pub struct DataSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotParams {
    pub product: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotResult {
    /// The text that `agentsi snapshot` prints: the context of a meeting.
    pub markdown: String,
}

impl Method for DataSnapshot {
    const NAME: &'static str = "data.snapshot";
    const DESCRIPTION: &'static str = "Builds the Markdown context of a meeting for a product, \
        as `agentsi snapshot` prints it, in at most 60 KiB. It reads the repos of the product \
        and the store, and writes nothing.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = SnapshotParams;
    type Result = SnapshotResult;
}

pub struct DataAudit;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuditParams {
    pub product: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OverdueObjective {
    pub id: String,
    pub due: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuditConfig {
    pub valid: bool,
    pub failures: Vec<String>,
    pub overdue_objectives: Vec<OverdueObjective>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AgentMeetingStats {
    pub agent: String,
    pub meetings: u64,
    pub answers: u64,
    pub errors: u64,
    pub timeouts: u64,
    pub unresolved_sources_per_meeting: f64,
    pub sin_datos_per_meeting: f64,
    pub mean_cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MeetingStats {
    pub count: u64,
    pub first_pass_rate: Option<f64>,
    pub repair_rate: Option<f64>,
    pub synthesis_cost_share: Option<f64>,
    pub invented_sources: u64,
    pub agents: Vec<AgentMeetingStats>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RepairGroup {
    pub minutes: Vec<String>,
    pub repaired: u64,
    /// A meeting has at most one repair round, so this is also the repair rate.
    pub per_meeting: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RepairComparison {
    pub baseline: RepairGroup,
    pub since: RepairGroup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LastRepair {
    pub minutes_id: String,
    pub rounds: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuditMarks {
    pub accepted: u64,
    pub changed: u64,
    pub rejected: u64,
    pub unmarked: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentMemoryStats {
    pub agent: String,
    pub active: u64,
    pub retired: u64,
    pub lessons: u64,
    pub never_helpful: u64,
    pub chars: u64,
    pub max_chars: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LastReflection {
    pub id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReflectionStats {
    pub last: Option<LastReflection>,
    pub pending_proposals: u64,
    pub reverted: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReadyAgent {
    pub agent: String,
    /// `failure_class` or `rejected_priorities`.
    pub reason: String,
    /// `unresolved_source`, `invented_source`, `missing_objective`, `timeout` or `sin_datos`.
    pub failure_class: Option<String>,
    pub count: u64,
}

/// The health report of a product. It is the `--json` output of `agentsi audit`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AuditReport {
    pub product: String,
    pub generated_at: String,
    pub config: AuditConfig,
    pub meetings: MeetingStats,
    pub repairs: RepairComparison,
    /// The repair rounds of the last 4 meetings, oldest first. Null with fewer than 4 meetings.
    pub last_repairs: Option<Vec<LastRepair>>,
    /// The marks of each owner role, in the order of first appearance.
    pub marks: Ordered<AuditMarks>,
    pub memory: Vec<AgentMemoryStats>,
    pub reflections: ReflectionStats,
    pub ready_to_reflect: Vec<ReadyAgent>,
}

impl Method for DataAudit {
    const NAME: &'static str = "data.audit";
    const DESCRIPTION: &'static str = "Reports the health of a product: its config, its meetings, \
        its marks, its memory and its reflections. It is deterministic: it calls no model and \
        writes nothing.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = AuditParams;
    type Result = AuditReport;
}
