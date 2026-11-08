//! Memory curation: one structured call for each agent. It sees the items as lines
//! and returns deltas. Plain code validates and applies them. The call never sees
//! or returns a whole memory file. The rules copy `reflect.ts`.

use super::memory::{active, make_lesson, retire};
use super::{Result, text};
use crate::story::claude::Call;
use agentsi_proto::reflect_api::Signal;
use agentsi_store::meeting::normalize_text;
use agentsi_store::read::MemoryItem;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::time::Duration;

pub const MAX_ADDS: usize = 5;
pub const MAX_RETIRES: usize = 10;
pub const CURATION_MODEL: &str = "haiku";
/// The default of `claude.ts`.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(6 * 60);

#[derive(Debug, Default, PartialEq, Eq, Clone)]
pub struct Deltas {
    pub add: Vec<String>,
    /// The id, and the reason.
    pub retire: Vec<(String, String)>,
    pub helpful: Vec<String>,
    pub harmful: Vec<String>,
}

pub fn delta_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["add", "retire", "helpful", "harmful"],
        "properties": {
            "add": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["text", "kind", "evidence"],
                    "properties": {
                        "text": {"type": "string"},
                        "kind": {"type": "string", "enum": ["fact", "lesson"]},
                        "evidence": {"type": "array", "items": {"type": "string"}},
                    },
                },
            },
            "retire": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "why"],
                    "properties": {"id": {"type": "string"}, "why": {"type": "string"}},
                },
            },
            "helpful": {"type": "array", "items": {"type": "string"}},
            "harmful": {"type": "array", "items": {"type": "string"}},
        },
    })
}

fn curation_system() -> String {
    [
        "You curate the memory of one AgentSI agent. Memory is a list of items with ids; you never rewrite it.".to_string(),
        "Return deltas only: `add` new items, `retire` items by id with a reason, mark ids `helpful` or `harmful`.".into(),
        "Propose only deltas backed by the listed signals. Every add cites in `evidence` at least one signal ref exactly as listed.".into(),
        "Learn from failures as well as successes: an accepted mark shows what to keep doing, a failure or a rejection shows what to avoid.".into(),
        "Retire items that are stale, duplicated or contradicted by the signals. Status snapshots (pending work, dates of today) do not belong in memory.".into(),
        format!("At most {MAX_ADDS} adds and {MAX_RETIRES} retires. Write new item texts in the language of the existing items, one short sentence each."),
        "Empty arrays are a valid answer.".into(),
    ]
    .join("\n")
}

pub fn curation_prompt(
    agent: &str,
    items: &[MemoryItem],
    signals: &[&Signal],
    brief_xml: Option<&str>,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(xml) = brief_xml {
        lines.push(xml.into());
        lines.push(String::new());
    }
    lines.push(format!("<agent>{agent}</agent>"));
    lines.push("<items>".into());
    let shown = active(items);
    if shown.is_empty() {
        lines.push("(no items)".into());
    }
    for i in shown {
        lines.push(format!(
            "[{}] ({}, helpful {}, harmful {}) {}",
            i.id, i.kind, i.helpful, i.harmful, i.text
        ));
    }
    lines.push("</items>".into());
    lines.push("<signals>".into());
    if signals.is_empty() {
        lines.push("(no signals)".into());
    }
    for s in signals {
        lines.push(format!("- ref {} · {} · {}", s.reference, s.kind, s.detail));
    }
    lines.push("</signals>".into());
    lines.join("\n")
}

