//! The roster of a product, its overlays and its brief: what `validateRoster`
//! of `packages/core/src/roster.ts` checks. The audit reports the failures.

use crate::clock;
use crate::error::Result;
use crate::registry::{Product, normalize};
use crate::roles::files::RoleFiles;
use agentsi_proto::agency_api::Ordered;
use agentsi_store::read;
use regex::Regex;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// A role, or `<role>.<specialty>`: lowercase words joined by single hyphens.
/// `AGENT_ID_TEXT` is how JavaScript prints the same pattern.
static AGENT_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z]+(-[a-z]+)*(\.[a-z]+(-[a-z]+)*)?$").unwrap());
const AGENT_ID_TEXT: &str = r"/^[a-z]+(-[a-z]+)*(\.[a-z]+(-[a-z]+)*)?$/";
static ROLE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[a-z][a-z-]*$").unwrap());
static FRONTMATTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)^---\r?\n(.*?)\r?\n---\r?\n?(.*)$").unwrap());
static KEY_VALUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Za-z][A-Za-z0-9_-]*):\s*(.*)$").unwrap());

const STAGES: [&str; 5] = ["idea", "building", "launched", "growing", "maintenance"];

pub fn is_agent_id(id: &str) -> bool {
    AGENT_ID.is_match(id)
}

/// The frontmatter keys of an agent file (`key: value` lines), and its body.
pub fn parse_agent_file(text: &str) -> (BTreeMap<String, String>, String) {
    let Some(m) = FRONTMATTER.captures(text) else {
        return (BTreeMap::new(), text.trim().into());
    };
    let frontmatter = m[1]
        .lines()
        .filter_map(|line| {
            let kv = KEY_VALUE.captures(line)?;
            Some((kv[1].to_string(), kv[2].trim().to_string()))
        })
        .collect();
    (frontmatter, m[2].trim().into())
}

pub(crate) fn read_global(role: &str, files: &RoleFiles) -> std::result::Result<String, String> {
    match ROLE_NAME.is_match(role).then(|| files.read(role)).flatten() {
        Some(text) => Ok(text),
        None => Err(format!(
            "Unknown role \"{role}\" (no {})",
            files.describe(role)
        )),
    }
}

/// The reason that an agent does not load, or `None` when it loads.
fn load_failure(
    id: &str,
    product: &str,
    files: &RoleFiles,
    overlays: &BTreeMap<String, String>,
) -> Option<String> {
    let overlay = overlays.get(id).map(|text| parse_agent_file(text));
    match overlay {
        None if id.contains('.') => {
            Some(format!("Specialist \"{id}\" has no overlay in {product}"))
        }
        None => read_global(id, files).err(),
        Some((frontmatter, _)) => match frontmatter.get("extends").filter(|p| !p.is_empty()) {
            Some(parent) => read_global(parent, files).err(),
            None => None,
        },
    }
}

/// `resolveUnderRoots`: the file that `reference` names under a root of the
/// product, or `None` when it escapes the roots or does not exist.
pub fn resolve_under_roots(reference: &str, roots: &[String]) -> Option<PathBuf> {
    if Path::new(reference).is_absolute() {
        return None;
    }
    roots.iter().find_map(|root| {
        let full = normalize(&Path::new(root).join(reference));
        let inside = full
            .strip_prefix(normalize(Path::new(root)))
            .is_ok_and(|rel| !rel.to_string_lossy().starts_with(".."));
        (inside && full.exists()).then_some(full)
    })
}

/// A JSON value as a JavaScript template string prints it.
pub(crate) fn js_text(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    }
}

/// The brief of a product with the order of the keys of `roles` kept.
#[derive(serde::Deserialize, Default)]
pub struct Brief {
    #[serde(default)]
    pub product: Value,
    #[serde(default)]
    pub objectives: Value,
    #[serde(default)]
    pub north_stars: Value,
    #[serde(default)]
    pub roles: Option<Ordered<Value>>,
}

