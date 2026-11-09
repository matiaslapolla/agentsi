//! Dictation in the core.
//!
//! A compositor bind runs `agentsi-core dictate start` on key press and `dictate stop` on
//! release. A bind of the toggle form runs `dictate toggle` on press only. All three call
//! `voice.dictate`, and the core sends `dictation.start` or
//! `dictation.stop` to the pill. The pill records, transcribes and calls `voice.transcript`.
//! The core then applies the vocabulary and types the text into the focused pane.
//! The text ends with no Return, unless `send_return = true` is in `dictation.toml`.

use crate::state::Core;
use agentsi_proto::RpcError;
use agentsi_proto::client::Client;
use agentsi_proto::core_api::Topic;
use agentsi_proto::events::{
    DictationEdgeParams, DictationStart, DictationStop, VoiceMic, VoiceMicParams,
};
use agentsi_proto::rpc::ErrorKind;
use agentsi_proto::socket::{SOCKET_ENV, socket_path};
use agentsi_proto::vocabulary_api::{VocabularyEntry, VocabularyList};
use agentsi_proto::voice_api::{Dictate, DictateAction, DictateParams, TranscriptResult};
use agentsi_voice::vocabulary::{self, Entry, Vocabulary};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

const USAGE: &str = "usage: agentsi-core dictate <start|stop|toggle>";
const CALL_LIMIT: Duration = Duration::from_secs(5);

pub(crate) fn no_pill() -> RpcError {
    RpcError::new(
        ErrorKind::Unavailable,
        "the pill is not running",
        "Start agentsi-pill. It records and transcribes the dictation.",
    )
}

/// Tells the pill to start or stop the recording. `Toggle` starts when no dictation runs and
/// stops otherwise. The pill ends a dictation by itself only through a stop call to the core, so
/// the core state stays right.
pub fn command(core: &Core, action: DictateAction, at_us: Option<u64>) -> Result<(), RpcError> {
    if !core.has_subscriber(Topic::Dictation) {
        core.set_dictating(false);
        return Err(no_pill());
    }
    let edge = DictationEdgeParams {
        at_us: at_us.unwrap_or_else(now_us),
    };
    let start = match action {
        DictateAction::Start => true,
        DictateAction::Stop => false,
        DictateAction::Toggle => !core.dictating(),
    };
    core.set_dictating(start);
    if start {
        // One microphone, one mode: a dictation ends conversation mode.
        crate::converse::end(core, None);
        core.publish::<DictationStart>(&edge);
    } else {
        core.publish::<DictationStop>(&edge);
    }
    core.publish::<VoiceMic>(&VoiceMicParams { open: start });
    Ok(())
}

/// Microseconds since the Unix epoch. The bind starts both commands on this machine, so
/// they share this clock.
fn now_us() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64)
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Settings {
    #[serde(default)]
    send_return: bool,
}

