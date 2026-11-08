//! The pass or fail check of minutes, and the resolvers of sources. The rules
//! copy `packages/core/src/check.ts`. The check runs `git` and `gh`, and writes nothing.

use crate::knowledge::exec;
use crate::knowledge::repos::repo_targets;
use crate::registry::{Product, normalize};
use crate::roster::{js_text, parse_agent_file, read_overlays, resolve_under_roots};
use agentsi_proto::meeting_api::MinutesCheckResult;
use agentsi_store::read;
use regex::Regex;
use rusqlite::Connection;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

const GH_TIMEOUT: Duration = Duration::from_secs(25);

static NORTH_STAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(CLAUDE|CONTEXT|PRODUCT|DESIGN|README)\.md$").unwrap());
static COMMIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[0-9a-f]{4,40}$").unwrap());
static ISSUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:(.+?)[#/])?#?(\d+)$").unwrap());
static LINE_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(#L\d+(-L?\d+)?|:\d+(-\d+)?)$").unwrap());

/// What the resolvers of one product read.
pub struct SourceContext {
    pub product: Product,
    pub roots: Vec<String>,
    pub repos: Vec<String>,
    /// The absolute paths that the brief declares as north stars. When set, only these count.
    pub north_stars: Option<Vec<PathBuf>>,
    /// The id and the title of each story.
    pub stories: Vec<(String, String)>,
}

/// `path.relative` on two absolute paths.
fn relative(from: &Path, to: &Path) -> String {
    let (from, to) = (normalize(from), normalize(to));
    let (a, b): (Vec<_>, Vec<_>) = (from.components().collect(), to.components().collect());
    let shared = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut parts: Vec<String> = vec!["..".into(); a.len() - shared];
    parts.extend(
        b[shared..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}

fn inside(root: &Path, full: &Path) -> bool {
    !relative(root, full).starts_with("..")
}

pub fn source_context(c: &Connection, product: &Product) -> SourceContext {
    let roots: Vec<String> = product
        .paths
        .iter()
        .filter(|p| Path::new(p).exists())
        .cloned()
        .collect();
    let mut repos: Vec<String> = repo_targets(product).into_iter().map(|t| t.path).collect();
    for root in &roots {
        if !repos.contains(root) {
            repos.push(root.clone());
        }
    }
    // A database that cannot answer leaves no story to resolve, as a product with no stories does.
    let stories = read::stories(c, &product.id)
        .unwrap_or_default()
        .iter()
        .filter_map(|s| {
            let id = s.get("id")?.as_str()?.to_string();
            Some((id, js_text(s.get("title"))))
        })
        .collect();
    SourceContext {
        product: product.clone(),
        roots,
        repos,
        north_stars: None,
        stories,
    }
}

fn resolve_commit(reference: &str, repos: &[String]) -> bool {
    let object = format!("{reference}^{{commit}}");
    COMMIT.is_match(reference)
        && repos
            .iter()
            .any(|repo| exec::git(Path::new(repo), &["cat-file", "-e", &object]).ok)
}

fn resolve_issue(reference: &str, ctx: &SourceContext) -> bool {
    let Some(m) = ISSUE.captures(reference.trim()) else {
        return false;
    };
    let number = &m[2];
    let candidates: Vec<String> = match m.get(1) {
        Some(prefix) => repo_targets(&ctx.product)
            .into_iter()
            .filter(|t| t.prefix == prefix.as_str())
            .map(|t| t.path)
            .collect(),
        None => ctx.repos.clone(),
    };
    candidates.iter().any(|repo| {
        exec::run(
            "gh",
            &["issue", "view", number, "--json", "number"],
            Path::new(repo),
            GH_TIMEOUT,
        )
        .ok
    })
}

fn resolve_path(raw: &str, roots: &[String]) -> Option<PathBuf> {
    let reference = LINE_SUFFIX.replace(raw, "");
    let path = Path::new(reference.as_ref());
    if path.is_absolute() {
        let known = path.exists() && roots.iter().any(|r| inside(Path::new(r), path));
        return known.then(|| path.to_path_buf());
    }
    roots.iter().find_map(|root| {
        let full = normalize(&Path::new(root).join(path));
        (inside(Path::new(root), &full) && full.exists()).then_some(full)
    })
}

fn is_north_star(full: &Path, roots: &[String]) -> bool {
    roots.iter().any(|root| {
        let rel = relative(Path::new(root), full);
        if rel.starts_with("..") {
            return false;
        }
        let parts: Vec<&str> = rel.split('/').collect();
        (parts.len() == 1 && NORTH_STAR.is_match(parts[0]))
            || (parts[0] == "docs" && parts.len() > 1)
    })
}

/// `new URL(ref)` with the protocol `http:` or `https:`.
fn is_http_url(reference: &str) -> bool {
    url::Url::parse(reference).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

/// Resolves one source: `(it resolves, it is a north star)`.
fn resolve_detailed(source: &Value, ctx: &SourceContext) -> (bool, bool) {
    let reference = source.get("ref").and_then(Value::as_str).unwrap_or("");
    match source.get("type").and_then(Value::as_str) {
        Some("commit") => (resolve_commit(reference, &ctx.repos), false),
        Some("issue") => (resolve_issue(reference, ctx), false),
        Some("url") => (is_http_url(reference), false),
        Some("story") => (ctx.stories.iter().any(|(id, _)| id == reference), false),
        Some("path") => {
            let full = resolve_path(reference, &ctx.roots);
            let north_star = full.as_deref().is_some_and(|f| match &ctx.north_stars {
                Some(declared) => declared.iter().any(|d| inside(d, f)),
                None => is_north_star(f, &ctx.roots),
            });
            (full.is_some(), north_star)
        }
        _ => (false, false),
    }
}

pub fn resolve_source(source: &Value, ctx: &SourceContext) -> bool {
    resolve_detailed(source, ctx).0
}

/// The role of an agent: the `extends` of its overlay, else the part before the dot.
fn agent_role(id: &str, overlays: &std::collections::BTreeMap<String, String>) -> String {
    overlays
        .get(id)
        .map(|text| parse_agent_file(text).0)
        .and_then(|fm| fm.get("extends").filter(|p| !p.is_empty()).cloned())
        .unwrap_or_else(|| id.split('.').next().unwrap_or(id).to_string())
}

fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    array(v, key).iter().map(|s| js_text(Some(s))).collect()
}

fn blank(v: Option<&Value>) -> bool {
    v.and_then(Value::as_str)
        .is_none_or(|s| s.trim().is_empty())
}

/// `check()`. `roles` are the agent ids that must answer, by default the roster.
/// `brief` is the brief of the product, or `None` when it has none.
pub fn check(
    c: &Connection,
    m: &Value,
    product: &Product,
    roles: Option<&[String]>,
    brief: Option<&Value>,
) -> MinutesCheckResult {
    let mut ctx = source_context(c, product);
    if let Some(brief) = brief {
        ctx.north_stars = Some(
            strings(brief, "north_stars")
                .iter()
                .filter_map(|p| resolve_under_roots(p, &product.paths))
                .collect(),
        );
    }
    let mut failures = Vec::new();
    let mut north_star = false;
    let priorities = array(m, "priorities");
    if priorities.is_empty() {
        failures.push("No priorities.".to_string());
    }

    for (i, p) in priorities.iter().enumerate() {
        let title = js_text(p.get("title"));
        if let Some(brief) = brief {
            let objectives = array(brief, "objectives");
            let objective = p.get("objective").filter(|o| !o.is_null());
            let known =
                objective.is_some_and(|o| objectives.iter().any(|b| b.get("id") == Some(o)));
            if !known {
                let named = match objective.filter(|o| *o != "") {
                    Some(o) => format!("\"{}\"", js_text(Some(o))),
                    None => "is missing and".to_string(),
                };
                let ids: Vec<String> = objectives.iter().map(|o| js_text(o.get("id"))).collect();
                failures.push(format!(
                    "Priority #{i} \"{title}\" objective {named} must be one of the brief ids: {}.",
                    ids.join(", ")
                ));
            }
        }
        let sources = array(p, "sources");
        if sources.is_empty() {
            failures.push(format!("Priority #{i} \"{title}\" has no source."));
            continue;
        }
        for s in sources {
            let (ok, star) = resolve_detailed(s, &ctx);
            north_star |= star;
            if !ok {
                failures.push(format!(
                    "Priority #{i}: {} source \"{}\" does not resolve.",
                    js_text(s.get("type")),
                    js_text(s.get("ref"))
                ));
            }
        }
    }

    for (i, d) in array(m, "decisions").iter().enumerate() {
        for idx in array(d, "priorities") {
            let exists = idx
                .as_f64()
                .is_some_and(|n| n.fract() == 0.0 && n >= 0.0 && (n as usize) < priorities.len());
            if !exists {
                failures.push(format!(
                    "Decision #{i} \"{}\" links priority index {}, which does not exist (0 to {}).",
                    js_text(d.get("text")),
                    js_text(Some(idx)),
                    priorities.len() as i64 - 1
                ));
            }
        }
    }

    let declared = brief.map(|b| strings(b, "north_stars"));
    match &declared {
        Some(list) if !north_star && !list.is_empty() => failures.push(format!(
            "No priority cites a north star declared in the brief ({}).",
            list.join(", ")
        )),
        None if !north_star => failures.push(
            "No priority cites a north-star or guideline doc (CLAUDE/CONTEXT/PRODUCT/DESIGN/README.md or docs/**)."
                .to_string(),
        ),
        _ => {}
    }

    // A database that cannot answer leaves no overlay, so the role is the part of the id before the dot.
    let overlays = read_overlays(c, &product.id).unwrap_or_default();
    let mut by_role: Vec<(String, Vec<&String>)> = Vec::new();
    for id in roles.unwrap_or(&product.roles) {
        let role = agent_role(id, &overlays);
        match by_role.iter_mut().find(|(r, _)| *r == role) {
            Some((_, agents)) => agents.push(id),
            None => by_role.push((role, vec![id])),
        }
    }
    for (role, agents) in &by_role {
        let answered = agents.iter().any(|id| {
            let answer = m.get("roles").and_then(|r| r.get(id.as_str()));
            !blank(answer.and_then(|a| a.get("answer")))
        });
        if answered {
            continue;
        }
        let named = if agents.len() == 1 && agents[0] == role {
            String::new()
        } else {
            let ids: Vec<&str> = agents.iter().map(|a| a.as_str()).collect();
            format!(" (agents: {})", ids.join(", "))
        };
        failures.push(format!("Role \"{role}\" has no answer{named}."));
    }

    let script = array(m, "script");
    if script.iter().all(|s| blank(s.get("text"))) {
        failures.push("Call script is empty.".to_string());
    }
    for (id, title) in &ctx.stories {
        // `\b` of JavaScript knows ASCII words only.
        let Ok(word) = Regex::new(&format!(r"(?-u:\b){}(?-u:\b)", regex::escape(id))) else {
            continue;
        };
        let named = script.iter().any(|s| {
            s.get("text")
                .and_then(Value::as_str)
                .is_some_and(|t| word.is_match(t))
        });
        if named {
            failures.push(format!(
                "Call script names story {id} by its id: say its title \"{title}\"."
            ));
        }
    }

    MinutesCheckResult {
        passed: failures.is_empty(),
        failures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let conn = agentsi_store::db::open_writer(&dir.path().join("agentsi.db")).unwrap();
        (dir, conn)
    }

    fn product(root: &Path, roles: &[&str]) -> Product {
        Product {
            id: "p".into(),
            name: "p".into(),
            paths: vec![root.to_string_lossy().into_owned()],
            status: "active".into(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
        }
    }

    fn folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("docs")).unwrap();
        std::fs::write(dir.path().join("docs/vision.md"), "v").unwrap();
        std::fs::write(dir.path().join("README.md"), "r").unwrap();
        std::fs::write(dir.path().join("notes.md"), "n").unwrap();
        dir
    }

    fn minutes(source: Value) -> Value {
        json!({
            "id": "m", "product": "p",
            "roles": {"dev": {"answer": "Yes."}},
            "priorities": [{"title": "T", "why": "W", "sources": [source], "owner_role": "dev"}],
            "decisions": [],
            "script": [{"role": "dev", "text": "Hello."}],
        })
    }

    #[test]
    fn a_good_minutes_passes_with_a_line_suffix_on_its_path() {
        let (_d, c) = db();
        let dir = folder();
        let m = minutes(json!({"type": "path", "ref": "docs/vision.md:12"}));
        let r = check(&c, &m, &product(dir.path(), &["dev"]), None, None);
        assert_eq!(r.failures, Vec::<String>::new());
        assert!(r.passed);
    }

    #[test]
    fn a_file_outside_the_guidelines_is_no_north_star() {
        let (_d, c) = db();
        let dir = folder();
        let m = minutes(json!({"type": "path", "ref": "notes.md"}));
        let r = check(&c, &m, &product(dir.path(), &["dev"]), None, None);
        assert_eq!(
            r.failures,
            [
                "No priority cites a north-star or guideline doc (CLAUDE/CONTEXT/PRODUCT/DESIGN/README.md or docs/**)."
            ]
        );
    }

    #[test]
    fn a_path_that_leaves_the_root_does_not_resolve() {
        let (_d, c) = db();
        let dir = folder();
        let m = minutes(json!({"type": "path", "ref": "../outside.md"}));
        let r = check(&c, &m, &product(dir.path(), &["dev"]), None, None);
        assert!(r.failures[0].contains("does not resolve"));
    }

    #[test]
    fn with_a_brief_only_the_declared_north_stars_count_and_the_objective_is_named() {
        let (_d, c) = db();
        let dir = folder();
        let brief =
            json!({"objectives": [{"id": "o1"}, {"id": "o2"}], "north_stars": ["docs/vision.md"]});
        let mut m = minutes(json!({"type": "path", "ref": "README.md"}));
        let r = check(&c, &m, &product(dir.path(), &["dev"]), None, Some(&brief));
        assert_eq!(
            r.failures,
            [
                "Priority #0 \"T\" objective is missing and must be one of the brief ids: o1, o2.",
                "No priority cites a north star declared in the brief (docs/vision.md)."
            ]
        );
        m["priorities"][0]["objective"] = json!("o9");
        m["priorities"][0]["sources"][0]["ref"] = json!("docs/vision.md");
        let r = check(&c, &m, &product(dir.path(), &["dev"]), None, Some(&brief));
        assert_eq!(
            r.failures,
            ["Priority #0 \"T\" objective \"o9\" must be one of the brief ids: o1, o2."]
        );
    }

    #[test]
    fn a_role_passes_when_one_of_its_agents_answers() {
        let (_d, c) = db();
        let dir = folder();
        let mut m = minutes(json!({"type": "path", "ref": "README.md"}));
        m["roles"] = json!({"dev.api": {"answer": "Yes."}, "dev.web": {"answer": " "}, "qa": {"answer": ""}});
        let r = check(
            &c,
            &m,
            &product(dir.path(), &["dev.api", "dev.web", "qa.load", "pm"]),
            None,
            None,
        );
        assert_eq!(
            r.failures,
            [
                "Role \"qa\" has no answer (agents: qa.load).",
                "Role \"pm\" has no answer."
            ]
        );
    }

    #[test]
    fn a_decision_link_past_the_priorities_and_an_empty_script_fail() {
        let (_d, c) = db();
        let dir = folder();
        let mut m = minutes(json!({"type": "path", "ref": "README.md"}));
        m["decisions"] = json!([{"text": "D", "why": "W", "priorities": [0, 3]}]);
        m["script"] = json!([{"role": "dev", "text": " "}]);
        let r = check(&c, &m, &product(dir.path(), &["dev"]), None, None);
        assert_eq!(
            r.failures,
            [
                "Decision #0 \"D\" links priority index 3, which does not exist (0 to 0).",
                "Call script is empty."
            ]
        );
    }

    #[test]
    fn urls_follow_the_url_parser_of_javascript() {
        assert!(is_http_url("https://example.com/a"));
        assert!(is_http_url("HTTP://example.com"));
        assert!(!is_http_url("ftp://example.com"));
        assert!(!is_http_url("example.com"));
        assert!(!is_http_url("http://"));
    }

    #[test]
    fn relative_matches_path_relative() {
        let rel = |a: &str, b: &str| relative(Path::new(a), Path::new(b));
        assert_eq!(rel("/a/b", "/a/b/c/d.md"), "c/d.md");
        assert_eq!(rel("/a/b", "/a/c"), "../c");
        assert_eq!(rel("/a/b", "/a/b"), "");
    }

    #[test]
    fn a_story_id_in_the_script_fails_only_as_a_whole_word() {
        let (_d, c) = db();
        let dir = folder();
        c.execute(
            "insert into stories (product, id, data) values ('p', 'S001', '{\"id\":\"S001\",\"title\":\"Sort mail\"}')",
            [],
        )
        .unwrap();
        let mut m = minutes(json!({"type": "story", "ref": "S001"}));
        m["priorities"][0]["sources"] =
            json!([{"type": "story", "ref": "S001"}, {"type": "path", "ref": "README.md"}]);
        m["script"] = json!([{"role": "dev", "text": "Story S0012 is not it."}]);
        let r = check(&c, &m, &product(dir.path(), &["dev"]), None, None);
        assert!(r.passed, "{:?}", r.failures);
        m["script"] = json!([{"role": "dev", "text": "We do S001 next."}]);
        let r = check(&c, &m, &product(dir.path(), &["dev"]), None, None);
        assert_eq!(
            r.failures,
            ["Call script names story S001 by its id: say its title \"Sort mail\"."]
        );
    }
}
