//! `data.audit`: the health report of a product. It is deterministic: it
//! reads the store and the roster, and it calls no model. The rules copy
//! `packages/core/src/audit.ts`.

use crate::clock;
use crate::error::Result;
use crate::registry::Product;
use crate::roster;
use agentsi_proto::agency_api::{
    AgentMeetingStats, AgentMemoryStats, AuditConfig, AuditMarks, AuditReport, LastReflection,
    LastRepair, MeetingStats, Ordered, OverdueObjective, ReadyAgent, ReflectionStats,
    RepairComparison, RepairGroup,
};
use agentsi_store::read::{self, MemoryItem};
use regex::Regex;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::LazyLock;

/// Meetings that the audit looks at.
pub const AUDIT_WINDOW: usize = 20;
/// Reflect proposes a prompt change when one agent shows the same failure class in this many meetings...
pub const READY_FAILURE_MEETINGS: u64 = 3;
/// ...or when this many of the priorities it owns were rejected.
pub const READY_REJECTED_PRIORITIES: u64 = 3;
/// `sin datos` in one meeting counts as a failure from this many occurrences.
pub const SIN_DATOS_FAILURE: u64 = 2;
/// An active item never marked helpful after this many meetings is reported.
pub const NEVER_HELPFUL_MEETINGS: usize = 10;
/// The o1 signal compares repair rounds against this many first meetings.
pub const REPAIR_BASELINE_MEETINGS: usize = 2;
/// The audit lists the repair rounds of this many last meetings.
pub const REPAIR_LAST_MEETINGS: usize = 4;
pub const MEMORY_MAX_CHARS: u64 = 6000;
/// The synthesis is written by `pm`, so its failures are charged to that agent.
const SYNTHESIS_AGENT: &str = "pm";

static TIMEOUT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)timeout|timed out|etimedout").unwrap());
static MISSING_OBJECTIVE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)\bobjective\b.*must be one of the brief ids").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureClass {
    UnresolvedSource,
    InventedSource,
    MissingObjective,
    Timeout,
    SinDatos,
}

impl FailureClass {
    fn name(self) -> &'static str {
        match self {
            FailureClass::UnresolvedSource => "unresolved_source",
            FailureClass::InventedSource => "invented_source",
            FailureClass::MissingObjective => "missing_objective",
            FailureClass::Timeout => "timeout",
            FailureClass::SinDatos => "sin_datos",
        }
    }
}

#[derive(Deserialize, Default)]
struct CheckOutcome {
    #[serde(default)]
    passed: bool,
    #[serde(default)]
    failures: Vec<String>,
}

#[derive(Deserialize, Default)]
struct AgentLog {
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    cost_usd: f64,
    #[serde(default)]
    unresolved_sources: Vec<Value>,
    #[serde(default)]
    sin_datos: u64,
}

#[derive(Deserialize, Default)]
struct SynthesisLog {
    #[serde(default)]
    first_check: CheckOutcome,
    #[serde(default)]
    repaired: bool,
    #[serde(default)]
    invented_sources: Vec<Value>,
    #[serde(default)]
    cost_usd: f64,
}

/// One line of the meeting log. The agents keep the order of the stored text.
#[derive(Deserialize)]
struct LogEntry {
    minutes_id: String,
    #[serde(default)]
    created_at: String,
    agents: Ordered<AgentLog>,
    #[serde(default)]
    synthesis: SynthesisLog,
    #[serde(default)]
    total_cost_usd: f64,
}

struct MarkedMinutes {
    id: String,
    created_at: String,
    priorities: Vec<(String, Option<String>)>,
    /// `check.repaired`, when the minutes has a `check`.
    repaired: Option<bool>,
}

