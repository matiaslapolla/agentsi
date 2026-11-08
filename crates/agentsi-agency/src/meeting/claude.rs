//! One headless Claude Code call. The rules copy `packages/core/src/claude.ts`.
//! The run uses `--output-format stream-json`, so a screen can follow each role.
//! The last line of that stream is the envelope of `--output-format json`.

use crate::sandbox::{die_with_parent, kill_group};
use crate::story::claude::{Call, call_args, call_output};
use agentsi_proto::meeting_api::MeetingUsage;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

/// How long a run may hold its pipe open after the program stopped: an MCP server can keep it.
const EXIT_GRACE: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(50);
const SHOWN_CHARS: usize = 300;

pub struct ClaudeResult {
    /// The structured output, or the plain text when the call has no schema.
    pub output: Value,
    /// The role is empty until the caller fills it.
    pub usage: MeetingUsage,
}

/// Stops the Claude runs of one meeting.
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The arguments of `call_args`, with a stream in place of the single envelope.
pub fn stream_args(c: &Call) -> Vec<String> {
    let mut args = call_args(c);
    let format = args
        .iter()
        .position(|a| a == "--output-format")
        .expect("call_args names the format");
    args[format + 1] = "stream-json".into();
    args.insert(format + 2, "--verbose".into());
    args
}

pub fn total_input_tokens(u: &MeetingUsage) -> u64 {
    u.input_tokens + u.cache_read + u.cache_write
}

fn clip(text: &str) -> String {
    text.chars().take(SHOWN_CHARS).collect()
}

fn drain(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<String> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = pipe.read_to_end(&mut bytes);
        let _ = send.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    receive
}

/// What ended a run early.
enum Stop {
    Timeout,
    Canceled,
}

