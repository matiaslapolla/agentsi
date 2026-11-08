//! What the prompts of meetings and stories read: the memory of an agent
//! (`memoryForPrompt` of `memory.ts`), the brief as XML (`renderBriefXml` of
//! `brief.ts`) and the compact snapshot (`compactSnapshot` of `meeting.ts`).

use crate::knowledge::memory_text::{MEMORY_MAX_CHARS, render_for_prompt};
use crate::roster::js_text;
use agentsi_store::read;
use rusqlite::Connection;
use serde_json::Value;

/// The active memory items of an agent, as a prompt reads them.
pub fn memory_for_prompt(
    c: &Connection,
    product: &str,
    agent: &str,
) -> agentsi_store::Result<String> {
    let items = read::memory(c, product, agent)?;
    Ok(render_for_prompt(&items, Some(MEMORY_MAX_CHARS)))
}

fn esc(v: &Value) -> String {
    // A field that the brief leaves out reads as empty.
    let text = if v.is_null() {
        String::new()
    } else {
        js_text(Some(v))
    };
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn list(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}

/// The brief of a product as the XML of the prompts.
pub fn brief_xml(brief: &Value) -> String {
    let p = &brief["product"];
    let mut lines = vec!["<brief>".to_string()];
    lines.push(format!(
        "<product stage=\"{}\"><what>{}</what><audience>{}</audience></product>",
        esc(&p["stage"]),
        esc(&p["what"]),
        esc(&p["audience"])
    ));
    lines.push("<objectives>".into());
    for o in list(&brief["objectives"]) {
        lines.push(format!(
            "<objective id=\"{}\" due=\"{}\" signal=\"{}\">{}</objective>",
            esc(&o["id"]),
            esc(&o["due"]),
            esc(&o["signal"]),
            esc(&o["text"])
        ));
    }
    lines.push("</objectives>".into());
    let paths: String = list(&brief["north_stars"])
        .iter()
        .map(|p| format!("<path>{}</path>", esc(p)))
        .collect();
    lines.push(format!("<north_stars>{paths}</north_stars>"));
    lines.push("<sources_of_truth>".into());
    for s in list(&brief["sources_of_truth"]) {
        lines.push(format!(
            "<source kind=\"{}\" ref=\"{}\">{}</source>",
            esc(&s["kind"]),
            esc(&s["ref"]),
            esc(&s["note"])
        ));
    }
    lines.push("</sources_of_truth>".into());
    let items: String = list(&brief["out_of_scope"])
        .iter()
        .map(|x| format!("<item>{}</item>", esc(x)))
        .collect();
    lines.push(format!("<out_of_scope>{items}</out_of_scope>"));
    lines.push("<roles>".into());
    if let Some(roles) = brief["roles"].as_object() {
        for (id, why) in roles {
            lines.push(format!(
                "<agent id=\"{}\">{}</agent>",
                esc(&Value::String(id.clone())),
                esc(why)
            ));
        }
    }
    lines.push("</roles>".into());
    lines.push("</brief>".into());
    lines.join("\n")
}

/// The ids of the objectives of a brief.
pub fn objective_ids(brief: &Value) -> Vec<String> {
    list(&brief["objectives"])
        .iter()
        .filter_map(|o| o["id"].as_str().map(String::from))
        .collect()
}

const SPLIT_AT: [&str; 9] = [
    "## North star",
    "## Open issues",
    "## Last minutes",
    "## Pending decisions",
    "## Stories",
    "## Commits",
    "## Journal tail",
    "## Docs index",
    "## Database sections",
];
const KEEP: [&str; 7] = [
    "# Snapshot",
    "## North star",
    "## Pending decisions",
    "## Last minutes",
    "## Stories",
    "## Commits",
    "## Docs index",
];

fn limit(section: &str) -> Option<usize> {
    match section {
        "## North star" => Some(5000),
        "## Stories" => Some(1500),
        "## Commits" => Some(2500),
        "## Docs index" => Some(3500),
        _ => None,
    }
}

/// The first `n` UTF-16 units of `s`, as `slice(0, n)` of JavaScript. A cut pair
/// becomes U+FFFD, as it does when JavaScript writes the text.
fn js_slice(s: &str, n: usize) -> String {
    if s.encode_utf16().count() <= n {
        return s.to_string();
    }
    let cut: Vec<u16> = s.encode_utf16().take(n).collect();
    String::from_utf16_lossy(&cut)
}

/// The sections of the snapshot that a meeting needs, each one cut to its limit.
pub fn compact_snapshot(full: &str, max_chars: usize) -> String {
    // Only the snapshot's own headings split it: the north-star docs and the journal carry `## ` headings of their own.
    let mut sections: Vec<&str> = Vec::new();
    let mut start = 0;
    for (i, _) in full.match_indices('\n') {
        let rest = &full[i + 1..];
        if SPLIT_AT.iter().any(|h| rest.starts_with(h)) {
            sections.push(&full[start..i]);
            start = i + 1;
        }
    }
    sections.push(&full[start..]);
    let mut out: Vec<String> = Vec::new();
    for s in sections {
        let Some(k) = KEEP.iter().find(|p| s.starts_with(*p)) else {
            continue;
        };
        match limit(k) {
            Some(n) if s.encode_utf16().count() > n => {
                out.push(format!("{}\n[cut]", js_slice(s, n)))
            }
            _ => out.push(s.to_string()),
        }
    }
    js_slice(&out.join("\n\n"), max_chars)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_compact_snapshot_keeps_its_sections_and_cuts_the_long_ones() {
        let full = format!(
            "# Snapshot of p\n\nintro\n## Open issues\nnone\n## Stories\n{}\n## Commits\n- a",
            "x".repeat(2000)
        );
        let out = compact_snapshot(&full, 14_000);
        assert!(out.starts_with("# Snapshot of p\n\nintro\n\n## Stories\n"));
        assert!(!out.contains("Open issues"));
        assert!(out.contains("\n[cut]\n\n## Commits\n- a"));
    }

    #[test]
    fn the_brief_xml_escapes_its_text() {
        let brief: Value = serde_json::from_str(
            r#"{"product":{"what":"a<b","audience":"x","stage":"s"},"objectives":[{"id":"o1","text":"t","signal":"\"q\"","due":"2026-01-01"}],"north_stars":["docs/v.md"],"sources_of_truth":[],"out_of_scope":[],"roles":{"pm":"P"}}"#,
        )
        .unwrap();
        let xml = brief_xml(&brief);
        assert!(xml.contains("<what>a&lt;b</what>"));
        assert!(xml.contains("signal=\"&quot;q&quot;\""));
        assert!(xml.contains("<north_stars><path>docs/v.md</path></north_stars>"));
    }

    #[test]
    fn a_cut_counts_utf16_units_as_javascript_does() {
        assert_eq!(js_slice("a😀b", 2), "a\u{fffd}");
        assert_eq!(js_slice("a😀b", 3), "a😀");
        assert_eq!(js_slice("añb", 9), "añb");
    }

    #[test]
    fn the_brief_keeps_the_order_of_its_roles_and_prints_a_number() {
        let brief: Value = serde_json::json!({
            "objectives": [{"id": "o1", "due": 2026}],
            "roles": {"qa": "Breaks.", "dev": "Builds."},
        });
        let xml = brief_xml(&brief);
        assert!(xml.contains("due=\"2026\""));
        assert!(xml.contains("signal=\"\""), "{xml}");
        assert!(
            xml.contains("<agent id=\"qa\">Breaks.</agent>\n<agent id=\"dev\">Builds.</agent>")
        );
    }
}
