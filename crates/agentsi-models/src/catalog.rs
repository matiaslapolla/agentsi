use serde::{Deserialize, Serialize};

use crate::Error;

const BUILTIN: &str = include_str!("../models.toml");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    SpeechOut,
    SpeechIn,
}

/// One pinned file of a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelFile {
    /// The file name inside the model folder.
    pub name: String,
    /// The path in the repo, when it differs from `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub size: u64,
    /// Lowercase hex SHA-256.
    pub sha256: String,
}

impl ModelFile {
    pub fn repo_path(&self) -> &str {
        self.path.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub license: String,
    pub description: String,
    /// Hugging Face repo, `owner/name`.
    pub repo: String,
    /// Full commit hash.
    pub revision: String,
    #[serde(rename = "file")]
    pub files: Vec<ModelFile>,
}

impl Model {
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Catalog {
    #[serde(rename = "model")]
    models: Vec<Model>,
}

impl Catalog {
    /// The catalog that ships in `models.toml`.
    pub fn builtin() -> Self {
        Self::parse(BUILTIN).expect("models.toml is valid, and a test checks it")
    }

    /// Parses and checks a catalog. Tests build their own.
    pub fn parse(text: &str) -> Result<Self, String> {
        let catalog: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        catalog.check()?;
        Ok(catalog)
    }

    fn check(&self) -> Result<(), String> {
        let mut ids = std::collections::HashSet::new();
        for m in &self.models {
            if !ids.insert(&m.id) {
                return Err(format!("duplicate model id `{}`", m.id));
            }
            if !safe_name(&m.id) {
                return Err(format!("unsafe model id `{}`", m.id));
            }
            if m.revision.len() != 40 || !m.revision.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(format!("`{}`: revision must be a full commit hash", m.id));
            }
            if m.files.is_empty() {
                return Err(format!("`{}` has no files", m.id));
            }
            for f in &m.files {
                if !safe_name(&f.name) || f.name.ends_with(".partial") {
                    return Err(format!("`{}`: unsafe file name `{}`", m.id, f.name));
                }
                if f.sha256.len() != 64
                    || !f
                        .sha256
                        .bytes()
                        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                {
                    return Err(format!(
                        "`{}`: `{}` needs a lowercase hex SHA-256",
                        m.id, f.name
                    ));
                }
                if f.size == 0 {
                    return Err(format!("`{}`: `{}` has no size", m.id, f.name));
                }
            }
        }
        Ok(())
    }

    pub fn models(&self) -> &[Model] {
        &self.models
    }

    pub fn get(&self, id: &str) -> Result<&Model, Error> {
        self.models
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| Error::UnknownModel(id.to_owned()))
    }
}

/// A name that stays inside its folder.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}
