//! The meeting log line of one meeting. The rules copy
//! `packages/core/src/meeting-log.ts`. The line goes into the table `meeting_log`,
//! and audit and reflection read it.

use agentsi_proto::meeting_api::MeetingUsage;
use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

/// The marker that the role prompt asks for when data is missing, in any answer language.
pub const NO_DATA: &str = "[no data]";

static SIN_DATOS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\[no data\]|sin datos").unwrap());

pub fn sha256(text: &str) -> String {
    let hash = Sha256::digest(text.as_bytes());
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// A number as JSON that holds no `.0`, as `JSON.stringify` writes it.
pub fn js_number(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

/// What a meeting knows about one agent that the minutes do not hold.
#[derive(Debug, Clone)]
pub struct AgentAnswerLog {
    pub id: String,
    pub prompt: String,
    pub memory: String,
    pub model: String,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub unresolved_sources: Vec<Value>,
}

fn same_source(a: &Value, b: &Value) -> bool {
    a.get("type") == b.get("type") && a.get("ref") == b.get("ref")
}

fn array<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// The sources of the synthesis that no agent gave and that the snapshot does not hold.
pub fn invented_sources(m: &Value, snapshot: &str) -> Vec<Value> {
    let empty = Map::new();
    let roles = m.get("roles").and_then(Value::as_object).unwrap_or(&empty);
    let given: Vec<&Value> = roles
        .values()
        .flat_map(|r| array(r, "priorities"))
        .flat_map(|p| array(p, "sources"))
        .collect();
    let mut invented: Vec<Value> = Vec::new();
    for s in array(m, "priorities")
        .iter()
        .flat_map(|p| array(p, "sources"))
    {
        let reference = s.get("ref").and_then(Value::as_str).unwrap_or("");
        if given.iter().any(|g| same_source(g, s))
            || snapshot.contains(reference)
            || invented.iter().any(|x| same_source(x, s))
        {
            continue;
        }
        invented.push(s.clone());
    }
    invented
}

fn cost(rows: &[MeetingUsage], keep: impl Fn(&MeetingUsage) -> bool) -> f64 {
    rows.iter()
        .filter(|u| keep(u))
        .fold(0.0, |n, u| n + u.cost_usd)
}

/// Counts the marker of missing data, and "sin datos", which the role prompt asked for before it.
pub(crate) fn sin_datos(answer: &Value) -> usize {
    let mut parts = vec![
        answer
            .get("answer")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    ];
    parts.extend(
        array(answer, "concerns")
            .iter()
            .filter_map(Value::as_str)
            .map(String::from),
    );
    SIN_DATOS.find_iter(&parts.join("\n")).count()
}

/// `buildLogEntry`. `m` is the finished minutes.
pub fn build_log_entry(
    m: &Value,
    usage: &[MeetingUsage],
    agents: &[AgentAnswerLog],
    snapshot: &str,
    brief_hash: Option<&str>,
) -> Value {
    let mut lines = Map::new();
    for run in agents {
        let answer = m
            .get("roles")
            .and_then(|r| r.get(&run.id))
            .cloned()
            .unwrap_or(Value::Null);
        let priorities = array(&answer, "priorities");
        let sources: usize = priorities.iter().map(|p| array(p, "sources").len()).sum();
        lines.insert(
            run.id.clone(),
            json!({
                "prompt_hash": sha256(&run.prompt),
                "memory_hash": sha256(&run.memory),
                "model": run.model,
                "cost_usd": js_number(cost(usage, |u| u.role == run.id)),
                "duration_ms": run.duration_ms,
                "error": run.error,
                "priorities": priorities.len(),
                "sources": sources,
                "unresolved_sources": run.unresolved_sources,
                "sin_datos": sin_datos(&answer),
            }),
        );
    }
    let check = m.get("check").cloned().unwrap_or(Value::Null);
    let failures = check.get("failures").cloned().unwrap_or(json!([]));
    let first = check
        .get("first_failures")
        .cloned()
        .unwrap_or(failures.clone());
    let first_passed = first.as_array().is_none_or(Vec::is_empty);
    json!({
        "minutes_id": m.get("id"),
        "created_at": m.get("created_at"),
        "question": m.get("question"),
        "scope": m.get("scope"),
        "brief_hash": brief_hash,
        "agents": lines,
        "synthesis": {
            "first_check": {"passed": first_passed, "failures": first},
            "repaired": check.get("repaired"),
            "final_check": {"passed": check.get("passed"), "failures": failures},
            "invented_sources": invented_sources(m, snapshot),
            "cost_usd": js_number(cost(usage, |u| u.role.starts_with("synthesis"))),
        },
        "total_cost_usd": js_number(cost(usage, |_| true)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(role: &str, cost_usd: f64) -> MeetingUsage {
        MeetingUsage {
            role: role.into(),
            model: "haiku".into(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read: 0,
            cache_write: 0,
            cost_usd,
            duration_ms: 0,
        }
    }

    #[test]
    fn a_log_line_counts_credit_for_each_agent() {
        let m = json!({
            "id": "m1", "created_at": "t", "question": "q", "scope": null,
            "roles": {"dev": {
                "answer": "[No data] sobre usuarios.",
                "priorities": [{"sources": [{"type": "path", "ref": "a"}, {"type": "url", "ref": "b"}]}],
                "concerns": ["Sin datos de ventas", "otro"],
            }},
            "priorities": [{"sources": [{"type": "path", "ref": "a"}, {"type": "path", "ref": "in-snap"}, {"type": "path", "ref": "made-up"}, {"type": "path", "ref": "made-up"}]}],
            "check": {"passed": true, "failures": [], "first_failures": ["bad"], "repaired": true},
        });
        let run = AgentAnswerLog {
            id: "dev".into(),
            prompt: "abc".into(),
            memory: String::new(),
            model: "haiku".into(),
            error: None,
            duration_ms: 12,
            unresolved_sources: vec![json!({"type": "url", "ref": "b"})],
        };
        let rows = [
            usage("dev", 0.25),
            usage("synthesis", 0.5),
            usage("synthesis-repair", 0.25),
        ];
        let e = build_log_entry(&m, &rows, &[run], "text in-snap text", Some("h"));
        let dev = &e["agents"]["dev"];
        assert_eq!(
            dev["prompt_hash"],
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            dev["memory_hash"],
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            (&dev["priorities"], &dev["sources"], &dev["sin_datos"]),
            (&json!(1), &json!(2), &json!(2))
        );
        assert_eq!(
            e["synthesis"]["first_check"],
            json!({"passed": false, "failures": ["bad"]})
        );
        assert_eq!(e["synthesis"]["final_check"]["passed"], true);
        assert_eq!(
            e["synthesis"]["invented_sources"],
            json!([{"type": "path", "ref": "made-up"}])
        );
        assert_eq!(e["synthesis"]["cost_usd"], json!(0.75));
        assert_eq!(e["total_cost_usd"].to_string(), "1");
    }

    #[test]
    fn a_whole_number_prints_with_no_point() {
        assert_eq!(js_number(0.0).to_string(), "0");
        assert_eq!(js_number(0.125).to_string(), "0.125");
    }
}
