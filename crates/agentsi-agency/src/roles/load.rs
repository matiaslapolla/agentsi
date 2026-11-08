//! `loadRole` and `describeRoster` of `roles.ts` and `roster.ts`: the
//! definition of an agent from its global role file and its overlay.

use super::files::RoleFiles;
use crate::roster::{is_agent_id, parse_agent_file, read_global};
use agentsi_proto::roles_api::{RoleDef, RoleSource, RosterEntry};
use std::collections::BTreeMap;

const ALLOWED_TOOLS: [&str; 4] = ["Read", "Grep", "Glob", "WebSearch"];

type Parsed = (BTreeMap<String, String>, String);

/// Keeps the built-in tools that a meeting may grant. `warn` reports the rest.
fn parse_tools(raw: &str, warn: bool, id: &str, warnings: &mut Vec<String>) -> Vec<String> {
    let names: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    let dropped: Vec<&str> = names
        .iter()
        .copied()
        .filter(|t| !ALLOWED_TOOLS.contains(t))
        .collect();
    if warn && !dropped.is_empty() {
        warnings.push(format!(
            "Agent \"{id}\": dropped tools not allowed in meetings: {}",
            dropped.join(", ")
        ));
    }
    let mut kept: Vec<String> = Vec::new();
    for t in names.into_iter().filter(|t| ALLOWED_TOOLS.contains(t)) {
        if !kept.iter().any(|k| k == t) {
            kept.push(t.to_string());
        }
    }
    kept
}

fn non_empty<'a>(fm: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    fm.get(key).map(String::as_str).filter(|v| !v.is_empty())
}

fn to_def(
    id: &str,
    role: &str,
    source: RoleSource,
    fm: &BTreeMap<String, String>,
    body: String,
    warn_tools: bool,
    warnings: &mut Vec<String>,
) -> RoleDef {
    RoleDef {
        id: id.into(),
        role: role.into(),
        source,
        model: non_empty(fm, "model").unwrap_or("sonnet").into(),
        effort: non_empty(fm, "effort").map(String::from),
        tools: non_empty(fm, "tools").map(|raw| parse_tools(raw, warn_tools, id, warnings)),
        body,
    }
}

/// `loadRole`. The overlays are the ones of the product (or of a stage). The
/// warnings that TypeScript prints on stderr go to `warnings`.
pub fn load_role(
    id: &str,
    product: &str,
    files: &RoleFiles,
    overlays: &BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) -> Result<RoleDef, String> {
    if !is_agent_id(id) {
        return Err(format!("Invalid agent id \"{id}\""));
    }
    let overlay: Option<Parsed> = overlays.get(id).map(|t| parse_agent_file(t));
    let Some((overlay_fm, overlay_body)) = overlay else {
        if id.contains('.') {
            return Err(format!("Specialist \"{id}\" has no overlay in {product}"));
        }
        let (fm, body) = parse_agent_file(&read_global(id, files)?);
        return Ok(to_def(
            id,
            id,
            RoleSource::Global,
            &fm,
            body,
            false,
            warnings,
        ));
    };
    let Some(parent) = non_empty(&overlay_fm, "extends") else {
        let role = id.split('.').next().unwrap_or(id);
        return Ok(to_def(
            id,
            role,
            RoleSource::Custom,
            &overlay_fm,
            overlay_body,
            true,
            warnings,
        ));
    };
    let (mut merged, base_body) = parse_agent_file(&read_global(parent, files)?);
    for key in ["model", "effort", "tools"] {
        if let Some(v) = non_empty(&overlay_fm, key) {
            merged.insert(key.into(), v.into());
        }
    }
    let body = if overlay_body.is_empty() {
        base_body
    } else {
        format!("{base_body}\n\n## This product\n\n{overlay_body}")
    };
    let warn = non_empty(&overlay_fm, "tools").is_some();
    Ok(to_def(
        id,
        parent,
        RoleSource::Overlay,
        &merged,
        body,
        warn,
        warnings,
    ))
}

