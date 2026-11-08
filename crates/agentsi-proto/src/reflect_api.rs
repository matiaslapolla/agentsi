//! The methods of `agentsi reflect` and of the Memory screen (U25): `reflect.run`,
//! `reflect.revert`, `reflect.cancel`, `memory.list` and `memory.retire`.
//!
//! The field order of each struct is the key order of `agentsi reflect --json`.
//! A stored entry can be older than a field, so each struct reads a missing field
//! as its default.

use crate::method::{Catalog, Method};
use crate::rpc::ErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub fn register(c: &mut Catalog) {
    c.method::<ReflectRun>()
        .method::<ReflectRevert>()
        .method::<ReflectCancel>()
        .method::<MemoryList>()
        .method::<MemoryRetire>();
}

/// One grounded signal: a failure or a mark that a reflection can learn from.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Signal {
    pub agent: String,
    /// `first_check_failure`, `unresolved_source`, `invented_source`, `error`,
    /// `sin_datos`, `mark_<mark>`, `decision_rejected` or `reverted_reflection`.
    pub kind: String,
    /// A minutes id, or a reflection id for `reverted_reflection`.
    #[serde(rename = "ref")]
    pub reference: String,
    pub detail: String,
}

/// What one reflection did to the memory of one agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MemoryChange {
    pub agent: String,
    pub added: Vec<String>,
    pub retired: Vec<String>,
    pub helpful: Vec<String>,
    pub harmful: Vec<String>,
    /// Items that a revert brought back from retirement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restored: Option<Vec<String>>,
    /// Deltas that the code refused, with the reason.
    pub dropped: Vec<String>,
    pub diff: String,
}

/// The threshold that an agent crossed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ProposalTrigger {
    pub reason: String,
    pub failure_class: Option<String>,
    pub count: u64,
}

/// A lesson that holds in two or more products.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SharedLesson {
    pub text: String,
    pub products: Vec<String>,
}

/// The figures of one replay case.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CaseMetrics {
    pub first_pass: bool,
    pub unresolved: u64,
    pub invented: u64,
    pub missing_objectives: u64,
    pub sin_datos: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReplayTotals {
    pub cases: u64,
    pub first_pass_rate: f64,
    pub unresolved_per_case: f64,
    pub invented: u64,
    pub missing_objectives: u64,
    pub sin_datos: u64,
    pub cost_usd: f64,
}

/// One held-out case, before and after the proposed prompt.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReplayRow {
    pub minutes_id: String,
    /// `stored`: the meeting log. `rerun`: run again with the prompt of today, because
    /// the stored prompt hash differs.
    pub baseline: String,
    pub before: CaseMetrics,
    pub after: CaseMetrics,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReplayVerdict {
    pub passed: bool,
    pub worse: Vec<String>,
    pub better: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReplayPair {
    pub before: ReplayTotals,
    pub after: ReplayTotals,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReplayReport {
    pub tuning: Vec<String>,
    pub held_out: Vec<String>,
    pub rows: Vec<ReplayRow>,
    pub totals: Option<ReplayPair>,
    pub verdict: ReplayVerdict,
    /// Why the proposal was discarded before a full comparison.
    pub aborted: Option<String>,
    pub cost_rule: String,
    /// Spent by the proposal call and the replay calls.
    pub cost_usd: f64,
    pub budget_usd: f64,
}

/// One prompt change that replay judged.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Proposal {
    pub agent: String,
    /// `workspace/<p>/agents/<agent>.md`, or `plugin/agents/<role>.md` for a global change.
    pub target: String,
    pub trigger: ProposalTrigger,
    pub shared_lesson: Option<SharedLesson>,
    pub rationale: String,
    pub expected_effect: String,
    /// Unified diff of the target, for display.
    pub diff: String,
    /// The full text that the target gets on approval.
    pub content: String,
    /// sha256 of the normalized new text.
    pub body_hash: String,
    /// sha256 of the target when the proposal was made. Null when it did not exist.
    pub target_hash: Option<String>,
    pub decision_id: Option<i64>,
    pub replay: ReplayReport,
    /// `pending`, `approved`, `rejected`, `discarded` or `stale`.
    pub status: String,
}

/// One agent past the proposal threshold, and what reflect did about it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ProposalCandidate {
    pub agent: String,
    pub reason: String,
    pub failure_class: Option<String>,
    pub count: u64,
    /// Replay cases, and how many of them are held out.
    pub cases: u64,
    pub held_out: u64,
    /// `proposed`, `insufficient_evidence`, `deferred` or `failed`.
    pub outcome: String,
    pub note: Option<String>,
}

