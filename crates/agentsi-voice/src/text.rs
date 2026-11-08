//! The text rules of voice mode. They port `plugin/hooks/voice-mode.mjs`.

use regex::Regex;
use std::sync::LazyLock;

/// The line above which a reply is spoken. Below it, the reply stays on screen.
pub const SCREEN_LINE: &str = "On screen only";
/// The role whose voice speaks a reply.
pub const VOICE: &str = "pm";

/// The style rules that the hook gives to Claude when voice mode is on.
pub const RULES: &str = "Voice mode is on: the owner listens to your reply through a speaker and reads the rest on screen.\n\
Write the reply in two parts. First, a short spoken part in plain sentences: at most a few sentences, no Markdown, no lists, no tables, no code, no file paths, no URLs.\n\
Then, if there is anything that needs the eyes (code, commands, paths, tables, long lists, details), put a line with exactly \"On screen only\" and write all of it below that line.\n\
Everything above the \"On screen only\" line is read aloud, so keep it short and say the result first.\n\
If the whole reply is short and plain, you do not need the \"On screen only\" line.";

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("a fixed pattern")
}

static FENCE: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)```.*?(?:```|\z)"));
static TILDE_FENCE: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)~~~.*?(?:~~~|\z)"));
static HEADING: LazyLock<Regex> = LazyLock::new(|| re(r"^\s{0,3}#{1,6}\s"));
static TABLE: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*\|"));
static RULE: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*[-*_]{3,}\s*$"));
static MARKER: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*(?:>|[-*+]|[0-9]+[.)])\s+"));
static IMAGE: LazyLock<Regex> = LazyLock::new(|| re(r"!\[[^\]]*\]\([^)]*\)"));
static LINK: LazyLock<Regex> = LazyLock::new(|| re(r"\[([^\]]*)\]\([^)]*\)"));
static CODE: LazyLock<Regex> = LazyLock::new(|| re(r"`[^`]*`"));
static URL: LazyLock<Regex> = LazyLock::new(|| re(r"https?://\S+"));
static PATH: LazyLock<Regex> = LazyLock::new(|| re(r"\S*/\S*"));
static FILE: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?-u:\b)(?-u:[\w-])+\.(?:tsx?|jsx?|mjs|cjs|json|md|mdx|py|sh|toml|ya?ml|sql|css|html|lock|log|txt|env)(?-u:\b)",
    )
});
static EMPHASIS: LazyLock<Regex> = LazyLock::new(|| re(r"\*\*|__|\*|_|~~"));
static PARAGRAPH: LazyLock<Regex> = LazyLock::new(|| re(r"\n\s*\n"));
static SPACE: LazyLock<Regex> = LazyLock::new(|| re(r"\s+"));
static BEFORE_PUNCT: LazyLock<Regex> = LazyLock::new(|| re(r"\s+([,.;:!?])"));

/// true when `line` is the "On screen only" line, with or without Markdown marks.
fn is_screen_line(line: &str) -> bool {
    let key: String = line
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '#' | '>' | ':') && !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    let want: String = SCREEN_LINE
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    key == want
}

/// Splits after `.`, `!` or `?` that is followed by white space. The input has no edge white space.
fn split_sentences(paragraph: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut skipping = false;
    let mut prev_end = false;
    for (i, c) in paragraph.char_indices() {
        if skipping {
            if c.is_whitespace() {
                continue;
            }
            skipping = false;
            start = i;
        } else if prev_end && c.is_whitespace() {
            out.push(&paragraph[start..i]);
            skipping = true;
        }
        prev_end = matches!(c, '.' | '!' | '?');
    }
    if !skipping {
        out.push(&paragraph[start..]);
    }
    out
}