fn failure_classes(entry: &LogEntry) -> Vec<(String, Vec<FailureClass>)> {
    let mut out: Vec<(String, Vec<FailureClass>)> = Vec::new();
    let mut add = |agent: &str, class: FailureClass| match out.iter_mut().find(|(a, _)| a == agent)
    {
        Some((_, classes)) => classes.push(class),
        None => out.push((agent.to_string(), vec![class])),
    };
    for (agent, a) in &entry.agents.0 {
        if !a.unresolved_sources.is_empty() {
            add(agent, FailureClass::UnresolvedSource);
        }
        if a.error.as_deref().is_some_and(|e| TIMEOUT.is_match(e)) {
            add(agent, FailureClass::Timeout);
        }
        if a.sin_datos >= SIN_DATOS_FAILURE {
            add(agent, FailureClass::SinDatos);
        }
    }
    if !entry.synthesis.invented_sources.is_empty() {
        add(SYNTHESIS_AGENT, FailureClass::InventedSource);
    }
    if entry
        .synthesis
        .first_check
        .failures
        .iter()
        .any(|f| MISSING_OBJECTIVE.is_match(f))
    {
        add(SYNTHESIS_AGENT, FailureClass::MissingObjective);
    }
    out
}

/// Evidence that earlier reflections already processed.
#[derive(Default)]
struct Consumed {
    ids: BTreeSet<String>,
    /// Lines written before reflections recorded `consumed` count everything up to their time.
    until: Option<String>,
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// What the given reflections processed. Reverts consume nothing.
fn consumed_by(lines: &[Value]) -> Consumed {
    let mut c = Consumed::default();
    for r in lines {
        let Some(created) = text(r, "created_at").filter(|t| !t.is_empty()) else {
            continue;
        };
        if is_truthy(r.get("reverts")) {
            continue;
        }
        match r.get("consumed").and_then(Value::as_array) {
            Some(ids) => c
                .ids
                .extend(ids.iter().filter_map(|i| i.as_str().map(String::from))),
            None if c.until.as_deref().is_none_or(|u| created > u) => {
                c.until = Some(created.into())
            }
            None => {}
        }
    }
    c
}

fn is_truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty(),
        _ => true,
    }
}

fn is_consumed(c: &Consumed, id: &str, at: &str) -> bool {
    c.ids.contains(id) || c.until.as_deref().is_some_and(|u| at <= u)
}

#[derive(Clone)]
struct Ready {
    agent: String,
    reason: &'static str,
    failure_class: Option<FailureClass>,
    count: u64,
}

fn ready_to_reflect(
    log: &[LogEntry],
    minutes: &[MarkedMinutes],
    consumed: &Consumed,
) -> Vec<Ready> {
    let mut ready = Vec::new();

    // The agents keep the order of their first failure, as a JavaScript `Map` does.
    let mut with: Vec<(String, Vec<(FailureClass, u64)>)> = Vec::new();
    for entry in log
        .iter()
        .filter(|e| !is_consumed(consumed, &e.minutes_id, &e.created_at))
    {
        for (agent, classes) in failure_classes(entry) {
            let i = with
                .iter()
                .position(|(a, _)| *a == agent)
                .unwrap_or_else(|| {
                    with.push((agent.clone(), Vec::new()));
                    with.len() - 1
                });
            let counts = &mut with[i].1;
            let mut unique: Vec<FailureClass> = Vec::new();
            for class in classes {
                if !unique.contains(&class) {
                    unique.push(class);
                }
            }
            for class in unique {
                match counts.iter_mut().find(|(c, _)| *c == class) {
                    Some((_, n)) => *n += 1,
                    None => counts.push((class, 1)),
                }
            }
        }
    }
    for (agent, counts) in &with {
        for (class, count) in counts {
            if *count >= READY_FAILURE_MEETINGS {
                ready.push(Ready {
                    agent: agent.clone(),
                    reason: "failure_class",
                    failure_class: Some(*class),
                    count: *count,
                });
            }
        }
    }

    let mut rejected: Vec<(String, u64)> = Vec::new();
    for m in minutes
        .iter()
        .filter(|m| !is_consumed(consumed, &m.id, &m.created_at))
    {
        for (role, mark) in &m.priorities {
            if mark.as_deref() != Some("rejected") {
                continue;
            }
            match rejected.iter_mut().find(|(r, _)| r == role) {
                Some((_, n)) => *n += 1,
                None => rejected.push((role.clone(), 1)),
            }
        }
    }
    for (agent, count) in rejected {
        if count >= READY_REJECTED_PRIORITIES {
            ready.push(Ready {
                agent,
                reason: "rejected_priorities",
                failure_class: None,
                count,
            });
        }
    }
    // A stable sort, as `Array.prototype.sort` is.
    ready.sort_by(|a, b| a.agent.cmp(&b.agent).then(b.count.cmp(&a.count)));
    ready
}

