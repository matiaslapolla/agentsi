//! `data.snapshot`: the Markdown context of a meeting. The sections, their
//! order and the 60 KiB limit copy `packages/core/src/snapshot.ts`.
//! It reads the repos of the product with `git`, as the TypeScript code does.

use crate::error::Result;
use crate::registry::Product;
use crate::roster::resolve_under_roots;
use agentsi_store::read;
use regex::Regex;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

const MAX_BYTES: usize = 60 * 1024;
const MAX_COMMITS: usize = 40;
const NORTH_STAR: [&str; 4] = ["PRODUCT.md", "CLAUDE.md", "CONTEXT.md", "README.md"];
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
const GROUP_ORDER: [&str; 4] = ["docs/ideas", "docs/specs", "docs/plans", "docs/adr"];

/// Commits that AgentSI makes on its own workspace: they repeat what the
/// minutes and memory sections already say.
static BOOKKEEPING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9_]+ \S+ (minutes|memory|reflect|roles)\([A-Za-z0-9_.-]+\): ").unwrap()
});
static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^#\s+(.+)$").unwrap());

/// A git repo of the product.
struct Target {
    path: PathBuf,
    scope: Option<String>,
}

impl Target {
    fn label(&self) -> String {
        self.scope
            .clone()
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }
}

fn targets(product: &Product, scope: Option<&str>) -> Vec<Target> {
    product
        .paths
        .iter()
        .map(PathBuf::from)
        .filter(|root| root.join(".git").exists())
        .map(|root| Target {
            path: root,
            scope: None,
        })
        .filter(|t| scope.is_none() || t.scope.as_deref() == scope)
        .collect()
}

/// The text of a file. A byte that is not UTF-8 becomes U+FFFD, as `readFileSync(file, "utf8")` does.
fn read_text(file: &Path) -> Option<String> {
    std::fs::read(file)
        .ok()
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

/// The first `lines` lines of a file, with no trailing space. `None` when the file does not exist.
fn head(file: &Path, lines: usize) -> Option<String> {
    let text = read_text(file)?;
    Some(
        text.split('\n')
            .take(lines)
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_string(),
    )
}

/// The last `lines` lines of a file.
fn tail(file: &Path, lines: usize) -> Option<String> {
    let text = read_text(file)?;
    let all: Vec<&str> = text.trim_end().split('\n').collect();
    Some(all[all.len().saturating_sub(lines)..].join("\n"))
}

/// A text cut to `n` UTF-16 units, as `slice(0, n)` does, without a split character.
fn cut(text: &str, n: usize) -> String {
    let mut units = 0;
    text.chars()
        .take_while(|c| {
            units += c.len_utf16();
            units <= n
        })
        .collect()
}

fn north_star(product: &Product, targets: &[Target], brief: Option<&Value>) -> String {
    let lines = if targets.len() > 1 { 25 } else { 80 };
    let mut out = vec!["## North star and guidelines".to_string()];
    let mut shown = BTreeSet::new();
    let declared = brief
        .and_then(|b| b.get("north_stars"))
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for reference in declared.iter().filter_map(Value::as_str) {
        let Some(full) = resolve_under_roots(reference, &product.paths) else {
            continue;
        };
        let text = if full.is_file() {
            head(&full, lines)
        } else {
            None
        };
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            out.extend([
                String::new(),
                format!("### {reference} (declared in the brief)"),
                String::new(),
                text,
            ]);
            shown.insert(full);
        }
    }
    for t in targets {
        for f in NORTH_STAR {
            let file = t.path.join(f);
            if shown.contains(&file) {
                continue;
            }
            if let Some(text) = head(&file, lines).filter(|t| !t.is_empty()) {
                let label = t
                    .scope
                    .as_ref()
                    .map(|s| format!("{s}: "))
                    .unwrap_or_default();
                out.extend([
                    String::new(),
                    format!("### {label}{f}"),
                    String::new(),
                    text,
                ]);
            }
        }
    }
    out.join("\n")
}

