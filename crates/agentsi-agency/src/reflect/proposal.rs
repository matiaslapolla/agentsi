//! The prompt proposal: one call reads the tuning traces of the agent with the
//! strongest evidence, code renders the change, replay judges it on the held-out
//! half. It writes nothing. The rules copy `proposal.ts`.

use super::curate::CALL_TIMEOUT;
use super::memory::active;
use super::replay::{
    Budget, COST_RULE, CallFailure, Caller, REPLAY_BUDGET_USD, ReplayCase, ReplayInput, ReplayRun,
    insufficient_cases, replay, replay_cases, split_cases,
};
use super::{SYNTHESIS_AGENT, list, text, typed_entry, unified_diff};
use crate::knowledge::memory_text::{MEMORY_MAX_CHARS, render_for_prompt};
use crate::meeting::check::source_context;
use crate::meeting::log::{NO_DATA, sha256};
use crate::prompt_inputs::brief_xml;
use crate::registry::Product;
use crate::roles::files::RoleFiles;
use crate::roles::load::load_role;
use crate::roster::{parse_agent_file, read_overlays};
use crate::story::claude::Call;
use agentsi_proto::agency_api::{Owner, ReadyAgent};
use agentsi_proto::reflect_api::{
    Proposal, ProposalCandidate, ProposalTrigger, ReplayReport, SharedLesson,
};
use agentsi_proto::roles_api::{RoleDef, RoleSource};
use agentsi_store::Store;
use agentsi_store::read::{self, MemoryItem};
use regex::Regex;
use rusqlite::Connection;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::LazyLock;

pub const PROPOSAL_MODEL: &str = "sonnet";
pub const OVERLAY_MAX_CHARS: usize = 4000;
/// The only frontmatter keys that a proposal may change.
pub const EDITABLE_FRONTMATTER: [&str; 2] = ["model", "effort"];
const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

static PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{P}\p{S}]").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static FRONTMATTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)\A---\r?\n(.*?)\r?\n---\r?\n(\n?)").unwrap());
static LINE_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Za-z][A-Za-z0-9_-]*):").unwrap());
static MODEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^[a-z0-9][A-Za-z0-9_.-]*$").unwrap());

/// Lowercase, no punctuation or symbols, collapsed white space: two lessons match
/// only when this is equal.
pub fn normalize_lesson(s: &str) -> String {
    let lower = s.to_lowercase();
    let plain = PUNCTUATION.replace_all(&lower, "");
    SPACES.replace_all(&plain, " ").trim().to_string()
}

/// `agentRole`: the role that an agent of `product` extends.
fn agent_role(c: &Connection, agent: &str, product: &str) -> agentsi_store::Result<String> {
    let overlay = read::overlay(c, product, agent)?;
    let extended = overlay
        .map(|t| parse_agent_file(&t).0)
        .and_then(|mut fm| fm.remove("extends"))
        .filter(|e| !e.is_empty());
    Ok(extended.unwrap_or_else(|| agent.split('.').next().unwrap_or(agent).to_string()))
}

