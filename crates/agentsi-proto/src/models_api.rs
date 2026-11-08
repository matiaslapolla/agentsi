//! The `models` namespace: the voice models of the catalog, their downloads, and the
//! voice of each role. Spec: `docs/specs/agentsi-desktop-protocol.md`, "Voice models (U15c)".

use crate::core_api::Empty;
use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<ModelsList>()
        .method::<ModelsDownload>()
        .method::<ModelsDelete>()
        .method::<ModelsVoices>();
}

/// Where a model is on this device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelState {
    /// No byte of the model is on disk.
    Missing,
    /// Finished files and `.partial` files hold `bytes`.
    Partial { bytes: u64 },
    /// A download of this core runs.
    Downloading {
        bytes: u64,
        total: u64,
        /// The core checks the hash of a file.
        verifying: bool,
    },
    /// Each file passed its hash check.
    Downloaded,
    /// A file with a catalog name is not the pinned file. The owner put it there, or an
    /// older catalog entry pinned it. It stays in place.
    Conflict { file: String, message: String },
    /// The last download of this core failed.
    Error {
        /// `hash_mismatch`, `network`, `bad_response`, `busy`, `canceled` or `io`.
        error: String,
        message: String,
    },
}

impl From<&crate::events::ModelsErrorParams> for ModelState {
    /// The state that a failed download leaves: a conflict names its file.
    fn from(e: &crate::events::ModelsErrorParams) -> ModelState {
        match &e.file {
            Some(file) if e.error == "conflict" => ModelState::Conflict {
                file: file.clone(),
                message: e.message.clone(),
            },
            _ => ModelState::Error {
                error: e.error.clone(),
                message: e.message.clone(),
            },
        }
    }
}

/// One model of the catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    /// `speech-out` or `speech-in`.
    pub kind: String,
    pub license: String,
    pub description: String,
    /// Bytes of all the files of the model.
    pub size: u64,
    #[serde(flatten)]
    pub state: ModelState,
}

pub struct ModelsList;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelsListResult {
    /// The models folder of this device.
    pub root: String,
    /// The models, in the order of the catalog.
    pub models: Vec<ModelInfo>,
}

impl Method for ModelsList {
    const NAME: &'static str = "models.list";
    const DESCRIPTION: &'static str = "Gives each model of the catalog with its size, license and \
        state. It never hashes a file.";
    const ERRORS: &'static [ErrorKind] = &[];
    type Params = Empty;
    type Result = ModelsListResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelParams {
    pub id: String,
}

pub struct ModelsDownload;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DownloadResult {
    pub id: String,
    /// false when a download of the model already runs, or the model is downloaded.
    pub started: bool,
}

impl Method for ModelsDownload {
    const NAME: &'static str = "models.download";
    const DESCRIPTION: &'static str = "Starts a download of a model and returns at once. The events \
        models.progress, then models.done or models.error, follow on the models topic.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = ModelParams;
    type Result = DownloadResult;
}

pub struct ModelsDelete;

impl Method for ModelsDelete {
    const NAME: &'static str = "models.delete";
    const DESCRIPTION: &'static str = "Deletes the files of a model that the download wrote, and \
        gives the model. A file that the core did not write stays.";
    const ERRORS: &'static [ErrorKind] =
        &[ErrorKind::NotFound, ErrorKind::Busy, ErrorKind::Unavailable];
    type Params = ModelParams;
    type Result = ModelInfo;
}

pub struct ModelsVoices;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RoleVoice {
    pub role: String,
    /// `engine:voice`, for example `kokoro:af_heart`.
    pub voice: String,
    /// Why the pill speaks with another voice, when it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VoicesResult {
    /// The path of the `voices.toml` of the data root, or `built-in`.
    pub source: String,
    pub lang: String,
    /// The voice of a role that has no entry.
    pub default: String,
    /// Why the pill speaks with another voice than `default`, when it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_note: Option<String>,
    /// In the order of the role names.
    pub roles: Vec<RoleVoice>,
}

impl Method for ModelsVoices {
    const NAME: &'static str = "models.voices";
    const DESCRIPTION: &'static str = "Gives the voice of each role from voices.toml, read only.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::Invalid];
    type Params = Empty;
    type Result = VoicesResult;
}
