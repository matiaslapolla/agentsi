//! `story-propose.ts`: the PM shapes the owner's text into one story
//! (`story new`), proposes up to 3 stories (`story suggest`) or rewrites a
//! story with a change note (`story changes`). One model call, repaired once
//! with its failures. A second failure writes nothing.

use super::claude::{Call, run_call};
use super::lifecycle::request_changes;
use super::record::{self, validate};
use super::store::{self, read_with, transaction};
use super::{Error, Result, inputs};
use crate::clock;
use crate::registry::{Product, normalize};
use agentsi_proto::agency_api::Owner;
use agentsi_proto::story_api::{
    Criterion, Source, Story, StoryConfig, StoryProposeResult, StoryUsage,
};
use agentsi_store::Store;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

const PM_TOOLS: [&str; 3] = ["Read", "Grep", "Glob"];
const PM_TIMEOUT: Duration = Duration::from_secs(8 * 60);
const MAX_SUGGESTIONS: usize = 3;
const SNAPSHOT_MAX_CHARS: usize = 14_000;

/// The PM call. The core uses `run_call`; a test passes a fake.
pub type PmCaller =
    dyn Fn(&Call, &str) -> std::result::Result<(Value, StoryUsage), String> + Send + Sync;

pub struct ProposeCtx {
    pub store: Arc<Store>,
    pub root: PathBuf,
    pub product: Product,
    pub config: StoryConfig,
    pub owner: Owner,
    pub caller: Arc<PmCaller>,
    /// The snapshot text. Tests pass a fixed one; `None` reads the product.
    pub snapshot: Option<String>,
}

impl ProposeCtx {
    pub fn new(
        store: Arc<Store>,
        root: PathBuf,
        product: Product,
        config: StoryConfig,
        owner: Owner,
    ) -> ProposeCtx {
        ProposeCtx {
            store,
            root,
            product,
            config,
            owner,
            caller: Arc::new(run_call),
            snapshot: None,
        }
    }
}

/// What the model writes. Code sets the id, the slug, the status, the
/// approval, `proposed_by` and the times.
#[derive(Debug, Clone, Deserialize)]
struct Content {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    as_a: Option<String>,
    #[serde(default)]
    i_want: String,
    #[serde(default)]
    so_that: String,
    #[serde(default)]
    criteria: Vec<Criterion>,
    #[serde(default)]
    owner_role: String,
    #[serde(default)]
    objective: Option<String>,
    #[serde(default)]
    sources: Vec<Source>,
}

pub fn content_schema(roster: &[String], objectives: Option<&[String]>) -> Value {
    let source = json!({
        "type": "object",
        "properties": {
            "type": { "type": "string", "enum": ["commit", "issue", "path", "url", "story"] },
            "ref": { "type": "string" },
        },
        "required": ["type", "ref"],
    });
    let criterion = json!({
        "type": "object",
        "properties": {
            "given": { "type": "string" },
            "when": { "type": "string" },
            "then": { "type": "string" },
            "test": { "type": ["string", "null"] },
            "checks": { "type": "string" },
        },
        "required": ["given", "when", "then", "test", "checks"],
    });
    let objective = match objectives {
        Some(ids) => {
            let mut e: Vec<Value> = ids.iter().map(|i| json!(i)).collect();
            e.push(Value::Null);
            json!({ "enum": e })
        }
        None => json!({ "type": "null" }),
    };
    json!({
        "type": "object",
        "properties": {
            "stories": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "type": { "type": "string", "enum": ["story", "task"] },
                        "title": { "type": "string" },
                        "as_a": { "type": ["string", "null"] },
                        "i_want": { "type": "string" },
                        "so_that": { "type": "string" },
                        "criteria": { "type": "array", "items": criterion },
                        "owner_role": { "type": "string", "enum": roster },
                        "objective": objective,
                        "sources": { "type": "array", "items": source },
                    },
                    "required": ["type", "title", "as_a", "i_want", "so_that", "criteria", "owner_role", "objective", "sources"],
                },
            },
        },
        "required": ["stories"],
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    New,
    Suggest,
    Changes,
}

