//! The reflection decision: `settleReflection`, `applyProposal` and `targetHash` of
//! `reflection-decision.ts`, and the allowlist of `workspace-write.ts` for the one
//! file path that remains. The overlays are rows, so a proposal writes a file only
//! for a global role.

use super::text;
use crate::meeting::log::sha256;
use crate::roles::commit::commit_files;
use agentsi_proto::roles_api::CommitInfo;
use agentsi_store::read;
use agentsi_store::reflection::{write_overlay, write_reflections};
use regex::Regex;
use rusqlite::{Connection, Transaction, params};
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

static OVERLAY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^workspace/([^/]+)/agents/([^/]+)\.md$").unwrap());
static GLOBAL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^plugin/agents/[a-z][a-z-]*\.md$").unwrap());

/// The overlay that a target names. Overlays are rows, not files.
fn overlay_of(target: &str) -> Option<(String, String)> {
    let m = OVERLAY.captures(target)?;
    Some((m[1].to_string(), m[2].to_string()))
}

/// sha256 of the text of the target, or `None` when it does not exist.
pub fn target_hash(
    c: &Connection,
    target: &str,
    root: &Path,
) -> agentsi_store::Result<Option<String>> {
    if let Some((product, agent)) = overlay_of(target) {
        return Ok(read::overlay(c, &product, &agent)?.map(|t| sha256(&t)));
    }
    Ok(std::fs::read_to_string(root.join(target))
        .ok()
        .map(|t| sha256(&t)))
}

/// A proposal may only write the overlay of its agent in its product, or a global role file.
fn allowed_target(product: &str, target: &str, agent: &str) -> bool {
    target == format!("workspace/{product}/agents/{agent}.md") || GLOBAL.is_match(target)
}

/// Resolves the links of the deepest ancestor that exists, so a link cannot lead a
/// write out of the allowlist.
fn real_path(path: &Path) -> std::io::Result<PathBuf> {
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut head = path.to_path_buf();
    while !head.exists() {
        let Some(name) = head.file_name().map(|n| n.to_os_string()) else {
            break;
        };
        tail.push(name);
        head.pop();
    }
    let mut real = std::fs::canonicalize(&head)?;
    real.extend(tail.iter().rev());
    Ok(real)
}

/// `writeAllowed` with `allowPlugin`: only `plugin/agents/<name>.md` of the data root,
/// and never a name with a leading `_`. Any other path is refused.
pub fn write_role_file(root: &Path, target: &str, content: &str) -> Result<(), String> {
    let refused = || format!("Path \"{target}\" is outside the workspace write allowlist.");
    let plain = Path::new(target)
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if !plain {
        return Err(refused());
    }
    let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let full = root.join(target);
    let real = real_path(&full).map_err(|e| e.to_string())?;
    let relative = real.strip_prefix(&root).map_err(|_| refused())?;
    let parts: Vec<_> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let [top, agents, file] = parts.as_slice() else {
        return Err(refused());
    };
    if top != "plugin" || agents != "agents" || !file.ends_with(".md") || file.starts_with('_') {
        return Err(refused());
    }
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&full, content).map_err(|e| e.to_string())
}

/// The insert of the decision of a pending proposal. Returns its id.
pub fn record_decision(
    tx: &Transaction,
    product: &str,
    reflection_id: &str,
    decision_text: &str,
    why: &str,
) -> rusqlite::Result<i64> {
    tx.query_row(
        "insert into decisions (product, kind, reflection_id, text, why)
         values (?1, 'reflection', ?2, ?3, ?4) returning id",
        params![product, reflection_id, decision_text, why],
        |r| r.get(0),
    )
}

/// A global role file that an approved proposal writes. The caller writes it after
/// its last row, so a failed write rolls each row back.
#[derive(Debug, PartialEq, Eq)]
pub struct RoleFile {
    target: String,
    content: String,
    message: String,
}

impl RoleFile {
    /// Writes the file and commits it alone.
    pub fn write(&self, root: &Path) -> Result<CommitInfo, String> {
        write_role_file(root, &self.target, &self.content)?;
        Ok(commit_files(root, &[&self.target], &self.message))
    }
}

/// How a reflection decision ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Settled {
    Approved {
        /// `None` for an overlay, which is a row, and for a retry.
        role_file: Option<RoleFile>,
    },
    Rejected,
    /// The target changed since the proposal, so nothing was applied.
    Stale,
    /// The reflection has no proposal, so the decision can never be applied.
    MissingProposal(String),
    /// The proposal was settled in another way, or may not write its target.
    Refused(String),
}

