//! The roster picker of the roles screen: it sets the roster of a product to a list of
//! agents that resolve. The brief follows: an agent that leaves loses its entry, and one
//! that joins gets an empty one, so `roles validate` stays clean.

use super::files::RoleFiles;
use super::load::load_role;
use super::registry_edit::{Patch, update_product};
use super::{Result, invalid};
use crate::registry::{self, FILE, Product};
use crate::roster::read_overlays;
use rusqlite::Transaction;
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

/// Checks `roster` and writes it in `tx`: the brief into the database, then the registry
/// file as the last step.
pub fn write_roster(
    tx: &Transaction,
    root: &Path,
    product: &Product,
    roster: &[String],
) -> Result<()> {
    if roster.is_empty() {
        return invalid("A roster needs one agent at least.");
    }
    let mut seen = BTreeSet::new();
    if let Some(twice) = roster.iter().find(|id| !seen.insert(id.as_str())) {
        return invalid(format!("Agent \"{twice}\" is repeated in the roster."));
    }
    let files = RoleFiles::at(root);
    let overlays = read_overlays(tx, &product.id)?;
    for id in roster {
        load_role(id, &product.id, &files, &overlays, &mut Vec::new())
            .map_err(super::Error::Invalid)?;
    }

    if let Some(mut brief) = agentsi_store::read::brief(tx, &product.id)?
        && let Some(roles) = brief.get_mut("roles").and_then(Value::as_object_mut)
    {
        let before = roles.len();
        roles.retain(|k, _| roster.contains(k));
        let mut changed = roles.len() != before;
        for id in roster {
            if !roles.contains_key(id) {
                roles.insert(id.clone(), Value::String(String::new()));
                changed = true;
            }
        }
        if changed {
            tx.execute(
                "update briefs set data = ?2 where product = ?1",
                [product.id.as_str(), brief.to_string().as_str()],
            )?;
        }
    }

    let file = root.join(FILE);
    let text = std::fs::read_to_string(&file)
        .map_err(|e| super::Error::Invalid(format!("cannot read {}: {e}", file.display())))?;
    let patch = Patch {
        roles: Some(roster.to_vec()),
        ..Patch::default()
    };
    let edited = update_product(&text, &product.id, &patch).map_err(super::Error::Invalid)?;
    registry::write_file(root, &edited)
        .map_err(|e| super::Error::Invalid(format!("cannot write {}: {e}", file.display())))
}

/// The message of the commit of the registry.
pub fn commit_message(product: &str) -> String {
    format!("roster({product}): set from the roles screen")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Registry;

    const REGISTRY: &str = "{\n  \"products\": [\n    { \"id\": \"p\", \"name\": \"P\", \"paths\": [\"/x\"], \"status\": \"active\", \"roles\": [\"pm\", \"dev\"] }\n  ]\n}\n";

    fn env() -> (tempfile::TempDir, rusqlite::Connection, Product) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FILE), REGISTRY).unwrap();
        let conn = agentsi_store::db::open_writer(&dir.path().join("t.db")).unwrap();
        conn.execute(
            "insert into briefs (product, data) values ('p', '{\"roles\":{\"pm\":\"a\",\"dev\":\"b\"}}')",
            [],
        )
        .unwrap();
        conn.execute(
            "insert into agent_overlays (product, agent, body) values ('p', 'seo', 'Rank.')",
            [],
        )
        .unwrap();
        let product = Registry::load(dir.path()).unwrap().products[0].clone();
        (dir, conn, product)
    }

    fn set(
        dir: &tempfile::TempDir,
        conn: &mut rusqlite::Connection,
        product: &Product,
        roster: &[&str],
    ) -> Result<()> {
        let roster: Vec<String> = roster.iter().map(|r| r.to_string()).collect();
        let tx = conn.transaction().unwrap();
        let r = write_roster(&tx, dir.path(), product, &roster);
        if r.is_ok() {
            tx.commit().unwrap();
        }
        r
    }

    #[test]
    fn the_roster_and_the_brief_follow_the_picker() {
        let (dir, mut conn, product) = env();
        set(&dir, &mut conn, &product, &["pm", "qa", "seo"]).unwrap();
        let after = Registry::load(dir.path()).unwrap();
        assert_eq!(after.products[0].roles, ["pm", "qa", "seo"]);
        assert_eq!(
            agentsi_store::read::brief(&conn, "p")
                .unwrap()
                .unwrap()
                .to_string(),
            r#"{"roles":{"pm":"a","qa":"","seo":""}}"#
        );
    }

    #[test]
    fn a_roster_that_does_not_resolve_writes_nothing() {
        let (dir, mut conn, product) = env();
        let message = |r: Result<()>| match r.unwrap_err() {
            super::super::Error::Invalid(m) => m,
            other => panic!("{other:?}"),
        };
        assert!(message(set(&dir, &mut conn, &product, &[])).contains("one agent"));
        assert!(message(set(&dir, &mut conn, &product, &["pm", "pm"])).contains("repeated"));
        assert!(message(set(&dir, &mut conn, &product, &["pm", "ghost"])).contains("ghost"));
        assert!(message(set(&dir, &mut conn, &product, &["dev.x"])).contains("no overlay"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join(FILE)).unwrap(),
            REGISTRY
        );
    }
}