pub fn read_brief(c: &Connection, product: &str) -> Result<Option<Brief>> {
    let text: Option<String> = c
        .prepare_cached("select data from briefs where product = ?1")?
        .query_map([product], |r| r.get(0))?
        .next()
        .transpose()?;
    Ok(text.and_then(|t| serde_json::from_str(&t).ok()))
}

/// Failures of a brief against its product and roster; empty when valid.
pub fn validate_brief(brief: &Brief, product: &Product, roster: &[String]) -> Vec<String> {
    let mut failures = Vec::new();
    let stage = brief.product.get("stage");
    if !stage
        .and_then(Value::as_str)
        .is_some_and(|s| STAGES.contains(&s))
    {
        failures.push(format!(
            "product.stage \"{}\" is not one of {}.",
            js_text(stage),
            STAGES.join(", ")
        ));
    }

    let objectives = brief.objectives.as_array().map_or(&[][..], Vec::as_slice);
    if !(1..=3).contains(&objectives.len()) {
        failures.push(format!(
            "objectives must hold 1 to 3 items, found {}.",
            objectives.len()
        ));
    }
    let mut seen = BTreeSet::new();
    for o in objectives {
        match o
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            None => failures.push("An objective has no id.".into()),
            Some(id) if !seen.insert(id) => {
                failures.push(format!("Objective id \"{id}\" is repeated."))
            }
            Some(_) => {}
        }
        let due = o.get("due");
        if !due.and_then(Value::as_str).is_some_and(clock::is_date) {
            failures.push(format!(
                "Objective \"{}\" has an invalid due date \"{}\" (expected YYYY-MM-DD).",
                js_text(o.get("id")),
                js_text(due)
            ));
        }
    }

    for reference in brief.north_stars.as_array().map_or(&[][..], Vec::as_slice) {
        if !reference
            .as_str()
            .is_some_and(|r| resolve_under_roots(r, &product.paths).is_some())
        {
            failures.push(format!(
                "north_stars path \"{}\" does not exist under a product path.",
                js_text(Some(reference))
            ));
        }
    }

    let declared: Vec<&str> = brief
        .roles
        .iter()
        .flat_map(|o| o.0.iter().map(|(k, _)| k.as_str()))
        .collect();
    for id in roster.iter().filter(|r| !declared.contains(&r.as_str())) {
        failures.push(format!("roles has no entry for roster agent \"{id}\"."));
    }
    for id in declared.iter().filter(|d| !roster.iter().any(|r| r == **d)) {
        failures.push(format!("roles entry \"{id}\" is not in the roster."));
    }
    failures
}

/// Every overlay of a product by agent id, in byte order of the id.
pub fn read_overlays(c: &Connection, product: &str) -> Result<BTreeMap<String, String>> {
    let mut overlays = BTreeMap::new();
    let mut stmt = c.prepare_cached(
        "select agent, body from agent_overlays where product = ?1 order by agent",
    )?;
    for row in stmt.query_map([product], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (agent, body) = row?;
        overlays.insert(agent, body);
    }
    Ok(overlays)
}

/// Failures of the roster, its overlays and its brief; empty when all holds.
pub fn validate_roster(c: &Connection, root: &Path, product: &Product) -> Result<Vec<String>> {
    let overlays = read_overlays(c, &product.id)?;
    let brief = read_brief(c, &product.id)?;
    Ok(check_roster(
        &RoleFiles::at(root),
        product,
        &overlays,
        brief.as_ref(),
    ))
}

