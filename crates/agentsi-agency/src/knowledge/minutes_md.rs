//! The Markdown of a minutes in the index. The rules copy
//! `renderMinutesMarkdown` of `packages/core/src/minutes-md.ts`, which reads
//! the JSON that the store keeps in `minutes.data`.

use serde_json::Value;

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) => "null".into(),
        None => "undefined".into(),
        Some(other) => other.to_string(),
    }
}

fn field(v: &Value, key: &str) -> String {
    text(v.get(key))
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(_) => true,
    }
}

fn items<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn mark_box(mark: Option<&Value>) -> &'static str {
    match mark.and_then(Value::as_str) {
        Some("accepted") => "[x]",
        Some("changed") => "[~]",
        Some("rejected") => "[-]",
        _ => "[ ]",
    }
}

pub fn render(m: &Value) -> String {
    let mut out: Vec<String> = Vec::new();
    let check = m.get("check").unwrap_or(&Value::Null);
    let repaired = truthy(check.get("repaired"));
    out.push(format!("# Minutes {}", field(m, "id")));
    out.push(String::new());
    let scope = if truthy(m.get("scope")) {
        format!(" ({})", field(m, "scope"))
    } else {
        String::new()
    };
    out.push(format!("- Product: {}{scope}", field(m, "product")));
    out.push(format!("- Date: {}", field(m, "created_at")));
    out.push(format!("- Question: {}", field(m, "question")));
    out.push(format!(
        "- Status: {}{}",
        field(m, "status"),
        if repaired { " (repaired)" } else { "" }
    ));
    out.push(String::new());

    out.push("## Priorities".into());
    out.push(String::new());
    let priorities = items(m, "priorities");
    if priorities.is_empty() {
        out.push("_None._".into());
        out.push(String::new());
    }
    for (i, p) in priorities.iter().enumerate() {
        let new = if truthy(p.get("is_new")) {
            " `new`"
        } else {
            ""
        };
        let objective = if truthy(p.get("objective")) {
            format!(" [{}]", field(p, "objective"))
        } else {
            String::new()
        };
        out.push(format!(
            "- #{i} {} **{}**{new} ({}){objective}",
            mark_box(p.get("mark")),
            field(p, "title"),
            field(p, "owner_role")
        ));
        out.push(format!("   - Why: {}", field(p, "why")));
        for s in items(p, "sources") {
            out.push(format!(
                "   - Source: {} `{}`",
                field(s, "type"),
                field(s, "ref")
            ));
        }
    }
    if !priorities.is_empty() {
        out.push(String::new());
    }

    let disagreements = items(m, "disagreements");
    if !disagreements.is_empty() {
        out.push("## Disagreements".into());
        out.push(String::new());
        for d in disagreements {
            out.push(format!("- **{}**", field(d, "topic")));
            for pos in items(d, "positions") {
                out.push(format!(
                    "  - {}: {}",
                    field(pos, "role"),
                    field(pos, "view")
                ));
            }
        }
        out.push(String::new());
    }

    out.push("## Decisions to confirm".into());
    out.push(String::new());
    let decisions = items(m, "decisions");
    if decisions.is_empty() {
        out.push("_None._".into());
    }
    for d in decisions {
        let status = if truthy(d.get("status")) {
            field(d, "status")
        } else {
            "pending".into()
        };
        let action = if truthy(d.get("action")) {
            format!(" → {}", field(d, "action"))
        } else {
            String::new()
        };
        let priorities: Vec<String> = items(d, "priorities")
            .iter()
            .map(|p| text(Some(p)))
            .collect();
        let linked = if priorities.is_empty() {
            String::new()
        } else {
            format!(" (priorities: {})", priorities.join(", "))
        };
        out.push(format!(
            "- [{status}] {} — {}{action}{linked}",
            field(d, "text"),
            field(d, "why")
        ));
    }
    out.push(String::new());

    out.push("## Roles".into());
    out.push(String::new());
    if let Some(roles) = m.get("roles").and_then(Value::as_object) {
        for (role, r) in roles {
            out.push(format!("### {role} ({})", field(r, "model")));
            out.push(String::new());
            out.push(field(r, "answer"));
            out.push(String::new());
            let concerns = items(r, "concerns");
            if !concerns.is_empty() {
                out.extend(
                    concerns
                        .iter()
                        .map(|c| format!("- Concern: {}", text(Some(c)))),
                );
                out.push(String::new());
            }
        }
    }

    let first = items(check, "first_failures");
    if repaired && !first.is_empty() {
        out.push("## Repaired failures".into());
        out.push(String::new());
        out.extend(first.iter().map(|f| format!("- {}", text(Some(f)))));
        out.push(String::new());
    }
    let failures = items(check, "failures");
    if !failures.is_empty() {
        out.push("## Check failures".into());
        out.push(String::new());
        out.extend(failures.iter().map(|f| format!("- {}", text(Some(f)))));
        out.push(String::new());
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn it_renders_the_sections_of_a_minutes() {
        let m = json!({
            "id": "m-1", "product": "alpha", "scope": null,
            "created_at": "2026-10-01T09:00:00.000Z", "question": "What first?",
            "status": "valid",
            "priorities": [{
                "title": "Ship it", "why": "It is ready", "owner_role": "pm",
                "mark": "accepted", "is_new": true, "objective": "o1",
                "sources": [{"type": "commit", "ref": "abc1234"}]
            }],
            "disagreements": [{"topic": "Timing", "positions": [{"role": "dev", "view": "later"}]}],
            "decisions": [{"text": "Go", "why": "Ready", "action": "Tag it", "priorities": [0]}],
            "roles": {"pm": {"model": "opus", "answer": "Ship.", "concerns": ["risk"]}},
            "check": {"failures": [], "repaired": true, "first_failures": ["f1"]}
        });
        let want = "# Minutes m-1\n\n- Product: alpha\n- Date: 2026-10-01T09:00:00.000Z\n\
- Question: What first?\n- Status: valid (repaired)\n\n## Priorities\n\n\
- #0 [x] **Ship it** `new` (pm) [o1]\n   - Why: It is ready\n   - Source: commit `abc1234`\n\n\
## Disagreements\n\n- **Timing**\n  - dev: later\n\n\
## Decisions to confirm\n\n- [pending] Go — Ready → Tag it (priorities: 0)\n\n\
## Roles\n\n### pm (opus)\n\nShip.\n\n- Concern: risk\n\n\
## Repaired failures\n\n- f1\n";
        assert_eq!(render(&m), want);
    }

    #[test]
    fn an_empty_minutes_says_none() {
        let m = json!({
            "id": "m-2", "product": "alpha", "scope": "web", "created_at": "d", "question": "q",
            "status": "invalid", "priorities": [], "disagreements": [], "decisions": [],
            "roles": {}, "check": {"failures": ["bad"], "repaired": false}
        });
        let out = render(&m);
        assert!(out.contains("- Product: alpha (web)\n"));
        assert!(out.contains("## Priorities\n\n_None._\n\n## Decisions to confirm\n\n_None._\n"));
        assert!(out.ends_with("## Check failures\n\n- bad\n"));
    }
}
