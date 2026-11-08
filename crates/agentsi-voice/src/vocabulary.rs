//! The vocabulary: terms that no speech model hears right, such as product names.
//!
//! `<config>/agentsi/vocabulary.toml` holds entries. Each entry has a `term` and the
//! `heard` forms that the model gives for it. [`Vocabulary::apply`] replaces each heard
//! form with its term, as a whole word and ignoring case.
//!
//! ```toml
//! [[entry]]
//! term = "AgentSI"
//! heard = ["Agent C"]
//! ```

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FILE: &str = "vocabulary.toml";

/// Forms that the P2 dry run heard for our own product name.
const KNOWN_HEARD: &[(&str, &[&str])] = &[("AgentSI", &["Agent C", "Agent CI", "Agent sea"])];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub term: String,
    #[serde(default)]
    pub heard: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vocabulary {
    #[serde(default, rename = "entry")]
    pub entries: Vec<Entry>,
}

/// `<config>/agentsi/vocabulary.toml`, where `<config>` is `$XDG_CONFIG_HOME` or `~/.config`.
pub fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".config"))
        })?;
    Some(base.join("agentsi").join(FILE))
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Vocabulary {
    pub fn parse(text: &str) -> Result<Vocabulary, String> {
        let mut v: Vocabulary = toml::from_str(text).map_err(|e| e.to_string())?;
        v.clean();
        Ok(v)
    }

    pub fn to_toml(&self) -> String {
        let mut out = String::from(
            "# Terms that the speech model hears wrong. The app replaces each heard form with its term.\n",
        );
        for e in &self.entries {
            out.push_str("\n[[entry]]\n");
            out.push_str(&format!("term = {}\n", toml_string(&e.term)));
            let heard: Vec<String> = e.heard.iter().map(|h| toml_string(h)).collect();
            out.push_str(&format!("heard = [{}]\n", heard.join(", ")));
        }
        out
    }

    /// Trims every text, drops empty ones and repeats, and merges entries with the same term.
    pub fn clean(&mut self) {
        let mut merged: Vec<Entry> = Vec::new();
        for entry in std::mem::take(&mut self.entries) {
            let term = entry.term.trim().to_owned();
            if term.is_empty() {
                continue;
            }
            let at = merged
                .iter()
                .position(|m| m.term.to_lowercase() == term.to_lowercase())
                .unwrap_or_else(|| {
                    merged.push(Entry {
                        term: term.clone(),
                        heard: Vec::new(),
                    });
                    merged.len() - 1
                });
            for h in entry.heard {
                let h = h.split_whitespace().collect::<Vec<_>>().join(" ");
                let known = merged[at]
                    .heard
                    .iter()
                    .any(|x| x.to_lowercase() == h.to_lowercase());
                if !h.is_empty() && !known {
                    merged[at].heard.push(h);
                }
            }
        }
        self.entries = merged;
    }

    /// Reads the file. A missing file gives `None`.
    pub fn load(path: &Path) -> Result<Option<Vocabulary>, String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Vocabulary::parse(&text)
                .map(Some)
                .map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Writes the file next to a temporary file and renames it, so a stop cannot leave half a file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let err = |e: std::io::Error| format!("{}: {e}", path.display());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, self.to_toml()).map_err(err)?;
        std::fs::rename(&tmp, path).map_err(err)
    }

    /// The list for a first start: one entry for each product name.
    ///
    /// Each name hears as itself in another case, as its camel-case words, and as the
    /// name with spaced letters. A name that the dry run mis-heard has those forms too.
    pub fn seed<S: AsRef<str>>(names: &[S]) -> Vocabulary {
        let mut v = Vocabulary {
            entries: names
                .iter()
                .map(|n| n.as_ref().trim())
                .filter(|n| !n.is_empty())
                .map(|name| Entry {
                    term: name.to_owned(),
                    heard: heard_forms(name),
                })
                .collect(),
        };
        v.clean();
        v
    }

    /// The matcher for the heard forms, or `None` when there are none or the pattern is too large.
    fn matcher(&self) -> Option<Matcher> {
        let mut forms: Vec<(String, &str)> = self
            .entries
            .iter()
            .flat_map(|e| e.heard.iter().map(|h| (h.to_lowercase(), e.term.as_str())))
            .collect();
        if forms.is_empty() {
            return None;
        }
        // The longest form is tried first where two start at the same place.
        forms.sort_by_key(|(h, _)| std::cmp::Reverse(h.chars().count()));
        let build = |pattern: &str| {
            RegexBuilder::new(pattern)
                .case_insensitive(true)
                .unicode(true)
                .build()
                .ok()
        };
        let finder = build(
            &forms
                .iter()
                .map(|(h, _)| regex::escape(h))
                .collect::<Vec<_>>()
                .join("|"),
        )?;
        let mut each: Vec<(Regex, String)> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (h, term) in forms {
            if seen.insert(h.clone()) {
                each.push((
                    build(&format!("^(?:{})", regex::escape(&h)))?,
                    term.to_owned(),
                ));
            }
        }
        Some(Matcher { finder, each })
    }

    /// Replaces each heard form with its term. The text is otherwise unchanged.
    pub fn apply(&self, text: &str) -> String {
        match self.matcher() {
            Some(m) => m.apply(text),
            None => text.to_owned(),
        }
    }
}

