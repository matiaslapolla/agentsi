//! Replay: runs one agent again on frozen meetings, keeps the other answers, runs the
//! synthesis and the check again, and compares the figures. The rules copy
//! `replay.ts`.
//!
//! Replay never calls `run_meeting`. It builds the calls and the minutes with the
//! parts of a meeting and holds no write path, so it cannot touch the record of the
//! owner. The test `a_replay_writes_no_row` compares each table before and after.

use super::{SYNTHESIS_AGENT, list, text};
use crate::meeting::check::{SourceContext, check, resolve_source};
use crate::meeting::claude::ClaudeResult;
use crate::meeting::log::{invented_sources, sha256, sin_datos};
use crate::meeting::minutes::{MinutesMeta, build_minutes, empty_answer, to_role_answer};
use crate::meeting::product_cwd;
use crate::meeting::prompts::{
    RoleContext, SynthesisContext, compact_snapshot, role_call, synthesis_call,
};
use crate::registry::Product;
use crate::story::claude::{Call, default_budget_usd, js_number};
use agentsi_proto::agency_api::Owner;
use agentsi_proto::reflect_api::{
    CaseMetrics, ReplayPair, ReplayReport, ReplayRow, ReplayTotals, ReplayVerdict, to_fixed,
};
use agentsi_proto::roles_api::RoleDef;
use agentsi_store::Store;
use regex::Regex;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

pub const REPLAY_MIN_CASES: usize = 4;
/// The held-out half needs this many cases to compare anything.
pub const REPLAY_MIN_HELD_OUT: usize = 2;
pub const REPLAY_BUDGET_USD: f64 = 15.0;
/// The cost of the agent counts as worse only when it rises more than this share.
pub const COST_TOLERANCE: f64 = 0.25;
pub const COST_RULE: &str = "cost counts as worse only above +25%; a lower cost is not an improvement (it is a budget constraint, not a goal)";

static MISSING_OBJECTIVE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)\bobjective\b.*must be one of the brief ids").unwrap());

/// One frozen meeting.
#[derive(Debug, Clone)]
pub struct ReplayCase {
    pub minutes_id: String,
    pub log: Value,
    pub minutes: Value,
    pub snapshot: String,
}

