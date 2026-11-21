//! General roles: role files of `<data root>/plugin/agents/` that are not one of the five
//! builtin roles. Every product can add one to its roster. Spec:
//! `docs/specs/agentsi-desktop-global-roles.md`.
//!
//! The first write makes the folder, and it copies in the roles that were in effect, so the
//! builtin roles keep loading: `RoleFiles::at` reads only the folder once it exists.

use super::files::{RoleFiles, is_builtin};
use super::{Error, Result, invalid};
use crate::registry::{Product, Registry};
use crate::roster::{is_role_id, parse_agent_file, read_overlays};
use agentsi_proto::roles_api::GeneralRole;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// The folder of the role files, relative to the data root.
pub const FOLDER: &str = "plugin/agents";

/// The path of a role file relative to the data root, as git names it.
pub fn rel_path(id: &str) -> String {
    format!("{FOLDER}/{id}.md")
}

fn dir(root: &Path) -> PathBuf {
    root.join(FOLDER)
}

fn io(what: &str, path: &Path, e: std::io::Error) -> Error {
    Error::Agency(crate::error::Error::Unavailable(format!(
        "cannot {what} {}: {e}",
        path.display()
    )))
}

/// Makes the folder of the data root with the roles in effect, when it does not exist yet.
fn ensure_dir(root: &Path) -> Result<PathBuf> {
    let own = dir(root);
    if own.exists() {
        return Ok(own);
    }
    let before = RoleFiles::at(root);
    let names = before
        .names()
        .map_err(|e| Error::Agency(crate::error::Error::Unavailable(e)))?;
    std::fs::create_dir_all(&own).map_err(|e| io("make", &own, e))?;
    for name in names {
        if let Some(text) = before.read(&name) {
            write_atomic(&own.join(format!("{name}.md")), &text)?;
        }
    }
    Ok(own)
}

/// Writes a temporary file next to `file`, then renames it.
fn write_atomic(file: &Path, text: &str) -> Result<()> {
    use std::io::Write;
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("role");
    let tmp = file.with_file_name(format!(".{name}.tmp"));
    let write = || -> std::io::Result<()> {
        let mut out = std::fs::File::create(&tmp)?;
        out.write_all(text.as_bytes())?;
        out.sync_all()?;
        std::fs::rename(&tmp, file)
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io("write", file, e)
    })
}

fn one_line(field: &str, value: &str) -> Result<String> {
    let v = value.trim();
    if v.contains('\n') || v.contains('\r') {
        return invalid(format!("The {field} must be one line."));
    }
    Ok(v.to_string())
}

fn check(role: &GeneralRole) -> Result<GeneralRole> {
    if !is_role_id(&role.id) {
        return invalid(format!(
            "\"{}\" is not a role id: use lowercase words joined by single hyphens, such as \
             designer or growth-lead.",
            role.id
        ));
    }
    if is_builtin(&role.id) {
        return invalid(format!(
            "\"{}\" is a builtin role. A product overlay (extends: {}) changes it.",
            role.id, role.id
        ));
    }
    let body = role.body.trim();
    if body.is_empty() {
        return invalid("The body of a role cannot be empty.");
    }
    if body.lines().next() == Some("---") {
        return invalid("The body cannot start with a front matter line (---).");
    }
    let name = one_line("name", &role.name)?;
    Ok(GeneralRole {
        id: role.id.clone(),
        name: if name.is_empty() {
            role.id.clone()
        } else {
            name
        },
        description: one_line("description", &role.description)?,
        body: body.to_string(),
    })
}

/// The text of a new role file: the front matter keys of the builtin roles. `name` is the
/// role id, as Claude Code reads it; `title` is the name that a person reads.
fn render(role: &GeneralRole) -> String {
    format!(
        "---\nname: {}\ntitle: {}\ndescription: {}\nmodel: sonnet\neffort: medium\n\
         tools: Read, Grep, Glob\nmaxTurns: 12\n---\n\n{}\n",
        role.id, role.name, role.description, role.body
    )
}

