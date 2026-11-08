//! What a repo gives to the index: its docs, its journal, its commits and its
//! open issues. The rules copy `collectDocs`, `collectCommits` and
//! `collectIssues` of `packages/core/src/indexer.ts`.

use super::exec::{self, git};
use super::repos::RepoTarget;
use crate::clock;
use regex::Regex;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const ROOT_DOCS: [&str; 6] = [
    "CLAUDE.md",
    "CONTEXT.md",
    "PRODUCT.md",
    "DESIGN.md",
    "README.md",
    "I18N.md",
];
const SKIP_DIRS: [&str; 5] = ["node_modules", ".worktrees", ".git", ".next", "dist"];
const MAX_FILE_BYTES: u64 = 200 * 1024;
const COMMIT_DAYS: u32 = 90;
const ISSUE_TIMEOUT: Duration = Duration::from_secs(25);
/// `body.trim().slice(0, 600)` counts UTF-16 units.
const COMMIT_BODY_UNITS: usize = 600;

static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^#\s+(.+)$").unwrap());

/// One row of the `documents` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocRow {
    pub scope: Option<String>,
    pub kind: &'static str,
    pub reference: String,
    pub title: String,
    pub body: String,
    /// ISO 8601 UTC with 3 fraction digits.
    pub updated_at: String,
}

/// The first `units` UTF-16 units of `text`, cut at a whole character.
pub fn utf16_prefix(text: &str, units: usize) -> &str {
    let mut used = 0;
    for (at, ch) in text.char_indices() {
        used += ch.len_utf16();
        if used > units {
            return &text[..at];
        }
    }
    text
}

fn first_heading(text: &str, fallback: &str) -> String {
    HEADING
        .captures(text)
        .map_or_else(|| fallback.to_string(), |c| c[1].trim().to_string())
}

fn mtime_iso(modified: SystemTime) -> String {
    let ms = modified
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    clock::iso(ms)
}

/// A file of 200 KiB or less. A byte that is not UTF-8 becomes U+FFFD.
fn read_capped(file: &Path) -> Option<(String, String)> {
    let meta = std::fs::metadata(file).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(file).ok()?;
    Some((
        String::from_utf8_lossy(&bytes).into_owned(),
        mtime_iso(meta.modified().ok()?),
    ))
}

fn walk_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            walk_markdown(&entry.path(), out);
        } else if kind.is_file() && name.ends_with(".md") {
            out.push(entry.path());
        }
    }
}

pub fn collect_docs(t: &RepoTarget) -> Vec<DocRow> {
    let root = t.dir();
    let mut files: Vec<PathBuf> = ROOT_DOCS.iter().map(|f| root.join(f)).collect();
    walk_markdown(&root.join("docs"), &mut files);
    let mut rows = Vec::new();
    for file in files {
        let Some((text, updated_at)) = read_capped(&file) else {
            continue;
        };
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        rows.push(DocRow {
            scope: t.scope.clone(),
            kind: "doc",
            reference: t.ref_of(&rel, '/'),
            title: first_heading(&text, &rel),
            body: text,
            updated_at,
        });
    }
    if let Some((text, updated_at)) = read_capped(&root.join(".claude/JOURNAL.md")) {
        rows.push(DocRow {
            scope: t.scope.clone(),
            kind: "journal",
            reference: t.ref_of(".claude/JOURNAL.md", '/'),
            title: "Journal".into(),
            body: text,
            updated_at,
        });
    }
    rows
}