struct Request {
    mode: Mode,
    text: String,
    kind: Option<String>,
    current: Option<Story>,
}

fn open_stories(stories: &[Story]) -> String {
    let lines: Vec<String> = stories
        .iter()
        .filter(|s| s.status == "not_implemented")
        .map(|s| {
            format!(
                "- {} \"{}\" ({}, owner {}, approval {}, session {})",
                s.id,
                s.title,
                s.kind,
                s.owner_role,
                s.approval,
                s.session.as_ref().map_or("none", |x| x.state.as_str())
            )
        })
        .collect();
    if lines.is_empty() {
        "_None._".into()
    } else {
        lines.join("\n")
    }
}

/// The content of a story, in the key order of the TypeScript `storyContent`.
fn content_json(s: &Story) -> Value {
    json!({
        "type": s.kind,
        "title": s.title,
        "as_a": s.as_a,
        "i_want": s.i_want,
        "so_that": s.so_that,
        "criteria": s.criteria,
        "owner_role": s.owner_role,
        "objective": s.objective,
        "sources": s.sources,
    })
}

struct Prompt<'a> {
    request: &'a Request,
    config: &'a StoryConfig,
    roster: &'a [String],
    snapshot: &'a str,
    memory: &'a str,
    brief: Option<&'a Value>,
    stories: &'a [Story],
    owner: &'a Owner,
    repair: Option<(&'a Value, &'a [String])>,
}

fn prompt(o: &Prompt) -> String {
    let r = o.request;
    let owner = o.owner;
    let task = match r.mode {
        Mode::New => format!(
            "A story request from {}. Shape the text into exactly one story:\n\n{}",
            owner.name, r.text
        ),
        Mode::Suggest => format!(
            "Propose up to {MAX_SUGGESTIONS} stories from the snapshot, for any role of the roster. Fewer is better than weak ones. Skip anything an open story already covers."
        ),
        Mode::Changes => format!(
            "Changes requested by {} for the story below. Rewrite that one story with the note applied. Keep what the note does not touch.\n\nNote: {}\n\nCurrent story:\n{}",
            owner.name,
            r.text,
            r.current.as_ref().map(content_json).unwrap_or_default()
        ),
    };
    let rules = [
        format!(
            "- Write the story content only. Return it in \"stories\": {}.",
            if r.mode == Mode::Suggest {
                format!("0 to {MAX_SUGGESTIONS} items")
            } else {
                "exactly 1 item".into()
            }
        ),
        match &r.kind {
            Some(k) => format!("- The type is \"{k}\"."),
            None => "- type is \"story\" when a user sees the change, \"task\" for debt, infra or a refactor. Prefer \"task\" for those.".into(),
        },
        "- A story has as_a (the user), i_want and so_that. A task has as_a null.".into(),
        format!(
            "- Everything the user reads is in {}: title, as_a, i_want, so_that, given, when, then and checks. Identifiers, paths and code stay in English.",
            owner.language
        ),
        "- title is short and says what the user gets. It never contains an id.".into(),
        "- Write 2 to 5 criteria. Each has one given, one when and one then.".into(),
        format!(
            "- Each criterion has a test: the repo-relative path of a test file QA will write. It exercises behaviour the user sees (CLI output, an HTTP response, a rendered page), not an internal function. Use a path that the test command runs: `{}`. Read the repo (Glob, Read) to follow its test layout. A task may use null.",
            o.config.test
        ),
        format!(
            "- checks is one plain sentence in {} of what the test proves.",
            owner.language
        ),
        format!("- owner_role is one of the roster: {}.", o.roster.join(", ")),
        match o.brief {
            Some(b) => format!(
                "- objective is the id of the brief objective the story serves ({}), or null.",
                inputs::objective_ids(b).join(", ")
            ),
            None => "- objective is null.".into(),
        },
        "- sources: cite only commits, issues, files, URLs or open stories (type \"story\", ref its id) that exist. Check a path with Glob before you cite it. Use an empty list when unsure.".into(),
        format!(
            "- Do not propose what {} already rejected or what an open story covers.",
            owner.name
        ),
        format!(
            "- Working directory: {} (read-only: Read, Grep, Glob). Cite paths relative to it.",
            o.config.repo
        ),
    ];
    let mut out: Vec<String> = Vec::new();
    if let Some(b) = o.brief {
        out.push(format!("# Brief of the product\n{}", inputs::brief_xml(b)));
    }
    out.push(format!(
        "# Snapshot of the product (compact)\n{}",
        o.snapshot
    ));
    out.push(format!("# Open stories\n{}", open_stories(o.stories)));
    out.push(format!(
        "# Your memory about this product\n{}",
        if o.memory.is_empty() {
            "_Empty._"
        } else {
            o.memory
        }
    ));
    out.push(format!("# Task\n{task}"));
    out.push(format!("# Instructions\n{}", rules.join("\n")));
    if let Some((previous, failures)) = o.repair {
        let list: Vec<String> = failures.iter().map(|f| format!("- {f}")).collect();
        out.push(format!(
            "# Repair\nYour previous output failed these checks. Fix them and return the full corrected output.\n\nFailures:\n{}\n\nPrevious output:\n{previous}",
            list.join("\n")
        ));
    }
    out.join("\n\n")
}

