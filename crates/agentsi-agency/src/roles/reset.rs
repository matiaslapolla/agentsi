//! `roles reset`: `reset.ts`. Returns one agent, or the whole product, to the
//! global roles. It deletes overlays (and the brief and the memory when the
//! plan says so) and edits the roster in the registry. The registry write is
//! the last step of the transaction.

use super::files::RoleFiles;
use super::registry_edit::{Patch, update_product};
use super::{Result, invalid};
use crate::registry::{FILE, Product};
use crate::roster::read_overlays;
use agentsi_proto::roles_api::ResetPlan;
use agentsi_store::read;
use rusqlite::{Connection, Transaction};
use serde_json::Value;
use std::path::Path;

/// A plan with the brief that replaces the stored one.
#[derive(Debug, Clone)]
pub struct Planned {
    pub plan: ResetPlan,
    brief_edit: Option<Value>,
}

/// What reset would do. `agent` is `None` for the whole product. `all` keeps
/// the global roles of the roster and drops specialists and custom roles (the
/// global roles when none is left). An agent with no global role to fall back
/// to leaves the roster, and its entry leaves the brief. A global role only
/// loses its overlay.
pub fn plan_reset(
    c: &Connection,
    root: &Path,
    product: &Product,
    agent: Option<&str>,
    memory: bool,
) -> Result<Planned> {
    let files = RoleFiles::at(root);
    let globals = files
        .names()
        .map_err(|e| super::Error::Agency(crate::error::Error::Unavailable(e)))?;
    let roster_before = product.roles.clone();
    let brief = read::brief(c, &product.id)?;
    // Sorted as file names, as `reset.ts` sorts them: `dev.api.md` before `dev.md`.
    let mut overlays: Vec<String> = read_overlays(c, &product.id)?
        .into_keys()
        .map(|id| format!("{id}.md"))
        .collect();
    overlays.sort();
    let mut memories: Vec<String> = read::memory_agents(c, &product.id)?
        .into_iter()
        .map(|id| format!("{id}.json"))
        .collect();
    memories.sort();
    let make = |removals: Vec<String>, brief_edit: Option<Value>, after: Vec<String>| {
        let edits: Vec<String> = brief_edit
            .iter()
            .map(|_| "brief.json".to_string())
            .collect();
        let noop = removals.is_empty() && edits.is_empty() && roster_before == after;
        Planned {
            plan: ResetPlan {
                product: product.id.clone(),
                agent: agent.map(String::from),
                removals,
                edits,
                roster_before: roster_before.clone(),
                roster_after: after,
                noop,
            },
            brief_edit,
        }
    };

    let Some(agent) = agent else {
        let mut removals = Vec::new();
        if brief.is_some() {
            removals.push("brief.json".to_string());
        }
        removals.extend(overlays.iter().map(|f| format!("agents/{f}")));
        if memory {
            removals.extend(memories.iter().map(|f| format!("memory/{f}")));
        }
        let kept: Vec<String> = roster_before
            .iter()
            .filter(|r| globals.contains(r))
            .cloned()
            .collect();
        return Ok(make(
            removals,
            None,
            if kept.is_empty() { globals } else { kept },
        ));
    };

    let mut removals = Vec::new();
    if overlays.contains(&format!("{agent}.md")) {
        removals.push(format!("agents/{agent}.md"));
    }
    if memory && memories.contains(&format!("{agent}.json")) {
        removals.push(format!("memory/{agent}.json"));
    }
    let in_roster = roster_before.iter().any(|r| r == agent);
    if !in_roster && removals.is_empty() {
        return invalid(format!(
            "Agent \"{agent}\" is not in the roster of {} and has no data to reset.",
            product.id
        ));
    }
    let has_global = !agent.contains('.') && globals.iter().any(|g| g == agent);
    let leaves = in_roster && !has_global;
    let after: Vec<String> = if leaves {
        roster_before
            .iter()
            .filter(|r| *r != agent)
            .cloned()
            .collect()
    } else {
        roster_before.clone()
    };
    if leaves && after.is_empty() {
        return invalid(format!(
            "Resetting \"{agent}\" would leave {} with an empty roster.",
            product.id
        ));
    }
    let mut brief_edit = None;
    if leaves
        && let Some(mut brief) = brief
        && let Some(roles) = brief.get_mut("roles").and_then(Value::as_object_mut)
        && roles.contains_key(agent)
    {
        roles.shift_remove(agent);
        brief_edit = Some(brief);
    }
    Ok(make(removals, brief_edit, after))
}

/// Changes the database and the registry in the transaction `tx`.
pub fn write_reset(tx: &Transaction, root: &Path, planned: &Planned) -> Result<()> {
    let plan = &planned.plan;
    let id = &plan.product;
    for rel in &plan.removals {
        if rel == "brief.json" {
            tx.execute("delete from briefs where product = ?1", [id])?;
        } else if let Some(file) = rel.strip_prefix("agents/") {
            let agent = file.strip_suffix(".md").unwrap_or(file);
            tx.execute(
                "delete from agent_overlays where product = ?1 and agent = ?2",
                [id.as_str(), agent],
            )?;
        } else if let Some(file) = rel.strip_prefix("memory/") {
            let agent = file.strip_suffix(".json").unwrap_or(file);
            tx.execute(
                "delete from memory where product = ?1 and agent = ?2",
                [id.as_str(), agent],
            )?;
        }
    }
    if let Some(brief) = &planned.brief_edit {
        let data = serde_json::to_string(brief).unwrap_or_default();
        tx.execute(
            "insert into briefs (product, data) values (?1, ?2) \
             on conflict (product) do update set data = excluded.data",
            [id.as_str(), data.as_str()],
        )?;
    }
    if plan.roster_before != plan.roster_after {
        let file = root.join(FILE);
        let text = std::fs::read_to_string(&file)
            .map_err(|e| super::Error::Invalid(format!("cannot read {}: {e}", file.display())))?;
        let patch = Patch {
            roles: Some(plan.roster_after.clone()),
            stories: None,
        };
        let edited = update_product(&text, id, &patch).map_err(super::Error::Invalid)?;
        std::fs::write(&file, edited)
            .map_err(|e| super::Error::Invalid(format!("cannot write {}: {e}", file.display())))?;
    }
    Ok(())
}

