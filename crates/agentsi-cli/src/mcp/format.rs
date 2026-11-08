//! The text of each tool. The rules copy `packages/mcp/src/server.ts`,
//! `list-minutes.ts` and `pending-decisions.ts`. Offsets and lengths count
//! UTF-16 units, as JavaScript counts them.

use super::tools::MAX_DOC_CHARS;
use agentsi_proto::knowledge_api::{KbDoc, KbPendingResult, KbRecentMinutes, KbSearchHit};
use regex::Regex;
use std::sync::LazyLock;

static FENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*(```|~~~)").unwrap());
static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(#{1,6})\s+(.+?)\s*#*\s*$").unwrap());

fn units(text: &str) -> usize {
    text.encode_utf16().count()
}

/// `text.slice(from, to)` in UTF-16 units. A cut inside a pair gives U+FFFD.
fn slice(text: &str, from: usize, to: usize) -> String {
    let all: Vec<u16> = text.encode_utf16().collect();
    let to = to.min(all.len());
    String::from_utf16_lossy(&all[from.min(to)..to])
}

struct Heading {
    level: usize,
    text: String,
    start: usize,
}

fn headings(body: &str) -> Vec<Heading> {
    let mut out = Vec::new();
    let mut fenced = false;
    let mut start = 0;
    for line in body.split('\n') {
        if FENCE.is_match(line) {
            fenced = !fenced;
        }
        if !fenced && let Some(m) = HEADING.captures(line) {
            out.push(Heading {
                level: m[1].len(),
                text: m[2].to_string(),
                start,
            });
        }
        start += units(line) + 1;
    }
    out
}

/// The section with this heading: the exact text first, then a heading that contains it.
fn section(body: &str, name: &str) -> Option<String> {
    let hs = headings(body);
    let want = name
        .trim_start_matches('#')
        .trim_start()
        .trim()
        .to_lowercase();
    let at = hs
        .iter()
        .position(|h| h.text.to_lowercase() == want)
        .or_else(|| {
            hs.iter()
                .position(|h| h.text.to_lowercase().contains(&want))
        })?;
    let h = &hs[at];
    let end = hs[at + 1..]
        .iter()
        .find(|n| n.level <= h.level)
        .map_or_else(|| units(body), |n| n.start);
    Some(slice(body, h.start, end).trim_end().to_string())
}

fn h2_list(body: &str) -> String {
    headings(body)
        .into_iter()
        .filter(|h| h.level == 2)
        .map(|h| h.text)
        .collect::<Vec<_>>()
        .join(" | ")
}

pub fn search_text(product: &str, query: &str, hits: &[KbSearchHit], any_term: bool) -> String {
    if hits.is_empty() {
        return format!("No hits in {product} for \"{query}\".");
    }
    let note = if any_term {
        "No hits for all terms; showing hits for any term.\n"
    } else {
        ""
    };
    let lines: Vec<String> = hits
        .iter()
        .enumerate()
        .map(|(i, h)| {
            let reference = if h.kind == "commit" {
                h.reference.chars().take(12).collect()
            } else {
                h.reference.clone()
            };
            let title = h.title.clone().unwrap_or_else(|| "null".into());
            let mut snippet = h.snippet.split_whitespace().collect::<Vec<_>>().join(" ");
            if let Some(rest) = snippet.strip_prefix(&title) {
                snippet = rest.trim().to_string();
            }
            let scope = h
                .scope
                .as_deref()
                .map(|s| format!(" scope={s}"))
                .unwrap_or_default();
            let snippet = if snippet.is_empty() {
                String::new()
            } else {
                format!("\n   {snippet}")
            };
            format!(
                "{}. [{}] ref={reference}{scope}\n   {title}{snippet}",
                i + 1,
                h.kind
            )
        })
        .collect();
    format!("{note}{}", lines.join("\n"))
}

pub fn doc_text(
    doc: &KbDoc,
    section_name: Option<&str>,
    offset: Option<usize>,
) -> Result<String, String> {
    let mut body = doc.body.clone().unwrap_or_default();
    if let Some(name) = section_name {
        body = section(&body, name).ok_or_else(|| {
            let sections = h2_list(&body);
            format!(
                "No section \"{name}\" in {}. Sections: {}",
                doc.reference,
                if sections.is_empty() {
                    "none"
                } else {
                    &sections
                }
            )
        })?;
    }
    let total = units(&body);
    let from = offset.unwrap_or(0);
    let to = total.min(from + MAX_DOC_CHARS);
    let title = doc.title.clone().unwrap_or_else(|| "null".into());
    let updated = doc.updated_at.as_deref().map_or_else(String::new, stamp);
    let head = format!(
        "# {title}\nkind={} ref={} updated={updated}\n\n",
        doc.kind, doc.reference
    );
    if from == 0 && to == total {
        return Ok(head + &body);
    }
    let sections = h2_list(&body);
    let tail = if to < total {
        let more = if sections.is_empty() {
            String::new()
        } else {
            format!(" or section=<heading>. Sections: {sections}")
        };
        format!("[truncated: chars {from}-{to} of {total}. Request offset={to}{more}]")
    } else {
        format!("[chars {from}-{to} of {total}]")
    };
    Ok(format!("{head}{}\n\n{tail}", slice(&body, from, to)))
}

pub fn minutes_text(product: &str, minutes: &[KbRecentMinutes]) -> String {
    if minutes.is_empty() {
        return format!("No minutes yet for {product}.");
    }
    minutes
        .iter()
        .map(|m| {
            let marks: Vec<String> = m
                .marks
                .iter()
                .map(|k| format!("{} {}", k.count, k.mark))
                .collect();
            let marks = if marks.is_empty() {
                "no priorities".to_string()
            } else {
                marks.join(", ")
            };
            let null = || "null".to_string();
            format!(
                "{} | {} | {} | {} | marks: {marks}{}",
                m.id,
                m.status.clone().unwrap_or_else(null),
                m.created_at.as_deref().map_or_else(String::new, stamp),
                m.question.clone().unwrap_or_else(null),
                if m.archived { " | archived" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn pending_text(product: &str, pending: &KbPendingResult) -> String {
    if pending.decisions.is_empty() {
        return format!("No pending decisions for {product}.");
    }
    let mut lines = Vec::new();
    let mut group: Option<&Option<String>> = None;
    for d in &pending.decisions {
        if group != Some(&d.minutes_id) {
            lines.push(format!(
                "minutes {}:",
                d.minutes_id.as_deref().unwrap_or("-")
            ));
        }
        group = Some(&d.minutes_id);
        let mut line = format!("  #{} {}", d.id, d.text);
        if let Some(why) = d.why.as_deref().filter(|w| !w.is_empty()) {
            line.push_str(&format!("\n     why: {why}"));
        }
        if let Some(action) = d.action.as_deref().filter(|a| !a.is_empty()) {
            line.push_str(&format!("\n     action: {action}"));
        }
        lines.push(line);
    }
    if pending.hidden > 0 {
        lines.push(format!("({} more not shown; raise limit)", pending.hidden));
    }
    lines.join("\n")
}

/// `YYYY-MM-DD HH:MM` in the local time zone, as the TypeScript `stamp` prints a `Date`.
pub fn stamp(iso: &str) -> String {
    let Some(secs) = epoch_seconds(iso) else {
        return iso.to_string();
    };
    let t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `t` and `tm` are valid for the call, and `localtime_r` keeps no pointer.
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return iso.to_string();
    }
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min
    )
}

/// The seconds since the epoch of `YYYY-MM-DDTHH:MM:SS` with a `Z` or no offset.
fn epoch_seconds(iso: &str) -> Option<i64> {
    let num = |from: usize, len: usize| iso.get(from..from + len)?.parse::<i64>().ok();
    let (y, m, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let (h, mi, s) = (
        num(11, 2).unwrap_or(0),
        num(14, 2).unwrap_or(0),
        num(17, 2).unwrap_or(0),
    );
    let y = y - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentsi_proto::knowledge_api::{KbMarkCount, KbPendingRow};

    fn hit(kind: &str, reference: &str, title: &str, snippet: &str) -> KbSearchHit {
        KbSearchHit {
            kind: kind.into(),
            reference: reference.into(),
            scope: None,
            title: Some(title.into()),
            rank: 1.0,
            snippet: snippet.into(),
        }
    }

    #[test]
    fn a_hit_shows_its_ref_title_and_snippet_without_the_repeated_title() {
        let hits = vec![
            hit(
                "commit",
                "0123456789abcdef0123",
                "feat: x",
                "feat: x\n body  line",
            ),
            hit("doc", "docs/a.md", "A", "other text"),
        ];
        let text = search_text("alpha", "q", &hits, true);
        assert_eq!(
            text,
            "No hits for all terms; showing hits for any term.\n\
1. [commit] ref=0123456789ab\n   feat: x\n   body line\n\
2. [doc] ref=docs/a.md\n   A\n   other text"
        );
        assert_eq!(
            search_text("alpha", "zzz", &[], false),
            "No hits in alpha for \"zzz\"."
        );
    }

    fn doc(body: &str) -> KbDoc {
        KbDoc {
            product: "alpha".into(),
            scope: None,
            kind: "doc".into(),
            reference: "docs/a.md".into(),
            title: Some("A".into()),
            body: Some(body.into()),
            updated_at: None,
        }
    }

    #[test]
    fn a_section_runs_to_the_next_heading_of_its_level_or_above() {
        let body = "# T\n\n## One\nx\n### Sub\ny\n## Two\nz\n```\n## not a heading\n```\n";
        let d = doc(body);
        let out = doc_text(&d, Some("one"), None).unwrap();
        assert!(out.ends_with("## One\nx\n### Sub\ny"), "{out}");
        let out = doc_text(&d, Some("## tw"), None).unwrap();
        assert!(out.contains("## Two\nz"), "{out}");
        let err = doc_text(&d, Some("missing"), None).unwrap_err();
        assert_eq!(
            err,
            "No section \"missing\" in docs/a.md. Sections: One | Two"
        );
    }

    #[test]
    fn a_long_body_is_cut_and_says_where_to_go_on() {
        let body = format!("## Head\n{}", "a".repeat(13_000));
        let out = doc_text(&doc(&body), None, None).unwrap();
        assert!(out.ends_with("[truncated: chars 0-12000 of 13008. Request offset=12000 or section=<heading>. Sections: Head]"));
        let rest = doc_text(&doc(&body), None, Some(12_000)).unwrap();
        assert!(rest.ends_with("[chars 12000-13008 of 13008]"));
    }

    #[test]
    fn the_lists_follow_the_typescript_text() {
        let minutes = vec![KbRecentMinutes {
            id: "m-1".into(),
            status: Some("valid".into()),
            created_at: None,
            question: Some("What?".into()),
            marks: vec![
                KbMarkCount {
                    mark: "accepted".into(),
                    count: 2,
                },
                KbMarkCount {
                    mark: "unmarked".into(),
                    count: 1,
                },
            ],
            archived: true,
        }];
        assert_eq!(
            minutes_text("alpha", &minutes),
            "m-1 | valid |  | What? | marks: 2 accepted, 1 unmarked | archived"
        );
        assert_eq!(minutes_text("alpha", &[]), "No minutes yet for alpha.");

        let row = |id, minutes_id: Option<&str>, why: Option<&str>| KbPendingRow {
            id,
            minutes_id: minutes_id.map(String::from),
            text: format!("do {id}"),
            why: why.map(String::from),
            action: None,
        };
        let pending = KbPendingResult {
            decisions: vec![
                row(1, Some("m-1"), Some("because")),
                row(2, Some("m-1"), None),
                row(3, None, None),
            ],
            hidden: 4,
        };
        assert_eq!(
            pending_text("alpha", &pending),
            "minutes m-1:\n  #1 do 1\n     why: because\n  #2 do 2\nminutes -:\n  #3 do 3\n(4 more not shown; raise limit)"
        );
    }

    #[test]
    fn the_stamp_uses_the_local_time_zone() {
        // The test env keeps the zone of the machine, so compare with the same libc call.
        let want = stamp("2026-11-04T10:30:15.000Z");
        assert_eq!(want.len(), 16);
        assert_eq!(&want[4..5], "-");
        assert_eq!(stamp("not a date"), "not a date");
    }
}