/// Meetings with a frozen snapshot and saved minutes in which the agent answered.
pub fn replay_cases(
    c: &Connection,
    product: &str,
    agent: &str,
    log: &[Value],
    minutes: &[Value],
) -> agentsi_store::Result<Vec<ReplayCase>> {
    let mut stmt = c.prepare_cached("select minutes_id, body from snapshots where product = ?1")?;
    let snapshots: HashMap<String, String> = stmt
        .query_map([product], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<std::result::Result<_, _>>()?;
    let mut out = Vec::new();
    for entry in log {
        let id = text(entry, "minutes_id");
        let Some(run) = entry.get("agents").and_then(|a| a.get(agent)) else {
            continue;
        };
        let Some(stored) = minutes.iter().find(|m| text(m, "id") == id) else {
            continue;
        };
        if run.get("error") != Some(&Value::Null) {
            continue;
        }
        // A product with stored snapshots must have this one. One with none replays with an empty snapshot.
        let snapshot = snapshots.get(id);
        if snapshot.is_none() && !snapshots.is_empty() {
            continue;
        }
        if text(&stored["roles"][agent], "answer").trim().is_empty() {
            continue;
        }
        out.push(ReplayCase {
            minutes_id: id.into(),
            log: entry.clone(),
            minutes: stored.clone(),
            snapshot: snapshot.cloned().unwrap_or_default(),
        });
    }
    Ok(out)
}

/// Held out when the first byte of sha256(minutes id) is even: a case never changes
/// half as cases accumulate.
pub fn is_held_out(minutes_id: &str) -> bool {
    u8::from_str_radix(&sha256(minutes_id)[..2], 16).is_ok_and(|b| b % 2 == 0)
}

/// The tuning half, then the held-out half.
pub fn split_cases(cases: Vec<ReplayCase>) -> (Vec<ReplayCase>, Vec<ReplayCase>) {
    let (held_out, tuning) = cases.into_iter().partition(|c| is_held_out(&c.minutes_id));
    (tuning, held_out)
}

/// Why the cases cannot back a proposal, or `None` when they can.
pub fn insufficient_cases(tuning: usize, held_out: usize) -> Option<String> {
    let n = tuning + held_out;
    if n < REPLAY_MIN_CASES {
        return Some(format!("{n} casos de replay (mínimo {REPLAY_MIN_CASES})"));
    }
    if held_out < REPLAY_MIN_HELD_OUT {
        return Some(format!(
            "{held_out} casos held-out (mínimo {REPLAY_MIN_HELD_OUT})"
        ));
    }
    if tuning == 0 {
        return Some("ningún caso de tuning para mostrarle a la propuesta".into());
    }
    None
}

pub fn is_missing_objective(failure: &str) -> bool {
    MISSING_OBJECTIVE.is_match(failure)
}

/// The figures of the stored meeting, from its log line.
pub fn stored_metrics(entry: &Value, agent: &str) -> CaseMetrics {
    let run = &entry["agents"][agent];
    let synthesis = &entry["synthesis"];
    CaseMetrics {
        first_pass: synthesis["first_check"]["passed"] == Value::Bool(true),
        unresolved: list(run, "unresolved_sources").len() as u64,
        invented: list(synthesis, "invented_sources").len() as u64,
        missing_objectives: list(&synthesis["first_check"], "failures")
            .iter()
            .filter(|f| is_missing_objective(f.as_str().unwrap_or_default()))
            .count() as u64,
        sin_datos: run["sin_datos"].as_u64().unwrap_or(0),
        cost_usd: run["cost_usd"].as_f64().unwrap_or(0.0),
    }
}

pub fn totals(ms: &[&CaseMetrics]) -> ReplayTotals {
    let n = ms.len();
    let over_n = |sum: f64| if n == 0 { 0.0 } else { sum / n as f64 };
    ReplayTotals {
        cases: n as u64,
        first_pass_rate: over_n(ms.iter().filter(|m| m.first_pass).count() as f64),
        unresolved_per_case: over_n(ms.iter().map(|m| m.unresolved).sum::<u64>() as f64),
        invented: ms.iter().map(|m| m.invented).sum(),
        missing_objectives: ms.iter().map(|m| m.missing_objectives).sum(),
        sin_datos: ms.iter().map(|m| m.sin_datos).sum(),
        cost_usd: ms.iter().fold(0.0, |a, m| a + m.cost_usd),
    }
}

/// Passes when no metric is worse on the held-out half and at least one is better.
pub fn compare_totals(before: &ReplayTotals, after: &ReplayTotals) -> ReplayVerdict {
    let mut worse: Vec<String> = Vec::new();
    let mut better: Vec<String> = Vec::new();
    let mut compare = |name: &str, b: f64, a: f64, more_is_worse: bool| {
        let (bad, good) = if more_is_worse {
            (a > b, a < b)
        } else {
            (a < b, a > b)
        };
        if bad {
            worse.push(name.into());
        }
        if good {
            better.push(name.into());
        }
    };
    compare(
        "first_pass_rate",
        before.first_pass_rate,
        after.first_pass_rate,
        false,
    );
    compare(
        "unresolved_per_case",
        before.unresolved_per_case,
        after.unresolved_per_case,
        true,
    );
    compare(
        "invented",
        before.invented as f64,
        after.invented as f64,
        true,
    );
    compare(
        "missing_objectives",
        before.missing_objectives as f64,
        after.missing_objectives as f64,
        true,
    );
    // Guard only: a fall in sin_datos is never counted as better.
    if after.sin_datos > before.sin_datos {
        worse.push("sin_datos".into());
    }
    if before.cost_usd > 0.0 && after.cost_usd > before.cost_usd * (1.0 + COST_TOLERANCE) {
        worse.push("cost_usd".into());
    }
    ReplayVerdict {
        passed: worse.is_empty() && !better.is_empty(),
        worse,
        better,
    }
}

/// Why a paid call did not give an answer.
#[derive(Debug, PartialEq, Eq)]
pub enum CallFailure {
    /// The cap of the replay stops the call. It ends the whole replay.
    Budget(String),
    Failed(String),
}

#[derive(Default)]
struct Spending {
    spent: f64,
    reserved: f64,
}

/// A spending cap over several calls. A call starts only when its own cap fits in
/// what is left, so the total never exceeds the limit. A failed call is charged its
/// whole cap, because its real cost is unknown.
pub struct Budget {
    pub limit: f64,
    state: Mutex<Spending>,
}

impl Budget {
    pub fn new(limit: f64) -> Budget {
        Budget {
            limit,
            state: Mutex::default(),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, Spending> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn spent(&self) -> f64 {
        self.state().spent
    }

    pub fn run(&self, call: &Caller, c: &Call) -> std::result::Result<ClaudeResult, CallFailure> {
        let cap = default_budget_usd(&c.model);
        {
            let mut s = self.state();
            if s.spent + s.reserved + cap > self.limit {
                return Err(CallFailure::Budget(format!(
                    "replay budget of {} USD exhausted ({} spent)",
                    js_number(self.limit),
                    to_fixed(s.spent, 2)
                )));
            }
            s.reserved += cap;
        }
        let result = call(c);
        let mut s = self.state();
        s.reserved -= cap;
        match result {
            Ok(r) => {
                s.spent += r.usage.cost_usd;
                Ok(r)
            }
            Err(e) => {
                s.spent += cap;
                Err(CallFailure::Failed(e))
            }
        }
    }
}

/// One paid call of `claude`. The error is the text of the failure.
pub type Caller<'a> = dyn Fn(&Call) -> std::result::Result<ClaudeResult, String> + Sync + 'a;

pub struct ReplayInput<'a> {
    pub store: &'a Store,
    pub product: &'a Product,
    pub agent: &'a str,
    /// The agent with the proposed prompt.
    pub candidate: &'a RoleDef,
    /// The agent with the prompt of today.
    pub current: &'a RoleDef,
    /// The pm of today, which writes the synthesis unless the agent is the pm.
    pub pm: &'a RoleDef,
    /// The memory of the agent as its prompt gets it.
    pub memory: &'a str,
    pub brief: Option<&'a Value>,
    pub owner: &'a Owner,
    pub held_out: &'a [ReplayCase],
    pub budget: &'a Budget,
    pub call: &'a Caller<'a>,
    pub sources: &'a SourceContext,
}

/// The answers of the meeting, with `answer` in the place of the agent.
fn with_answer(minutes: &Value, agent: &str, answer: Value) -> Vec<(String, Value)> {
    let mut roles: Vec<(String, Value)> = minutes
        .get("roles")
        .and_then(Value::as_object)
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    match roles.iter_mut().find(|(k, _)| k == agent) {
        Some((_, slot)) => *slot = answer,
        None => roles.push((agent.into(), answer)),
    }
    roles
}

/// Runs one agent on a frozen meeting, keeps the other answers, runs the synthesis
/// and the check again. It writes nothing.
fn run_case(
    def: &RoleDef,
    c: &ReplayCase,
    i: &ReplayInput,
) -> std::result::Result<CaseMetrics, CallFailure> {
    let scope = c.log.get("scope").and_then(Value::as_str);
    let cwd = product_cwd(i.product, scope).map_err(|e| CallFailure::Failed(e.to_string()))?;
    let question = text(&c.minutes, "question");
    let role_context = RoleContext {
        question,
        snapshot: &c.snapshot,
        memory: i.memory,
        scope,
        cwd: &cwd,
        brief: i.brief,
        owner: i.owner,
        code_graph: &[],
    };

    let (answer, cost) = match i.budget.run(i.call, &role_call(def, &role_context, true)) {
        Ok(r) => (to_role_answer(&r.output, &r.usage.model), r.usage.cost_usd),
        Err(stop @ CallFailure::Budget(_)) => return Err(stop),
        Err(CallFailure::Failed(m)) => (empty_answer(&def.model, &m), 0.0),
    };

    let unresolved = list(&answer, "priorities")
        .iter()
        .flat_map(|p| list(p, "sources"))
        .filter(|s| !resolve_source(s, i.sources))
        .count() as u64;
    let no_data = sin_datos(&answer) as u64;

    let roles = with_answer(&c.minutes, i.agent, answer);
    let pm = if i.agent == SYNTHESIS_AGENT {
        def
    } else {
        i.pm
    };
    let compact = compact_snapshot(&c.snapshot);
    let synthesis_context = SynthesisContext {
        question,
        answers: &roles,
        compact: &compact,
        cwd: &cwd,
        brief: i.brief,
        owner: i.owner,
    };
    let synthesis = match i
        .budget
        .run(i.call, &synthesis_call(pm, &synthesis_context, None))
    {
        Ok(r) => Some(r.output),
        Err(stop @ CallFailure::Budget(_)) => return Err(stop),
        Err(CallFailure::Failed(_)) => None,
    };

    let meta = MinutesMeta {
        id: text(&c.minutes, "id").into(),
        product: text(&c.minutes, "product").into(),
        scope: scope.map(String::from),
        question: question.into(),
        created_at: text(&c.minutes, "created_at").into(),
    };
    let m = build_minutes(&meta, &roles, synthesis.as_ref(), false, &[]);
    let attending: Vec<String> = c
        .log
        .get("agents")
        .and_then(Value::as_object)
        .map(|a| a.keys().cloned().collect())
        .unwrap_or_default();
    let result = i
        .store
        .read(|conn| Ok(check(conn, &m, i.product, Some(&attending), i.brief)))
        .map_err(|e| CallFailure::Failed(e.to_string()))?;
    Ok(CaseMetrics {
        first_pass: result.passed,
        unresolved,
        invented: invented_sources(&m, &c.snapshot).len() as u64,
        missing_objectives: result
            .failures
            .iter()
            .filter(|f| is_missing_objective(f))
            .count() as u64,
        sin_datos: no_data,
        cost_usd: cost,
    })
}

fn replay_case(
    c: &ReplayCase,
    i: &ReplayInput,
    today: &str,
) -> std::result::Result<ReplayRow, CallFailure> {
    let stored = text(&c.log["agents"][i.agent], "prompt_hash") == today;
    let before = if stored {
        stored_metrics(&c.log, i.agent)
    } else {
        run_case(i.current, c, i)?
    };
    let after = run_case(i.candidate, c, i)?;
    Ok(ReplayRow {
        minutes_id: c.minutes_id.clone(),
        baseline: if stored { "stored" } else { "rerun" }.into(),
        before,
        after,
    })
}

/// The fields of a report that a replay fills.
#[derive(Debug, Default)]
pub struct ReplayRun {
    pub rows: Vec<ReplayRow>,
    pub totals: Option<ReplayPair>,
    pub verdict: ReplayVerdict,
    pub aborted: Option<String>,
}

impl ReplayRun {
    pub fn into_report(self, mut report: ReplayReport) -> ReplayReport {
        report.rows = self.rows;
        report.totals = self.totals;
        report.verdict = self.verdict;
        report.aborted = self.aborted;
        report
    }
}

/// Compares the proposed prompt with the baseline on the held-out cases, one thread
/// for each case. The baseline is the stored meeting, run again with the prompt of
/// today only when the stored prompt hash differs from it. The error is a failure
/// that is not the budget.
pub fn replay(i: &ReplayInput) -> std::result::Result<ReplayRun, String> {
    let today = sha256(&i.current.body);
    let settled: Vec<std::result::Result<ReplayRow, CallFailure>> = std::thread::scope(|scope| {
        let handles: Vec<_> = i
            .held_out
            .iter()
            .map(|c| {
                let today = &today;
                scope.spawn(move || replay_case(c, i, today))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("a case thread does not panic"))
            .collect()
    });
    let mut rows = Vec::new();
    for result in settled {
        match result {
            Ok(row) => rows.push(row),
            Err(CallFailure::Failed(m)) => return Err(m),
            Err(CallFailure::Budget(m)) => {
                return Ok(ReplayRun {
                    aborted: Some(m),
                    ..ReplayRun::default()
                });
            }
        }
    }
    let before = totals(&rows.iter().map(|r| &r.before).collect::<Vec<_>>());
    let after = totals(&rows.iter().map(|r| &r.after).collect::<Vec<_>>());
    let verdict = compare_totals(&before, &after);
    Ok(ReplayRun {
        rows,
        totals: Some(ReplayPair { before, after }),
        verdict,
        aborted: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsi_proto::meeting_api::MeetingUsage;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn metrics(cost: f64) -> CaseMetrics {
        CaseMetrics {
            first_pass: true,
            cost_usd: cost,
            ..CaseMetrics::default()
        }
    }

    fn case(id: &str) -> ReplayCase {
        ReplayCase {
            minutes_id: id.into(),
            log: Value::Null,
            minutes: Value::Null,
            snapshot: String::new(),
        }
    }

    fn result(cost: f64) -> ClaudeResult {
        ClaudeResult {
            output: json!({}),
            usage: MeetingUsage {
                role: String::new(),
                model: String::new(),
                input_tokens: 0,
                output_tokens: 0,
                cache_read: 0,
                cache_write: 0,
                cost_usd: cost,
                duration_ms: 0,
            },
        }
    }

    fn call(model: &str) -> Call {
        Call {
            prompt: String::new(),
            model: model.into(),
            system: String::new(),
            cwd: String::new(),
            tools: Vec::new(),
            schema: None,
            effort: None,
            timeout: std::time::Duration::from_secs(1),
            mcp: None,
        }
    }

    #[test]
    fn a_case_keeps_its_half_as_cases_accumulate() {
        assert!(is_held_out("a") && is_held_out("b"));
        let few = split_cases(vec![case("a"), case("x")]);
        let many = split_cases(vec![case("a"), case("x"), case("b"), case("y")]);
        assert!(few.1.iter().any(|c| c.minutes_id == "a"));
        assert!(many.1.iter().any(|c| c.minutes_id == "a"));
    }

    #[test]
    fn a_proposal_needs_4_cases_2_held_out_and_1_for_tuning() {
        assert_eq!(
            insufficient_cases(1, 2).unwrap(),
            "3 casos de replay (mínimo 4)"
        );
        assert_eq!(
            insufficient_cases(3, 1).unwrap(),
            "1 casos held-out (mínimo 2)"
        );
        assert_eq!(
            insufficient_cases(0, 4).unwrap(),
            "ningún caso de tuning para mostrarle a la propuesta"
        );
        assert_eq!(insufficient_cases(2, 2), None);
    }

    #[test]
    fn the_guards_need_nothing_worse_and_one_metric_better() {
        let base = ReplayTotals {
            cases: 2,
            first_pass_rate: 0.5,
            unresolved_per_case: 1.0,
            invented: 1,
            missing_objectives: 1,
            sin_datos: 1,
            cost_usd: 1.0,
        };
        let better = ReplayTotals {
            first_pass_rate: 1.0,
            ..base.clone()
        };
        let v = compare_totals(&base, &better);
        assert!(v.passed);
        assert_eq!(v.better, ["first_pass_rate"]);

        assert!(!compare_totals(&base, &base).passed);

        let costly = ReplayTotals {
            first_pass_rate: 1.0,
            cost_usd: 1.26,
            ..base.clone()
        };
        let v = compare_totals(&base, &costly);
        assert!(!v.passed);
        assert_eq!(v.worse, ["cost_usd"]);

        let within = ReplayTotals {
            cost_usd: 1.25,
            first_pass_rate: 1.0,
            ..base.clone()
        };
        assert!(compare_totals(&base, &within).passed);

        let cheaper = ReplayTotals {
            cost_usd: 0.1,
            ..base.clone()
        };
        let v = compare_totals(&base, &cheaper);
        assert!(!v.passed && v.better.is_empty());

        let no_data = ReplayTotals {
            first_pass_rate: 1.0,
            sin_datos: 2,
            ..base.clone()
        };
        assert_eq!(compare_totals(&base, &no_data).worse, ["sin_datos"]);

        let less_no_data = ReplayTotals {
            sin_datos: 0,
            ..base.clone()
        };
        assert!(compare_totals(&base, &less_no_data).better.is_empty());
    }

    #[test]
    fn stored_figures_come_from_the_log_line() {
        let entry = json!({
            "agents": {"dev": {"unresolved_sources": [{"ref": "a"}], "cost_usd": 0.5}},
            "synthesis": {
                "first_check": {"passed": false, "failures": [
                    "Priority 1: objective \"x\" must be one of the brief ids (o1)",
                    "other"
                ]},
                "invented_sources": [{"ref": "b"}, {"ref": "c"}],
            },
        });
        let m = stored_metrics(&entry, "dev");
        assert!(!m.first_pass);
        assert_eq!(m.unresolved, 1);
        assert_eq!(m.invented, 2);
        assert_eq!(m.missing_objectives, 1);
        assert_eq!(m.sin_datos, 0, "an old line without the field reads 0");
        assert_eq!(m.cost_usd, 0.5);
    }

    #[test]
    fn totals_average_the_rates_and_add_the_counts() {
        let (a, mut b) = (metrics(0.25), metrics(0.5));
        b.first_pass = false;
        b.unresolved = 3;
        b.invented = 2;
        let t = totals(&[&a, &b]);
        assert_eq!(t.first_pass_rate, 0.5);
        assert_eq!(t.unresolved_per_case, 1.5);
        assert_eq!(t.invented, 2);
        assert_eq!(t.cost_usd, 0.75);
        assert_eq!(totals(&[]).first_pass_rate, 0.0);
    }

    #[test]
    fn the_budget_starts_a_call_only_when_its_cap_fits() {
        let budget = Budget::new(0.5);
        let calls = AtomicUsize::new(0);
        let caller = |_: &Call| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(result(0.25))
        };
        budget.run(&caller, &call("sonnet")).unwrap();
        assert_eq!(budget.spent(), 0.25);
        let stopped = budget.run(&caller, &call("sonnet"));
        assert!(matches!(
            stopped,
            Err(CallFailure::Budget(m)) if m == "replay budget of 0.5 USD exhausted (0.25 spent)"
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        budget.run(&caller, &call("haiku")).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_failed_call_is_charged_its_whole_cap() {
        let budget = Budget::new(1.0);
        let caller = |_: &Call| Err("boom".to_string());
        let failed = budget.run(&caller, &call("sonnet"));
        assert!(matches!(failed, Err(CallFailure::Failed(m)) if m == "boom"));
        assert_eq!(budget.spent(), 0.4);
    }

    #[test]
    fn a_new_answer_keeps_the_place_of_the_agent() {
        let minutes = json!({"roles": {"pm": {"a": 1}, "dev": {"a": 2}, "qa": {"a": 3}}});
        let roles = with_answer(&minutes, "dev", json!({"a": 9}));
        let names: Vec<&str> = roles.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["pm", "dev", "qa"]);
        assert_eq!(roles[1].1, json!({"a": 9}));
    }
}