/// The message of the commit of the registry.
pub fn commit_message(plan: &ResetPlan) -> String {
    format!(
        "reset({}): {}",
        plan.product,
        plan.agent.as_deref().unwrap_or("all")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const REGISTRY: &str = "{\n  \"products\": [\n    { \"id\": \"p\", \"name\": \"P\", \"paths\": [\"/x\"], \"status\": \"active\", \"roles\": [\"pm\", \"dev\", \"dev.api\", \"growth\"] }\n  ]\n}\n";

    struct Env {
        _dir: tempfile::TempDir,
        root: std::path::PathBuf,
        conn: Connection,
        product: Product,
    }

    fn env() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(FILE), REGISTRY).unwrap();
        let conn = agentsi_store::db::open_writer(&dir.path().join("t.db")).unwrap();
        for sql in [
            "insert into briefs (product, data) values ('p', '{\"roles\":{\"pm\":\"a\",\"growth\":\"b\"}}')",
            "insert into agent_overlays (product, agent, body) values ('p', 'dev.api', 'x')",
            "insert into agent_overlays (product, agent, body) values ('p', 'dev', 'y')",
            "insert into agent_overlays (product, agent, body) values ('p', 'growth', 'z')",
            "insert into memory (product, agent, id, position, text, kind, helpful, harmful, source, created_at) values ('p', 'growth', 'm1', 0, 't', 'fact', 0, 0, 's', '2026-01-01T00:00:00.000Z')",
        ] {
            conn.execute(sql, []).unwrap();
        }
        let product = Product {
            id: "p".into(),
            name: "P".into(),
            paths: vec!["/x".into()],
            status: "active".into(),
            roles: ["pm", "dev", "dev.api", "growth"]
                .map(String::from)
                .to_vec(),
        };
        Env {
            root,
            conn,
            product,
            _dir: dir,
        }
    }

    fn plan(e: &Env, agent: Option<&str>, memory: bool) -> Result<Planned> {
        plan_reset(&e.conn, &e.root, &e.product, agent, memory)
    }

    #[test]
    fn all_drops_the_brief_the_overlays_and_the_non_global_roles() {
        let e = env();
        let p = plan(&e, None, false).unwrap().plan;
        assert_eq!(
            p.removals,
            [
                "brief.json",
                "agents/dev.api.md",
                "agents/dev.md",
                "agents/growth.md"
            ]
        );
        assert_eq!(p.roster_after, ["pm", "dev"]);
        let with_memory = plan(&e, None, true).unwrap().plan;
        assert!(
            with_memory
                .removals
                .contains(&"memory/growth.json".to_string())
        );
    }

    #[test]
    fn a_global_role_only_loses_its_overlay() {
        let e = env();
        let p = plan(&e, Some("dev"), false).unwrap().plan;
        assert_eq!(
            (p.removals.as_slice(), p.roster_after.len()),
            (&["agents/dev.md".to_string()][..], 4)
        );
        assert!(!p.noop);
    }

    #[test]
    fn a_custom_agent_leaves_the_roster_and_the_brief() {
        let mut e = env();
        let planned = plan(&e, Some("growth"), true).unwrap();
        assert_eq!(planned.plan.edits, ["brief.json"]);
        assert_eq!(planned.plan.roster_after, ["pm", "dev", "dev.api"]);
        let tx = e.conn.transaction().unwrap();
        write_reset(&tx, &e.root, &planned).unwrap();
        tx.commit().unwrap();
        assert!(read::overlay(&e.conn, "p", "growth").unwrap().is_none());
        assert!(read::memory_agents(&e.conn, "p").unwrap().is_empty());
        assert_eq!(
            read::brief(&e.conn, "p").unwrap().unwrap().to_string(),
            r#"{"roles":{"pm":"a"}}"#
        );
        let text = std::fs::read_to_string(e.root.join(FILE)).unwrap();
        assert!(text.contains(r#""roles": ["pm", "dev", "dev.api"] }"#));
    }

    #[test]
    fn an_unknown_agent_and_an_empty_roster_are_errors() {
        let mut e = env();
        let msg = |r: Result<Planned>| match r.unwrap_err() {
            super::super::Error::Invalid(m) => m,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            msg(plan(&e, Some("ghost"), false)),
            "Agent \"ghost\" is not in the roster of p and has no data to reset."
        );
        e.product.roles = vec!["growth".into()];
        assert_eq!(
            msg(plan(&e, Some("growth"), false)),
            "Resetting \"growth\" would leave p with an empty roster."
        );
        e.product.roles = vec!["pm".into()];
        e.conn.execute("delete from agent_overlays", []).unwrap();
        assert!(plan(&e, Some("pm"), false).unwrap().plan.noop);
        assert!(plan(&e, None, false).unwrap().plan.removals == ["brief.json"]);
    }
}