/// Agents ready for a prompt proposal, strongest evidence first, one reason each.
/// An agent's evidence excludes what the reflections up to its last proposal consumed.
fn ready_for_proposal(
    log: &[LogEntry],
    minutes: &[MarkedMinutes],
    reflections: &[Value],
) -> Vec<Ready> {
    let consumed = |agent: &str| {
        let last = reflections.iter().rposition(|r| {
            r.get("proposal").and_then(|p| text(p, "agent")) == Some(agent)
                && text(r, "created_at").is_some_and(|t| !t.is_empty())
        });
        match last {
            Some(i) => consumed_by(&reflections[..=i]),
            None => Consumed::default(),
        }
    };
    let mut agents: Vec<String> = Vec::new();
    for r in ready_to_reflect(log, minutes, &Consumed::default()) {
        if !agents.contains(&r.agent) {
            agents.push(r.agent);
        }
    }
    let mut out: Vec<Ready> = agents
        .iter()
        .flat_map(|agent| {
            ready_to_reflect(log, minutes, &consumed(agent))
                .into_iter()
                .filter(|r| r.agent == *agent)
                .take(1)
        })
        .collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.agent.cmp(&b.agent)));
    out
}

fn rate(n: usize, of: usize) -> Option<f64> {
    (of > 0).then(|| n as f64 / of as f64)
}

fn agent_stats(window: &[&LogEntry], agent: &str) -> AgentMeetingStats {
    let runs: Vec<&AgentLog> = window
        .iter()
        .filter_map(|e| e.agents.0.iter().find(|(k, _)| k == agent).map(|(_, a)| a))
        .collect();
    let n = runs.len();
    let per = |f: &dyn Fn(&AgentLog) -> f64| {
        if n > 0 {
            runs.iter().map(|r| f(r)).sum::<f64>() / n as f64
        } else {
            0.0
        }
    };
    AgentMeetingStats {
        agent: agent.into(),
        meetings: n as u64,
        answers: runs.iter().filter(|r| r.error.is_none()).count() as u64,
        errors: runs.iter().filter(|r| r.error.is_some()).count() as u64,
        timeouts: runs
            .iter()
            .filter(|r| r.error.as_deref().is_some_and(|e| TIMEOUT.is_match(e)))
            .count() as u64,
        unresolved_sources_per_meeting: per(&|r| r.unresolved_sources.len() as f64),
        sin_datos_per_meeting: per(&|r| r.sin_datos as f64),
        mean_cost_usd: per(&|r| r.cost_usd),
    }
}

/// Repairs from the minutes, which exist for every meeting.
fn repair_baseline(minutes: &[MarkedMinutes]) -> RepairComparison {
    let mut checked: Vec<&MarkedMinutes> =
        minutes.iter().filter(|m| m.repaired.is_some()).collect();
    checked.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    let group = |ms: &[&MarkedMinutes]| {
        let repaired = ms.iter().filter(|m| m.repaired == Some(true)).count();
        RepairGroup {
            minutes: ms.iter().map(|m| m.id.clone()).collect(),
            repaired: repaired as u64,
            per_meeting: rate(repaired, ms.len()),
        }
    };
    let split = REPAIR_BASELINE_MEETINGS.min(checked.len());
    RepairComparison {
        baseline: group(&checked[..split]),
        since: group(&checked[split..]),
    }
}

fn mark_counts(minutes: &[MarkedMinutes]) -> Ordered<AuditMarks> {
    let mut out: Vec<(String, AuditMarks)> = Vec::new();
    for m in minutes {
        for (role, mark) in &m.priorities {
            let i = out.iter().position(|(r, _)| r == role).unwrap_or_else(|| {
                out.push((role.clone(), AuditMarks::default()));
                out.len() - 1
            });
            let c = &mut out[i].1;
            match mark.as_deref() {
                Some("accepted") => c.accepted += 1,
                Some("changed") => c.changed += 1,
                Some("rejected") => c.rejected += 1,
                _ => c.unmarked += 1,
            }
        }
    }
    Ordered(out)
}

/// The length that `renderForPrompt(items, Infinity)` has in JavaScript string units.
fn rendered_chars(active: &[&MemoryItem]) -> u64 {
    let lines: Vec<usize> = active
        .iter()
        .map(|i| format!("- [{}] {}", i.id, i.text).encode_utf16().count())
        .collect();
    (lines.iter().sum::<usize>() + lines.len().saturating_sub(1)) as u64
}