/// The commits of the last 90 days on any branch. `None` when `git log` fails.
pub fn collect_commits(t: &RepoTarget) -> Option<Vec<DocRow>> {
    let since = format!("--since={COMMIT_DAYS}.days");
    let res = git(
        &t.dir(),
        &[
            "log",
            "--branches",
            &since,
            "--format=%H%x1f%aI%x1f%s%x1f%b%x1e",
        ],
    );
    if !res.ok {
        return None;
    }
    let mut rows = Vec::new();
    for record in res.stdout.split('\x1e') {
        let mut parts = record.trim().split('\x1f');
        let (Some(hash), Some(date), Some(subject)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let body = parts.next().unwrap_or("");
        if hash.is_empty() || date.is_empty() || subject.is_empty() {
            continue;
        }
        let Some(ms) = clock::parse_iso_ms(date) else {
            continue;
        };
        let body = format!(
            "{subject}\n{}",
            utf16_prefix(body.trim(), COMMIT_BODY_UNITS)
        );
        rows.push(DocRow {
            scope: t.scope.clone(),
            kind: "commit",
            reference: hash.to_string(),
            title: subject.to_string(),
            body: body.trim().to_string(),
            updated_at: clock::iso(ms),
        });
    }
    Some(rows)
}

/// The open issues, through `gh`. `None` when the repo has no remote, or `gh` fails.
pub fn collect_issues(t: &RepoTarget, now_ms: i64) -> Option<Vec<DocRow>> {
    let dir = t.dir();
    let remotes = git(&dir, &["remote"]);
    if !remotes.ok || remotes.stdout.trim().is_empty() {
        return None;
    }
    let res = exec::run(
        "gh",
        &[
            "issue",
            "list",
            "--state",
            "open",
            "--json",
            "number,title,body,labels",
            "--limit",
            "50",
        ],
        &dir,
        ISSUE_TIMEOUT,
    );
    if !res.ok {
        return None;
    }
    let issues: Value = serde_json::from_str(&res.stdout).ok()?;
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    Some(
        issues
            .as_array()?
            .iter()
            .map(|i| {
                let number = i.get("number").map(|n| n.to_string()).unwrap_or_default();
                let title = text(i, "title");
                let labels: Vec<&str> = i
                    .get("labels")
                    .and_then(Value::as_array)
                    .map(|l| {
                        l.iter()
                            .filter_map(|x| x.get("name").and_then(Value::as_str))
                            .collect()
                    })
                    .unwrap_or_default();
                DocRow {
                    scope: t.scope.clone(),
                    kind: "issue",
                    reference: t.ref_of(&number, '#'),
                    body: format!("{title}\n{}\n{}", labels.join(", "), text(i, "body"))
                        .trim()
                        .to_string(),
                    title,
                    updated_at: clock::iso(now_ms),
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(path: &Path) -> RepoTarget {
        RepoTarget {
            path: path.to_string_lossy().into_owned(),
            scope: None,
            prefix: String::new(),
        }
    }

    #[test]
    fn a_doc_takes_its_title_from_the_first_heading_or_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("docs/specs")).unwrap();
        std::fs::create_dir_all(root.join("docs/node_modules")).unwrap();
        std::fs::create_dir_all(root.join(".claude")).unwrap();
        std::fs::write(root.join("README.md"), "intro\n# The product\r\nbody\n").unwrap();
        std::fs::write(root.join("docs/specs/a.md"), "no heading here\n").unwrap();
        std::fs::write(root.join("docs/node_modules/x.md"), "# skipped\n").unwrap();
        std::fs::write(root.join("docs/big.md"), "x".repeat(201 * 1024)).unwrap();
        std::fs::write(root.join("docs/notes.txt"), "# not markdown\n").unwrap();
        std::fs::write(root.join(".claude/JOURNAL.md"), "# Day 1\n").unwrap();

        let rows = collect_docs(&target(root));
        let seen: Vec<(&str, &str, &str)> = rows
            .iter()
            .map(|r| (r.kind, r.reference.as_str(), r.title.as_str()))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("doc", "README.md", "The product"),
                ("doc", "docs/specs/a.md", "docs/specs/a.md"),
                ("journal", ".claude/JOURNAL.md", "Journal"),
            ]
        );
    }

    #[test]
    fn a_commit_body_is_cut_at_600_units_of_a_whole_character() {
        assert_eq!(utf16_prefix("abc", 2), "ab");
        assert_eq!(utf16_prefix("a😀b", 2), "a", "an emoji takes two units");
        assert_eq!(utf16_prefix("a😀b", 3), "a😀");
        assert_eq!(utf16_prefix("abc", 9), "abc");
    }

    #[test]
    fn the_commits_of_a_repo_come_from_git_log() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let run = |args: &[&str]| {
            let ok = std::process::Command::new("git")
                .args(["-c", "user.name=T", "-c", "user.email=t@example.invalid"])
                .args(["-c", "commit.gpgsign=false"])
                .args(args)
                .current_dir(root)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .status()
                .unwrap()
                .success();
            assert!(ok, "git {args:?}");
        };
        run(&["init", "-q", "-b", "main"]);
        std::fs::write(root.join("a.txt"), "a").unwrap();
        run(&["add", "-A"]);
        run(&["commit", "-q", "-m", "feat: one", "-m", "Why it matters."]);
        let rows = collect_commits(&target(root)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "feat: one");
        assert_eq!(rows[0].body, "feat: one\nWhy it matters.");
        assert_eq!(rows[0].reference.len(), 40);
        assert!(collect_commits(&target(&root.join("missing"))).is_none());
        assert!(
            collect_issues(&target(root), 0).is_none(),
            "no remote, no issues"
        );
    }
}
