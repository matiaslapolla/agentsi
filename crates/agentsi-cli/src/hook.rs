//! `agentsi hook pre-tool-use`: the push guard. It is a port of
//! `plugin/hooks/push-guard.mjs`. When the Bash command of a tool call runs
//! `git push` or `gh pr create`, it prints an `ask` decision. For any other
//! command, or a payload that it cannot read, it prints nothing. It always
//! exits with 0, so the normal permission flow decides.
//!
//! The port keeps the quirks of the Node tokenizer on purpose: a verdict that
//! differs is a push that goes through. `tests/push_guard.rs` holds the table
//! of the Node test and the verdicts that the Node guard gave on more cases.

use crate::commands::Fail;
use serde_json::{Value, json};
use std::io::Read;

pub const USAGE: &str = "agentsi hook pre-tool-use";

const REASON: &str = "AgentSI: this command publishes code (push or pull request). Confirm only if the owner asked for it explicitly.";

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    match argv {
        [event] if event == "pre-tool-use" => {
            let mut input = Vec::new();
            // A payload that cannot be read is not a push.
            // The whole payload, as the Node guard reads it: a cut payload would not parse,
            // and the push in it would go through.
            let _ = std::io::stdin().lock().read_to_end(&mut input);
            if let Some(out) = answer(&input) {
                print!("{out}");
            }
            Ok(0)
        }
        _ => Err(Fail::Usage("Unknown hook event.".into())),
    }
}

/// The text to print for a hook payload, or `None` when nothing is guarded.
pub fn answer(payload: &[u8]) -> Option<String> {
    // Node decodes bad bytes to U+FFFD and goes on.
    let value: Value = serde_json::from_str(&String::from_utf8_lossy(payload)).ok()?;
    let command = value.get("tool_input")?.get("command")?.as_str()?;
    decision(command).map(|d| d.to_string())
}

pub fn decision(command: &str) -> Option<Value> {
    is_guarded(command).then(|| {
        json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "ask",
            "permissionDecisionReason": REASON,
        }})
    })
}

fn is_separator(c: char) -> bool {
    matches!(c, ';' | '|' | '&' | '\n' | '(' | ')')
}

/// The `\s` of JavaScript.
fn is_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

/// Splits a command line into words and operators. Quotes group words and
/// hide operators. A comment ends the line. An empty quoted string is no word.
fn tokenize(command: &[char]) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let flush = |word: &mut String, tokens: &mut Vec<String>| {
        if !word.is_empty() {
            tokens.push(std::mem::take(word));
        }
    };
    let at = |i: usize| command.get(i).copied();
    let mut i = 0;
    while i < command.len() {
        let c = command[i];
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else if c == '\\' && q == '"' && i + 1 < command.len() {
                i += 1;
                word.push(command[i]);
            } else {
                word.push(c);
            }
        } else if c == '$' && word.is_empty() && matches!(at(i + 1), Some('\'' | '"')) {
            // `$'...'` and `$"..."`: the `$` is dropped.
        } else if c == '\'' || c == '"' {
            quote = Some(c);
        } else if c == '\\' && at(i + 1) == Some('\n') {
            flush(&mut word, &mut tokens);
            i += 1;
        } else if c == '\\' && i + 1 < command.len() {
            i += 1;
            word.push(command[i]);
        } else if c == '#' && word.is_empty() {
            while i + 1 < command.len() && command[i + 1] != '\n' {
                i += 1;
            }
        } else if is_separator(c) {
            flush(&mut word, &mut tokens);
            match (c, at(i + 1)) {
                ('&', Some('&')) => {
                    tokens.push("&&".into());
                    i += 1;
                }
                ('|', Some('|')) => {
                    tokens.push("||".into());
                    i += 1;
                }
                _ => tokens.push(c.to_string()),
            }
        } else if is_space(c) {
            flush(&mut word, &mut tokens);
        } else {
            word.push(c);
        }
        i += 1;
    }
    flush(&mut word, &mut tokens);
    tokens
}

/// The `$(...)` and backtick substitutions that the shell runs, at one nesting
/// level. Single quotes keep everything literal. Double quotes hide
/// parentheses but not substitutions. With `nested`, the scan stops at the `)`
/// that closes `$(`. Returns the index where it stopped and the bodies.
fn scan(command: &[char], from: usize, nested: bool) -> (usize, Vec<String>) {
    let mut found = Vec::new();
    let (mut depth, mut single, mut double) = (1, false, false);
    let mut i = from;
    while i < command.len() {
        let c = command[i];
        if single {
            single = c != '\'';
        } else if c == '\\' {
            i += 1;
        } else if c == '\'' && !double {
            single = true;
        } else if c == '"' {
            double = !double;
        } else if c == '$' && command.get(i + 1) == Some(&'(') {
            let (end, _) = scan(command, i + 2, true);
            found.push(command[i + 2..end].iter().collect());
            i = end;
        } else if c == '`' {
            let mut body = String::new();
            i += 1;
            while i < command.len() && command[i] != '`' {
                if command[i] == '\\' && command.get(i + 1) == Some(&'`') {
                    i += 1;
                }
                body.push(command[i]);
                i += 1;
            }
            found.push(body);
        } else if nested && !double && c == '(' {
            depth += 1;
        } else if nested && !double && c == ')' {
            depth -= 1;
            if depth == 0 {
                return (i, found);
            }
        }
        i += 1;
    }
    (command.len(), found)
}

/// The first word after the options, and the word after it. `with_arg`
/// options take the next word.
fn subcommand<'a>(words: &'a [String], with_arg: &[&str]) -> Option<(&'a str, Option<&'a str>)> {
    let mut i = 0;
    while i < words.len() {
        if !words[i].starts_with('-') {
            return Some((&words[i], words.get(i + 1).map(String::as_str)));
        }
        if with_arg.contains(&words[i].as_str()) {
            i += 1;
        }
        i += 1;
    }
    None
}

/// `-c`, `-lc`: the `/^-[a-z]*c$/` of the Node guard.
fn is_command_flag(t: &str) -> bool {
    t.strip_prefix('-')
        .and_then(|s| s.strip_suffix('c'))
        .is_some_and(|s| s.chars().all(|c| c.is_ascii_lowercase()))
}

/// True when the command line runs `git push` or `gh pr create`. It looks at
/// every word, so a wrapper such as `time`, `env -i` or `xargs` does not hide
/// the command. A word inside a quoted string does not count, except the
/// string that follows `-c` or `eval`, which is a command line itself. A
/// `$(...)` or backtick substitution is a command line too, also inside double
/// quotes.
pub fn is_guarded(command: &str) -> bool {
    let chars: Vec<char> = command.chars().collect();
    let tokens = tokenize(&chars);
    if scan(&chars, 0, false).1.iter().any(|s| is_guarded(s)) {
        return true;
    }
    tokens.iter().enumerate().any(|(i, t)| {
        let name = t.rsplit('/').next().unwrap_or("");
        let rest = &tokens[i + 1..];
        if name == "git"
            && subcommand(rest, &["-C", "-c", "--git-dir", "--work-tree"])
                .is_some_and(|(a, _)| a == "push")
        {
            return true;
        }
        if name == "gh"
            && subcommand(rest, &["-R", "--repo"])
                .is_some_and(|(a, b)| a == "pr" && b == Some("create"))
        {
            return true;
        }
        (t == "eval" || is_command_flag(t)) && rest.first().is_some_and(|inner| is_guarded(inner))
    })
}