/// `<config>/agentsi/dictation.toml`. A missing or broken file gives the defaults.
fn settings() -> Settings {
    vocabulary::path()
        .map(|p| p.with_file_name("dictation.toml"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

/// The text with each control character (Return, Escape, Tab) as a space. A transcript
/// must never run a command or move the cursor of the program in the pane.
pub fn plain(text: &str) -> String {
    let spaced: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The product names of the registry of the data root. Empty when there is no registry.
fn product_names() -> Option<Vec<String>> {
    let root = agentsi_store::paths::data_root(None).ok()?;
    let registry = agentsi_agency::registry::Registry::load(&root).ok()?;
    Some(registry.products.into_iter().map(|p| p.name).collect())
}

fn unavailable(message: String) -> RpcError {
    RpcError::new(
        ErrorKind::Unavailable,
        message,
        "Check the vocabulary file in the config folder.",
    )
}

/// The vocabulary and its file. A missing file is written with the seed, when the registry reads.
/// It blocks: call it off the event loop.
pub fn load_vocabulary() -> Result<(Vocabulary, PathBuf), RpcError> {
    let path = vocabulary::path().ok_or_else(|| unavailable("HOME is not set".into()))?;
    if let Some(found) = Vocabulary::load(&path).map_err(unavailable)? {
        return Ok((found, path));
    }
    let names = product_names();
    let seed = Vocabulary::seed(names.as_deref().unwrap_or(&["AgentSI".to_string()]));
    if names.is_some() {
        seed.save(&path).map_err(unavailable)?;
    }
    Ok((seed, path))
}

pub fn list_of(v: &Vocabulary, path: &std::path::Path) -> VocabularyList {
    VocabularyList {
        entries: v
            .entries
            .iter()
            .map(|e| VocabularyEntry {
                term: e.term.clone(),
                heard: e.heard.clone(),
            })
            .collect(),
        path: path.display().to_string(),
    }
}

pub fn save_vocabulary(entries: Vec<VocabularyEntry>) -> Result<VocabularyList, RpcError> {
    let path = vocabulary::path().ok_or_else(|| unavailable("HOME is not set".into()))?;
    let mut v = Vocabulary {
        entries: entries
            .into_iter()
            .map(|e| Entry {
                term: e.term,
                heard: e.heard,
            })
            .collect(),
    };
    v.clean();
    v.save(&path).map_err(unavailable)?;
    Ok(list_of(&v, &path))
}

/// Applies the vocabulary to a transcript and types it into the focused pane. One space at the
/// start of the transcript stays: it joins a piece of toggle dictation to the piece before.
/// It blocks: call it off the event loop.
pub fn transcript(core: &Core, text: &str) -> Result<TranscriptResult, RpcError> {
    // A broken vocabulary file must not lose the sentence: the text goes in as it was heard.
    let vocab = load_vocabulary().map(|(v, _)| v).unwrap_or_else(|e| {
        eprintln!("agentsi-core: the vocabulary is not applied: {}", e.message);
        Vocabulary::default()
    });
    // A piece of toggle dictation after the first starts with a space, which `plain` would drop.
    let joined = text.starts_with(' ');
    let mut text = plain(&vocab.apply(&plain(text)));
    if joined && !text.is_empty() {
        text.insert(0, ' ');
    }
    // In conversation mode the text is a turn: it goes to the target pane, with a Return.
    let converse = crate::converse::target(core);
    let focus = converse.clone().or_else(|| core.focus());
    let target = focus
        .clone()
        .and_then(|id| core.terminals.get(&id).ok())
        .filter(|_| !text.is_empty());
    let Some(term) = target else {
        return Ok(TranscriptResult {
            text,
            terminal: None,
        });
    };
    if converse.is_some() || settings().send_return {
        term.submit(text.as_bytes())?;
    } else {
        term.input(text.as_bytes())?;
    }
    Ok(TranscriptResult {
        terminal: focus,
        text,
    })
}

/// `agentsi-core dictate <start|stop|toggle>`. It never starts a core.
pub fn run(args: &[&str]) -> ExitCode {
    let action = match args {
        ["start"] => DictateAction::Start,
        ["stop"] => DictateAction::Stop,
        ["toggle"] => DictateAction::Toggle,
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let result = (|| -> Result<(), String> {
        let socket = match std::env::var_os(SOCKET_ENV).filter(|v| !v.is_empty()) {
            Some(path) => PathBuf::from(path),
            None => socket_path()?.path,
        };
        let mut client =
            Client::connect(&socket).map_err(|e| format!("cannot reach the core: {e}"))?;
        client
            .set_timeout(Some(CALL_LIMIT))
            .map_err(|e| e.to_string())?;
        client
            .call::<Dictate>(&DictateParams {
                action,
                at_us: Some(now_us()),
            })
            .map(|_| ())
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("agentsi-core dictate: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_characters_never_reach_the_pane() {
        assert_eq!(plain("hello\nworld\r\x1b[31m"), "hello world [31m");
        assert_eq!(plain("  a\t b  "), "a b");
        assert_eq!(plain("\n\n"), "");
        assert_eq!(plain("¿Qué tal?"), "¿Qué tal?");
    }
}