fn memory_stats(agent: &str, items: &[MemoryItem], log: &[LogEntry]) -> AgentMemoryStats {
    let active: Vec<&MemoryItem> = items.iter().filter(|i| i.retired_at.is_none()).collect();
    let after = |at: &str| log.iter().filter(|e| e.created_at.as_str() > at).count();
    AgentMemoryStats {
        agent: agent.into(),
        active: active.len() as u64,
        retired: (items.len() - active.len()) as u64,
        lessons: active.iter().filter(|i| i.kind == "lesson").count() as u64,
        never_helpful: active
            .iter()
            .filter(|i| i.helpful == 0 && after(&i.created_at) >= NEVER_HELPFUL_MEETINGS)
            .count() as u64,
        chars: rendered_chars(&active),
        max_chars: MEMORY_MAX_CHARS,
    }
}

struct Input<'a> {
    product: &'a Product,
    roster_failures: Vec<String>,
    brief: Option<&'a Value>,
    log: Vec<LogEntry>,
    minutes: Vec<MarkedMinutes>,
    memory: BTreeMap<String, Vec<MemoryItem>>,
    reflections: Vec<Value>,
    now_ms: i64,
}

fn build(input: Input) -> AuditReport {
    let Input {
        product,
        log,
        minutes,
        reflections,
        ..
    } = &input;
    let window: Vec<&LogEntry> = log[log.len().saturating_sub(AUDIT_WINDOW)..]
        .iter()
        .collect();
    let now = clock::iso(input.now_ms);
    let today = &now[..10];

    let mut agents: Vec<&str> = Vec::new();
    let from_log = window
        .iter()
        .flat_map(|e| e.agents.0.iter().map(|(k, _)| k.as_str()));
    for name in product.roles.iter().map(String::as_str).chain(from_log) {
        if !agents.contains(&name) {
            agents.push(name);
        }
    }
    let total: f64 = window.iter().map(|e| e.total_cost_usd).sum();

    let lines: Vec<&Value> = reflections
        .iter()
        .filter(|r| text(r, "created_at").is_some_and(|t| !t.is_empty()))
        .collect();
    let last_entry = lines
        .iter()
        .rev()
        .find(|r| is_truthy(r.get("id")) && !is_truthy(r.get("reverts")));

    let memory_agents: BTreeSet<&str> = product
        .roles
        .iter()
        .map(String::as_str)
        .chain(input.memory.keys().map(String::as_str))
        .collect();

    let overdue = input
        .brief
        .and_then(|b| b.get("objectives"))
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter(|o| text(o, "due").is_some_and(|d| d < today))
        .map(|o| OverdueObjective {
            id: text(o, "id").unwrap_or_default().into(),
            due: text(o, "due").unwrap_or_default().into(),
            text: text(o, "text").unwrap_or_default().into(),
        })
        .collect();

    AuditReport {
        product: product.id.clone(),
        generated_at: now.clone(),
        config: AuditConfig {
            valid: input.roster_failures.is_empty(),
            failures: input.roster_failures.clone(),
            overdue_objectives: overdue,
        },
        meetings: MeetingStats {
            count: window.len() as u64,
            first_pass_rate: rate(
                window
                    .iter()
                    .filter(|e| e.synthesis.first_check.passed)
                    .count(),
                window.len(),
            ),
            repair_rate: rate(
                window.iter().filter(|e| e.synthesis.repaired).count(),
                window.len(),
            ),
            synthesis_cost_share: (total != 0.0)
                .then(|| window.iter().map(|e| e.synthesis.cost_usd).sum::<f64>() / total),
            invented_sources: window
                .iter()
                .map(|e| e.synthesis.invented_sources.len() as u64)
                .sum(),
            agents: agents.iter().map(|a| agent_stats(&window, a)).collect(),
        },
        repairs: repair_baseline(minutes),
        last_repairs: (log.len() >= REPAIR_LAST_MEETINGS).then(|| {
            log[log.len() - REPAIR_LAST_MEETINGS..]
                .iter()
                .map(|e| LastRepair {
                    minutes_id: e.minutes_id.clone(),
                    rounds: u64::from(e.synthesis.repaired),
                })
                .collect()
        }),
        marks: mark_counts(minutes),
        memory: memory_agents
            .iter()
            .map(|a| memory_stats(a, input.memory.get(*a).map_or(&[][..], Vec::as_slice), log))
            .collect(),
        reflections: ReflectionStats {
            last: last_entry.map(|r| LastReflection {
                id: text(r, "id").unwrap_or_default().into(),
                created_at: text(r, "created_at").unwrap_or_default().into(),
            }),
            pending_proposals: reflections
                .iter()
                .filter(|r| {
                    r.get("proposal").and_then(|p| text(p, "status")) == Some("pending")
                        && text(r, "status") != Some("reverted")
                })
                .count() as u64,
            reverted: reflections
                .iter()
                .filter(|r| is_truthy(r.get("reverts")))
                .count() as u64,
        },
        ready_to_reflect: ready_for_proposal(log, minutes, reflections)
            .into_iter()
            .map(|r| ReadyAgent {
                agent: r.agent,
                reason: r.reason.into(),
                failure_class: r.failure_class.map(|c| c.name().into()),
                count: r.count,
            })
            .collect(),
    }
}