/// Runs `program` with the prompt on stdin, and gives each line of its stdout to
/// `on_line`. Returns the last `result` line. The error is the text of the failure.
fn run_stream(
    program: &str,
    call: &Call,
    on_line: &(dyn Fn(&str) + Sync),
    cancel: &Cancel,
) -> Result<Value, String> {
    use std::os::unix::process::CommandExt;
    let mut command = Command::new(program);
    command
        .args(stream_args(call))
        .current_dir(&call.cwd)
        .envs(crate::git_env::env_for_cwd(Path::new(&call.cwd)))
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // A call without tools is a single turn: a cache write would cost 25% more and never be read.
    if call.tools.is_empty() {
        command.env("DISABLE_PROMPT_CACHING", "1");
    }
    die_with_parent(&mut command);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let pid = child.id();
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let prompt = call.prompt.clone();
    std::thread::spawn(move || {
        let _ = stdin.write_all(prompt.as_bytes());
    });
    let stderr = drain(child.stderr.take().expect("stderr is piped"));
    let stdout = child.stdout.take().expect("stdout is piped");
    let (send, lines) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            match reader.read_until(b'\n', &mut bytes) {
                Ok(n) if n > 0 => {
                    let line = String::from_utf8_lossy(&bytes);
                    if send.send(line.trim_end().to_string()).is_err() {
                        break;
                    }
                }
                _ => break,
            }
        }
    });

    let deadline = Instant::now() + call.timeout;
    let (mut result, mut first_text, mut stopped) = (None, String::new(), None);
    let mut exited_at: Option<Instant> = None;
    loop {
        match lines.recv_timeout(POLL) {
            Ok(line) => {
                if first_text.len() < SHOWN_CHARS {
                    first_text.push_str(&line);
                }
                on_line(&line);
                if let Ok(value) = serde_json::from_str::<Value>(&line)
                    && value.get("type").and_then(Value::as_str) == Some("result")
                {
                    result = Some(value);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if exited_at.is_none() && matches!(child.try_wait(), Ok(Some(_))) {
                    exited_at = Some(Instant::now());
                }
                if exited_at.is_some_and(|t| t.elapsed() >= EXIT_GRACE) {
                    kill_group(pid);
                    break;
                }
            }
        }
        // After each line too: a run that writes without a pause never reaches the timeout arm.
        if cancel.is_set() {
            stopped = Some(Stop::Canceled);
        } else if Instant::now() >= deadline {
            stopped = Some(Stop::Timeout);
        }
        if stopped.is_some() {
            kill_group(pid);
            break;
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    match stopped {
        Some(Stop::Canceled) => return Err("the meeting was canceled".into()),
        Some(Stop::Timeout) => {
            return Err(format!(
                "claude timed out after {}s",
                call.timeout.as_secs()
            ));
        }
        None => {}
    }
    let Some(raw) = result else {
        let err = stderr.recv_timeout(EXIT_GRACE).unwrap_or_default();
        let text = if err.trim().is_empty() {
            first_text
        } else {
            err
        };
        let code = status.code().map_or("null".to_string(), |c| c.to_string());
        return Err(format!(
            "claude exited {code} without JSON: {}",
            clip(text.trim())
        ));
    };
    Ok(raw)
}

/// One headless call of `program` (`claude`, or a fake one in a test).
pub fn claude(
    program: &str,
    call: &Call,
    on_line: &(dyn Fn(&str) + Sync),
    cancel: &Cancel,
) -> Result<ClaudeResult, String> {
    let raw = run_stream(program, call, on_line, cancel)?;
    let (output, usage) = call_output(&raw, call, "")?;
    Ok(ClaudeResult { output, usage })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::story::claude::map_usage;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::Mutex;

    fn call(dir: &Path, schema: Option<Value>, tools: &[&str]) -> Call {
        Call {
            prompt: "the prompt".into(),
            model: "haiku".into(),
            system: "the system".into(),
            cwd: dir.to_string_lossy().into_owned(),
            tools: tools.iter().map(|t| t.to_string()).collect(),
            schema,
            effort: Some("low".into()),
            timeout: Duration::from_secs(20),
            mcp: None,
        }
    }

    fn script(dir: &Path, body: &str) -> String {
        let file = dir.join("fake-claude");
        std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        file.to_string_lossy().into_owned()
    }

    #[test]
    fn the_arguments_are_those_of_claude_ts_with_a_stream() {
        let dir = tempfile::tempdir().unwrap();
        let args = stream_args(&call(
            dir.path(),
            Some(json!({"type": "object"})),
            &["Read"],
        ));
        let text = args.join(" ");
        assert!(text.starts_with("-p --model haiku --system-prompt the system --output-format stream-json --verbose --no-session-persistence --safe-mode --restricted"));
        assert!(text.ends_with(
            "--max-budget-usd 0.1 --tools=Read --allowedTools Read --effort low --json-schema {\"type\":\"object\"}"
        ));
    }

    #[test]
    fn an_mcp_call_drops_safe_mode_and_allows_its_tools() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = call(dir.path(), None, &["Read"]);
        c.model = "sonnet".into();
        c.mcp = Some(crate::story::claude::McpAccess {
            config: "{}".into(),
            tools: vec!["mcp__x__y".into()],
        });
        let text = stream_args(&c).join(" ");
        assert!(!text.contains("--safe-mode"));
        assert!(text.contains(
            "--max-budget-usd 0.4 --tools=Read --mcp-config {} --allowedTools Read,mcp__x__y"
        ));
    }

    #[test]
    fn usage_takes_the_model_with_the_highest_cost() {
        let raw = json!({
            "usage": {"input_tokens": 1, "output_tokens": 2, "cache_read_input_tokens": 3, "cache_creation_input_tokens": 4},
            "total_cost_usd": 0.5, "duration_ms": 12.6,
            "modelUsage": {"small": {"costUSD": 0.1}, "big": {"costUSD": 0.4}, "tie": {"costUSD": 0.4}},
        });
        let u = map_usage(&raw, "", "sonnet");
        assert_eq!(u.model, "big");
        assert_eq!(
            (u.input_tokens, u.output_tokens, u.cache_read, u.cache_write),
            (1, 2, 3, 4)
        );
        assert_eq!((u.cost_usd, u.duration_ms), (0.5, 13));
        assert_eq!(total_input_tokens(&u), 8);
        assert_eq!(map_usage(&json!({}), "", "sonnet").model, "sonnet");
    }

    #[test]
    fn a_run_gives_each_line_and_reads_the_last_result() {
        let dir = tempfile::tempdir().unwrap();
        let program = script(
            dir.path(),
            r#"cat >/dev/null
echo '{"type":"system"}'
echo 'not json'
echo '{"type":"result","structured_output":{"answer":"Yes"},"total_cost_usd":0.25,"duration_ms":7}'"#,
        );
        let seen = Mutex::new(Vec::new());
        let r = claude(
            &program,
            &call(dir.path(), Some(json!({})), &["Read"]),
            &|l| seen.lock().unwrap().push(l.to_string()),
            &Cancel::default(),
        )
        .unwrap();
        assert_eq!(r.output, json!({"answer": "Yes"}));
        assert_eq!((r.usage.cost_usd, r.usage.model.as_str()), (0.25, "haiku"));
        assert_eq!(seen.lock().unwrap().len(), 3);
    }

    #[test]
    fn a_call_with_no_tools_turns_off_the_prompt_cache() {
        let dir = tempfile::tempdir().unwrap();
        let program = script(
            dir.path(),
            r#"cat >/dev/null
printf '{"type":"result","result":"%s"}\n' "$DISABLE_PROMPT_CACHING""#,
        );
        let r = claude(
            &program,
            &call(dir.path(), None, &[]),
            &|_| {},
            &Cancel::default(),
        )
        .unwrap();
        assert_eq!(r.output, json!("1"));
    }

    #[test]
    fn the_failures_read_as_in_typescript() {
        let dir = tempfile::tempdir().unwrap();
        let run = |body: &str, schema: Option<Value>| {
            let program = script(dir.path(), body);
            claude(
                &program,
                &call(dir.path(), schema, &["Read"]),
                &|_| {},
                &Cancel::default(),
            )
            .err()
            .unwrap()
        };
        assert_eq!(
            run(
                r#"echo '{"type":"result","is_error":true,"subtype":"error_max_budget_usd","errors":["over","budget"]}'"#,
                None
            ),
            "claude error (error_max_budget_usd): over; budget"
        );
        assert_eq!(
            run(
                r#"echo '{"type":"result","subtype":"success"}'"#,
                Some(json!({}))
            ),
            "claude returned no structured output (success)"
        );
        assert_eq!(
            run(r#"echo '{"type":"result","result":""}'"#, None),
            "claude returned no text (unknown)"
        );
        assert_eq!(
            run("echo boom >&2; exit 3", None),
            "claude exited 3 without JSON: boom"
        );
    }

    #[test]
    fn a_program_that_does_not_start_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let e = claude(
            "/no/such/claude",
            &call(dir.path(), None, &[]),
            &|_| {},
            &Cancel::default(),
        )
        .err()
        .unwrap();
        assert!(e.contains("No such file"), "{e}");
    }

    #[test]
    fn a_timeout_kills_the_group() {
        let dir = tempfile::tempdir().unwrap();
        let program = script(dir.path(), "sleep 30 & sleep 30");
        let mut c = call(dir.path(), None, &[]);
        c.timeout = Duration::from_millis(200);
        let started = Instant::now();
        let e = claude(&program, &c, &|_| {}, &Cancel::default())
            .err()
            .unwrap();
        assert_eq!(e, "claude timed out after 0s");
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_cancel_stops_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let program = script(dir.path(), "sleep 30");
        let cancel = Cancel::default();
        let other = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            other.cancel();
        });
        let started = Instant::now();
        let e = claude(&program, &call(dir.path(), None, &[]), &|_| {}, &cancel)
            .err()
            .unwrap();
        assert_eq!(e, "the meeting was canceled");
        assert!(started.elapsed() < Duration::from_secs(10));
    }
}
