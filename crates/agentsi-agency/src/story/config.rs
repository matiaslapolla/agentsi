//! `storyConfig` of `registry.ts`: the `stories` block of a product in
//! `agentsi.config.json`, with its defaults. The `Product` of the registry
//! module does not hold the block, so this reads the registry text again.

use super::{Error, Result};
use crate::registry::{FILE, Product};
use agentsi_proto::story_api::StoryConfig;
use serde_json::Value;
use std::path::Path;

pub const BUDGET_USD: f64 = 5.0;

/// The stories block of `product`, from the registry of `root`.
pub fn story_config(root: &Path, product: &Product) -> Result<StoryConfig> {
    let file = root.join(FILE);
    let text = std::fs::read_to_string(&file)
        .map_err(|e| Error::invalid(format!("cannot read {}: {e}", file.display())))?;
    let doc: Value = serde_json::from_str(&text)
        .map_err(|e| Error::invalid(format!("{FILE} does not parse: {e}")))?;
    let block = doc
        .get("products")
        .and_then(Value::as_array)
        .and_then(|ps| {
            ps.iter()
                .find(|p| p.get("id").and_then(Value::as_str) == Some(&product.id))
        })
        .and_then(|p| p.get("stories"))
        .filter(|b| !b.is_null());
    from_block(product, block)
}

/// The rules of `storyConfig` on one block.
pub fn from_block(product: &Product, block: Option<&Value>) -> Result<StoryConfig> {
    let id = &product.id;
    let Some(block) = block else {
        return Err(Error::invalid(format!(
            "Stories mode is off for {id}: add a stories block to agentsi.config.json"
        )));
    };
    let repo = match block.get("repo") {
        Some(Value::String(r)) => Some(r.clone()),
        _ => product.paths.first().cloned(),
    };
    let Some(repo) = repo.filter(|r| !r.is_empty()) else {
        return Err(Error::invalid(format!(
            "Product {id} has no path for its stories repo."
        )));
    };
    let test = match block.get("test") {
        Some(Value::String(t)) if t.contains("{files}") => t.clone(),
        _ => {
            return Err(Error::invalid(format!(
                "The stories block of {id} needs a test command with {{files}}."
            )));
        }
    };
    let gate: Option<Vec<String>> = block.get("gate").and_then(Value::as_array).and_then(|g| {
        g.iter()
            .map(|c| {
                c.as_str()
                    .filter(|s| !s.trim().is_empty())
                    .map(String::from)
            })
            .collect()
    });
    let Some(gate) = gate.filter(|g| !g.is_empty()) else {
        return Err(Error::invalid(format!(
            "The stories block of {id} needs gate as a list of commands."
        )));
    };
    let budget = match block.get("budget_usd") {
        None => BUDGET_USD,
        Some(Value::Number(n)) if n.as_f64().is_some_and(|b| b.is_finite() && b > 0.0) => {
            n.as_f64().unwrap_or(BUDGET_USD)
        }
        Some(other) => {
            return Err(Error::invalid(format!(
                "The stories block of {id} needs budget_usd as a positive number of USD, got {other}."
            )));
        }
    };
    Ok(StoryConfig {
        repo,
        test,
        gate,
        budget_usd: budget,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn product() -> Product {
        Product {
            id: "p".into(),
            name: "P".into(),
            paths: vec!["/src/p".into()],
            status: "active".into(),
            roles: vec![],
        }
    }

    #[test]
    fn the_block_takes_its_defaults_and_names_each_bad_key() {
        let ok = from_block(
            &product(),
            Some(&json!({"test": "node --test {files}", "gate": ["pnpm lint"]})),
        )
        .unwrap();
        assert_eq!((ok.repo.as_str(), ok.budget_usd), ("/src/p", 5.0));
        let err = |b: Value| from_block(&product(), Some(&b)).unwrap_err().to_string();
        assert!(
            from_block(&product(), None)
                .unwrap_err()
                .to_string()
                .contains("Stories mode is off for p")
        );
        assert!(err(json!({"test": "x", "gate": ["a"]})).contains("{files}"));
        assert!(err(json!({"test": "{files}", "gate": []})).contains("gate as a list"));
        assert!(
            err(json!({"test": "{files}", "gate": ["a"], "budget_usd": 0})).ends_with("got 0.")
        );
        assert!(
            err(json!({"test": "{files}", "gate": ["a"], "budget_usd": "5"}))
                .ends_with("got \"5\".")
        );
    }
}