struct Fixed {
    proposed_by: String,
    minutes: Option<String>,
    now: String,
}

fn to_story(c: &Content, id: &str, fixed: &Fixed) -> Story {
    Story {
        id: id.into(),
        kind: c.kind.clone(),
        slug: record::slug(&c.title),
        title: c.title.clone(),
        as_a: c.as_a.clone(),
        i_want: c.i_want.clone(),
        so_that: c.so_that.clone(),
        criteria: c.criteria.clone(),
        owner_role: c.owner_role.clone(),
        objective: c.objective.clone(),
        sources: c.sources.clone(),
        status: "not_implemented".into(),
        approval: "pending".into(),
        changes_note: None,
        proposed_by: fixed.proposed_by.clone(),
        minutes: fixed.minutes.clone(),
        session: None,
        results: Vec::new(),
        created_at: fixed.now.clone(),
        updated_at: fixed.now.clone(),
        archived: None,
    }
}

/// The items of the output and the failures of its shape.
fn parse_output(
    output: &Value,
    mode: Mode,
) -> (Vec<std::result::Result<Content, String>>, Vec<String>) {
    let Some(items) = output.get("stories").and_then(Value::as_array) else {
        return (
            Vec::new(),
            vec!["The output has no \"stories\" list.".into()],
        );
    };
    let mut failures = Vec::new();
    let max = if mode == Mode::Suggest {
        MAX_SUGGESTIONS
    } else {
        1
    };
    if items.len() > max {
        failures.push(format!(
            "\"stories\" has {} items; at most {max}.",
            items.len()
        ));
    }
    if mode != Mode::Suggest && items.is_empty() {
        failures.push("\"stories\" is empty; return exactly 1 story.".into());
    }
    let parsed = items
        .iter()
        .map(|i| Content::deserialize(i).map_err(|e| e.to_string()))
        .collect();
    (parsed, failures)
}

static COMMIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[0-9a-f]{4,40}$").unwrap());
static LINE_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(#L\d+(-L?\d+)?|:\d+(-\d+)?)$").unwrap());

/// Where the sources of a proposal can be: the existing product paths and the story repo.
struct Roots {
    roots: Vec<PathBuf>,
    repos: Vec<PathBuf>,
}

fn roots(product: &Product, repo: &str) -> Roots {
    let mut roots: Vec<PathBuf> = product
        .paths
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();
    let mut repos: Vec<PathBuf> = roots
        .iter()
        .filter(|p| p.join(".git").exists())
        .cloned()
        .collect();
    for r in &roots {
        if !repos.contains(r) {
            repos.push(r.clone());
        }
    }
    // The story repo is searched too, even when it is not one of the product paths.
    let repo = PathBuf::from(repo);
    if repo.exists() {
        if !roots.contains(&repo) {
            roots.push(repo.clone());
        }
        if !repos.contains(&repo) {
            repos.push(repo);
        }
    }
    Roots { roots, repos }
}

fn inside(root: &Path, full: &Path) -> bool {
    full.strip_prefix(root)
        .is_ok_and(|rel| !rel.to_string_lossy().starts_with(".."))
}

fn resolves(src: &Source, r: &Roots) -> bool {
    match src.kind.as_str() {
        "commit" => {
            COMMIT.is_match(&src.reference)
                && r.repos.iter().any(|repo| {
                    super::git::git(
                        repo,
                        &["cat-file", "-e", &format!("{}^{{commit}}", src.reference)],
                    )
                    .ok
                })
        }
        "path" => {
            let reference = LINE_SUFFIX.replace(&src.reference, "");
            let path = Path::new(reference.as_ref());
            if path.is_absolute() {
                return path.exists() && r.roots.iter().any(|root| inside(root, path));
            }
            r.roots.iter().any(|root| {
                let full = normalize(&root.join(path));
                inside(root, &full) && full.exists()
            })
        }
        _ => true,
    }
}

fn check_all(
    items: &[std::result::Result<Content, String>],
    build: &dyn Fn(&Content) -> Story,
    request: &Request,
    roster: &[String],
    known: &[String],
    r: &Roots,
) -> Vec<String> {
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let prefix = if items.len() > 1 {
            format!("stories[{}]: ", i + 1)
        } else {
            String::new()
        };
        let c = match item {
            Ok(c) => c,
            Err(e) => {
                out.push(format!("{prefix}The story does not parse: {e}"));
                continue;
            }
        };
        let mut failures = validate(&build(c), roster);
        for src in &c.sources {
            if src.kind == "story" && !known.contains(&src.reference) {
                failures.push(format!(
                    "The story source \"{}\" is not a story of the product.",
                    src.reference
                ));
            }
            if (src.kind == "path" || src.kind == "commit") && !resolves(src, r) {
                failures.push(format!(
                    "The {} source \"{}\" does not exist in the repo.",
                    src.kind, src.reference
                ));
            }
        }
        let forced = match request.mode {
            Mode::Changes => request.current.as_ref().map(|s| s.kind.clone()),
            _ => request.kind.clone(),
        };
        if let Some(f) = forced
            && c.kind != f
        {
            failures.push(format!("type must be \"{f}\", found \"{}\".", c.kind));
        }
        out.extend(failures.into_iter().map(|f| format!("{prefix}{f}")));
    }
    out
}