/// Every (product, agent) that holds memory, sorted.
fn all_memory_agents(c: &Connection) -> agentsi_store::Result<Vec<(String, String)>> {
    let mut stmt =
        c.prepare_cached("select distinct product, agent from memory order by product, agent")?;
    let rows = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

/// A lesson of this agent that agents of the same role hold in other products, with
/// the most products first.
fn shared_lesson(
    c: &Connection,
    product: &str,
    role: &str,
    items: &[MemoryItem],
) -> agentsi_store::Result<Option<SharedLesson>> {
    let own: HashMap<String, &str> = active(items)
        .into_iter()
        .filter(|i| i.kind == "lesson")
        .map(|i| (normalize_lesson(&i.text), i.text.as_str()))
        .collect();
    if own.is_empty() {
        return Ok(None);
    }
    let mut holders: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (other, agent) in all_memory_agents(c)? {
        if other == product || agent_role(c, &agent, &other)? != role {
            continue;
        }
        let theirs = read::memory(c, &other, &agent)?;
        for i in active(&theirs) {
            let key = normalize_lesson(&i.text);
            if i.kind == "lesson" && own.contains_key(&key) {
                holders.entry(key).or_default().insert(other.clone());
            }
        }
    }
    // The map is sorted by key, so the first of the most products wins a tie.
    let best = holders
        .iter()
        .max_by(|a, b| a.1.len().cmp(&b.1.len()).then_with(|| b.0.cmp(a.0)));
    Ok(best.map(|(key, others)| {
        let mut products = vec![product.to_string()];
        products.extend(others.iter().cloned());
        SharedLesson {
            text: own[key].to_string(),
            products,
        }
    }))
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProposalTarget {
    /// Relative to the data root.
    pub path: String,
    pub global: bool,
    pub role: String,
    /// The text of the target today. `None` when it does not exist yet.
    pub current: Option<String>,
    /// The body that the proposal rewrites: the overlay body, or the body of the global role.
    pub editable: String,
    pub shared: Option<SharedLesson>,
}

/// The overlay of the product (a new one with `extends` when it has none), or the
/// global role when a lesson holds in two or more products.
pub fn choose_target(
    c: &Connection,
    root: &Path,
    product: &str,
    def: &RoleDef,
    items: &[MemoryItem],
) -> agentsi_store::Result<ProposalTarget> {
    let shared = if def.source == RoleSource::Custom {
        None
    } else {
        shared_lesson(c, product, &def.role, items)?
    };
    // Only a root that holds the global roles may change them.
    let role_file = root
        .join("plugin")
        .join("agents")
        .join(format!("{}.md", def.role));
    let global = shared.is_some() && role_file.exists();
    let path = if global {
        format!("plugin/agents/{}.md", def.role)
    } else {
        format!("workspace/{product}/agents/{}.md", def.id)
    };
    let current = if global {
        std::fs::read_to_string(&role_file).ok()
    } else {
        read::overlay(c, product, &def.id)?
    };
    let editable = current
        .as_deref()
        .filter(|t| !t.is_empty())
        .map(|t| parse_agent_file(t).1)
        .unwrap_or_default();
    Ok(ProposalTarget {
        path,
        global,
        role: def.role.clone(),
        current,
        editable,
        shared: if global { shared } else { None },
    })
}

/// The keys of a frontmatter block, in the order of the text.
fn frontmatter_keys(text: &str) -> Vec<String> {
    let Some(m) = FRONTMATTER.captures(text) else {
        return Vec::new();
    };
    m[1].lines()
        .filter_map(|l| LINE_KEY.captures(l).map(|c| c[1].to_string()))
        .collect()
}

fn units(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// The new text of the target from the proposed body. A leading frontmatter block
/// may change only model and effort. The error is the reason, for the owner.
pub fn render_proposal(t: &ProposalTarget, proposed: &str) -> Result<String, String> {
    let (frontmatter, body) = parse_agent_file(proposed);
    let current_text = t.current.as_deref().filter(|c| !c.is_empty());
    let current_fm = match current_text {
        Some(c) => parse_agent_file(c).0,
        None => BTreeMap::from([("extends".to_string(), t.role.clone())]),
    };
    let mut changes: Vec<(String, String)> = Vec::new();
    for key in frontmatter_keys(proposed) {
        let Some(value) = frontmatter.get(&key) else {
            continue;
        };
        if current_fm.get(&key) == Some(value) || changes.iter().any(|(k, _)| *k == key) {
            continue;
        }
        if !EDITABLE_FRONTMATTER.contains(&key.as_str()) {
            return Err(format!(
                "cambia el frontmatter \"{key}\"; solo puede cambiar {}",
                EDITABLE_FRONTMATTER.join(" y ")
            ));
        }
        if key == "model" && !MODEL.is_match(value) {
            return Err(format!("modelo inválido \"{value}\""));
        }
        if key == "effort" && !EFFORTS.contains(&value.as_str()) {
            return Err(format!("effort inválido \"{value}\""));
        }
        changes.push((key, value.clone()));
    }
    if body.is_empty() {
        return Err("el cuerpo está vacío".into());
    }
    if !t.global && units(&body) > OVERLAY_MAX_CHARS {
        return Err(format!(
            "el overlay tiene {} caracteres (máximo {OVERLAY_MAX_CHARS})",
            units(&body)
        ));
    }

    let block = current_text.and_then(|c| FRONTMATTER.captures(c));
    let lines: Vec<String> = match (&block, current_text) {
        (Some(m), _) => m[1].lines().map(String::from).collect(),
        (None, Some(_)) => Vec::new(),
        (None, None) => vec![format!("extends: {}", t.role)],
    };
    let mut out: Vec<String> = lines
        .iter()
        .map(|line| {
            let key = LINE_KEY.captures(line).map(|c| c[1].to_string());
            match key.and_then(|k| changes.iter().find(|(c, _)| *c == k)) {
                Some((k, v)) => format!("{k}: {v}"),
                None => line.clone(),
            }
        })
        .collect();
    for (key, value) in &changes {
        if !lines.iter().any(|l| l.starts_with(&format!("{key}:"))) {
            out.push(format!("{key}: {value}"));
        }
    }
    let gap = block
        .as_ref()
        .map_or("", |m| m.get(2).map_or("", |g| g.as_str()));
    let head = if out.is_empty() {
        String::new()
    } else {
        format!("---\n{}\n---\n{gap}", out.join("\n"))
    };
    Ok(format!("{head}{body}\n"))
}

/// The agent as it would load with `content` in its target. It writes nothing that
/// outlives the call: a global target goes to a folder of this call.
fn effective_with(
    root: &Path,
    product: &str,
    agent: &str,
    t: &ProposalTarget,
    content: &str,
    overlays: &BTreeMap<String, String>,
    warnings: &mut Vec<String>,
) -> Result<RoleDef, String> {
    if !t.global {
        let mut with = overlays.clone();
        with.insert(agent.to_string(), content.to_string());
        return load_role(agent, product, &RoleFiles::at(root), &with, warnings);
    }
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let name = Path::new(&t.path)
        .file_name()
        .ok_or_else(|| format!("no file name in {}", t.path))?;
    std::fs::write(dir.path().join(name), content).map_err(|e| e.to_string())?;
    let files = RoleFiles::Dir(dir.path().to_path_buf());
    load_role(agent, product, &files, overlays, warnings)
}

fn body_key(global: bool) -> &'static str {
    if global {
        "new_global_body"
    } else {
        "new_overlay_body"
    }
}

pub fn proposal_schema(global: bool) -> Value {
    let key = body_key(global);
    let mut properties = Map::new();
    properties.insert(key.into(), json!({"type": "string"}));
    properties.insert("rationale".into(), json!({"type": "string"}));
    properties.insert("expected_effect".into(), json!({"type": "string"}));
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [key, "rationale", "expected_effect"],
        "properties": properties,
    })
}