/// One execution of `reflect`, or the line that a revert appends.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReflectionEntry {
    pub id: String,
    pub product: String,
    pub created_at: String,
    pub trigger: String,
    /// Set on the line that a revert appends: the id of the reverted entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverts: Option<String>,
    pub signals: Vec<Signal>,
    pub memory: Vec<MemoryChange>,
    pub proposal: Option<Proposal>,
    /// Agents past the proposal threshold. Absent when none was.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates: Option<Vec<ProposalCandidate>>,
    /// The minutes ids and the memory item sources that this reflection processed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumed: Option<Vec<String>>,
    /// `applied` or `reverted`.
    pub status: String,
    /// Always null: the rows have no commit.
    pub commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

impl ReflectionEntry {
    /// An applied entry whose memory changed something, and that no later line reverted.
    pub fn can_revert(&self) -> bool {
        self.reverts.is_none()
            && self.status == "applied"
            && self.memory.iter().any(|m| !m.diff.trim().is_empty())
    }
}

/// `Number.prototype.toFixed`: the exact decimal value of the float, with a half rounded up.
pub fn to_fixed(v: f64, digits: usize) -> String {
    let exact = format!("{:.60}", v.abs());
    let (int, frac) = exact.split_once('.').unwrap_or((&exact, ""));
    let mut kept: Vec<u8> = int.bytes().chain(frac.bytes().take(digits)).collect();
    if frac.as_bytes().get(digits).is_some_and(|d| *d >= b'5') {
        let mut i = kept.len();
        loop {
            if i == 0 {
                kept.insert(0, b'1');
                break;
            }
            i -= 1;
            if kept[i] == b'9' {
                kept[i] = b'0';
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    let text = String::from_utf8_lossy(&kept).into_owned();
    let split = text.len() - digits;
    let sign = if v < 0.0 && text.bytes().any(|d| d != b'0') {
        "-"
    } else {
        ""
    };
    if digits == 0 {
        format!("{sign}{text}")
    } else {
        format!("{sign}{}.{}", &text[..split], &text[split..])
    }
}

fn pct(x: f64) -> String {
    format!("{}%", (x * 100.0 + 0.5).floor() as i64)
}

fn usd(x: f64) -> String {
    format!("${}", to_fixed(x, 3))
}

fn case_line(m: &CaseMetrics) -> String {
    format!(
        "check {}, sin resolver {}, inventadas {}, objetivos faltantes {}, sin datos {}, costo {}",
        if m.first_pass { "ok" } else { "falla" },
        m.unresolved,
        m.invented,
        m.missing_objectives,
        m.sin_datos,
        usd(m.cost_usd)
    )
}

fn total_pair(b: &ReplayTotals, a: &ReplayTotals) -> String {
    [
        format!(
            "check al primer intento {} → {}",
            pct(b.first_pass_rate),
            pct(a.first_pass_rate)
        ),
        format!(
            "sin resolver por caso {} → {}",
            to_fixed(b.unresolved_per_case, 2),
            to_fixed(a.unresolved_per_case, 2)
        ),
        format!("inventadas {} → {}", b.invented, a.invented),
        format!(
            "objetivos faltantes {} → {}",
            b.missing_objectives, a.missing_objectives
        ),
        format!("sin datos {} → {}", b.sin_datos, a.sin_datos),
        format!("costo {} → {}", usd(b.cost_usd), usd(a.cost_usd)),
    ]
    .join(", ")
}

impl ReplayReport {
    /// The replay table as text: one line for each held-out case, then the totals.
    pub fn table(&self) -> Vec<String> {
        let Some(totals) = &self.totals else {
            let why = self.aborted.as_deref().unwrap_or("sin casos");
            return vec![format!("Replay sin comparación: {why}.")];
        };
        let mut lines = vec![format!(
            "Replay sobre {} casos held-out (antes → después):",
            totals.after.cases
        )];
        for row in &self.rows {
            let rerun = if row.baseline == "rerun" {
                " (baseline re-corrido)"
            } else {
                ""
            };
            lines.push(format!(
                "- {}{rerun}: {} → {}",
                row.minutes_id,
                case_line(&row.before),
                case_line(&row.after)
            ));
        }
        lines.push(format!(
            "Totales: {}.",
            total_pair(&totals.before, &totals.after)
        ));
        lines
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReflectRunParams {
    pub product: String,
    /// Curates memory only, and makes no prompt proposal.
    #[serde(default)]
    pub memory_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReflectRunResult {
    /// Null when no agent had signals or new items and no proposal was made, so
    /// nothing was written.
    pub entry: Option<ReflectionEntry>,
    /// Agents past the proposal threshold, and what came of each.
    pub candidates: Vec<ProposalCandidate>,
    /// The lines that TypeScript prints on stderr.
    pub warnings: Vec<String>,
}

pub struct ReflectRun;

impl Method for ReflectRun {
    const NAME: &'static str = "reflect.run";
    const DESCRIPTION: &'static str = "Curates the memory of each agent from the signals since \
        the last reflection, then proposes one prompt change that replay judges. It costs money. \
        It writes after each model call has returned, in one transaction.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Conflict,
        ErrorKind::Unavailable,
    ];
    type Params = ReflectRunParams;
    type Result = ReflectRunResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReflectRevertParams {
    pub product: String,
    pub reflection: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReflectRevertResult {
    pub entry: ReflectionEntry,
}

pub struct ReflectRevert;

impl Method for ReflectRevert {
    const NAME: &'static str = "reflect.revert";
    const DESCRIPTION: &'static str = "Applies the inverse of the memory changes of one \
        reflection on the items of today, and appends a revert line.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Conflict,
        ErrorKind::Unavailable,
    ];
    type Params = ReflectRevertParams;
    type Result = ReflectRevertResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReflectCancelParams {
    pub product: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReflectCancelResult {
    /// False when no reflection of the product runs.
    pub canceled: bool,
}

pub struct ReflectCancel;

impl Method for ReflectCancel {
    const NAME: &'static str = "reflect.cancel";
    const DESCRIPTION: &'static str = "Stops the model calls of the running reflection of a \
        product. A canceled reflection writes nothing.";
    const ERRORS: &'static [ErrorKind] = &[];
    type Params = ReflectCancelParams;
    type Result = ReflectCancelResult;
}

/// One memory item, as the table holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryEntry {
    pub id: String,
    pub text: String,
    /// `fact` or `lesson`.
    pub kind: String,
    pub helpful: i64,
    pub harmful: i64,
    pub source: String,
    pub created_at: String,
    pub retired_at: Option<String>,
    pub retired_why: Option<String>,
}

/// The items of one agent, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentMemory {
    pub agent: String,
    pub items: Vec<MemoryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryListParams {
    pub product: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryListResult {
    /// The roster first, then each other agent that holds memory.
    pub agents: Vec<AgentMemory>,
    /// The reflections of the product, newest first.
    pub reflections: Vec<ReflectionEntry>,
}

pub struct MemoryList;

impl Method for MemoryList {
    const NAME: &'static str = "memory.list";
    const DESCRIPTION: &'static str =
        "Returns the memory items of each agent of a product, and its reflections.";
    const ERRORS: &'static [ErrorKind] = &[ErrorKind::NotFound, ErrorKind::Unavailable];
    type Params = MemoryListParams;
    type Result = MemoryListResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryRetireParams {
    pub product: String,
    pub agent: String,
    pub id: String,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryRetireResult {
    pub item: MemoryEntry,
}

pub struct MemoryRetire;

impl Method for MemoryRetire {
    const NAME: &'static str = "memory.retire";
    const DESCRIPTION: &'static str = "Retires one active memory item of an agent, with a reason.";
    const ERRORS: &'static [ErrorKind] = &[
        ErrorKind::NotFound,
        ErrorKind::Conflict,
        ErrorKind::Unavailable,
    ];
    type Params = MemoryRetireParams;
    type Result = MemoryRetireResult;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_fixed_rounds_a_half_up_as_javascript_does() {
        assert_eq!(to_fixed(0.0625, 3), "0.063");
        assert_eq!(to_fixed(1.005, 2), "1.00");
        assert_eq!(to_fixed(0.0, 3), "0.000");
        assert_eq!(to_fixed(9.999, 2), "10.00");
    }

    #[test]
    fn a_stored_entry_without_the_new_fields_reads_as_defaults() {
        let e: ReflectionEntry = serde_json::from_str(
            r#"{"id":"r-1","created_at":"t","status":"applied","proposal":{"agent":"dev"}}"#,
        )
        .unwrap();
        assert_eq!(e.proposal.unwrap().agent, "dev");
        assert!(e.memory.is_empty() && e.consumed.is_none());
    }

    #[test]
    fn only_an_applied_entry_with_a_diff_can_be_reverted() {
        let change = MemoryChange {
            diff: "@@".into(),
            ..MemoryChange::default()
        };
        let mut e = ReflectionEntry {
            status: "applied".into(),
            memory: vec![change],
            ..ReflectionEntry::default()
        };
        assert!(e.can_revert());
        e.status = "reverted".into();
        assert!(!e.can_revert());
    }

    #[test]
    fn the_table_of_an_aborted_replay_gives_the_reason() {
        let r = ReplayReport {
            aborted: Some("sin plata".into()),
            ..ReplayReport::default()
        };
        assert_eq!(r.table(), ["Replay sin comparación: sin plata."]);
    }
}
