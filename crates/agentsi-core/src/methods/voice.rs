//! `voice.*`, `hook.event` and `focus.report`: voice mode of a terminal, the
//! hooks of the Claude session inside it, and the focused pane of the app.

use super::{Ctx, Router};
use crate::clock::iso_now;
use crate::state::Core;
use crate::voice_session;
use agentsi_proto::RpcError;
use agentsi_proto::core_api::Empty;
use agentsi_proto::events::{
    ProgramState, VoiceSpeak, VoiceSpeakParams, VoiceStop, VoiceStopParams,
};
use agentsi_proto::voice_api::{
    Converse, ConverseParams, ConverseResult, Dictate, DictateParams, FocusReport,
    FocusReportParams, HookEvent, HookEventParams, HookEventResult, HookKind, Mode, ModeParams,
    ModeResult, SessionMode, SessionModeParams, SessionModeResult, Transcript, TranscriptParams,
    TranscriptResult,
};
use agentsi_voice::{RULES, VOICE, spoken_sentences, transcript_entry, transcript_head};
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;

pub fn register(r: Router) -> Router {
    r.add(Mode, mode)
        .add(SessionMode, session_mode)
        .add(Dictate, dictate)
        .add(Converse, converse)
        .add(Transcript, transcript)
        .add(HookEvent, hook)
        .add(FocusReport, focus)
}

/// The app reports its focused pane. A change moves the dictation target.
async fn focus(ctx: Ctx, p: FocusReportParams) -> Result<Empty, RpcError> {
    if let Some(id) = &p.terminal {
        ctx.core.terminals.get(id)?;
    }
    ctx.core.set_focus(p.terminal);
    Ok(Empty {})
}

async fn mode(ctx: Ctx, p: ModeParams) -> Result<ModeResult, RpcError> {
    let term = ctx.core.terminals.get(&p.terminal)?;
    Ok(ModeResult {
        on: term.voice_mode(p.on),
        terminal: p.terminal,
    })
}

async fn session_mode(ctx: Ctx, p: SessionModeParams) -> Result<SessionModeResult, RpcError> {
    let root = data_root()?;
    let was_on = voice_session::is_on(&root, &p.session);
    let on = voice_session::set(&root, &p.session, p.on).map_err(|e| match e.kind() {
        std::io::ErrorKind::InvalidInput => RpcError::invalid_params(
            "the session id needs 1 to 100 letters, digits, _ or -, and one letter or digit at least",
        ),
        _ => RpcError::internal(format!("voice mode of the session: {e}")),
    })?;
    // A turn that began while voice mode was off is not a turn of voice mode.
    if !on || !was_on {
        ctx.core.forget_session_prompt(&p.session);
    }
    Ok(SessionModeResult {
        session: p.session,
        on,
    })
}

fn data_root() -> Result<PathBuf, RpcError> {
    agentsi_store::paths::data_root(None).map_err(|e| RpcError::internal(e.to_string()))
}

async fn dictate(ctx: Ctx, p: DictateParams) -> Result<Empty, RpcError> {
    crate::dictate::command(&ctx.core, p.action, p.at_us)?;
    Ok(Empty {})
}

async fn converse(ctx: Ctx, p: ConverseParams) -> Result<ConverseResult, RpcError> {
    crate::converse::command(&ctx.core, p.action, p.terminal)
}

async fn transcript(ctx: Ctx, p: TranscriptParams) -> Result<TranscriptResult, RpcError> {
    let core = ctx.core.clone();
    crate::blocking::spawn(move || crate::dictate::transcript(&core, &p.text))
        .await
        .map_err(|e| RpcError::internal(e.to_string()))?
}

/// The state that a hook event gives, or `None` when it leaves the state as it is.
fn state_of(
    event: HookKind,
    payload: &Value,
) -> Option<(ProgramState, Option<String>, Option<String>)> {
    let text = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    match event {
        HookKind::SessionStart => Some((ProgramState::Idle, None, None)),
        HookKind::UserPromptSubmit => Some((ProgramState::Working, None, None)),
        HookKind::Stop => Some((ProgramState::Done, None, None)),
        // `Term::hook_resume` handles it: only a blocked state moves.
        HookKind::PostToolUse => None,
        HookKind::PermissionRequest => Some((
            ProgramState::Blocked,
            Some("permission".into()),
            text("tool_name"),
        )),
        HookKind::Notification => {
            let kind = text("notification_type").unwrap_or_default();
            match kind.as_str() {
                "idle_prompt" => None,
                k if k.contains("permission") => Some((
                    ProgramState::Blocked,
                    Some("permission".into()),
                    text("message"),
                )),
                k if k.contains("elicitation") => Some((
                    ProgramState::Blocked,
                    Some("question".into()),
                    text("message"),
                )),
                // `auth_success`, a missing type or a new type: no sign that Claude waits for the owner.
                _ => None,
            }
        }
    }
}