fn read_minutes(c: &Connection, product: &str) -> Result<Vec<MarkedMinutes>> {
    let mut stmt = c.prepare_cached(
        "select data from minutes where product = ?1 and data is not null order by id",
    )?;
    let rows: Vec<String> = stmt
        .query_map([product], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows
        .iter()
        .filter_map(|text| serde_json::from_str::<Value>(text).ok())
        .map(|m| MarkedMinutes {
            id: text(&m, "id").unwrap_or_default().into(),
            created_at: text(&m, "created_at").unwrap_or_default().into(),
            priorities: m
                .get("priorities")
                .and_then(Value::as_array)
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .map(|p| {
                    (
                        text(p, "owner_role").unwrap_or_default().into(),
                        text(p, "mark").map(String::from),
                    )
                })
                .collect(),
            repaired: m
                .get("check")
                .filter(|c| !c.is_null())
                .map(|c| is_truthy(c.get("repaired"))),
        })
        .collect())
}

fn read_log(c: &Connection, product: &str) -> Result<Vec<LogEntry>> {
    let mut stmt =
        c.prepare_cached("select entry from meeting_log where product = ?1 order by position")?;
    let rows: Vec<String> = stmt
        .query_map([product], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    // A stored row that is not a log entry is skipped, as `readMeetingLog` does.
    Ok(rows
        .iter()
        .filter_map(|text| serde_json::from_str::<LogEntry>(text).ok())
        .collect())
}

/// Reads the data of the product and reports its health.
pub fn audit(c: &Connection, root: &Path, product: &Product, now_ms: i64) -> Result<AuditReport> {
    let roster_failures = roster::validate_roster(c, root, product)?;
    let brief = read::brief(c, &product.id)?;

    let mut agents: BTreeSet<String> = product.roles.iter().cloned().collect();
    agents.extend(read::memory_agents(c, &product.id)?);
    let mut memory = BTreeMap::new();
    for agent in agents {
        memory.insert(agent.clone(), roster::memory_items(c, &product.id, &agent)?);
    }
    Ok(build(Input {
        product,
        roster_failures,
        brief: brief.as_ref(),
        log: read_log(c, &product.id)?,
        minutes: read_minutes(c, &product.id)?,
        memory,
        reflections: read::reflections(c, &product.id)?,
        now_ms,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(json: &str) -> LogEntry {
        serde_json::from_str(json).unwrap()
    }

    fn unresolved(id: &str, agent: &str) -> LogEntry {
        entry(&format!(
            r#"{{"minutes_id":"{id}","created_at":"2026-10-01T00:00:00Z","agents":{{"{agent}":{{"unresolved_sources":[{{}}]}}}}}}"#
        ))
    }

    fn minutes(
        id: &str,
        created: &str,
        repaired: Option<bool>,
        marks: &[(&str, Option<&str>)],
    ) -> MarkedMinutes {
        MarkedMinutes {
            id: id.into(),
            created_at: created.into(),
            priorities: marks
                .iter()
                .map(|(r, m)| ((*r).into(), m.map(String::from)))
                .collect(),
            repaired,
        }
    }

    #[test]
    fn an_agent_is_ready_after_the_same_failure_in_three_meetings() {
        let log = vec![
            unresolved("m1", "dev"),
            unresolved("m22", "dev"),
            unresolved("m3", "qa"),
        ];
        assert!(ready_to_reflect(&log, &[], &Consumed::default()).is_empty());
        let log = vec![
            unresolved("m1", "dev"),
            unresolved("m22", "dev"),
            unresolved("m333", "dev"),
        ];
        let ready = ready_to_reflect(&log, &[], &Consumed::default());
        assert_eq!(ready.len(), 1);
        assert_eq!((ready[0].agent.as_str(), ready[0].count), ("dev", 3));
        assert_eq!(ready[0].failure_class, Some(FailureClass::UnresolvedSource));
    }

    #[test]
    fn evidence_that_a_proposal_consumed_does_not_count_again() {
        let log = vec![
            unresolved("m1", "dev"),
            unresolved("m22", "dev"),
            unresolved("m333", "dev"),
        ];
        let reflections: Vec<Value> = vec![serde_json::json!({
            "id": "r-1", "created_at": "2026-10-09T00:00:00Z",
            "consumed": ["m1"], "proposal": {"agent": "dev", "status": "pending"}
        })];
        assert!(ready_for_proposal(&log, &[], &reflections).is_empty());
        assert_eq!(ready_for_proposal(&log, &[], &[]).len(), 1);
    }

    #[test]
    fn three_rejected_priorities_of_one_role_make_it_ready() {
        let ms = vec![
            minutes(
                "a",
                "1",
                None,
                &[("dev", Some("rejected")), ("dev", Some("rejected"))],
            ),
            minutes("b", "2", None, &[("dev", Some("rejected")), ("pm", None)]),
        ];
        let ready = ready_to_reflect(&[], &ms, &Consumed::default());
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].reason, "rejected_priorities");
        let marks = mark_counts(&ms);
        assert_eq!(marks.0[0].0, "dev");
        assert_eq!(marks.0[1].1.unmarked, 1);
    }

    #[test]
    fn the_repair_baseline_takes_the_first_two_meetings_by_date() {
        let ms = vec![
            minutes("c", "3", Some(true), &[]),
            minutes("a", "1", Some(false), &[]),
            minutes("b", "2", Some(true), &[]),
            minutes("d", "4", None, &[]),
        ];
        let r = repair_baseline(&ms);
        assert_eq!(r.baseline.minutes, ["a", "b"]);
        assert_eq!(r.baseline.per_meeting, Some(0.5));
        assert_eq!(r.since.minutes, ["c"]);
        assert_eq!(r.since.per_meeting, Some(1.0));
        assert_eq!(repair_baseline(&[]).since.per_meeting, None);
    }

    #[test]
    fn an_agent_repeated_in_the_roster_is_reported_once() {
        let product = Product {
            id: "p".into(),
            name: "P".into(),
            paths: vec![],
            status: "active".into(),
            roles: vec!["dev".into(), "dev".into(), "pm".into()],
        };
        let report = build(Input {
            product: &product,
            roster_failures: vec![],
            brief: None,
            log: vec![],
            minutes: vec![],
            memory: BTreeMap::new(),
            reflections: vec![],
            now_ms: 0,
        });
        let names: Vec<_> = report
            .meetings
            .agents
            .iter()
            .map(|a| a.agent.as_str())
            .collect();
        assert_eq!(names, ["dev", "pm"]);
    }

    #[test]
    fn the_memory_length_counts_the_characters_of_the_prompt() {
        let item = |id: &str, text: &str, retired: Option<&str>| MemoryItem {
            id: id.into(),
            text: text.into(),
            kind: "fact".into(),
            helpful: 0,
            harmful: 0,
            source: "s".into(),
            created_at: "2026-10-01".into(),
            retired_at: retired.map(String::from),
            retired_why: None,
        };
        let items = vec![
            item("m-1", "ab", None),
            item("m-2", "cd", None),
            item("m-3", "zz", Some("x")),
        ];
        let stats = memory_stats("dev", &items, &[]);
        assert_eq!((stats.active, stats.retired), (2, 1));
        // "- [m-1] ab" is 10 characters; two lines and one newline.
        assert_eq!(stats.chars, 21);
        assert_eq!(memory_stats("none", &[], &[]).chars, 0);
    }
}