pub fn proposal_system(global: bool, owner: &Owner) -> String {
    let name = &owner.name;
    let body = if global {
        format!(
            "Return in `{}` the complete new body of the global role file, without its frontmatter. Every product with this role gets it.",
            body_key(true)
        )
    } else {
        format!(
            "Return in `{}` the complete new body of the product overlay: the text appended to the global role under \"## This product\". At most {OVERLAY_MAX_CHARS} characters.",
            body_key(false)
        )
    };
    [
        "You improve the prompt of one AgentSI agent from the traces of its past meetings.".to_string(),
        format!("Read the traces: the agent's answers, the check failures, the sources that did not resolve, the invented sources and the marks of {name}. Find the one cause behind the repeated failure and change the instructions to remove it."),
        "Change one thing. Keep the role's voice, its language and every rule that is not the cause.".into(),
        format!("The check is a floor, not a goal. Never weaken a rule about sources or about writing \"{NO_DATA}\" when data is missing. Never tell the agent to cite a file it did not read, or a doc only to pass a check."),
        body,
        format!("You may start it with a frontmatter block that changes only {}.", EDITABLE_FRONTMATTER.join(" or ")),
        format!("`rationale`: the cause you found, citing minutes ids. `expected_effect`: which replay metric should improve (first-pass check, unresolved sources, invented sources, missing objectives) and why. Write both in {}: {name} reads them before approving.", owner.language),
    ]
    .join("\n")
}