fn walk_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            walk_markdown(&path, out);
        } else if kind.is_file() && name.ends_with(".md") {
            out.push(path);
        }
    }
}

/// The documents of a repo that the index holds as `doc`: ref and title.
fn collect_docs(t: &Target) -> Vec<(String, String)> {
    let mut files: Vec<PathBuf> = ROOT_DOCS.iter().map(|f| t.path.join(f)).collect();
    walk_markdown(&t.path.join("docs"), &mut files);
    let mut docs = Vec::new();
    for file in files {
        let Ok(meta) = std::fs::metadata(&file) else {
            continue;
        };
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Some(text) = read_text(&file) else {
            continue;
        };
        let rel = file
            .strip_prefix(&t.path)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let title = HEADING
            .captures(&text)
            .map_or_else(|| rel.clone(), |m| m[1].trim().to_string());
        docs.push((rel, title));
    }
    docs
}

/// An approximation of `localeCompare` for repo paths: the letters compare
/// without case first, and a lowercase letter comes before its uppercase.
fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| b.cmp(a))
}

fn docs_index(targets: &[Target]) -> String {
    let mut out = vec!["## Docs index".to_string()];
    for t in targets {
        let mut docs = collect_docs(t);
        docs.sort_by(|a, b| {
            let group = |r: &str| {
                GROUP_ORDER
                    .iter()
                    .position(|g| r.strip_prefix(g).is_some_and(|rest| rest.starts_with('/')))
                    .unwrap_or(99)
            };
            group(&a.0)
                .cmp(&group(&b.0))
                .then_with(|| locale_compare(&a.0, &b.0))
        });
        out.push(String::new());
        if targets.len() > 1 {
            out.extend([format!("### {}", t.label()), String::new()]);
        }
        out.extend(
            docs.iter()
                .map(|(r, title)| format!("- {r} — {}", cut(title, 120))),
        );
    }
    out.join("\n")
}

fn journal(targets: &[Target]) -> String {
    let mut out = vec!["## Journal tail".to_string()];
    for t in targets {
        if let Some(text) = tail(&t.path.join(".claude/JOURNAL.md"), 80).filter(|t| !t.is_empty()) {
            out.push(String::new());
            if targets.len() > 1 {
                out.extend([format!("### {}", t.label()), String::new()]);
            }
            out.push(text);
        }
    }
    if out.len() > 1 {
        out.join("\n")
    } else {
        "## Journal tail\n\n_No journal._".into()
    }
}

/// `git` gets 20 seconds, as `exec.ts` gives it. After that the section is empty.
const GIT_TIMEOUT: Duration = Duration::from_secs(20);

fn wait_with_timeout(mut child: Child) -> std::io::Result<Output> {
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let deadline = Instant::now() + GIT_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "git timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    Ok(Output {
        status,
        stdout: reader.join().unwrap_or_default(),
        stderr: Vec::new(),
    })
}

fn git_log(path: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args([
            "log",
            "--branches",
            "--no-merges",
            "--since=14.days",
            "--format=%h %as %s",
            "-n",
            "80",
        ])
        .current_dir(path)
        .env("GIT_TERMINAL_PROMPT", "0")
        .envs(crate::git_env::env_for_cwd(path))
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .and_then(wait_with_timeout);
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .trim()
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
        _ => Vec::new(),
    }
}