/// A Markdown reply to the plain sentences to speak: only what is above the
/// "On screen only" line, with no code, tables, links, paths or marks.
pub fn spoken_sentences(reply: &str) -> Vec<String> {
    let lines: Vec<&str> = reply.split('\n').collect();
    let cut = lines.iter().position(|l| is_screen_line(l));
    let text = lines[..cut.unwrap_or(lines.len())].join("\n");
    let text = FENCE.replace_all(&text, "\n");
    let text = TILDE_FENCE.replace_all(&text, "\n");
    let text = text
        .split('\n')
        .filter(|l| !HEADING.is_match(l) && !TABLE.is_match(l) && !RULE.is_match(l))
        .map(|l| MARKER.replace(l, "").into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    let text = IMAGE.replace_all(&text, " ");
    let text = LINK.replace_all(&text, "$1");
    let text = CODE.replace_all(&text, " ");
    let text = URL.replace_all(&text, " ");
    let text = PATH.replace_all(&text, " ");
    let text = FILE.replace_all(&text, " ");
    let text = EMPHASIS.replace_all(&text, "");
    PARAGRAPH
        .split(&text)
        .map(|p| SPACE.replace_all(p, " ").trim().to_string())
        .filter(|p| !p.is_empty())
        .flat_map(|p| {
            split_sentences(&p)
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .map(|s| BEFORE_PUNCT.replace_all(&s, "$1").trim().to_string())
        .filter(|s| s.chars().any(char::is_alphanumeric))
        .collect()
}

/// The first lines of a transcript file.
pub fn transcript_head(session: &str) -> String {
    format!("# Voice session {session}\n\n")
}

/// One turn of a transcript file, as `voice-mode.mjs` writes it.
pub fn transcript_entry(
    timestamp: &str,
    prompt: &str,
    reply: &str,
    sentences: &[String],
) -> String {
    let spoken = if sentences.is_empty() {
        "> (nothing spoken)".to_string()
    } else {
        sentences
            .iter()
            .map(|s| format!("> {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "## {timestamp}\n\n**Owner:**\n\n{prompt}\n\n**Reply:**\n\n{reply}\n\n**Spoken part (voice {VOICE}):**\n\n{spoken}\n\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &str = "The build passes now. I fixed two tests.\n\n## Heading-words\n\nI changed packages/core/src/replay.ts to do it, with **bold** care.\n\n```ts\nconst secretCode = 42;\n```\n\nOn screen only\n\nTail-details: a long table that must stay on screen.";

    #[test]
    fn it_speaks_only_the_clean_prose_above_the_line() {
        let s = spoken_sentences(REPLY);
        assert_eq!(
            s,
            [
                "The build passes now.",
                "I fixed two tests.",
                "I changed to do it, with bold care."
            ]
        );
        let joined = s.join("\n");
        for banned in [
            "secretCode",
            "const ",
            "On screen only",
            "Tail-details",
            "packages/core",
            "replay.ts",
            "**",
            "```",
            "##",
        ] {
            assert!(!joined.contains(banned), "{banned}");
        }
    }

    #[test]
    fn the_line_matches_with_marks_and_case() {
        for line in [
            "**On screen only**",
            "## on screen only:",
            "> ON SCREEN  ONLY",
        ] {
            let reply = format!("Spoken part here.\n\n{line}\n\nHidden part.");
            assert_eq!(spoken_sentences(&reply), ["Spoken part here."], "{line}");
        }
    }

    #[test]
    fn a_reply_with_no_line_is_spoken_whole() {
        assert_eq!(
            spoken_sentences("Done! Is it good? Yes."),
            ["Done!", "Is it good?", "Yes."]
        );
    }

    #[test]
    fn it_drops_lists_tables_links_and_urls() {
        let reply = "Result first.\n\n- item one here\n1. item two here\n| a | b |\n---\nSee [the docs](https://x.y/z) and https://x.y now. ![img](a.png) `inline code` end.";
        assert_eq!(
            spoken_sentences(reply),
            [
                "Result first.",
                "item one here item two here See the docs and now.",
                "end."
            ]
        );
    }

    #[test]
    fn an_open_fence_hides_the_rest() {
        assert_eq!(
            spoken_sentences("Before.\n```\ncode never closed. more"),
            ["Before."]
        );
        assert_eq!(
            spoken_sentences("Before.\n~~~\ncode never closed."),
            ["Before."]
        );
    }

    #[test]
    fn word_marks_follow_javascript_ascii_rules() {
        // JS `\w`, `\b` and `\d` are ASCII: a file name with an accent stays, an Arabic digit marker stays.
        assert_eq!(
            spoken_sentences("See café.json now."),
            ["See café.json now."]
        );
        assert_eq!(spoken_sentences("x.mdé is here."), ["é is here."]);
        assert_eq!(
            spoken_sentences("\u{661}. first item."),
            ["\u{661}.", "first item."]
        );
    }

    #[test]
    fn it_gives_nothing_for_a_reply_without_words() {
        assert!(spoken_sentences("```\nonly code\n```").is_empty());
        assert!(spoken_sentences("").is_empty());
        assert!(spoken_sentences("...\n\n!!!").is_empty());
    }

    #[test]
    fn the_transcript_has_the_format_of_today() {
        let entry = transcript_entry(
            "2026-11-03T10:00:00.000Z",
            "hi",
            "Hello there.",
            &["Hello there.".into()],
        );
        assert_eq!(
            entry,
            "## 2026-11-03T10:00:00.000Z\n\n**Owner:**\n\nhi\n\n**Reply:**\n\nHello there.\n\n**Spoken part (voice pm):**\n\n> Hello there.\n\n"
        );
        assert!(transcript_entry("t", "p", "r", &[]).contains("> (nothing spoken)"));
        assert_eq!(transcript_head("s1"), "# Voice session s1\n\n");
    }
}