async fn hook(ctx: Ctx, p: HookEventParams) -> Result<HookEventResult, RpcError> {
    if p.terminal.is_empty() {
        return session_hook(ctx, p).await;
    }
    let term = ctx.core.terminals.get(&p.terminal)?;
    let tool = p.payload.get("tool_name").and_then(Value::as_str);
    if p.event == HookKind::PostToolUse {
        term.hook_resume(tool);
    } else if let Some((state, kind, msg)) = state_of(p.event, &p.payload) {
        let wait = (p.event == HookKind::PermissionRequest).then(|| tool.map(String::from));
        term.hook_state(state, kind, msg, wait.flatten());
    }
    // The prompt is kept with voice mode off too, so a switch in mid-turn does not pair a stale prompt.
    if p.event == HookKind::UserPromptSubmit {
        term.set_prompt(str_field(&p.payload, "prompt").to_string());
    }
    if !term.voice_on() {
        if p.event == HookKind::Stop {
            term.take_prompt();
            // The owner turned voice mode off in mid-conversation: no reply is spoken.
            crate::converse::silent_reply(&ctx.core, &p.terminal);
        }
        return Ok(HookEventResult::default());
    }
    voice_turn(&ctx.core, &p.terminal, &p, || Some(term.take_prompt())).await
}

/// A hook of a Claude Code session outside the app. Only a session with voice mode on gets
/// here from the bridge, and the core tests its file again.
async fn session_hook(ctx: Ctx, p: HookEventParams) -> Result<HookEventResult, RpcError> {
    let root = data_root()?;
    let Some(file) = voice_session::file(&root, &p.session).filter(|f| f.exists()) else {
        return Ok(HookEventResult::default());
    };
    voice_session::touch(&file);
    if p.event == HookKind::UserPromptSubmit {
        ctx.core
            .set_session_prompt(&p.session, str_field(&p.payload, "prompt").to_string());
    }
    let label = format!(
        "session:{}",
        voice_session::safe_name(&p.session).unwrap_or_default()
    );
    voice_turn(&ctx.core, &label, &p, || {
        ctx.core.take_session_prompt(&p.session)
    })
    .await
}

/// The voice part of a hook with voice mode on, for a terminal and for a session alike.
/// `label` names the speaker for the pill.
async fn voice_turn(
    core: &Core,
    label: &str,
    p: &HookEventParams,
    take_prompt: impl FnOnce() -> Option<String>,
) -> Result<HookEventResult, RpcError> {
    let mut result = HookEventResult::default();
    match p.event {
        HookKind::UserPromptSubmit => {
            core.publish::<VoiceStop>(&VoiceStopParams {
                terminal: label.to_string(),
            });
            result.context = Some(RULES.to_string());
        }
        HookKind::Stop => {
            let reply = str_field(&p.payload, "last_assistant_message")
                .trim()
                .to_string();
            if reply.is_empty() {
                crate::converse::silent_reply(core, label);
                return Ok(result);
            }
            let sentences = spoken_sentences(&reply);
            // A session whose voice mode went on in mid-turn has no prompt: the turn is not its own.
            let Some(prompt) = take_prompt() else {
                crate::converse::silent_reply(core, label);
                return Ok(result);
            };
            let session = if p.session.is_empty() {
                p.terminal.clone()
            } else {
                p.session.clone()
            };
            let to_save = sentences.clone();
            match crate::blocking::spawn(move || {
                save_transcript(&session, prompt.trim(), &reply, &to_save)
            })
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => eprintln!("core: transcript of {label}: {e}"),
                Err(e) => return Err(RpcError::internal(e.to_string())),
            }
            // Another Stop hook can force a reply. It must not cut off the reply that the owner hears.
            let forced = p
                .payload
                .get("stop_hook_active")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if !sentences.is_empty() && !forced {
                core.publish::<VoiceSpeak>(&VoiceSpeakParams {
                    terminal: label.to_string(),
                    role: VOICE.into(),
                    text: sentences.join("\n"),
                });
            } else if !forced {
                // Conversation mode waits for the end of the speech. With none, it listens now.
                crate::converse::silent_reply(core, label);
            }
        }
        _ => {}
    }
    Ok(result)
}

fn str_field<'a>(payload: &'a Value, key: &str) -> &'a str {
    payload.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn transcripts_dir() -> std::io::Result<PathBuf> {
    agentsi_store::paths::data_root(None)
        .map(|root| root.join("voice").join("transcripts"))
        .map_err(|e| std::io::Error::other(e.to_string()))
}

/// Appends one turn to `<data root>/voice/transcripts/<session>.md`.
fn save_transcript(
    session: &str,
    prompt: &str,
    reply: &str,
    sentences: &[String],
) -> std::io::Result<()> {
    let dir = transcripts_dir()?;
    std::fs::create_dir_all(&dir)?;
    let name: String = session
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    let file = dir.join(format!("{name}.md"));
    let mut text = String::new();
    if !file.exists() {
        text.push_str(&transcript_head(&name));
    }
    text.push_str(&transcript_entry(&iso_now(), prompt, reply, sentences));
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?
        .write_all(text.as_bytes())
}
