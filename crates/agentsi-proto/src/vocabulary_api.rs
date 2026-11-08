//! The `vocabulary` namespace: the terms that the transcript is corrected with.
//! Spec: `docs/specs/agentsi-desktop-voice.md`, "Vocabulary".

use crate::core_api::Empty;
use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<VocabularyGet>().method::<VocabularySet>();
}

/// One term and the forms that the speech model gives for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VocabularyEntry {
    pub term: String,
    #[serde(default)]
    pub heard: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VocabularyList {
    pub entries: Vec<VocabularyEntry>,
    /// The file that holds the list.
    pub path: String,
}

pub struct VocabularyGet;

impl Method for VocabularyGet {
    const NAME: &'static str = "vocabulary.get";
    const DESCRIPTION: &'static str = "Reads the vocabulary. On the first read the core writes \\
        the file with one entry for each product name of the registry.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::Unavailable];
    type Params = Empty;
    type Result = VocabularyList;
}

pub struct VocabularySet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct VocabularySetParams {
    pub entries: Vec<VocabularyEntry>,
}

impl Method for VocabularySet {
    const NAME: &'static str = "vocabulary.set";
    const DESCRIPTION: &'static str = "Replaces the vocabulary. The core trims the texts, drops \\
        empty ones and repeats, and returns the saved list.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::Invalid, ErrorKind::Unavailable];
    type Params = VocabularySetParams;
    type Result = VocabularyList;
}