pub fn curation_call(
    agent: &str,
    items: &[MemoryItem],
    signals: &[&Signal],
    brief_xml: Option<&str>,
    root: &Path,
) -> Call {
    Call {
        prompt: curation_prompt(agent, items, signals, brief_xml),
        model: CURATION_MODEL.into(),
        system: curation_system(),
        cwd: root.to_string_lossy().into_owned(),
        tools: Vec::new(),
        schema: Some(delta_schema()),
        effort: Some("low".into()),
        timeout: CALL_TIMEOUT,
        mcp: None,
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map_or(Vec::new(), |list| {
        list.iter()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect()
    })
}

/// The records of an answer. As `typeof x === "object"` does, this keeps an array,
/// which then has no fields.
fn records(v: Option<&Value>) -> Vec<&Value> {
    v.and_then(Value::as_array).map_or(Vec::new(), |list| {
        list.iter()
            .filter(|r| r.is_object() || r.is_array())
            .collect()
    })
}

/// True when `evidence` names this signal: its ref, or its whole line of the prompt.
/// A real model cites the line (`rf-02 · invented_source · docs/x.md`), not the bare
/// ref. A line with another kind or detail is no citation, so an invented one fails.
fn cites(evidence: &str, s: &Signal) -> bool {
    let plain = evidence.trim();
    let plain = plain.strip_prefix("- ").unwrap_or(plain);
    let plain = plain.strip_prefix("ref ").unwrap_or(plain);
    plain == s.reference || plain == format!("{} · {} · {}", s.reference, s.kind, s.detail)
}

/// Keeps only deltas on active ids and adds that cite the signals of the agent,
/// within the caps. The second value lists what was dropped, with the reason.
pub fn validate_deltas(
    raw: &Value,
    items: &[MemoryItem],
    signals: &[&Signal],
) -> (Deltas, Vec<String>) {
    let live = active(items);
    let ids: HashSet<&str> = live.iter().map(|i| i.id.as_str()).collect();
    let mut texts: HashSet<String> = live.iter().map(|i| normalize_text(&i.text)).collect();
    let mut d = Deltas::default();
    let mut dropped = Vec::new();

    for a in records(raw.get("add")) {
        let text = normalize_text(text(a, "text"));
        let cited = strings(a.get("evidence"))
            .iter()
            .any(|e| signals.iter().any(|s| cites(e, s)));
        if text.is_empty() {
            dropped.push("add: empty text".to_string());
        } else if !cited {
            dropped.push(format!("add \"{text}\": cites no signal"));
        } else if texts.contains(&text) {
            dropped.push(format!("add \"{text}\": duplicates an item"));
        } else if d.add.len() >= MAX_ADDS {
            dropped.push(format!("add \"{text}\": over the limit of {MAX_ADDS}"));
        } else {
            texts.insert(text.clone());
            d.add.push(text);
        }
    }

    for x in records(raw.get("retire")) {
        let id = text(x, "id");
        let why = normalize_text(text(x, "why"));
        if !ids.contains(id) {
            let shown = if id.is_empty() { "(no id)" } else { id };
            dropped.push(format!("retire {shown}: no active item"));
        } else if why.is_empty() {
            dropped.push(format!("retire {id}: no reason"));
        } else if d.retire.iter().any(|(i, _)| i == id) {
            dropped.push(format!("retire {id}: repeated"));
        } else if d.retire.len() >= MAX_RETIRES {
            dropped.push(format!("retire {id}: over the limit of {MAX_RETIRES}"));
        } else {
            d.retire.push((id.into(), why));
        }
    }

    for (counter, list) in [("helpful", &mut d.helpful), ("harmful", &mut d.harmful)] {
        for id in strings(raw.get(counter)) {
            if !ids.contains(id.as_str()) {
                dropped.push(format!("{counter} {id}: no active item"));
            } else if !list.contains(&id) {
                list.push(id);
            }
        }
    }
    (d, dropped)
}

#[derive(Debug, Default)]
pub struct Applied {
    pub items: Vec<MemoryItem>,
    pub added: Vec<String>,
    pub retired: Vec<String>,
    pub helpful: Vec<String>,
    pub harmful: Vec<String>,
    /// Deltas on items that changed since the curation call read them.
    pub skipped: Vec<String>,
}

/// Every signal is a failure or a mark, so every add is a lesson. Ids that are no
/// longer active are skipped.
pub fn apply_deltas(
    items: &[MemoryItem],
    d: &Deltas,
    source: &str,
    now_iso: &str,
) -> Result<Applied> {
    let mut out = items.to_vec();
    let live: BTreeSet<String> = active(items).iter().map(|i| i.id.clone()).collect();
    let mut texts: HashSet<String> = active(items)
        .iter()
        .map(|i| normalize_text(&i.text))
        .collect();
    let mut a = Applied::default();
    for add in &d.add {
        if !texts.insert(add.clone()) {
            a.skipped.push(format!("add \"{add}\": duplicates an item"));
            continue;
        }
        let item = make_lesson(add, source, now_iso, &out)?;
        a.added.push(item.id.clone());
        out.push(item);
    }
    for id in &d.helpful {
        if !live.contains(id) {
            a.skipped.push(format!("helpful {id}: no longer active"));
        } else {
            if let Some(i) = out.iter_mut().find(|i| i.id == *id) {
                i.helpful += 1;
            }
            a.helpful.push(id.clone());
        }
    }
    for id in &d.harmful {
        if !live.contains(id) {
            a.skipped.push(format!("harmful {id}: no longer active"));
        } else {
            if let Some(i) = out.iter_mut().find(|i| i.id == *id) {
                i.harmful += 1;
            }
            a.harmful.push(id.clone());
        }
    }
    for (id, why) in &d.retire {
        if !live.contains(id) {
            a.skipped.push(format!("retire {id}: no longer active"));
        } else {
            retire(&mut out, id, why, now_iso);
            a.retired.push(id.clone());
        }
    }
    a.items = out;
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, text: &str) -> MemoryItem {
        MemoryItem {
            id: id.into(),
            text: text.into(),
            kind: "fact".into(),
            helpful: 0,
            harmful: 0,
            source: "migration".into(),
            created_at: "2026-10-01T00:00:00.000Z".into(),
            retired_at: None,
            retired_why: None,
        }
    }

    fn signal(reference: &str) -> Signal {
        Signal {
            agent: "dev".into(),
            kind: "mark_rejected".into(),
            reference: reference.into(),
            detail: "#0 A".into(),
        }
    }

    /// The forms that a real `haiku` wrote in `evidence`.
    #[test]
    fn evidence_may_repeat_the_line_of_a_signal_but_not_change_it() {
        let s = signal("rf-02");
        let add = |evidence: &str| {
            let raw = json!({"add": [{"text": "A lesson.", "evidence": [evidence]}]});
            validate_deltas(&raw, &[], &[&s]).0.add.len()
        };
        assert_eq!(add("rf-02"), 1);
        assert_eq!(add("ref rf-02"), 1);
        assert_eq!(add("rf-02 · mark_rejected · #0 A"), 1);
        assert_eq!(add("ref rf-02 · mark_rejected · #0 A"), 1);
        assert_eq!(add("- ref rf-02 · mark_rejected · #0 A"), 1);
        assert_eq!(add("rf-02 · mark_rejected · #0 Another"), 0);
        assert_eq!(add("rf-02 · mark_accepted · #0 A"), 0);
        assert_eq!(add("rf-09 · mark_rejected · #0 A"), 0);
        assert_eq!(add("rf-0"), 0);
    }

    #[test]
    fn deltas_without_evidence_and_on_unknown_ids_are_dropped() {
        let items = [item("m-1", "one"), item("m-2", "two")];
        let s = signal("m1");
        let raw = json!({
            "add": [
                {"text": " a  lesson ", "kind": "lesson", "evidence": ["m1", "other"]},
                {"text": "no proof", "kind": "lesson", "evidence": ["other"]},
                {"text": "one", "kind": "fact", "evidence": ["m1"]},
                {"text": "a lesson", "kind": "fact", "evidence": ["m1"]},
                {"text": "", "kind": "fact", "evidence": ["m1"]},
            ],
            "retire": [
                {"id": "m-1", "why": "old"},
                {"id": "m-1", "why": "again"},
                {"id": "m-2", "why": " "},
                {"id": "m-9", "why": "x"},
                {"why": "x"},
            ],
            "helpful": ["m-2", "m-2", "m-9"],
            "harmful": ["m-9"],
        });
        let (d, dropped) = validate_deltas(&raw, &items, &[&s]);
        assert_eq!(d.add, ["a lesson"]);
        assert_eq!(d.retire, [("m-1".to_string(), "old".to_string())]);
        assert_eq!(d.helpful, ["m-2"]);
        assert!(d.harmful.is_empty());
        assert_eq!(
            dropped,
            [
                "add \"no proof\": cites no signal",
                "add \"one\": duplicates an item",
                "add \"a lesson\": duplicates an item",
                "add: empty text",
                "retire m-1: repeated",
                "retire m-2: no reason",
                "retire m-9: no active item",
                "retire (no id): no active item",
                "helpful m-9: no active item",
                "harmful m-9: no active item",
            ]
        );
    }

    #[test]
    fn the_caps_hold() {
        let items: Vec<MemoryItem> = (0..12)
            .map(|n| item(&format!("m-{n}"), &format!("t{n}")))
            .collect();
        let s = signal("m1");
        let adds: Vec<Value> = (0..7)
            .map(|n| json!({"text": format!("new {n}"), "evidence": ["m1"]}))
            .collect();
        let retires: Vec<Value> = (0..12)
            .map(|n| json!({"id": format!("m-{n}"), "why": "x"}))
            .collect();
        let raw = json!({"add": adds, "retire": retires});
        let (d, dropped) = validate_deltas(&raw, &items, &[&s]);
        assert_eq!(d.add.len(), MAX_ADDS);
        assert_eq!(d.retire.len(), MAX_RETIRES);
        assert_eq!(dropped.len(), 4);
        assert!(dropped[0].ends_with("over the limit of 5"));
    }

    #[test]
    fn a_malformed_answer_drops_everything() {
        for raw in [json!(null), json!("x"), json!({"add": "x", "retire": 3})] {
            let (d, dropped) = validate_deltas(&raw, &[], &[]);
            assert_eq!(d, Deltas::default());
            assert!(dropped.is_empty());
        }
        let (_, dropped) = validate_deltas(&json!({"add": [[1], 2]}), &[], &[]);
        assert_eq!(dropped, ["add: empty text"]);
    }

    #[test]
    fn the_prompt_holds_the_brief_the_active_lines_and_the_signals() {
        let mut gone = item("m-2", "gone");
        gone.retired_at = Some("t".into());
        let s = signal("m1");
        let prompt = curation_prompt("dev", &[item("m-1", "one"), gone], &[&s], Some("<brief/>"));
        assert_eq!(
            prompt,
            "<brief/>\n\n<agent>dev</agent>\n<items>\n[m-1] (fact, helpful 0, harmful 0) one\n</items>\n<signals>\n- ref m1 · mark_rejected · #0 A\n</signals>"
        );
        let empty = curation_prompt("qa", &[], &[], None);
        assert!(empty.contains("(no items)") && empty.contains("(no signals)"));
    }

    #[test]
    fn adds_are_lessons_and_deltas_on_gone_items_are_skipped() {
        let d = Deltas {
            add: vec!["new".into(), "one".into()],
            retire: vec![("m-1".into(), "old".into()), ("m-8".into(), "x".into())],
            helpful: vec!["m-1".into(), "m-8".into()],
            harmful: vec!["m-9".into()],
        };
        let a = apply_deltas(&[item("m-1", "one")], &d, "r-1", "now").unwrap();
        assert_eq!(a.items.len(), 2);
        assert_eq!(a.items[1].kind, "lesson");
        assert_eq!(a.items[1].source, "r-1");
        assert_eq!(a.items[1].created_at, "now");
        assert_eq!(a.added, [a.items[1].id.clone()]);
        assert_eq!(a.items[0].helpful, 1);
        assert_eq!(a.items[0].retired_at.as_deref(), Some("now"));
        assert_eq!(a.items[0].retired_why.as_deref(), Some("old"));
        assert_eq!(
            a.skipped,
            [
                "add \"one\": duplicates an item",
                "helpful m-8: no longer active",
                "harmful m-9: no longer active",
                "retire m-8: no longer active",
            ]
        );
    }
}