struct Matcher {
    /// Finds where any heard form starts.
    finder: Regex,
    /// Each heard form alone, anchored, longest first, with its term.
    each: Vec<(Regex, String)>,
}

impl Matcher {
    /// The longest heard form at the start of `text` that is a whole word.
    fn at(&self, text: &str, start: usize) -> Option<(usize, &str)> {
        let before = text[..start].chars().next_back();
        self.each.iter().find_map(|(regex, term)| {
            let end = start + regex.find(&text[start..])?.end();
            let first = text[start..end].chars().next();
            let last = text[start..end].chars().next_back();
            let after = text[end..].chars().next();
            let whole = !(first.is_some_and(is_word) && before.is_some_and(is_word))
                && !(last.is_some_and(is_word) && after.is_some_and(is_word));
            whole.then_some((end, term.as_str()))
        })
    }

    fn apply(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let (mut copied, mut from) = (0, 0);
        while from <= text.len()
            && let Some(m) = self.finder.find_at(text, from)
        {
            if let Some((end, term)) = self.at(text, m.start()) {
                out.push_str(&text[copied..m.start()]);
                out.push_str(term);
                copied = end;
                from = end;
            } else {
                // Not a whole word: try again one character on.
                from = m.start() + text[m.start()..].chars().next().map_or(1, char::len_utf8);
            }
        }
        out.push_str(&text[copied..]);
        out
    }
}

fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Words of a camel-case name: `AgentSI` gives `Agent`, `SI`.
fn camel_words(name: &str) -> Vec<String> {
    let chars: Vec<char> = name.chars().collect();
    let mut words: Vec<String> = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        let starts = i > 0
            && c.is_uppercase()
            && (chars[i - 1].is_lowercase()
                || chars[i - 1].is_numeric()
                || (chars[i - 1].is_uppercase()
                    && chars.get(i + 1).is_some_and(|n| n.is_lowercase())));
        if starts || words.is_empty() {
            words.push(String::new());
        }
        if let Some(w) = words.last_mut() {
            w.push(c);
        }
    }
    words
}

