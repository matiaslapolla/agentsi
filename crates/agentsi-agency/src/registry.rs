//! The registry `agentsi.config.json` of the data root, and the lookup of a
//! product by folder. It stays a file (ADR 0003). The rules copy
//! `packages/core/src/registry.ts` and `paths.ts`.

use crate::error::{Error, Result};
use agentsi_proto::agency_api::{DataRoot, Owner, ProductIdentity};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

pub const FILE: &str = "agentsi.config.json";

#[derive(Debug, Clone, PartialEq)]
pub struct Product {
    pub id: String,
    pub name: String,
    pub paths: Vec<String>,
    pub status: String,
    /// The agent ids of the roster: a role (`dev`) or a specialist (`dev.backend`).
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Registry {
    pub owner: Owner,
    pub products: Vec<Product>,
}

impl Product {
    /// Writes the `products` row of the product, which `product.list` reads. The registry
    /// file stays the source of the roster and the stories block.
    pub fn store_row(&self, c: &rusqlite::Connection) -> rusqlite::Result<()> {
        c.execute(
            "insert into products (id, name, paths, status) values (?1, ?2, ?3, ?4) \
             on conflict (id) do update set name = excluded.name, paths = excluded.paths, \
             status = excluded.status",
            rusqlite::params![
                self.id,
                self.name,
                Value::from(self.paths.clone()).to_string(),
                self.status
            ],
        )?;
        Ok(())
    }
}

/// Replaces the registry file of `root` with `text` in one step: a write of a temporary
/// file next to it, then a rename. A failed step leaves the file as it was.
pub fn write_file(root: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let file = root.join(FILE);
    let tmp = root.join(format!("{FILE}.tmp"));
    let mode = std::fs::metadata(&file).map(|m| m.permissions()).ok();
    let mut out = std::fs::File::create(&tmp)?;
    out.write_all(text.as_bytes())?;
    out.sync_all()?;
    drop(out);
    if let Some(mode) = mode {
        std::fs::set_permissions(&tmp, mode)?;
    }
    std::fs::rename(&tmp, &file)
}

const DEFAULT_OWNER: (&str, &str) = ("the owner", "English");

fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// The owner block with its defaults. It fails on a block that is not an
/// object, and on a key that is not a non-empty string.
fn owner_of(block: Option<&Value>) -> Result<Owner> {
    let mut owner = Owner {
        name: DEFAULT_OWNER.0.into(),
        language: DEFAULT_OWNER.1.into(),
    };
    let Some(block) = block else {
        return Ok(owner);
    };
    let Some(map) = block.as_object() else {
        return Err(Error::Unavailable(
            "The owner block of agentsi.config.json must be an object.".into(),
        ));
    };
    for key in ["name", "language"] {
        let Some(value) = map.get(key) else {
            continue;
        };
        match value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            Some(text) if key == "name" => owner.name = text.into(),
            Some(text) => owner.language = text.into(),
            None => {
                return Err(Error::Unavailable(format!(
                    "owner.{key} in agentsi.config.json must be a non-empty string."
                )));
            }
        }
    }
    Ok(owner)
}

impl Registry {
    pub fn parse(text: &str) -> Result<Registry> {
        let doc: Value = serde_json::from_str(text)
            .map_err(|e| Error::Unavailable(format!("{FILE} does not parse: {e}")))?;
        let products = doc
            .get("products")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|p| {
                        let id = p.get("id").and_then(Value::as_str).unwrap_or_default();
                        Product {
                            id: id.into(),
                            name: p.get("name").and_then(Value::as_str).unwrap_or(id).into(),
                            paths: strings(p, "paths"),
                            status: p
                                .get("status")
                                .and_then(Value::as_str)
                                .unwrap_or("active")
                                .into(),
                            roles: strings(p, "roles"),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Registry {
            owner: owner_of(doc.get("owner"))?,
            products,
        })
    }

    /// The registry of a data root.
    pub fn load(root: &Path) -> Result<Registry> {
        let file = root.join(FILE);
        let text = std::fs::read_to_string(&file)
            .map_err(|e| Error::Unavailable(format!("cannot read {}: {e}", file.display())))?;
        Registry::parse(&text)
    }

    pub fn product(&self, id: &str) -> Result<&Product> {
        self.products
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| Error::NotFound(format!("Unknown product \"{id}\"")))
    }