/// One entry for each agent of the roster. An agent that does not resolve
/// carries its error.
pub fn describe_roster(
    roster: &[String],
    product: &str,
    files: &RoleFiles,
    overlays: &BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) -> Vec<RosterEntry> {
    roster
        .iter()
        .map(
            |id| match load_role(id, product, files, overlays, warnings) {
                Ok(def) => RosterEntry {
                    id: id.clone(),
                    role: Some(def.role),
                    source: Some(def.source),
                    model: Some(def.model),
                    effort: def.effort,
                    error: None,
                },
                Err(error) => RosterEntry {
                    id: id.clone(),
                    role: None,
                    source: None,
                    model: None,
                    effort: None,
                    error: Some(error),
                },
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlays(items: &[(&str, &str)]) -> BTreeMap<String, String> {
        items
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn load(id: &str, o: &BTreeMap<String, String>) -> (Result<RoleDef, String>, Vec<String>) {
        let mut warnings = Vec::new();
        let def = load_role(id, "p", &RoleFiles::Builtin, o, &mut warnings);
        (def, warnings)
    }

    #[test]
    fn a_global_role_loads_from_its_file() {
        let (def, warnings) = load("pm", &BTreeMap::new());
        let def = def.unwrap();
        assert_eq!((def.role.as_str(), def.source), ("pm", RoleSource::Global));
        assert!(warnings.is_empty());
        assert!(def.tools.is_some());
    }

    #[test]
    fn an_overlay_changes_the_model_and_adds_a_section() {
        let o = overlays(&[(
            "dev.backend",
            "---\nextends: dev\nmodel: opus\ntools: Read, Bash, Read, Grep\n---\nUse Postgres.",
        )]);
        let (def, warnings) = load("dev.backend", &o);
        let def = def.unwrap();
        assert_eq!(
            (def.role.as_str(), def.source),
            ("dev", RoleSource::Overlay)
        );
        assert_eq!(def.model, "opus");
        assert_eq!(def.tools.unwrap(), ["Read", "Grep"]);
        assert!(def.body.ends_with("## This product\n\nUse Postgres."));
        assert_eq!(
            warnings,
            ["Agent \"dev.backend\": dropped tools not allowed in meetings: Bash"]
        );
    }

    #[test]
    fn an_overlay_without_extends_is_a_custom_role() {
        let o = overlays(&[("growth.data", "---\nmodel: haiku\n---\nCount users.")]);
        let def = load("growth.data", &o).0.unwrap();
        assert_eq!(
            (def.role.as_str(), def.source),
            ("growth", RoleSource::Custom)
        );
        assert_eq!(
            (def.model.as_str(), def.body.as_str()),
            ("haiku", "Count users.")
        );
    }

    #[test]
    fn a_bad_agent_does_not_load() {
        let none = BTreeMap::new();
        assert_eq!(
            load("Bad", &none).0.unwrap_err(),
            "Invalid agent id \"Bad\""
        );
        assert_eq!(
            load("dev.x", &none).0.unwrap_err(),
            "Specialist \"dev.x\" has no overlay in p"
        );
        assert!(
            load("ghost", &none)
                .0
                .unwrap_err()
                .starts_with("Unknown role \"ghost\"")
        );
    }

    #[test]
    fn the_roster_lists_the_error_of_an_agent_that_does_not_load() {
        let roster = vec!["pm".to_string(), "ghost".to_string()];
        let mut warnings = Vec::new();
        let entries = describe_roster(
            &roster,
            "p",
            &RoleFiles::Builtin,
            &BTreeMap::new(),
            &mut warnings,
        );
        assert_eq!(entries[0].source, Some(RoleSource::Global));
        assert!(entries[1].error.as_deref().unwrap().contains("ghost"));
        assert_eq!(entries[1].role, None);
    }
}