/// `text` with `title` and `description` set and the body replaced. The other front matter
/// lines stay as they were, in their order.
fn edit(text: &str, role: &GeneralRole) -> String {
    let front: Vec<&str> = text
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .map(|(fm, _)| fm.lines().collect())
        .unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let (mut title, mut description) = (false, false);
    for line in front {
        if line.starts_with("title:") {
            lines.push(format!("title: {}", role.name));
            title = true;
        } else if line.starts_with("description:") {
            lines.push(format!("description: {}", role.description));
            description = true;
        } else {
            lines.push(line.to_string());
        }
    }
    if lines.is_empty() {
        return render(role);
    }
    let at = lines
        .iter()
        .position(|l| l.starts_with("name:"))
        .map_or(0, |i| i + 1);
    if !description {
        lines.insert(at, format!("description: {}", role.description));
    }
    if !title {
        lines.insert(at, format!("title: {}", role.name));
    }
    format!("---\n{}\n---\n\n{}\n", lines.join("\n"), role.body)
}

/// The role in a role file.
fn parse(id: &str, text: &str) -> GeneralRole {
    let (fm, body) = parse_agent_file(text);
    GeneralRole {
        id: id.into(),
        name: fm
            .get("title")
            .filter(|t| !t.is_empty())
            .cloned()
            .unwrap_or_else(|| id.into()),
        description: fm.get("description").cloned().unwrap_or_default(),
        body,
    }
}

/// The general roles, by id.
pub fn list(root: &Path) -> Result<Vec<GeneralRole>> {
    let files = RoleFiles::at(root);
    let mut roles: Vec<GeneralRole> = files
        .names()
        .map_err(|e| Error::Agency(crate::error::Error::Unavailable(e)))?
        .into_iter()
        .filter(|n| !is_builtin(n))
        .filter_map(|n| Some(parse(&n, &files.read(&n)?)))
        .collect();
    roles.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(roles)
}

pub fn create(root: &Path, role: &GeneralRole) -> Result<GeneralRole> {
    let role = check(role)?;
    if RoleFiles::at(root).has(&role.id) {
        return invalid(format!("A role \"{}\" exists already.", role.id));
    }
    let dir = ensure_dir(root)?;
    write_atomic(&dir.join(format!("{}.md", role.id)), &render(&role))?;
    Ok(role)
}

pub fn update(root: &Path, role: &GeneralRole) -> Result<GeneralRole> {
    let role = check(role)?;
    let file = dir(root).join(format!("{}.md", role.id));
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(not_found(&role.id));
        }
        Err(e) => return Err(io("read", &file, e)),
    };
    write_atomic(&file, &edit(&text, &role))?;
    Ok(role)
}

fn not_found(id: &str) -> Error {
    Error::Agency(crate::error::Error::NotFound(format!(
        "There is no general role \"{id}\"."
    )))
}

/// True when agent `agent` of a roster plays role `id`: the agent is the role, a specialist
/// of it, or its overlay extends it.
fn plays(agent: &str, id: &str, overlay: Option<&String>) -> bool {
    agent.split('.').next() == Some(id)
        || overlay
            .is_some_and(|o| parse_agent_file(o).0.get("extends").map(String::as_str) == Some(id))
}

/// The products whose roster plays role `id`, in registry order.
pub fn used_by(c: &Connection, products: &[Product], id: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for p in products {
        let overlays = read_overlays(c, &p.id)?;
        if p.roles.iter().any(|a| plays(a, id, overlays.get(a))) {
            out.push(p.id.clone());
        }
    }
    Ok(out)
}

/// Deletes a general role that no roster uses.
pub fn delete(c: &Connection, root: &Path, registry: &Registry, id: &str) -> Result<()> {
    if is_builtin(id) {
        return invalid(format!("\"{id}\" is a builtin role. It cannot be deleted."));
    }
    let file = dir(root).join(format!("{id}.md"));
    if !is_role_id(id) || !file.exists() {
        return Err(not_found(id));
    }
    let users = used_by(c, &registry.products, id)?;
    if !users.is_empty() {
        return invalid(format!(
            "The roster of {} uses \"{id}\". Take it out of the roster first.",
            users.join(", ")
        ));
    }
    std::fs::remove_file(&file).map_err(|e| io("delete", &file, e))
}