    /// The product whose folder holds `cwd`. The longest folder wins.
    pub fn detect(&self, cwd: &Path) -> Option<&Product> {
        let abs = normalize(cwd);
        let mut best: Option<(&Product, usize)> = None;
        for product in &self.products {
            for root in &product.paths {
                let Ok(rel) = abs.strip_prefix(normalize(Path::new(root))) else {
                    continue;
                };
                // The TypeScript test is `!rel.startsWith("..")`, so a folder named `..x` is outside.
                if rel.to_string_lossy().starts_with("..") {
                    continue;
                }
                if best.is_none_or(|(_, len)| root.len() > len) {
                    best = Some((product, root.len()));
                }
            }
        }
        best.map(|(p, _)| p)
    }

    pub fn identity(&self, product: &Product) -> ProductIdentity {
        ProductIdentity {
            product: product.id.clone(),
            name: product.name.clone(),
            scope: None,
            roles: product.roles.clone(),
            paths: product.paths.clone(),
            owner: self.owner.clone(),
        }
    }
}

/// An absolute path with `.` and `..` resolved by text, as `path.resolve` does.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// The data root and where the core found it. The Rust programs have no code
/// checkout, so the source is `AGENTSI_HOME` or the home file.
pub fn data_root() -> Result<DataRoot> {
    let root = agentsi_store::paths::data_root(None)?;
    let from = if std::env::var("AGENTSI_HOME").is_ok_and(|v| !v.trim().is_empty()) {
        "AGENTSI_HOME"
    } else {
        "home file"
    };
    Ok(DataRoot {
        root: root.to_string_lossy().into_owned(),
        from: from.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry::parse(
            r#"{"owner":{"name":" Ada ","language":"Spanish"},"products":[
                {"id":"a","name":"A","paths":["/src/a"],"status":"active","roles":["pm"]},
                {"id":"b","paths":["/src/a/inner","/other/b"],"roles":[]}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn the_longest_folder_wins_and_a_sibling_prefix_does_not_match() {
        let r = registry();
        assert_eq!(r.detect(Path::new("/src/a/x/../y")).unwrap().id, "a");
        assert_eq!(r.detect(Path::new("/src/a/inner/deep")).unwrap().id, "b");
        assert_eq!(r.detect(Path::new("/src/a")).unwrap().id, "a");
        assert!(r.detect(Path::new("/src/ab")).is_none());
        assert!(r.detect(Path::new("/elsewhere")).is_none());
    }

    #[test]
    fn the_owner_block_has_defaults_and_rejects_bad_values() {
        assert_eq!(registry().owner.name, "Ada");
        let none = Registry::parse(r#"{"products":[]}"#).unwrap();
        assert_eq!(none.owner.language, "English");
        let bad = Registry::parse(r#"{"owner":{"name":3},"products":[]}"#);
        assert!(bad.unwrap_err().to_string().contains("owner.name"));
        let list = Registry::parse(r#"{"owner":[],"products":[]}"#);
        assert!(list.unwrap_err().to_string().contains("must be an object"));
    }

    #[test]
    fn an_unknown_product_is_not_found_by_the_name_the_cli_prints() {
        let err = registry().product("zz").unwrap_err();
        assert_eq!(err.to_string(), "Unknown product \"zz\"");
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[test]
    fn a_product_takes_its_id_as_the_default_name() {
        assert_eq!(registry().product("b").unwrap().name, "b");
    }
}