fn commits(targets: &[Target]) -> String {
    let mut out = vec!["## Commits, last 14 days".to_string()];
    for t in targets {
        let lines: Vec<String> = git_log(&t.path)
            .into_iter()
            .filter(|l| !BOOKKEEPING.is_match(l))
            .take(MAX_COMMITS)
            .collect();
        out.push(String::new());
        if targets.len() > 1 {
            out.push(format!("### {} ({})", t.label(), lines.len()));
        } else {
            out.push(format!("{} commits", lines.len()));
        }
        out.push(String::new());
        out.extend(lines.iter().map(|l| format!("- {}", cut(l, 120))));
    }
    out.join("\n")
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// The open stories and a count of the closed ones. `None` when the product has none.
fn stories_section(c: &Connection, product: &str) -> Result<Option<String>> {
    let hidden: BTreeSet<String> = read::archived(c, product)?
        .into_iter()
        .filter(|a| a.kind == "story")
        .map(|a| a.reference)
        .collect();
    let number = |s: &Value| {
        str_of(s, "id")
            .and_then(|id| id.get(1..))
            .and_then(|n| n.parse::<f64>().ok())
            .unwrap_or(f64::NAN)
    };
    let mut stories: Vec<Value> = read::stories(c, product)?
        .into_iter()
        .filter(|s| !str_of(s, "id").is_some_and(|id| hidden.contains(id)))
        .collect();
    if stories.is_empty() {
        return Ok(None);
    }
    stories.sort_by(|a, b| {
        number(a)
            .partial_cmp(&number(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let status = |s: &Value| str_of(s, "status").unwrap_or_default().to_string();
    let count = |wanted: &str| stories.iter().filter(|s| status(s) == wanted).count();
    let open: Vec<&Value> = stories
        .iter()
        .filter(|s| status(s) == "not_implemented")
        .collect();
    let mut out = vec!["## Stories".to_string(), String::new()];
    if open.is_empty() {
        out.push("_No open stories._".into());
    }
    for s in open {
        out.push(format!(
            "- {} {} — {}, approval {}, session {}",
            str_of(s, "id").unwrap_or_default(),
            str_of(s, "title").unwrap_or_default(),
            status(s),
            str_of(s, "approval").unwrap_or_default(),
            s.get("session")
                .and_then(|x| str_of(x, "state"))
                .unwrap_or("none"),
        ));
    }
    out.push(String::new());
    out.push(format!(
        "Implemented: {}. Discarded: {}.",
        count("implemented"),
        count("discarded")
    ));
    Ok(Some(out.join("\n")))
}

fn mark_box(mark: Option<&str>) -> &'static str {
    match mark {
        Some("accepted") => "[x]",
        Some("changed") => "[~]",
        Some("rejected") => "[-]",
        _ => "[ ]",
    }
}

fn db_sections_inner(c: &Connection, product: &str, scope: Option<&str>) -> Result<Vec<String>> {
    let mut sections = Vec::new();

    let mut stmt = c.prepare_cached(
        "select ref, title from documents where product = ?1 and kind = 'issue' and (?2 is null or scope = ?2)
         order by updated_at desc, ref limit 50",
    )?;
    let issues: Vec<(String, Option<String>)> = stmt
        .query_map(rusqlite::params![product, scope], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let mut out = vec!["## Open issues".to_string(), String::new()];
    if issues.is_empty() {
        out.push("_None indexed._".into());
    }
    out.extend(
        issues
            .iter()
            .map(|(r, t)| format!("- #{r} {}", t.as_deref().unwrap_or("null"))),
    );
    sections.push(out.join("\n"));

    let mut stmt = c.prepare_cached(
        "select id, question from minutes where product = ?1 order by created_at desc limit 3",
    )?;
    let recent: Vec<(String, Option<String>)> = stmt
        .query_map([product], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut out = vec!["## Last minutes, priorities and marks".to_string()];
    for (id, question) in &recent {
        out.extend([
            String::new(),
            format!("### {id} — {}", question.as_deref().unwrap_or("null")),
            String::new(),
        ]);
        for p in read::priorities(c, id)? {
            out.push(format!(
                "- #{} {} {}{}",
                p.idx,
                mark_box(p.mark.as_deref()),
                p.title.as_deref().unwrap_or("null"),
                if p.is_new == Some(true) { " (new)" } else { "" }
            ));
        }
    }
    if recent.is_empty() {
        out.extend([String::new(), "_No meetings yet._".into()]);
    }
    sections.push(out.join("\n"));

    let pending = read::decisions(c, product, Some("pending"))?;
    let mut out = vec!["## Pending decisions".to_string(), String::new()];
    if pending.is_empty() {
        out.push("_None._".into());
    }
    for d in &pending {
        out.push(format!(
            "- [{}] {}{}",
            d.id,
            d.text.as_deref().unwrap_or("null"),
            d.action
                .as_ref()
                .filter(|a| !a.is_empty())
                .map(|a| format!(" → {a}"))
                .unwrap_or_default()
        ));
    }
    sections.push(out.join("\n"));
    Ok(sections)
}

pub fn snapshot(c: &Connection, product: &Product, scope: Option<&str>) -> Result<String> {
    let targets = targets(product, scope);
    let paths = if targets.is_empty() {
        product.paths.clone()
    } else {
        targets
            .iter()
            .map(|t| t.path.to_string_lossy().into_owned())
            .collect()
    };
    let header = [
        format!(
            "# Snapshot: {}{}",
            product.name,
            scope.map(|s| format!(" / {s}")).unwrap_or_default()
        ),
        String::new(),
        format!("- Product: {} ({})", product.id, product.status),
        format!("- Active roles: {}", product.roles.join(", ")),
        format!("- Paths: {}", paths.join(", ")),
    ]
    .join("\n");

    let brief = read::brief(c, &product.id).ok().flatten();
    let mut sections = vec![header, north_star(product, &targets, brief.as_ref())];
    // A failed stories read leaves the section out, as the TypeScript code does.
    if let Ok(Some(stories)) = stories_section(c, &product.id) {
        sections.push(stories);
    }
    match db_sections_inner(c, &product.id, scope) {
        Ok(db) => sections.extend(db),
        Err(e) => sections.push(format!("## Database sections unavailable\n\n{e}")),
    }
    sections.extend([commits(&targets), journal(&targets), docs_index(&targets)]);

    let mut budget = MAX_BYTES;
    let mut kept = Vec::new();
    for s in sections {
        let size = s.len() + 2;
        if size <= budget {
            kept.push(s);
            budget -= size;
        } else {
            let end = budget.saturating_sub(40).min(s.len());
            let cut = String::from_utf8_lossy(&s.as_bytes()[..end]).into_owned();
            kept.push(format!(
                "{cut}\n\n_[truncated to fit {} KB]_",
                MAX_BYTES / 1024
            ));
            break;
        }
    }
    Ok(kept.join("\n\n") + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bookkeeping_commits_of_the_workspace_are_left_out() {
        assert!(BOOKKEEPING.is_match("abc1234 2026-10-01 minutes(alpha-001): add the minutes"));
        assert!(BOOKKEEPING.is_match("abc1234 2026-10-01 reflect(alpha): note"));
        assert!(!BOOKKEEPING.is_match("abc1234 2026-10-01 feat(minutes): a real change"));
        assert!(!BOOKKEEPING.is_match("abc1234 2026-10-01 minutes: no scope"));
    }

    #[test]
    fn the_docs_come_in_the_order_of_their_groups_then_by_name() {
        assert_eq!(
            locale_compare("docs/specs/a.md", "docs/specs/B.md"),
            std::cmp::Ordering::Less
        );
        assert_eq!(locale_compare("a.md", "a.md"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn a_cut_text_keeps_whole_characters() {
        assert_eq!(cut("héllo", 2), "hé");
        // An emoji is two UTF-16 units, as in JavaScript.
        assert_eq!(cut("a😀b", 2), "a");
        assert_eq!(cut("a😀b", 3), "a😀");
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("latin1.md");
        std::fs::write(&file, b"# caf\xe9\n").unwrap();
        assert_eq!(head(&file, 5).unwrap(), "# caf\u{fffd}");
        assert_eq!(tail(Path::new("/does/not/exist"), 3), None);
    }
}
