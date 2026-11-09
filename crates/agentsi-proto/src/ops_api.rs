//! `core.status`: what `agentsi up` and `agentsi migrate` print (U24). `data.backup`:
//! the online backup of `agentsi backup` (U26b). `core.doctor`: the checks of
//! `agentsi doctor`.

use crate::core_api::Empty;
use crate::doctor::{Level, Status};
use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<CoreStatus>()
        .method::<DataBackup>()
        .method::<CoreDoctor>();
}

pub struct CoreStatus;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatabaseStatus {
    /// The database file.
    pub path: String,
    /// The number of the last migration that the file has.
    pub schema_version: u32,
    /// The number of the last migration that this core knows.
    pub latest: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CoreStatusResult {
    /// The version of the core program.
    pub version: String,
    /// The data root of the core.
    pub data_root: String,
    /// `None` while the data root has no `agentsi.db`.
    pub database: Option<DatabaseStatus>,
    /// True while the pill is connected. Only the pill subscribes to the dictation topic.
    #[serde(default)]
    pub pill: bool,
}

impl Method for CoreStatus {
    const NAME: &'static str = "core.status";
    const DESCRIPTION: &'static str = "Gives the version of the core, its data root and the schema version of \
        the database. The core applies the migrations when it opens the database, so this is the version it applied.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::Unavailable];
    type Params = Empty;
    type Result = CoreStatusResult;
}

pub struct DataBackup;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DataBackupParams {
    /// The number of backups to keep. The default is 14. It must be 1 or more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DataBackupResult {
    /// The backup file, `<data root>/backups/agentsi-<UTC date>.db`.
    pub file: String,
    /// The copy of `agentsi.config.json`. `None` while the data root has no registry.
    pub registry: Option<String>,
    /// The files of the old backups that this call removed.
    pub removed: Vec<String>,
}

impl Method for DataBackup {
    const NAME: &'static str = "data.backup";
    const DESCRIPTION: &'static str = "Copies the database with the SQLite online backup API and the registry \
        into the backups folder of the data root, then keeps the newest backups. The copy is complete \
        while the core writes. A second backup on the same UTC date takes the place of the first.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::Invalid, ErrorKind::Unavailable];
    type Params = DataBackupParams;
    type Result = DataBackupResult;
}

pub struct CoreDoctor;

/// One check of `agentsi doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DoctorCheck {
    pub name: String,
    /// `required` checks decide the exit code of `agentsi doctor`.
    pub level: Level,
    pub status: Status,
    /// The version or the found path when ok. Why it failed otherwise. Empty when missing.
    pub detail: String,
    /// The features that a failed check blocks.
    pub blocks: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CoreDoctorResult {
    /// The checks in the order of `agentsi doctor`.
    pub checks: Vec<DoctorCheck>,
}

impl Method for CoreDoctor {
    const NAME: &'static str = "core.doctor";
    const DESCRIPTION: &'static str = "Runs the checks of `agentsi doctor` on the host of the core \
        and gives each row: the core, the data root, the tools and the voice. It writes nothing. \
        A run takes up to 15 seconds for each tool.";
    const ERRORS: &'static [ErrorKind] = &[];
    type Params = Empty;
    type Result = CoreDoctorResult;
}
