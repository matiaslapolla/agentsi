//! The minutes of a meeting as JSON. The rules copy `newMinutesId` of
//! `minutes.ts` and `toRoleAnswer`, `emptyAnswer` and `buildMinutes` of
//! `meeting.ts`. The key order is the order of those objects, because the
//! store keeps the JSON text.

use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;

/// The fields of minutes that do not come from Claude.
#[derive(Debug, Clone)]
pub struct MinutesMeta {
    pub id: String,
    pub product: String,
    pub scope: Option<String>,
    pub question: String,
    pub created_at: String,
}

/// The local time as `YYYY-MM-DD-HHMM`.
pub(crate) fn local_stamp(ms: i64) -> String {
    let secs = (ms / 1000) as libc::time_t;
    // SAFETY: `tm` is plain data, and `localtime_r` only reads `secs` and writes `tm`.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        tm
    };
    format!(
        "{}-{:02}-{:02}-{:02}{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

fn slug(question: &str) -> String {
    let plain: String = question
        .nfd()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .collect::<String>()
        .to_lowercase();
    let mut out = String::new();
    for c in plain.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let cut: String = out.trim_end_matches('-').chars().take(40).collect();
    cut.trim_end_matches('-').to_string()
}

/// `newMinutesId`: the local minute, then the slug of the question.
pub fn new_minutes_id(question: &str, ms: i64) -> String {
    let slug = slug(question);
    let name = if slug.is_empty() { "meeting" } else { &slug };
    format!("{}-{name}", local_stamp(ms))
}

fn or_empty(v: &Value, key: &str, default: Value) -> Value {
    v.get(key)
        .filter(|x| !x.is_null())
        .cloned()
        .unwrap_or(default)
}

/// Normalizes a structured role output into an answer.
pub fn to_role_answer(output: &Value, model: &str) -> Value {
    json!({
        "model": model,
        "answer": or_empty(output, "answer", json!("")),
        "priorities": or_empty(output, "priorities", json!([])),
        "concerns": or_empty(output, "concerns", json!([])),
        "memory_notes": or_empty(output, "memory_notes", json!([])),
    })
}

pub fn empty_answer(model: &str, error: &str) -> Value {
    json!({
        "model": model,
        "answer": "",
        "priorities": [],
        "concerns": [format!("Role call failed: {error}")],
        "memory_notes": [],
    })
}

/// Minutes from a synthesis (`None` when it failed), before the check fills the result.
pub fn build_minutes(
    meta: &MinutesMeta,
    roles: &[(String, Value)],
    synthesis: Option<&Value>,
    repaired: bool,
    first_failures: &[String],
) -> Value {
    let part = |key: &str| {
        synthesis
            .map(|s| or_empty(s, key, json!([])))
            .unwrap_or(json!([]))
    };
    let priorities: Vec<Value> = part("priorities")
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|p| {
            let mut out = p.as_object().cloned().unwrap_or_default();
            let objective = p.get("objective").cloned().unwrap_or(Value::Null);
            out.insert("objective".into(), objective);
            out.insert("mark".into(), Value::Null);
            out.insert("is_new".into(), json!(false));
            Value::Object(out)
        })
        .collect();
    let answers: Map<String, Value> = roles.iter().cloned().collect();
    json!({
        "id": meta.id,
        "product": meta.product,
        "scope": meta.scope,
        "question": meta.question,
        "created_at": meta.created_at,
        "roles": answers,
        "priorities": priorities,
        "disagreements": part("disagreements"),
        "decisions": part("decisions"),
        "script": part("script"),
        "check": {"passed": false, "failures": [], "first_failures": first_failures, "repaired": repaired},
        "status": "invalid",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slug_drops_accents_and_keeps_forty_characters() {
        assert_eq!(
            slug("¿Qué hacemos con el índice?"),
            "que-hacemos-con-el-indice"
        );
        assert_eq!(slug("¿¡!?"), "");
        assert_eq!(
            slug("a very long question that goes past the limit of forty"),
            "a-very-long-question-that-goes-past-the"
        );
    }

    #[test]
    fn an_id_has_a_stamp_and_falls_back_to_meeting() {
        let id = new_minutes_id("¿?", 0);
        assert_eq!(id.len(), "1970-01-01-0000-meeting".len());
        assert!(id.ends_with("-meeting"));
    }

    #[test]
    fn the_minutes_keep_the_key_order_of_typescript() {
        let meta = MinutesMeta {
            id: "m".into(),
            product: "p".into(),
            scope: None,
            question: "q".into(),
            created_at: "t".into(),
        };
        let s = json!({
            "priorities": [{"title": "T", "objective": "o1", "why": "W", "sources": [], "owner_role": "dev"},
                           {"title": "U", "why": "W", "sources": [], "owner_role": "dev"}],
            "disagreements": [], "decisions": [{"text": "D", "why": "Y"}], "script": [],
        });
        let roles = vec![(
            "dev".to_string(),
            to_role_answer(&json!({"answer": "A"}), "haiku"),
        )];
        let m = build_minutes(&meta, &roles, Some(&s), true, &["f".into()]);
        assert_eq!(
            m.to_string(),
            r#"{"id":"m","product":"p","scope":null,"question":"q","created_at":"t","roles":{"dev":{"model":"haiku","answer":"A","priorities":[],"concerns":[],"memory_notes":[]}},"priorities":[{"title":"T","objective":"o1","why":"W","sources":[],"owner_role":"dev","mark":null,"is_new":false},{"title":"U","why":"W","sources":[],"owner_role":"dev","objective":null,"mark":null,"is_new":false}],"disagreements":[],"decisions":[{"text":"D","why":"Y"}],"script":[],"check":{"passed":false,"failures":[],"first_failures":["f"],"repaired":true},"status":"invalid"}"#
        );
    }

    #[test]
    fn no_synthesis_gives_empty_minutes() {
        let meta = MinutesMeta {
            id: "m".into(),
            product: "p".into(),
            scope: Some("web".into()),
            question: "q".into(),
            created_at: "t".into(),
        };
        let m = build_minutes(&meta, &[], None, false, &[]);
        assert_eq!(m["priorities"], json!([]));
        assert_eq!(m["scope"], "web");
    }
}