fn set_status(
    tx: &Transaction,
    product: &str,
    mut entries: Vec<Value>,
    index: usize,
    status: &str,
) -> agentsi_store::Result<()> {
    entries[index]["proposal"]["status"] = json!(status);
    write_reflections(tx, product, &entries)
}

/// Settles the pending proposal of one reflection, in the transaction of the caller.
/// Approval first checks that the target still has the hash that it had when the
/// proposal was made. Settling again with the same verdict returns the earlier
/// outcome, so a retry works.
///
/// `root` is the data root. This function writes no file: `Settled::Approved` hands the
/// caller the role file to write.
pub fn settle(
    tx: &Transaction,
    root: &Path,
    product: &str,
    reflection_id: &str,
    approve: bool,
) -> agentsi_store::Result<Settled> {
    let entries = read::reflections(tx, product)?;
    let found = entries.iter().position(|e| {
        text(e, "id") == reflection_id && e.get("proposal").is_some_and(|p| !p.is_null())
    });
    let Some(index) = found else {
        return Ok(Settled::MissingProposal(format!(
            "Reflection \"{reflection_id}\" has no proposal in {product}."
        )));
    };
    let proposal = entries[index]["proposal"].clone();
    let status = text(&proposal, "status");
    let verdict = if approve { "approved" } else { "rejected" };
    if status == verdict {
        return Ok(if approve {
            Settled::Approved { role_file: None }
        } else {
            Settled::Rejected
        });
    }
    if status == "stale" && approve {
        return Ok(Settled::Stale);
    }
    if status != "pending" {
        return Ok(Settled::Refused(format!(
            "The proposal of \"{reflection_id}\" is already {status}."
        )));
    }
    if !approve {
        set_status(tx, product, entries, index, "rejected")?;
        return Ok(Settled::Rejected);
    }

    let target = text(&proposal, "target");
    if !allowed_target(product, target, text(&proposal, "agent")) {
        return Ok(Settled::Refused(format!(
            "The proposal of \"{reflection_id}\" targets \"{target}\", which it may not write."
        )));
    }
    let wanted = proposal.get("target_hash").and_then(Value::as_str);
    if target_hash(tx, target, root)?.as_deref() != wanted {
        set_status(tx, product, entries, index, "stale")?;
        return Ok(Settled::Stale);
    }

    let content = text(&proposal, "content");
    if let Some((owner, agent)) = overlay_of(target) {
        write_overlay(tx, &owner, &agent, content)?;
        set_status(tx, product, entries, index, "approved")?;
        return Ok(Settled::Approved { role_file: None });
    }
    set_status(tx, product, entries, index, "approved")?;
    Ok(Settled::Approved {
        role_file: Some(RoleFile {
            target: target.into(),
            content: content.into(),
            message: format!("reflect({product}): {reflection_id} proposal approved"),
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsi_store::db::open_writer;

    fn entry(status: &str, target: &str, hash: Value) -> Value {
        json!({"id": "r-1", "created_at": "t", "status": "applied", "proposal": {
            "agent": "dev", "target": target, "status": status, "target_hash": hash,
            "content": "---\nextends: dev\n---\nNew.\n",
        }})
    }

    struct Db {
        dir: tempfile::TempDir,
        conn: Connection,
    }

    fn db() -> Db {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_writer(&dir.path().join("agentsi.db")).unwrap();
        Db { dir, conn }
    }

    const OVERLAY_TARGET: &str = "workspace/p/agents/dev.md";

    #[test]
    fn approval_writes_the_overlay_and_marks_the_proposal() {
        let mut d = db();
        let root = d.dir.path().to_path_buf();
        let tx = d.conn.transaction().unwrap();
        write_reflections(&tx, "p", &[entry("pending", OVERLAY_TARGET, Value::Null)]).unwrap();
        let got = settle(&tx, &root, "p", "r-1", true).unwrap();
        assert_eq!(got, Settled::Approved { role_file: None });
        let body = read::overlay(&tx, "p", "dev").unwrap().unwrap();
        assert!(body.contains("New."));
        let stored = read::reflections(&tx, "p").unwrap();
        assert_eq!(stored[0]["proposal"]["status"], "approved");
    }

    #[test]
    fn a_target_that_changed_makes_the_proposal_stale_and_applies_nothing() {
        let mut d = db();
        let root = d.dir.path().to_path_buf();
        let tx = d.conn.transaction().unwrap();
        write_overlay(&tx, "p", "dev", "someone wrote this").unwrap();
        write_reflections(&tx, "p", &[entry("pending", OVERLAY_TARGET, Value::Null)]).unwrap();
        assert_eq!(
            settle(&tx, &root, "p", "r-1", true).unwrap(),
            Settled::Stale
        );
        let body = read::overlay(&tx, "p", "dev").unwrap().unwrap();
        assert_eq!(body, "someone wrote this");
        assert_eq!(
            read::reflections(&tx, "p").unwrap()[0]["proposal"]["status"],
            "stale"
        );
        assert_eq!(
            settle(&tx, &root, "p", "r-1", true).unwrap(),
            Settled::Stale
        );
    }

    #[test]
    fn a_rejection_writes_no_overlay_and_a_retry_gives_the_same_outcome() {
        let mut d = db();
        let root = d.dir.path().to_path_buf();
        let tx = d.conn.transaction().unwrap();
        write_reflections(&tx, "p", &[entry("pending", OVERLAY_TARGET, Value::Null)]).unwrap();
        assert_eq!(
            settle(&tx, &root, "p", "r-1", false).unwrap(),
            Settled::Rejected
        );
        assert!(read::overlay(&tx, "p", "dev").unwrap().is_none());
        assert_eq!(
            settle(&tx, &root, "p", "r-1", false).unwrap(),
            Settled::Rejected
        );
        let other = settle(&tx, &root, "p", "r-1", true).unwrap();
        assert_eq!(
            other,
            Settled::Refused("The proposal of \"r-1\" is already rejected.".into())
        );
    }

    #[test]
    fn a_reflection_without_a_proposal_is_reported() {
        let mut d = db();
        let root = d.dir.path().to_path_buf();
        let tx = d.conn.transaction().unwrap();
        write_reflections(&tx, "p", &[json!({"id": "r-1", "proposal": null})]).unwrap();
        for id in ["r-1", "r-9"] {
            assert_eq!(
                settle(&tx, &root, "p", id, true).unwrap(),
                Settled::MissingProposal(format!("Reflection \"{id}\" has no proposal in p."))
            );
        }
    }

    #[test]
    fn a_proposal_may_write_only_its_overlay_or_a_global_role() {
        let mut d = db();
        let root = d.dir.path().to_path_buf();
        let tx = d.conn.transaction().unwrap();
        for target in [
            "workspace/q/agents/dev.md",
            "workspace/p/agents/qa.md",
            "packages/core/src/check.ts",
            "plugin/agents/../../x.md",
            "plugin/skills/agentsi/SKILL.md",
        ] {
            write_reflections(&tx, "p", &[entry("pending", target, Value::Null)]).unwrap();
            let got = settle(&tx, &root, "p", "r-1", true).unwrap();
            assert!(matches!(got, Settled::Refused(_)), "{target}: {got:?}");
        }
    }

    #[test]
    fn the_allowlist_refuses_each_path_but_a_global_role_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("plugin/agents")).unwrap();
        for target in [
            "packages/core/src/check.ts",
            "plugin/agents/_shared.md",
            "plugin/agents/dev.txt",
            "plugin/agents/sub/dev.md",
            "plugin/skills/agentsi/SKILL.md",
            "../outside.md",
            "/etc/x.md",
        ] {
            let got = write_role_file(root, target, "x");
            assert_eq!(
                got,
                Err(format!(
                    "Path \"{target}\" is outside the workspace write allowlist."
                ))
            );
        }
        write_role_file(root, "plugin/agents/dev.md", "new").unwrap();
        let text = std::fs::read_to_string(root.join("plugin/agents/dev.md")).unwrap();
        assert_eq!(text, "new");
    }

    #[test]
    fn a_link_cannot_lead_a_write_out_of_the_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(root.join("plugin")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("plugin/agents")).unwrap();
        let got = write_role_file(&root, "plugin/agents/dev.md", "x");
        assert!(got.is_err());
        assert!(!outside.join("dev.md").exists());
    }

    #[test]
    fn a_decision_row_has_the_kind_and_the_reflection() {
        let mut d = db();
        let tx = d.conn.transaction().unwrap();
        let id = record_decision(&tx, "p", "r-1", "Change it", "Why").unwrap();
        let got: (String, String, String) = tx
            .query_row(
                "select kind, reflection_id, status from decisions where id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(got, ("reflection".into(), "r-1".into(), "pending".into()));
    }
}