fn sources_text(sources: &[Value]) -> String {
    if sources.is_empty() {
        return "(none)".into();
    }
    sources
        .iter()
        .map(|s| format!("{} {}", text(s, "type"), text(s, "ref")))
        .collect::<Vec<_>>()
        .join(", ")
}

fn bullet_list(lines: &[String]) -> String {
    if lines.is_empty() {
        return "(none)".into();
    }
    lines
        .iter()
        .map(|l| format!("- {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `JSON.stringify({answer, priorities, concerns})`: a missing field leaves no key.
fn answer_json(a: &Value) -> String {
    let mut out = Map::new();
    for key in ["answer", "priorities", "concerns"] {
        if let Some(v) = a.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    Value::Object(out).to_string()
}

fn trace(c: &ReplayCase, agent: &str) -> String {
    let marks: Vec<String> = list(&c.minutes, "priorities")
        .iter()
        .enumerate()
        .filter(|(_, p)| !text(p, "mark").is_empty())
        .map(|(idx, p)| {
            format!(
                "#{idx} \"{}\" (owner {}): {}",
                text(p, "title"),
                text(p, "owner_role"),
                text(p, "mark")
            )
        })
        .collect();
    let failures: Vec<String> = list(&c.log["synthesis"]["first_check"], "failures")
        .iter()
        .map(|f| f.as_str().unwrap_or_default().to_string())
        .collect();
    [
        format!("<case minutes=\"{}\">", c.minutes_id),
        format!("<question>{}</question>", text(&c.minutes, "question")),
        format!(
            "<answer>{}</answer>",
            answer_json(&c.minutes["roles"][agent])
        ),
        format!(
            "<check_failures>\n{}\n</check_failures>",
            bullet_list(&failures)
        ),
        format!(
            "<unresolved_sources>{}</unresolved_sources>",
            sources_text(list(&c.log["agents"][agent], "unresolved_sources"))
        ),
        format!(
            "<invented_sources>{}</invented_sources>",
            sources_text(list(&c.log["synthesis"], "invented_sources"))
        ),
        format!("<marks>\n{}\n</marks>", bullet_list(&marks)),
        "</case>".into(),
    ]
    .join("\n")
}

/// One earlier proposal of the agent, for the prompt.
pub struct History {
    pub id: String,
    pub status: String,
    pub rationale: String,
}

pub struct PromptInput<'a> {
    pub brief_xml: Option<&'a str>,
    pub agent: &'a str,
    pub target: &'a ProposalTarget,
    pub ready: &'a ReadyAgent,
    pub effective: &'a str,
    pub tuning: &'a [ReplayCase],
    pub history: &'a [History],
}

pub fn proposal_prompt(o: &PromptInput) -> String {
    let trigger = match &o.ready.failure_class {
        Some(class) => format!("{class} in {} meetings", o.ready.count),
        None => format!("{} rejected priorities", o.ready.count),
    };
    let shared = o
        .target
        .shared
        .as_ref()
        .map_or(String::new(), |s| format!(" shared_lesson=\"{}\"", s.text));
    let editable = if o.target.editable.is_empty() {
        "(empty: a new overlay)"
    } else {
        &o.target.editable
    };
    let mut lines: Vec<String> = Vec::new();
    if let Some(xml) = o.brief_xml {
        lines.push(xml.into());
        lines.push(String::new());
    }
    lines.push(format!(
        "<agent id=\"{}\" role=\"{}\" target=\"{}\"{shared}/>",
        o.agent, o.target.role, o.target.path
    ));
    lines.push(format!("<trigger>{trigger}</trigger>"));
    lines.push(format!(
        "<effective_prompt>\n{}\n</effective_prompt>",
        o.effective
    ));
    lines.push(format!("<editable>\n{editable}\n</editable>"));
    lines.push("<traces>".into());
    lines.extend(o.tuning.iter().map(|c| trace(c, o.agent)));
    lines.push("</traces>".into());
    lines.push("<earlier_proposals>".into());
    let earlier: Vec<String> = o
        .history
        .iter()
        .map(|h| format!("{} {}: {}", h.id, h.status, h.rationale))
        .collect();
    lines.push(bullet_list(&earlier));
    lines.push("</earlier_proposals>".into());
    lines.join("\n")
}

/// The text and the why of the decision of a pending proposal.
pub fn decision_text(p: &Proposal, reflection_id: &str) -> (String, String) {
    let place = if p.target.starts_with("plugin/") {
        format!("{}, el rol global de todos los productos", p.target)
    } else if p.target_hash.is_none() {
        format!("{}, overlay nuevo", p.target)
    } else {
        p.target.clone()
    };
    let mut lines = vec![format!(
        "Cambiar el prompt de {} ({place}) según la reflexión {reflection_id}. Efecto esperado: {}",
        p.agent, p.expected_effect
    )];
    lines.extend(p.replay.table());
    (lines.join("\n"), p.rationale.clone())
}

pub struct ProposeInput<'a> {
    pub store: &'a Store,
    pub product: &'a Product,
    pub owner: &'a Owner,
    pub root: &'a Path,
    pub ready: &'a [ReadyAgent],
    pub log: &'a [Value],
    pub minutes: &'a [Value],
    pub reflections: &'a [Value],
    /// The memory of the agent after the curation of this reflection.
    pub items: &'a (dyn Fn(&str) -> Result<Vec<MemoryItem>, String> + Sync),
    pub call: &'a Caller<'a>,
    pub budget_usd: Option<f64>,
}

#[derive(Default)]
pub struct ProposeResult {
    pub candidates: Vec<ProposalCandidate>,
    pub proposal: Option<Proposal>,
    pub cost_usd: f64,
    pub warnings: Vec<String>,
}

fn reading<T>(store: &Store, f: impl FnOnce(&Connection) -> T) -> Result<T, String> {
    store.read(|c| Ok(f(c))).map_err(|e| e.to_string())
}

struct Chosen {
    ready: ReadyAgent,
    tuning: Vec<ReplayCase>,
    held_out: Vec<ReplayCase>,
    /// The index of its candidate.
    index: usize,
}

fn report(c: &Chosen, budget: &Budget) -> ReplayReport {
    ReplayReport {
        tuning: c.tuning.iter().map(|x| x.minutes_id.clone()).collect(),
        held_out: c.held_out.iter().map(|x| x.minutes_id.clone()).collect(),
        cost_rule: COST_RULE.into(),
        cost_usd: budget.spent(),
        budget_usd: budget.limit,
        ..ReplayReport::default()
    }
}

fn agent_history(reflections: &[Value], agent: &str) -> Vec<History> {
    reflections
        .iter()
        .filter(|e| text(&e["proposal"], "agent") == agent && !e["proposal"].is_null())
        .map(|e| History {
            id: text(e, "id").into(),
            status: text(&e["proposal"], "status").into(),
            rationale: text(&e["proposal"], "rationale").into(),
        })
        .collect()
}

/// An earlier proposal of this target with this text that was rejected or discarded.
fn repeated(reflections: &[Value], target: &str, body_hash: &str) -> Option<(String, String)> {
    reflections.iter().find_map(|e| {
        let p = typed_entry(e)?.proposal?;
        let closed = p.status == "rejected" || p.status == "discarded";
        (p.target == target && p.body_hash == body_hash && closed)
            .then(|| (text(e, "id").to_string(), p.status))
    })
}

fn call_failure(f: CallFailure) -> String {
    match f {
        CallFailure::Budget(m) | CallFailure::Failed(m) => m,
    }
}

/// Makes the proposal of the chosen agent. The error ends in a `failed` candidate.
fn propose(
    i: &ProposeInput,
    chosen: &Chosen,
    budget: &Budget,
    warnings: &mut Vec<String>,
) -> Result<Proposal, String> {
    let p = &i.product.id;
    let agent = chosen.ready.agent.as_str();
    let files = RoleFiles::at(i.root);
    let items = (i.items)(agent)?;
    let (overlays, current, target, pm, brief) = reading(i.store, |c| {
        let overlays = read_overlays(c, p).map_err(|e| e.to_string())?;
        let current = load_role(agent, p, &files, &overlays, warnings)?;
        let target = choose_target(c, i.root, p, &current, &items).map_err(|e| e.to_string())?;
        let pm = if agent == SYNTHESIS_AGENT {
            current.clone()
        } else {
            load_role(SYNTHESIS_AGENT, p, &files, &overlays, warnings)?
        };
        // A brief that does not read counts as none, as `readBriefSafe` does.
        let brief = read::brief(c, p).ok().flatten();
        Ok::<_, String>((overlays, current, target, pm, brief))
    })??;

    let history = agent_history(i.reflections, agent);
    let xml = brief.as_ref().map(brief_xml);
    let prompt = proposal_prompt(&PromptInput {
        brief_xml: xml.as_deref(),
        agent,
        target: &target,
        ready: &chosen.ready,
        effective: &current.body,
        tuning: &chosen.tuning,
        history: &history,
    });
    let res = budget
        .run(
            i.call,
            &Call {
                prompt,
                model: PROPOSAL_MODEL.into(),
                system: proposal_system(target.global, i.owner),
                cwd: i.root.to_string_lossy().into_owned(),
                tools: Vec::new(),
                schema: Some(proposal_schema(target.global)),
                effort: Some("high".into()),
                timeout: CALL_TIMEOUT,
                mcp: None,
            },
        )
        .map_err(call_failure)?;
    let field = |k: &str| {
        res.output
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let rendered = render_proposal(&target, &field(body_key(target.global)));
    let content = rendered.clone().unwrap_or_default();
    let body_hash = if content.is_empty() {
        String::new()
    } else {
        sha256(&normalize_lesson(&content))
    };
    let diff = if content.is_empty() {
        String::new()
    } else {
        let before = target.current.as_deref().unwrap_or_default();
        unified_diff(before.trim_end(), content.trim_end(), &target.path)
            .map_err(|e| e.to_string())?
    };
    let earlier = if body_hash.is_empty() {
        None
    } else {
        repeated(i.reflections, &target.path, &body_hash)
    };
    let aborted = match (&rendered, diff.is_empty(), earlier) {
        (Err(e), _, _) => Some(format!("propuesta inválida: {e}")),
        (_, true, _) => Some("la propuesta no cambia el prompt".to_string()),
        (_, _, Some((id, status))) => Some(format!("repite la propuesta de {id} ({status})")),
        _ => None,
    };
    // The hash of the text that the proposal saw: a later edit must make it stale.
    let target_hash = target.current.as_deref().map(sha256);
    let base = Proposal {
        agent: agent.into(),
        target: target.path.clone(),
        trigger: ProposalTrigger {
            reason: chosen.ready.reason.clone(),
            failure_class: chosen.ready.failure_class.clone(),
            count: chosen.ready.count,
        },
        shared_lesson: target.shared.clone(),
        rationale: field("rationale"),
        expected_effect: field("expected_effect"),
        diff,
        content: content.clone(),
        body_hash,
        target_hash,
        decision_id: None,
        replay: ReplayReport::default(),
        status: String::new(),
    };
    if let Some(why) = aborted {
        let replay = ReplayReport {
            aborted: Some(why),
            ..report(chosen, budget)
        };
        return Ok(Proposal {
            replay,
            status: "discarded".into(),
            ..base
        });
    }

    let candidate = effective_with(i.root, p, agent, &target, &content, &overlays, warnings)?;
    let sources = reading(i.store, |c| source_context(c, i.product))?;
    let memory = render_for_prompt(&items, Some(MEMORY_MAX_CHARS));
    let run: ReplayRun = replay(&ReplayInput {
        store: i.store,
        product: i.product,
        agent,
        candidate: &candidate,
        current: &current,
        pm: &pm,
        memory: &memory,
        brief: brief.as_ref(),
        owner: i.owner,
        held_out: &chosen.held_out,
        budget,
        call: i.call,
        sources: &sources,
    })?;
    let passed = run.verdict.passed;
    Ok(Proposal {
        replay: run.into_report(report(chosen, budget)),
        status: if passed { "pending" } else { "discarded" }.into(),
        ..base
    })
}

/// For the agent with the strongest evidence past the threshold and enough replay
/// cases. A passing change becomes a pending proposal, still without its decision.
/// Anything else is `discarded` with its figures.
pub fn propose_for_ready(i: &ProposeInput) -> Result<ProposeResult, String> {
    let mut out = ProposeResult::default();
    let mut chosen: Option<Chosen> = None;
    for ready in i.ready {
        let p = &i.product.id;
        let cases = reading(i.store, |c| {
            replay_cases(c, p, &ready.agent, i.log, i.minutes)
        })?
        .map_err(|e| e.to_string())?;
        let (tuning, held_out) = split_cases(cases);
        let note = insufficient_cases(tuning.len(), held_out.len());
        let outcome = match (&note, &chosen) {
            (Some(_), _) => "insufficient_evidence",
            (None, Some(_)) => "deferred",
            (None, None) => "proposed",
        };
        out.candidates.push(ProposalCandidate {
            agent: ready.agent.clone(),
            reason: ready.reason.clone(),
            failure_class: ready.failure_class.clone(),
            count: ready.count,
            cases: (tuning.len() + held_out.len()) as u64,
            held_out: held_out.len() as u64,
            outcome: outcome.into(),
            note,
        });
        if outcome == "proposed" {
            chosen = Some(Chosen {
                ready: ready.clone(),
                tuning,
                held_out,
                index: out.candidates.len() - 1,
            });
        }
    }
    let Some(chosen) = chosen else {
        return Ok(out);
    };

    let budget = Budget::new(i.budget_usd.unwrap_or(REPLAY_BUDGET_USD));
    match propose(i, &chosen, &budget, &mut out.warnings) {
        Ok(proposal) => out.proposal = Some(proposal),
        Err(e) => {
            out.warnings.push(format!(
                "reflect {}: proposal failed: {e}",
                chosen.ready.agent
            ));
            let candidate = &mut out.candidates[chosen.index];
            candidate.outcome = "failed".into();
            candidate.note = Some(e);
        }
    }
    out.cost_usd = budget.spent();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsi_store::db::open_writer;
    use agentsi_store::reflection::{write_memory, write_overlay};

    fn def(id: &str, source: RoleSource) -> RoleDef {
        RoleDef {
            id: id.into(),
            role: id.split('.').next().unwrap().into(),
            source,
            model: "sonnet".into(),
            effort: None,
            tools: None,
            body: "Body.".into(),
        }
    }

    fn lesson(id: &str, text: &str) -> MemoryItem {
        MemoryItem {
            id: id.into(),
            text: text.into(),
            kind: "lesson".into(),
            helpful: 0,
            harmful: 0,
            source: "r-1".into(),
            created_at: "t".into(),
            retired_at: None,
            retired_why: None,
        }
    }

    fn overlay_target(current: Option<&str>) -> ProposalTarget {
        ProposalTarget {
            path: "workspace/p/agents/dev.md".into(),
            global: false,
            role: "dev".into(),
            current: current.map(String::from),
            editable: String::new(),
            shared: None,
        }
    }

    #[test]
    fn lessons_match_without_case_punctuation_or_extra_space() {
        assert_eq!(
            normalize_lesson("  Citá  ¡las FUENTES! (siempre). "),
            "citá las fuentes siempre"
        );
    }

    #[test]
    fn a_new_overlay_extends_its_role() {
        let got = render_proposal(&overlay_target(None), "Cite the file.").unwrap();
        assert_eq!(got, "---\nextends: dev\n---\nCite the file.\n");
    }

    #[test]
    fn an_overlay_keeps_its_frontmatter_and_may_change_only_model_and_effort() {
        let t = overlay_target(Some("---\nextends: dev\nmodel: sonnet\n---\n\nOld.\n"));
        let got = render_proposal(&t, "---\nmodel: haiku\neffort: high\n---\nNew.").unwrap();
        assert_eq!(
            got,
            "---\nextends: dev\nmodel: haiku\neffort: high\n---\n\nNew.\n"
        );
        let refused = render_proposal(&t, "---\ntools: Bash\n---\nNew.").unwrap_err();
        assert_eq!(
            refused,
            "cambia el frontmatter \"tools\"; solo puede cambiar model y effort"
        );
        assert_eq!(
            render_proposal(&t, "---\neffort: huge\n---\nNew.").unwrap_err(),
            "effort inválido \"huge\""
        );
        assert_eq!(
            render_proposal(&t, "---\nmodel: a b\n---\nNew.").unwrap_err(),
            "modelo inválido \"a b\""
        );
        assert_eq!(render_proposal(&t, "").unwrap_err(), "el cuerpo está vacío");
    }

    #[test]
    fn an_overlay_over_the_limit_is_refused_and_a_global_body_is_not() {
        let long = "x".repeat(OVERLAY_MAX_CHARS + 1);
        assert_eq!(
            render_proposal(&overlay_target(None), &long).unwrap_err(),
            "el overlay tiene 4001 caracteres (máximo 4000)"
        );
        let mut global = overlay_target(Some("Old."));
        global.global = true;
        assert!(render_proposal(&global, &long).is_ok());
    }

    #[test]
    fn the_target_is_the_overlay_unless_a_lesson_holds_in_two_products() {
        let dir = tempfile::tempdir().unwrap();
        let agents = dir.path().join("plugin/agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("dev.md"), "---\nname: dev\n---\nGlobal.\n").unwrap();
        let mut c = open_writer(&dir.path().join("agentsi.db")).unwrap();
        let tx = c.transaction().unwrap();
        write_overlay(&tx, "p", "dev", "---\nextends: dev\n---\nMine.\n").unwrap();
        write_memory(&tx, "q", "dev", &[lesson("m-1", "Cite sources!")]).unwrap();
        tx.commit().unwrap();

        let dev = def("dev", RoleSource::Overlay);
        let other = [lesson("m-2", "Check the build.")];
        let t = choose_target(&c, dir.path(), "p", &dev, &other).unwrap();
        assert_eq!(t.path, "workspace/p/agents/dev.md");
        assert_eq!(t.editable, "Mine.");
        assert!(!t.global);

        let same = [lesson("m-3", "cite sources")];
        let t = choose_target(&c, dir.path(), "p", &dev, &same).unwrap();
        assert_eq!(t.path, "plugin/agents/dev.md");
        assert_eq!(t.editable, "Global.");
        let shared = t.shared.unwrap();
        assert_eq!(shared.text, "cite sources");
        assert_eq!(shared.products, ["p", "q"]);

        let custom = def("dev", RoleSource::Custom);
        let t = choose_target(&c, dir.path(), "p", &custom, &same).unwrap();
        assert!(!t.global, "a custom role never changes a global file");
    }

    #[test]
    fn a_root_without_role_files_never_picks_a_global_target() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = open_writer(&dir.path().join("agentsi.db")).unwrap();
        let tx = c.transaction().unwrap();
        write_memory(&tx, "q", "dev", &[lesson("m-1", "Cite sources")]).unwrap();
        tx.commit().unwrap();
        let dev = def("dev", RoleSource::Global);
        let same = [lesson("m-3", "cite sources")];
        let t = choose_target(&c, dir.path(), "p", &dev, &same).unwrap();
        assert_eq!(t.path, "workspace/p/agents/dev.md");
        assert!(t.shared.is_none() && t.current.is_none());
    }

    #[test]
    fn the_system_prompt_names_the_owner_and_keeps_the_no_data_rule() {
        let owner = Owner {
            name: "Ada".into(),
            language: "Spanish".into(),
        };
        let text = proposal_system(false, &owner);
        assert!(text.contains("the marks of Ada"));
        assert!(text.contains("Write both in Spanish: Ada reads them"));
        assert!(text.contains(NO_DATA));
        assert!(text.contains("`new_overlay_body`"));
        assert!(proposal_system(true, &owner).contains("`new_global_body`"));
    }

    #[test]
    fn the_schema_names_the_body_of_its_target() {
        assert_eq!(proposal_schema(true)["required"][0], "new_global_body");
        assert!(proposal_schema(false)["properties"]["new_overlay_body"].is_object());
    }
}