fn heard_forms(name: &str) -> Vec<String> {
    let mut forms = vec![name.to_lowercase()];
    let words = camel_words(name);
    if words.len() > 1 {
        forms.push(words.join(" "));
        // "SI" is heard as "S I" or "C": spell the all-capital words letter by letter.
        let spelled: Vec<String> = words
            .iter()
            .map(|w| {
                if w.chars().count() > 1 && w.chars().all(char::is_uppercase) {
                    w.chars().map(String::from).collect::<Vec<_>>().join(" ")
                } else {
                    w.clone()
                }
            })
            .collect();
        forms.push(spelled.join(" "));
    }
    if let Some((_, known)) = KNOWN_HEARD
        .iter()
        .find(|(t, _)| t.eq_ignore_ascii_case(name))
    {
        forms.extend(known.iter().map(|s| s.to_string()));
    }
    forms.retain(|f| f != name);
    forms
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab(term: &str, heard: &[&str]) -> Vocabulary {
        Vocabulary {
            entries: vec![Entry {
                term: term.into(),
                heard: heard.iter().map(|s| s.to_string()).collect(),
            }],
        }
    }

    #[test]
    fn agent_c_becomes_agentsi() {
        let v = vocab("AgentSI", &["Agent C"]);
        assert_eq!(v.apply("Open Agent C now."), "Open AgentSI now.");
        assert_eq!(v.apply("agent c, please"), "AgentSI, please");
        assert_eq!(v.apply("AGENT C"), "AgentSI");
    }

    #[test]
    fn a_shorter_form_applies_where_the_longer_one_is_not_a_whole_word() {
        let v = vocab("AgentSI", &["agent c", "agent"]);
        assert_eq!(v.apply("Agent Cat"), "AgentSI Cat");
        assert_eq!(v.apply("Agent C here"), "AgentSI here");
        assert_eq!(v.apply("an Agent here"), "an AgentSI here");
    }

    #[test]
    fn only_a_whole_word_is_replaced() {
        let v = vocab("AgentSI", &["Agent C"]);
        assert_eq!(v.apply("The Agent Cat sat."), "The Agent Cat sat.");
        assert_eq!(v.apply("Magent C"), "Magent C");
        assert_eq!(v.apply("Agent C Agent C"), "AgentSI AgentSI");
    }

    #[test]
    fn the_longest_form_wins_and_a_term_is_not_replaced_again() {
        let v = Vocabulary {
            entries: vec![
                Entry {
                    term: "Hex".into(),
                    heard: vec!["hecks".into()],
                },
                Entry {
                    term: "Hex Pro".into(),
                    heard: vec!["hecks pro".into(), "hecks".into()],
                },
            ],
        };
        assert_eq!(v.apply("hecks pro and hecks"), "Hex Pro and Hex");
    }

    #[test]
    fn marks_in_a_heard_form_and_other_alphabets_work() {
        let v = vocab("C++", &["see plus plus", "c plus plus"]);
        assert_eq!(v.apply("I like see plus plus."), "I like C++.");
        let es = vocab("José", &["jose"]);
        assert_eq!(es.apply("hola Jose, ¿qué tal?"), "hola José, ¿qué tal?");
        assert_eq!(es.apply("josesito"), "josesito");
    }

    #[test]
    fn an_empty_list_changes_nothing() {
        assert_eq!(Vocabulary::default().apply("Agent C"), "Agent C");
    }

    #[test]
    fn the_file_round_trips_and_is_cleaned() {
        let v = Vocabulary::parse(
            "[[entry]]\nterm = \" AgentSI \"\nheard = [\"Agent C\", \"agent  c\", \"\"]\n\n[[entry]]\nterm = \"agentsi\"\nheard = [\"Agent see\"]\n\n[[entry]]\nterm = \"\"\n",
        )
        .unwrap();
        assert_eq!(v.entries.len(), 1);
        assert_eq!(v.entries[0].term, "AgentSI");
        assert_eq!(v.entries[0].heard, vec!["Agent C", "Agent see"]);
        assert_eq!(Vocabulary::parse(&v.to_toml()).unwrap(), v);
        assert!(Vocabulary::parse("[[entry]]\nterm = 1\n").is_err());
        assert!(Vocabulary::parse("[[wrong]]\n").is_err());
    }

    #[test]
    fn a_quote_in_a_term_survives_the_file() {
        let v = vocab("say \"hi\"\\", &["a\tb"]);
        let back = Vocabulary::parse(&v.to_toml()).unwrap();
        assert_eq!(back.entries[0].term, "say \"hi\"\\");
    }

    #[test]
    fn save_and_load_use_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agentsi").join(FILE);
        assert_eq!(Vocabulary::load(&path).unwrap(), None);
        let v = vocab("AgentSI", &["Agent C"]);
        v.save(&path).unwrap();
        assert_eq!(Vocabulary::load(&path).unwrap(), Some(v));
        std::fs::write(&path, "not [toml").unwrap();
        assert!(Vocabulary::load(&path).is_err());
    }

    #[test]
    fn the_seed_has_the_product_names_with_their_forms() {
        let v = Vocabulary::seed(&["AgentSI", "Herdr", "Lumen", "  "]);
        let terms: Vec<&str> = v.entries.iter().map(|e| e.term.as_str()).collect();
        assert_eq!(terms, ["AgentSI", "Herdr", "Lumen"]);
        assert_eq!(v.apply("open Agent C and herdr"), "open AgentSI and Herdr");
        assert_eq!(v.apply("agent S I"), "AgentSI");
        assert_eq!(v.apply("Agent SI"), "AgentSI");
    }

    #[test]
    fn camel_case_names_split_into_words() {
        assert_eq!(camel_words("AgentSI"), ["Agent", "SI"]);
        assert_eq!(camel_words("GPUIKit"), ["GPUI", "Kit"]);
        assert_eq!(camel_words("herdr"), ["herdr"]);
    }
}
