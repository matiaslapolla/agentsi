//! The memory of a role as text. The rules copy `renderForPrompt` and
//! `memoryForPrompt` of `packages/core/src/memory.ts`.

use agentsi_store::read::MemoryItem;
use regex::Regex;
use std::sync::LazyLock;

/// The most characters of memory that a prompt gets.
pub const MEMORY_MAX_CHARS: usize = 6000;

/// `AGENT_ID` of `roles.ts`: `pm`, `dev`, or a specialist such as `dev.backend`.
static AGENT_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z]+(-[a-z]+)*(\.[a-z]+(-[a-z]+)*)?$").unwrap());

pub fn is_agent_id(id: &str) -> bool {
    AGENT_ID.is_match(id)
}

fn units(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// The active items, newest first, as `- [id] text`. Items that do not fit in
/// `max_chars` UTF-16 units are counted in a last line. None is cut.
pub fn render_for_prompt(items: &[MemoryItem], max_chars: Option<usize>) -> String {
    let mut active: Vec<(usize, &MemoryItem)> = items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.retired_at.is_none())
        .collect();
    // Newest first; items of one time come in reverse order of position.
    active.sort_by(|a, b| {
        b.1.created_at
            .cmp(&a.1.created_at)
            .then_with(|| b.0.cmp(&a.0))
    });
    let lines: Vec<String> = active
        .iter()
        .map(|(_, i)| format!("- [{}] {}", i.id, i.text))
        .collect();
    let with_count = |kept: usize| {
        let hidden = lines.len() - kept;
        let mut shown = lines[..kept].to_vec();
        if hidden > 0 {
            let noun = if hidden == 1 { "item" } else { "items" };
            shown.push(format!("({hidden} older {noun} not shown)"));
        }
        shown.join("\n")
    };
    let mut kept = lines.len();
    if let Some(max) = max_chars {
        while kept > 0 && units(&with_count(kept)) > max {
            kept -= 1;
        }
    }
    with_count(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, text: &str, created: &str, retired: bool) -> MemoryItem {
        MemoryItem {
            id: id.into(),
            text: text.into(),
            kind: "fact".into(),
            helpful: 0,
            harmful: 0,
            source: "t".into(),
            created_at: created.into(),
            retired_at: retired.then(|| "2026-10-02T00:00:00.000Z".into()),
            retired_why: None,
        }
    }

    #[test]
    fn the_newest_active_items_come_first() {
        let items = vec![
            item("m-a", "old", "2026-10-01T00:00:00.000Z", false),
            item("m-b", "gone", "2026-10-03T00:00:00.000Z", true),
            item("m-c", "new", "2026-10-02T00:00:00.000Z", false),
            item("m-d", "same time", "2026-10-02T00:00:00.000Z", false),
        ];
        assert_eq!(
            render_for_prompt(&items, None),
            "- [m-d] same time\n- [m-c] new\n- [m-a] old"
        );
        assert_eq!(render_for_prompt(&[], None), "");
    }

    #[test]
    fn items_past_the_limit_are_counted_not_cut() {
        let items = vec![
            item(
                "m-a",
                "aaaaaaaaaaaaaaaaaaaa",
                "2026-10-01T00:00:00.000Z",
                false,
            ),
            item(
                "m-b",
                "bbbbbbbbbbbbbbbbbbbb",
                "2026-10-02T00:00:00.000Z",
                false,
            ),
        ];
        let out = render_for_prompt(&items, Some(55));
        assert_eq!(
            out,
            "- [m-b] bbbbbbbbbbbbbbbbbbbb\n(1 older item not shown)"
        );
        assert_eq!(
            render_for_prompt(&items, Some(1)),
            "(2 older items not shown)"
        );
    }

    #[test]
    fn an_agent_id_is_a_role_or_a_specialist() {
        for ok in ["pm", "dev.backend", "front-end.web-ui"] {
            assert!(is_agent_id(ok), "{ok}");
        }
        for bad in ["", "PM", "dev.", "a..b", "x_y", "../etc"] {
            assert!(!is_agent_id(bad), "{bad}");
        }
    }
}