/// Moves the custom role `agent` of `product` to a general role. The overlay leaves the
/// database in `tx`, and the role file is the last write, so a failed write keeps the
/// overlay.
pub fn promote(
    tx: &rusqlite::Transaction,
    root: &Path,
    product: &Product,
    agent: &str,
) -> Result<GeneralRole> {
    let overlays = read_overlays(tx, &product.id)?;
    let Some(text) = overlays.get(agent) else {
        return Err(Error::Agency(crate::error::Error::NotFound(format!(
            "{} has no custom role \"{agent}\".",
            product.id
        ))));
    };
    let (fm, body) = parse_agent_file(text);
    if fm.get("extends").is_some_and(|e| !e.is_empty()) {
        return invalid(format!(
            "\"{agent}\" extends a global role: it is an overlay, not a custom role."
        ));
    }
    if !is_role_id(agent) {
        return invalid(format!(
            "\"{agent}\" is a specialist. Only a custom role with no specialty can be general."
        ));
    }
    let role = check(&GeneralRole {
        id: agent.into(),
        name: fm.get("title").cloned().unwrap_or_default(),
        description: fm.get("description").cloned().unwrap_or_default(),
        body,
    })?;
    if RoleFiles::at(root).has(agent) {
        return invalid(format!("A global role \"{agent}\" exists already."));
    }
    tx.execute(
        "delete from agent_overlays where product = ?1 and agent = ?2",
        [product.id.as_str(), agent],
    )?;
    let mut keys = String::new();
    for key in ["model", "effort", "tools"] {
        if let Some(v) = fm.get(key).filter(|v| !v.is_empty()) {
            keys.push_str(&format!("{key}: {v}\n"));
        }
    }
    let text = if keys.is_empty() {
        render(&role)
    } else {
        format!(
            "---\nname: {}\ntitle: {}\ndescription: {}\n{keys}---\n\n{}\n",
            role.id, role.name, role.description, role.body
        )
    };
    let dir = ensure_dir(root)?;
    write_atomic(&dir.join(format!("{agent}.md")), &text)?;
    Ok(role)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roles::load::load_role;
    use agentsi_proto::roles_api::RoleSource;
    use std::collections::BTreeMap;

    fn role(id: &str) -> GeneralRole {
        GeneralRole {
            id: id.into(),
            name: "Designer".into(),
            description: "Reads the screens.".into(),
            body: "You are the designer.".into(),
        }
    }

    fn message(r: Result<impl std::fmt::Debug>) -> String {
        match r.unwrap_err() {
            Error::Invalid(m) => m,
            Error::Agency(e) => format!("agency: {e}"),
        }
    }

    fn product(id: &str, roles: &[&str]) -> Product {
        Product {
            id: id.into(),
            name: id.into(),
            paths: vec![],
            status: "active".into(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
        }
    }

    #[test]
    fn a_new_role_is_a_file_and_the_builtin_roles_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let made = create(dir.path(), &role("designer")).unwrap();
        assert_eq!(made, role("designer"));
        let file = dir.path().join("plugin/agents/designer.md");
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("---\nname: designer\ntitle: Designer\n"));
        assert!(text.ends_with("---\n\nYou are the designer.\n"));

        let files = RoleFiles::at(dir.path());
        assert_eq!(
            files.names().unwrap(),
            ["pm", "social", "dev", "qa", "scout", "designer"]
        );
        let mut warnings = Vec::new();
        for product in ["a", "b"] {
            let def =
                load_role("designer", product, &files, &BTreeMap::new(), &mut warnings).unwrap();
            assert_eq!(def.source, RoleSource::Global);
            assert_eq!(def.body, "You are the designer.");
            assert_eq!(def.tools.unwrap(), ["Read", "Grep", "Glob"]);
        }
        let pm = load_role("pm", "a", &files, &BTreeMap::new(), &mut warnings).unwrap();
        assert_eq!(
            pm.body,
            parse_agent_file(&RoleFiles::Builtin.read("pm").unwrap()).1
        );
        assert_eq!(list(dir.path()).unwrap(), [role("designer")]);
    }

    #[test]
    fn a_builtin_a_taken_or_a_bad_id_is_invalid() {
        let dir = tempfile::tempdir().unwrap();
        assert!(message(create(dir.path(), &role("dev"))).contains("builtin role"));
        assert!(!dir.path().join("plugin").exists());
        for bad in ["Designer", "ux.lead", "ux--lead", ""] {
            assert!(
                message(create(dir.path(), &role(bad))).contains("not a role id"),
                "{bad}"
            );
        }
        create(dir.path(), &role("designer")).unwrap();
        assert!(message(create(dir.path(), &role("designer"))).contains("exists already"));
        let mut empty = role("ux");
        empty.body = "  ".into();
        assert!(message(create(dir.path(), &empty)).contains("cannot be empty"));
        let mut two = role("ux");
        two.description = "a\nb".into();
        assert!(message(create(dir.path(), &two)).contains("one line"));
    }

    #[test]
    fn an_update_keeps_the_other_front_matter_keys() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join("plugin/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("seo.md"),
            "---\nname: seo\ndescription: old\nmodel: haiku\n---\nOld body.\n",
        )
        .unwrap();
        let mut next = role("seo");
        next.name = "SEO".into();
        update(dir.path(), &next).unwrap();
        assert_eq!(
            std::fs::read_to_string(agents.join("seo.md")).unwrap(),
            "---\nname: seo\ntitle: SEO\ndescription: Reads the screens.\nmodel: haiku\n---\n\nYou are the designer.\n"
        );
        assert!(message(update(dir.path(), &role("ghost"))).starts_with("agency: There is no"));
        assert!(message(update(dir.path(), &role("qa"))).contains("builtin"));
    }

    #[test]
    fn a_role_that_a_roster_uses_is_not_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let conn = agentsi_store::db::open_writer(&dir.path().join("t.db")).unwrap();
        conn.execute(
            "insert into agent_overlays (product, agent, body) values ('c', 'ux', '---\nextends: designer\n---\nx')",
            [],
        )
        .unwrap();
        create(dir.path(), &role("designer")).unwrap();
        let registry = |products: Vec<Product>| Registry {
            owner: Registry::parse("{}").unwrap().owner,
            products,
        };
        let users = registry(vec![
            product("a", &["pm", "designer"]),
            product("b", &["pm", "designer.web"]),
            product("c", &["ux"]),
            product("d", &["pm"]),
        ]);
        assert_eq!(
            message(delete(&conn, dir.path(), &users, "designer")),
            "The roster of a, b, c uses \"designer\". Take it out of the roster first."
        );
        assert!(message(delete(&conn, dir.path(), &users, "dev")).contains("builtin"));
        assert!(message(delete(&conn, dir.path(), &users, "ghost")).contains("no general role"));
        delete(
            &conn,
            dir.path(),
            &registry(vec![product("d", &["pm"])]),
            "designer",
        )
        .unwrap();
        assert!(!dir.path().join("plugin/agents/designer.md").exists());
    }

    #[test]
    fn a_custom_role_moves_to_a_general_role_and_the_roster_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = agentsi_store::db::open_writer(&dir.path().join("t.db")).unwrap();
        for (agent, body) in [
            (
                "seo",
                "---\nmodel: haiku\ndescription: Search\n---\nYou rank pages.",
            ),
            ("dev", "---\nextends: dev\n---\nUse Go."),
            ("growth.data", "Count users."),
        ] {
            conn.execute(
                "insert into agent_overlays (product, agent, body) values ('a', ?1, ?2)",
                [agent, body],
            )
            .unwrap();
        }
        let a = product("a", &["pm", "seo", "dev", "growth.data"]);
        let tx = conn.transaction().unwrap();
        assert!(message(promote(&tx, dir.path(), &a, "dev")).contains("not a custom role"));
        assert!(message(promote(&tx, dir.path(), &a, "growth.data")).contains("specialist"));
        assert!(message(promote(&tx, dir.path(), &a, "ghost")).contains("no custom role"));
        let made = promote(&tx, dir.path(), &a, "seo").unwrap();
        tx.commit().unwrap();
        assert_eq!(
            (made.name.as_str(), made.description.as_str()),
            ("seo", "Search")
        );
        assert!(
            agentsi_store::read::overlay(&conn, "a", "seo")
                .unwrap()
                .is_none()
        );

        let files = RoleFiles::at(dir.path());
        let overlays = read_overlays(&conn, "a").unwrap();
        let def = load_role("seo", "a", &files, &overlays, &mut Vec::new()).unwrap();
        assert_eq!(
            (def.source, def.model.as_str(), def.body.as_str()),
            (RoleSource::Global, "haiku", "You rank pages.")
        );
        assert_eq!(list(dir.path()).unwrap()[0].id, "seo");
    }
}