/// One PM call, repaired once with the failure list. A second failure fails.
fn propose(
    ctx: &ProposeCtx,
    request: &Request,
    build: &dyn Fn(&Content) -> Story,
) -> Result<(Vec<Content>, Vec<StoryUsage>)> {
    let product = &ctx.product;
    let (pm, brief, snapshot, stories, memory) = read_with(&ctx.store, |c| {
        let snapshot = match &ctx.snapshot {
            Some(s) => s.clone(),
            None => crate::snapshot::snapshot(c, product, None)?,
        };
        Ok((
            inputs::role(c, &ctx.root, &product.id, "pm")?,
            agentsi_store::read::brief(c, &product.id)?,
            inputs::compact_snapshot(&snapshot, SNAPSHOT_MAX_CHARS),
            store::list(c, &product.id, false)?,
            inputs::memory_for_prompt(c, &product.id, "pm")?,
        ))
    })?;
    let objectives = brief.as_ref().map(inputs::objective_ids);
    let schema = content_schema(&product.roles, objectives.as_deref());
    let known: Vec<String> = stories.iter().map(|s| s.id.clone()).collect();
    let where_ = roots(product, &ctx.config.repo);
    let mut usage = Vec::new();
    let mut ask = |repair: Option<(&Value, &[String])>| -> Result<Value> {
        let text = prompt(&Prompt {
            request,
            config: &ctx.config,
            roster: &product.roles,
            snapshot: &snapshot,
            memory: &memory,
            brief: brief.as_ref(),
            stories: &stories,
            owner: &ctx.owner,
            repair,
        });
        let call = Call {
            budget_usd: None,
            prompt: text,
            model: pm.model.clone(),
            effort: pm.effort.clone(),
            system: format!(
                "{}\n\nNow you shape stories for Stories mode. You write story content only.",
                pm.body
            ),
            cwd: ctx.config.repo.clone(),
            tools: PM_TOOLS.iter().map(|t| t.to_string()).collect(),
            schema: Some(schema.clone()),
            timeout: PM_TIMEOUT,
            mcp: None,
        };
        let role = if repair.is_some() { "pm-repair" } else { "pm" };
        let (output, u) = (ctx.caller)(&call, role).map_err(Error::invalid)?;
        usage.push(u);
        Ok(output)
    };
    let check = |output: &Value| {
        let (items, mut failures) = parse_output(output, request.mode);
        failures.extend(check_all(
            &items,
            build,
            request,
            &product.roles,
            &known,
            &where_,
        ));
        (items, failures)
    };
    let first = ask(None)?;
    let (mut items, mut failures) = check(&first);
    if !failures.is_empty() {
        let second = ask(Some((&first, &failures)))?;
        (items, failures) = check(&second);
        if !failures.is_empty() {
            let list: Vec<String> = failures.iter().map(|f| format!("- {f}")).collect();
            return Err(Error::invalid(format!(
                "The PM output is invalid after one repair, so nothing was written:\n{}",
                list.join("\n")
            )));
        }
    }
    let items = items
        .into_iter()
        .filter_map(std::result::Result::ok)
        .collect();
    Ok((items, usage))
}

fn total(usage: &[StoryUsage]) -> f64 {
    usage.iter().map(|u| u.cost_usd).sum()
}

/// Saves the stories one by one, so each gets the next id, in one story transaction.
fn save_all(ctx: &ProposeCtx, items: &[Content], fixed: &Fixed) -> Result<Vec<Story>> {
    transaction(&ctx.product.id, || {
        let mut saved = Vec::new();
        for c in items {
            let id = read_with(&ctx.store, |conn| {
                store::next_id(conn, &ctx.product.id, &c.kind)
            })?;
            saved.push(store::save(
                &ctx.store,
                &ctx.product,
                to_story(c, &id, fixed),
            )?);
        }
        Ok(saved)
    })
}

/// The id only feeds the validator. `save_all` gives the real one.
fn provisional(ctx: &ProposeCtx) -> Result<(String, String)> {
    read_with(&ctx.store, |c| {
        Ok((
            store::next_id(c, &ctx.product.id, "story")?,
            store::next_id(c, &ctx.product.id, "task")?,
        ))
    })
}

static MINUTES_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[\w.-]+$").unwrap());

fn assert_minutes(ctx: &ProposeCtx, minutes: Option<&str>) -> Result<()> {
    let Some(m) = minutes else { return Ok(()) };
    let found = MINUTES_ID.is_match(m)
        && read_with(&ctx.store, |c| {
            Ok(c.query_row(
                "select count(*) from minutes where id = ?1 and product = ?2 and data is not null",
                [m, ctx.product.id.as_str()],
                |r| r.get::<_, i64>(0),
            )? > 0)
        })?;
    if !found {
        return Err(Error::invalid(format!(
            "Unknown minutes \"{m}\" in {}",
            ctx.product.id
        )));
    }
    Ok(())
}

