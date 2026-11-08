//! The grounded signals of a reflection, and what earlier reflections consumed.
//! `collectSignals` of `reflect.ts`, `consumedBy` and `isConsumed` of `audit.ts`.

use super::{SYNTHESIS_AGENT, list, text};
use crate::audit::SIN_DATOS_FAILURE;
use agentsi_proto::reflect_api::Signal;
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};

/// Evidence that earlier reflections already processed.
#[derive(Default)]
pub struct Consumed {
    ids: BTreeSet<String>,
    /// Lines written before reflections recorded `consumed` count everything up to their time.
    until: Option<String>,
}

fn is_truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty(),
        _ => true,
    }
}

/// What the given reflections processed. Reverts consume nothing.
pub fn consumed_by(lines: &[Value]) -> Consumed {
    let mut c = Consumed::default();
    for r in lines {
        let created = text(r, "created_at");
        if created.is_empty() || is_truthy(r.get("reverts")) {
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

pub fn is_consumed(c: &Consumed, id: &str, at: &str) -> bool {
    c.ids.contains(id) || c.until.as_deref().is_some_and(|u| at <= u)
}

fn refs(sources: &[Value]) -> String {
    sources
        .iter()
        .map(|s| text(s, "ref"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn signal(agent: &str, kind: &str, reference: &str, detail: impl Into<String>) -> Signal {
    Signal {
        agent: agent.into(),
        kind: kind.into(),
        reference: reference.into(),
        detail: detail.into(),
    }
}

fn log_signals(entry: &Value) -> Vec<Signal> {
    let mut out = Vec::new();
    let reference = text(entry, "minutes_id");
    let synthesis = &entry["synthesis"];
    for failure in list(&synthesis["first_check"], "failures") {
        let detail = failure.as_str().unwrap_or_default();
        out.push(signal(
            SYNTHESIS_AGENT,
            "first_check_failure",
            reference,
            detail,
        ));
    }
    let invented = list(synthesis, "invented_sources");
    if !invented.is_empty() {
        out.push(signal(
            SYNTHESIS_AGENT,
            "invented_source",
            reference,
            refs(invented),
        ));
    }
    let Some(agents) = entry.get("agents").and_then(Value::as_object) else {
        return out;
    };
    for (agent, a) in agents {
        if is_truthy(a.get("error")) {
            out.push(signal(agent, "error", reference, text(a, "error")));
        }
        let unresolved = list(a, "unresolved_sources");
        if !unresolved.is_empty() {
            out.push(signal(
                agent,
                "unresolved_source",
                reference,
                refs(unresolved),
            ));
        }
        let sin_datos = a.get("sin_datos").and_then(Value::as_u64).unwrap_or(0);
        if sin_datos >= SIN_DATOS_FAILURE {
            let detail = format!("{sin_datos} \"sin datos\"");
            out.push(signal(agent, "sin_datos", reference, detail));
        }
    }
    out
}

fn minutes_signals(m: &Value) -> Vec<Signal> {
    let mut out = Vec::new();
    let reference = text(m, "id");
    for (idx, p) in list(m, "priorities").iter().enumerate() {
        if is_truthy(p.get("mark")) {
            out.push(signal(
                text(p, "owner_role"),
                &format!("mark_{}", text(p, "mark")),
                reference,
                format!("#{idx} {}", text(p, "title")),
            ));
        }
    }
    for d in list(m, "decisions") {
        if text(d, "status") == "rejected" {
            out.push(signal(
                SYNTHESIS_AGENT,
                "decision_rejected",
                reference,
                text(d, "text"),
            ));
        }
    }
    out
}

fn key(agent: &str, kind: &str, reference: &str, detail: &str) -> String {
    format!("{agent}|{kind}|{reference}|{detail}")
}

/// Grounded signals that no earlier reflection consumed. A meeting log line counts
/// by minutes id, so a meeting that landed after a reflection started counts in the
/// next one. Marks, rejected decisions and reverts count when no earlier entry
/// recorded them.
pub fn collect_signals(log: &[Value], minutes: &[Value], reflections: &[Value]) -> Vec<Signal> {
    let consumed = consumed_by(reflections);
    let mut seen: HashSet<String> = reflections
        .iter()
        .flat_map(|e| list(e, "signals"))
        .map(|s| {
            key(
                text(s, "agent"),
                text(s, "kind"),
                text(s, "ref"),
                text(s, "detail"),
            )
        })
        .collect();
    let mut out = Vec::new();
    let mut push = |s: Signal| {
        if seen.insert(key(&s.agent, &s.kind, &s.reference, &s.detail)) {
            out.push(s);
        }
    };

    for entry in log {
        if !is_consumed(
            &consumed,
            text(entry, "minutes_id"),
            text(entry, "created_at"),
        ) {
            log_signals(entry).into_iter().for_each(&mut push);
        }
    }
    for m in minutes {
        minutes_signals(m).into_iter().for_each(&mut push);
    }
    for revert in reflections.iter().filter(|e| is_truthy(e.get("reverts"))) {
        let target = text(revert, "reverts");
        let reverted = reflections.iter().find(|e| text(e, "id") == target);
        for change in reverted.map_or(&[][..], |e| list(e, "memory")) {
            let detail = format!("reverted by {}", text(revert, "id"));
            push(signal(
                text(change, "agent"),
                "reverted_reflection",
                target,
                detail,
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str, at: &str, synthesis: Value, agents: Value) -> Value {
        let mut s =
            json!({"first_check": {"passed": true, "failures": []}, "invented_sources": []});
        s.as_object_mut()
            .unwrap()
            .extend(synthesis.as_object().unwrap().clone());
        json!({"minutes_id": id, "created_at": at, "agents": agents, "synthesis": s})
    }

    #[test]
    fn failures_of_the_synthesis_are_credited_to_pm() {
        let log = [entry(
            "m1",
            "2026-10-01",
            json!({"first_check": {"passed": false, "failures": ["no source"]},
                   "invented_sources": [{"type": "path", "ref": "a.md"}, {"type": "path", "ref": "b.md"}]}),
            json!({"dev": {"error": "boom", "unresolved_sources": [{"type": "path", "ref": "x"}], "sin_datos": 2},
                   "qa": {"error": null, "unresolved_sources": [], "sin_datos": 1}}),
        )];
        let got = collect_signals(&log, &[], &[]);
        let kinds: Vec<(&str, &str, &str)> = got
            .iter()
            .map(|s| (s.agent.as_str(), s.kind.as_str(), s.detail.as_str()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("pm", "first_check_failure", "no source"),
                ("pm", "invented_source", "a.md, b.md"),
                ("dev", "error", "boom"),
                ("dev", "unresolved_source", "x"),
                ("dev", "sin_datos", "2 \"sin datos\""),
            ]
        );
    }

    #[test]
    fn marks_go_to_the_owner_role_and_rejected_decisions_to_pm() {
        let m = json!({"id": "m1", "priorities": [
            {"title": "A", "owner_role": "dev", "mark": "accepted"},
            {"title": "B", "owner_role": "qa", "mark": null},
            {"title": "C", "owner_role": "qa", "mark": "rejected"},
        ], "decisions": [{"text": "D", "status": "rejected"}, {"text": "E", "status": "pending"}]});
        let got = collect_signals(&[], &[m], &[]);
        assert_eq!(got.len(), 3);
        assert_eq!(got[0].kind, "mark_accepted");
        assert_eq!(got[0].detail, "#0 A");
        assert_eq!(got[1].agent, "qa");
        assert_eq!(got[1].detail, "#2 C");
        assert_eq!(got[2].agent, "pm");
        assert_eq!(got[2].kind, "decision_rejected");
    }

    #[test]
    fn a_consumed_line_and_a_recorded_mark_do_not_count_again() {
        let log = [
            entry(
                "m1",
                "2026-10-01",
                json!({}),
                json!({"dev": {"error": "old"}}),
            ),
            entry(
                "m2",
                "2026-10-02",
                json!({}),
                json!({"dev": {"error": "new"}}),
            ),
        ];
        let minutes = [json!({"id": "m1", "priorities": [
            {"title": "A", "owner_role": "dev", "mark": "accepted"}], "decisions": []})];
        let reflections = [
            json!({"id": "r-1", "created_at": "2026-10-03", "consumed": ["m1"],
            "signals": [{"agent": "dev", "kind": "mark_accepted", "ref": "m1", "detail": "#0 A"}]}),
        ];
        let got = collect_signals(&log, &minutes, &reflections);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].detail, "new");
    }

    #[test]
    fn an_old_line_without_consumed_counts_everything_up_to_its_time() {
        let c = consumed_by(&[json!({"id": "r", "created_at": "2026-10-02"})]);
        assert!(is_consumed(&c, "m", "2026-10-01"));
        assert!(!is_consumed(&c, "m", "2026-10-03"));
    }

    #[test]
    fn a_revert_is_a_signal_for_each_agent_it_touched_and_counts_once() {
        let reflections = [
            json!({"id": "r-1", "created_at": "t", "consumed": [], "signals": [],
                   "memory": [{"agent": "dev"}, {"agent": "qa"}]}),
            json!({"id": "r-2", "created_at": "t", "reverts": "r-1", "signals": [], "memory": []}),
        ];
        let got = collect_signals(&[], &[], &reflections);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].kind, "reverted_reflection");
        assert_eq!(got[0].reference, "r-1");
        assert_eq!(got[0].detail, "reverted by r-2");
        assert_eq!(got[1].agent, "qa");
    }
}