/// `validateRoster` on given overlays and a given brief: what the database
/// holds, or a stage that `roles apply` checks before it writes.
pub fn check_roster(
    files: &RoleFiles,
    product: &Product,
    overlays: &BTreeMap<String, String>,
    brief: Option<&Brief>,
) -> Vec<String> {
    let mut failures = Vec::new();
    let roster = &product.roles;
    if roster.is_empty() {
        failures.push("The roster is empty.".into());
    }

    let mut seen = BTreeSet::new();
    for id in roster {
        if !seen.insert(id) {
            failures.push(format!("Agent \"{id}\" is repeated in the roster."));
        }
        if !is_agent_id(id) {
            failures.push(format!("Agent id \"{id}\" does not match {AGENT_ID_TEXT}."));
        }
    }
    for id in roster.iter().filter(|id| is_agent_id(id)) {
        if let Some(error) = load_failure(id, &product.id, files, overlays) {
            failures.push(format!("Agent \"{id}\": {error}"));
        }
    }

    // `BTreeMap` iterates in byte order, as JavaScript sorts the keys.
    for (id, body) in overlays {
        let file = format!("{id}.md");
        if !is_agent_id(id) {
            failures.push(format!(
                "Overlay \"{file}\" is not named after a valid agent id."
            ));
            continue;
        }
        if roster.contains(id) {
            continue;
        }
        failures.push(format!(
            "Overlay \"{file}\" belongs to an agent that is not in the roster."
        ));
        if let Some(parent) = parse_agent_file(body)
            .0
            .get("extends")
            .filter(|p| !p.is_empty())
            && !files.has(parent)
        {
            failures.push(format!(
                "Overlay \"{file}\" extends \"{parent}\", which is not a global role."
            ));
        }
    }

    if let Some(brief) = brief {
        failures.extend(
            validate_brief(brief, product, roster)
                .into_iter()
                .map(|f| format!("Brief: {f}")),
        );
    }
    failures
}

/// Used by the audit: the memory of an agent.
pub fn memory_items(c: &Connection, product: &str, agent: &str) -> Result<Vec<read::MemoryItem>> {
    Ok(read::memory(c, product, agent)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_file_has_frontmatter_keys_and_a_body() {
        let (fm, body) = parse_agent_file("---\nmodel: haiku\nextends:  dev \n---\nHello.\n");
        assert_eq!(fm["model"], "haiku");
        assert_eq!(fm["extends"], "dev");
        assert_eq!(body, "Hello.");
        let (fm, body) = parse_agent_file("  no frontmatter ");
        assert!(fm.is_empty());
        assert_eq!(body, "no frontmatter");
    }

    #[test]
    fn an_agent_id_is_a_role_with_an_optional_specialty() {
        for ok in ["pm", "dev.backend", "growth-lead.data-eng"] {
            assert!(is_agent_id(ok), "{ok}");
        }
        for bad in ["Pm", "dev.", "dev..x", "dev_x", "-dev", "a.b.c", ""] {
            assert!(!is_agent_id(bad), "{bad}");
        }
    }

    #[test]
    fn a_reference_cannot_leave_its_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.md"), "x").unwrap();
        let roots = vec![dir.path().to_string_lossy().into_owned()];
        assert!(resolve_under_roots("a.md", &roots).is_some());
        assert!(resolve_under_roots("../a.md", &roots).is_none());
        assert!(resolve_under_roots("missing.md", &roots).is_none());
        assert!(resolve_under_roots("/etc/passwd", &roots).is_none());
    }

    #[test]
    fn a_brief_is_checked_against_the_roster() {
        let brief: Brief = serde_json::from_str(
            r#"{"product":{"stage":"dreaming"},
                "objectives":[{"id":"a","due":"2026-13-40"},{"id":"a","due":"2026-01-01"}],
                "north_stars":["nope.md",3],
                "roles":{"pm":"x","extra":"y"}}"#,
        )
        .unwrap();
        let product = Product {
            id: "p".into(),
            name: "P".into(),
            paths: vec![],
            status: "active".into(),
            roles: vec!["pm".into(), "dev".into()],
        };
        let failures = validate_brief(&brief, &product, &product.roles);
        assert_eq!(
            failures,
            [
                "product.stage \"dreaming\" is not one of idea, building, launched, growing, maintenance.",
                "Objective \"a\" has an invalid due date \"2026-13-40\" (expected YYYY-MM-DD).",
                "Objective id \"a\" is repeated.",
                "north_stars path \"nope.md\" does not exist under a product path.",
                "north_stars path \"3\" does not exist under a product path.",
                "roles has no entry for roster agent \"dev\".",
                "roles entry \"extra\" is not in the roster.",
            ]
        );
    }
}