fn result(stories: Vec<Story>, usage: Vec<StoryUsage>) -> StoryProposeResult {
    StoryProposeResult {
        total_cost_usd: total(&usage),
        stories,
        usage,
    }
}

/// `story new`: the PM shapes the owner's text into one story.
pub fn propose_story(
    ctx: &ProposeCtx,
    text: &str,
    kind: Option<&str>,
    minutes: Option<&str>,
) -> Result<StoryProposeResult> {
    if text.trim().is_empty() {
        return Err(Error::invalid("A story needs a text."));
    }
    assert_minutes(ctx, minutes)?;
    let fixed = Fixed {
        proposed_by: "owner".into(),
        minutes: minutes.map(String::from),
        now: clock::iso(clock::now_ms()),
    };
    let (sid, tid) = provisional(ctx)?;
    let build = |c: &Content| to_story(c, if c.kind == "task" { &tid } else { &sid }, &fixed);
    let request = Request {
        mode: Mode::New,
        text: text.trim().into(),
        kind: kind.map(String::from),
        current: None,
    };
    let (items, usage) = propose(ctx, &request, &build)?;
    Ok(result(save_all(ctx, &items, &fixed)?, usage))
}

/// `story suggest`: up to 3 stories from the snapshot.
pub fn suggest_stories(ctx: &ProposeCtx) -> Result<StoryProposeResult> {
    let fixed = Fixed {
        proposed_by: "pm".into(),
        minutes: None,
        now: clock::iso(clock::now_ms()),
    };
    let (sid, tid) = provisional(ctx)?;
    let build = |c: &Content| to_story(c, if c.kind == "task" { &tid } else { &sid }, &fixed);
    let request = Request {
        mode: Mode::Suggest,
        text: String::new(),
        kind: None,
        current: None,
    };
    let (items, usage) = propose(ctx, &request, &build)?;
    let saved = if items.is_empty() {
        Vec::new()
    } else {
        save_all(ctx, &items, &fixed)?
    };
    Ok(result(saved, usage))
}

/// `story changes`: the PM rewrites the story with the note. The id, the slug
/// and the history stay. The approval goes back to `pending`.
pub fn revise_story(ctx: &ProposeCtx, id: &str, note: &str) -> Result<StoryProposeResult> {
    let product = &ctx.product;
    let current = read_with(&ctx.store, |c| store::load(c, &product.id, id))?;
    let asked = request_changes(current.clone(), note)?;
    let fixed = Fixed {
        proposed_by: current.proposed_by.clone(),
        minutes: current.minutes.clone(),
        now: clock::iso(clock::now_ms()),
    };
    let slug = current.slug.clone();
    let build = |c: &Content| Story {
        slug: slug.clone(),
        ..to_story(c, &current.id, &fixed)
    };
    let request = Request {
        mode: Mode::Changes,
        text: asked.changes_note.clone().unwrap_or_default(),
        kind: None,
        current: Some(current.clone()),
    };
    let (items, usage) = propose(ctx, &request, &build)?;
    let Some(c) = items.into_iter().next() else {
        return Err(Error::invalid("The PM returned no story."));
    };
    let saved = transaction(&product.id, || {
        // The model call ran outside the lock, so the story must still accept the change.
        let fresh = request_changes(
            read_with(&ctx.store, |conn| store::load(conn, &product.id, id))?,
            note,
        )?;
        store::save(
            &ctx.store,
            product,
            Story {
                kind: c.kind.clone(),
                title: c.title.clone(),
                as_a: c.as_a.clone(),
                i_want: c.i_want.clone(),
                so_that: c.so_that.clone(),
                criteria: c.criteria.clone(),
                owner_role: c.owner_role.clone(),
                objective: c.objective.clone(),
                sources: c.sources.clone(),
                approval: "pending".into(),
                ..fresh
            },
        )
    })?;
    Ok(result(vec![saved], usage))
}
