//! The hook bridge and voice mode on a real core: every path of the bridge
//! stops with exit code 0, fast and silent, and a Stop hook in a terminal
//! reaches the subscribers as `voice.speak`.

mod common;

use common::framed::Framed;
use common::{BIN, CoreProc, TempDir, wait_until};
use serde_json::{Value, json};
use std::io::Write;
use std::os::unix::net::UnixListener;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// The bridge, run with `stdin`. `terminal` and `socket` set the pane variables.
fn bridge(
    event: &str,
    stdin: &str,
    terminal: Option<&str>,
    socket: Option<&std::path::Path>,
    home: &std::path::Path,
) -> (Output, Duration) {
    let mut c = Command::new(BIN);
    c.args(["hook", event])
        .env("AGENTSI_HOME", home)
        .env_remove("AGENTSI_TERMINAL")
        .env_remove("AGENTSI_CORE_SOCKET")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(t) = terminal {
        c.env("AGENTSI_TERMINAL", t);
    }
    if let Some(s) = socket {
        c.env("AGENTSI_CORE_SOCKET", s);
    }
    let start = Instant::now();
    let mut child = c.spawn().unwrap();
    let mut stdin_pipe = child.stdin.take().unwrap();
    let _ = stdin_pipe.write_all(stdin.as_bytes());
    drop(stdin_pipe);
    let out = child.wait_with_output().unwrap();
    (out, start.elapsed())
}

fn log_lines(home: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(home.join("logs/hook.log"))
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

fn assert_silent_success(out: &Output) {
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stdout.is_empty(), "stdout: {:?}", out.stdout);
    assert!(out.stderr.is_empty(), "stderr: {:?}", out.stderr);
}

/// A core whose data root is `home`.
fn start_core(dir: &TempDir, home: &std::path::Path) -> CoreProc {
    let socket = dir.socket();
    let mut c = CoreProc::command(&socket);
    c.env("AGENTSI_HOME", home);
    c.stderr(std::fs::File::create(dir.0.join("core.err")).unwrap());
    let child = c.spawn().unwrap();
    wait_until("the core listens", || {
        std::os::unix::net::UnixStream::connect(&socket.path).is_ok()
    });
    CoreProc {
        child: Some(child),
        log: dir.0.join("core.err"),
        socket,
    }
}

#[test]
fn without_a_terminal_the_bridge_stops_at_once_and_reads_nothing() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    // The stdin never closes: the bridge must not read it.
    let mut c = Command::new(BIN);
    c.args(["hook", "stop"])
        .env("AGENTSI_HOME", &home)
        .env_remove("AGENTSI_TERMINAL")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let start = Instant::now();
    let mut child = c.spawn().unwrap();
    let _open = child.stdin.take();
    let out = child.wait_with_output().unwrap();
    assert!(start.elapsed() < Duration::from_millis(500));
    assert_silent_success(&out);
    assert!(!home.join("logs").exists());
}

#[test]
fn with_no_core_the_bridge_logs_one_line_and_stops_with_0() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let socket = dir.0.join("none.sock");
    let (out, took) = bridge("Stop", "{}", Some("t1"), Some(&socket), &home);
    assert_silent_success(&out);
    assert!(took < Duration::from_millis(400), "{took:?}");
    assert!(!socket.exists(), "the bridge must not start a core");
    let lines = log_lines(&home);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("Stop t1"), "{lines:?}");
}

#[test]
fn a_core_that_does_not_answer_costs_at_most_the_time_limit() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let socket = dir.0.join("mute.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let hold = std::thread::spawn(move || {
        let held = listener.accept().map(|(s, _)| s);
        std::thread::sleep(Duration::from_secs(2));
        drop(held);
    });
    let (out, took) = bridge("Stop", "{}", Some("t1"), Some(&socket), &home);
    assert_silent_success(&out);
    assert!(took < Duration::from_millis(400), "{took:?}");
    assert_eq!(log_lines(&home).len(), 1);
    hold.join().unwrap();
}

#[test]
fn a_stdin_that_never_closes_ends_at_the_time_limit() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let mut c = Command::new(BIN);
    c.args(["hook", "Stop"])
        .env("AGENTSI_HOME", &home)
        .env("AGENTSI_TERMINAL", "t1")
        .env("AGENTSI_CORE_SOCKET", dir.0.join("none.sock"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let start = Instant::now();
    let mut child = c.spawn().unwrap();
    let _open = child.stdin.take();
    let out = child.wait_with_output().unwrap();
    assert!(start.elapsed() < Duration::from_millis(500));
    assert_silent_success(&out);
}

#[test]
fn an_unknown_event_logs_and_stops_with_0() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let (out, _) = bridge("PreToolUse", "{}", Some("t1"), None, &home);
    assert_silent_success(&out);
    assert_eq!(log_lines(&home).len(), 1);
}

#[test]
fn bad_json_is_logged_and_the_state_still_reaches_the_core() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let core = start_core(&dir, &home);
    let mut c = Framed::connect(&core.socket.path);
    let id = c.ok(
        "terminal.create",
        json!({"cwd": std::env::temp_dir(), "argv": ["sleep", "30"]}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (out, _) = bridge(
        "Stop",
        "{not json",
        Some(&id),
        Some(&core.socket.path),
        &home,
    );
    assert_silent_success(&out);
    let lines = log_lines(&home);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("bad JSON"), "{lines:?}");
    let t = &c.ok("terminal.list", json!({}))["terminals"][0];
    assert_eq!(t["state"], "done");
}

#[test]
fn an_unknown_terminal_is_logged_and_stops_with_0() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let core = start_core(&dir, &home);
    let (out, _) = bridge("Stop", "{}", Some("t99"), Some(&core.socket.path), &home);
    assert_silent_success(&out);
    assert_eq!(log_lines(&home).len(), 1);
}

#[test]
fn the_hooks_set_the_state_of_a_program_that_sends_no_osc_7501() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let core = start_core(&dir, &home);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["terminal"]}));
    let id = c.ok(
        "terminal.create",
        json!({"cwd": std::env::temp_dir(), "argv": ["sleep", "30"]}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    let run = |event: &str, payload: Value| {
        let (out, _) = bridge(
            event,
            &payload.to_string(),
            Some(&id),
            Some(&core.socket.path),
            &home,
        );
        assert_silent_success(&out);
    };
    run(
        "UserPromptSubmit",
        json!({"session_id": "s1", "prompt": "hi"}),
    );
    assert_eq!(
        c.next_event("terminal.state"),
        json!({"id": id, "state": "working"})
    );
    run(
        "PermissionRequest",
        json!({"session_id": "s1", "tool_name": "Bash"}),
    );
    assert_eq!(
        c.next_event("terminal.state"),
        json!({"id": id, "state": "blocked", "kind": "permission", "msg": "Bash"})
    );
    run("Notification", json!({"notification_type": "idle_prompt"}));
    // A tool that ran in parallel does not end the wait for Bash.
    run(
        "PostToolUse",
        json!({"session_id": "s1", "tool_name": "Read"}),
    );
    let t = &c.ok("terminal.list", json!({}))["terminals"][0];
    assert_eq!(t["state"], "blocked");
    // The owner granted the permission and the tool ran.
    run(
        "PostToolUse",
        json!({"session_id": "s1", "tool_name": "Bash"}),
    );
    assert_eq!(
        c.next_event("terminal.state"),
        json!({"id": id, "state": "working"})
    );
    // A tool that ran with no wait leaves `working` as it is: no event.
    run(
        "PostToolUse",
        json!({"session_id": "s1", "tool_name": "Read"}),
    );
    run("Stop", json!({"session_id": "s1"}));
    assert_eq!(
        c.next_event("terminal.state"),
        json!({"id": id, "state": "done"})
    );
    assert!(log_lines(&home).is_empty());
}

#[test]
fn the_hooks_never_override_the_state_of_osc_7501() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let core = start_core(&dir, &home);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["terminal"]}));
    let id = c.ok(
        "terminal.create",
        json!({"cwd": std::env::temp_dir(), "argv": ["sh", "-c", "printf '\\033]7501;state=working:app=test\\007'; sleep 30"]}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        c.next_event("terminal.state"),
        json!({"id": id, "state": "working"})
    );
    let (out, _) = bridge("Stop", "{}", Some(&id), Some(&core.socket.path), &home);
    assert_silent_success(&out);
    let t = &c.ok("terminal.list", json!({}))["terminals"][0];
    assert_eq!(t["state"], "working");
}

#[test]
fn post_tool_use_never_moves_a_blocked_state_of_osc_7501() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let core = start_core(&dir, &home);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["terminal"]}));
    let id = c.ok(
        "terminal.create",
        json!({"cwd": std::env::temp_dir(), "argv": ["sh", "-c", "printf '\\033]7501;state=blocked:kind=permission:app=test\\007'; sleep 30"]}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(c.next_event("terminal.state")["state"], "blocked");
    let (out, _) = bridge(
        "PostToolUse",
        "{}",
        Some(&id),
        Some(&core.socket.path),
        &home,
    );
    assert_silent_success(&out);
    let t = &c.ok("terminal.list", json!({}))["terminals"][0];
    assert_eq!(t["state"], "blocked");
}

#[test]
fn voice_mode_gives_the_rules_on_a_prompt_and_stops_the_speech() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let core = start_core(&dir, &home);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["voice"]}));
    let id = c.ok(
        "terminal.create",
        json!({"cwd": std::env::temp_dir(), "argv": ["sleep", "30"]}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(c.ok("voice.mode", json!({"terminal": id}))["on"], false);
    let (off, _) = bridge(
        "UserPromptSubmit",
        r#"{"prompt":"hi"}"#,
        Some(&id),
        Some(&core.socket.path),
        &home,
    );
    assert_silent_success(&off);

    assert_eq!(
        c.ok("voice.mode", json!({"terminal": id, "on": true}))["on"],
        true
    );
    let (on, _) = bridge(
        "UserPromptSubmit",
        r#"{"prompt":"hi"}"#,
        Some(&id),
        Some(&core.socket.path),
        &home,
    );
    assert_eq!(on.status.code(), Some(0));
    let printed: Value = serde_json::from_slice(&on.stdout).unwrap();
    assert_eq!(
        printed["hookSpecificOutput"]["hookEventName"],
        "UserPromptSubmit"
    );
    assert!(
        printed["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("On screen only")
    );
    assert_eq!(c.next_event("voice.stop"), json!({"terminal": id}));
    let missing = c.call("voice.mode", json!({"terminal": "t99"}));
    assert_eq!(missing["error"]["data"]["kind"], "not_found");
}

#[test]
fn a_stop_hook_in_a_terminal_speaks_the_text_above_the_line_and_saves_the_transcript() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let core = start_core(&dir, &home);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["voice"]}));
    // The child waits for a line, then runs the bridge as Claude Code would.
    let script = format!(
        "read go; printf '%s' '{}' | '{BIN}' hook Stop; sleep 30",
        r#"{"session_id":"s1","last_assistant_message":"The build passes now. I fixed two tests.\n\n```ts\nconst secret = 1;\n```\n\nOn screen only\n\nHidden details."}"#
    );
    let id = c.ok(
        "terminal.create",
        json!({"cwd": std::env::temp_dir(), "argv": ["sh", "-c", script]}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    c.ok("voice.mode", json!({"terminal": id, "on": true}));
    c.ok(
        "terminal.input",
        json!({"id": id, "text": "go", "submit": true}),
    );
    let speak = c.next_event("voice.speak");
    assert_eq!(
        speak,
        json!({"terminal": id, "role": "pm", "text": "The build passes now.\nI fixed two tests."})
    );
    let md = std::fs::read_to_string(home.join("voice/transcripts/s1.md")).unwrap();
    assert!(md.starts_with("# Voice session s1\n\n## "), "{md}");
    assert!(md.contains("const secret = 1;"), "the full reply is saved");
    assert!(
        md.contains("> The build passes now.\n> I fixed two tests."),
        "{md}"
    );
    assert!(log_lines(&home).is_empty(), "{:?}", log_lines(&home));
}

#[test]
fn a_plain_claude_code_session_with_voice_mode_on_speaks_and_saves_its_turns() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let core = start_core(&dir, &home);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["voice"]}));
    let sock = &core.socket.path;
    assert_eq!(c.ok("voice.session", json!({"session": "s1"}))["on"], false);
    let prompt = r#"{"session_id":"s1","prompt":"Is the build green?"}"#;
    let stop = |session: &str, text: &str| {
        json!({"session_id": session, "last_assistant_message": text}).to_string()
    };
    // Voice mode is off: the bridge says nothing and the core does nothing.
    let (off, _) = bridge("UserPromptSubmit", prompt, None, Some(sock), &home);
    assert_silent_success(&off);

    let on = c.ok("voice.session", json!({"session": "s1", "on": true}));
    assert_eq!(on, json!({"session": "s1", "on": true}));
    assert!(home.join("voice/sessions/s1").exists());
    let (out, _) = bridge("UserPromptSubmit", prompt, None, Some(sock), &home);
    assert_eq!(out.status.code(), Some(0));
    let printed: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        printed["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("On screen only")
    );
    assert_eq!(
        c.next_event("voice.stop"),
        json!({"terminal": "session:s1"})
    );

    // Another session has no file: its reply is not spoken. Only the reply of s1 reaches the pill.
    let (other, _) = bridge("Stop", &stop("s2", "Not for you."), None, Some(sock), &home);
    assert_silent_success(&other);
    let (out, _) = bridge(
        "Stop",
        &stop("s1", "Yes, it is green.\n\nOn screen only\n\n42 tests."),
        None,
        Some(sock),
        &home,
    );
    assert_silent_success(&out);
    assert_eq!(
        c.next_event("voice.speak"),
        json!({"terminal": "session:s1", "role": "pm", "text": "Yes, it is green."})
    );
    let md = std::fs::read_to_string(home.join("voice/transcripts/s1.md")).unwrap();
    assert!(
        md.contains("Is the build green?") && md.contains("42 tests."),
        "{md}"
    );
    assert!(!home.join("voice/transcripts/s2.md").exists());
    assert!(log_lines(&home).is_empty(), "{:?}", log_lines(&home));

    // The state is a file: a core that restarts keeps it, and off removes it.
    assert_eq!(c.ok("voice.session", json!({"session": "s1"}))["on"], true);
    assert_eq!(
        c.ok("voice.session", json!({"session": "s1", "on": false}))["on"],
        false
    );
    assert!(!home.join("voice/sessions/s1").exists());
    let bad = c.call("voice.session", json!({"session": "///"}));
    assert_eq!(bad["error"]["data"]["kind"], "invalid");
}

#[test]
fn two_turns_of_a_session_make_one_transcript_with_both_prompts_and_replies() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let core = start_core(&dir, &home);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["voice"]}));
    let sock = &core.socket.path;
    c.ok("voice.session", json!({"session": "two", "on": true}));
    let mut turn = |prompt: &str, reply: &str| {
        let p = json!({"session_id": "two", "prompt": prompt}).to_string();
        let s = json!({"session_id": "two", "last_assistant_message": reply}).to_string();
        assert_eq!(
            bridge("UserPromptSubmit", &p, None, Some(sock), &home)
                .0
                .status
                .code(),
            Some(0)
        );
        c.next_event("voice.stop");
        assert_eq!(
            bridge("Stop", &s, None, Some(sock), &home).0.status.code(),
            Some(0)
        );
        c.next_event("voice.speak");
    };
    turn(
        "first prompt from owner",
        "The build passes now. I fixed two tests.\n\n```ts\nconst secretCode = 42;\n```\n\nOn screen only\n\nTail-details stay written.",
    );
    turn(
        "second prompt from owner",
        "Second answer is short. It has one more line.\n\nOn screen only\n\nSecond-tail-details stay written.",
    );
    let files: Vec<_> = std::fs::read_dir(home.join("voice/transcripts"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(files, ["two.md"], "one transcript for one session");
    let md = std::fs::read_to_string(home.join("voice/transcripts/two.md")).unwrap();
    for part in [
        "first prompt from owner",
        "second prompt from owner",
        "The build passes now",
        "secretCode",
        "Tail-details",
        "Second answer is short",
        "Second-tail-details",
    ] {
        assert!(md.contains(part), "the transcript lacks {part:?}: {md}");
    }
    assert_eq!(md.matches("first prompt from owner").count(), 1, "{md}");
    assert!(
        md.contains("> The build passes now.\n> I fixed two tests."),
        "{md}"
    );
    assert!(md.find("first prompt").unwrap() < md.find("second prompt").unwrap());
}

#[test]
fn a_transcript_that_cannot_be_written_never_blocks_the_speech() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let core = start_core(&dir, &home);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["voice"]}));
    let sock = &core.socket.path;
    c.ok("voice.session", json!({"session": "bad", "on": true}));
    // A file where the folder of the transcripts should be: every write fails.
    std::fs::create_dir_all(home.join("voice")).unwrap();
    std::fs::write(home.join("voice/transcripts"), "not a folder").unwrap();
    let p = json!({"session_id": "bad", "prompt": "go"}).to_string();
    let s = json!({"session_id": "bad", "last_assistant_message": "Still spoken."}).to_string();
    bridge("UserPromptSubmit", &p, None, Some(sock), &home);
    c.next_event("voice.stop");
    let (out, _) = bridge("Stop", &s, None, Some(sock), &home);
    assert_silent_success(&out);
    assert_eq!(c.next_event("voice.speak")["text"], "Still spoken.");
    let err = std::fs::read_to_string(dir.0.join("core.err")).unwrap_or_default();
    assert!(
        err.contains("transcript of session:bad"),
        "the error is logged: {err}"
    );
}

#[test]
fn a_session_id_that_is_not_a_plain_id_is_invalid() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let core = start_core(&dir, &home);
    let mut c = Framed::connect(&core.socket.path);
    let cjk = "漢".repeat(100);
    for bad in ["---", "a/b", "../x", cjk.as_str(), &"a".repeat(101)] {
        let r = c.call("voice.session", json!({"session": bad, "on": true}));
        assert_eq!(r["error"]["data"]["kind"], "invalid", "{bad}");
    }
    assert!(
        !home.join("voice/sessions").exists()
            || std::fs::read_dir(home.join("voice/sessions"))
                .unwrap()
                .next()
                .is_none()
    );
}

#[test]
fn a_session_speaks_only_a_turn_that_began_after_voice_mode_went_on() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let core = start_core(&dir, &home);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["voice"]}));
    let sock = &core.socket.path;
    let prompt = |text: &str| json!({"session_id": "s1", "prompt": text}).to_string();
    let stop = |text: &str| json!({"session_id": "s1", "last_assistant_message": text}).to_string();
    let hook = |event: &str, payload: String| {
        let (out, _) = bridge(event, &payload, None, Some(sock), &home);
        assert_eq!(out.status.code(), Some(0), "{out:?}");
    };
    let transcript =
        || std::fs::read_to_string(home.join("voice/transcripts/s1.md")).unwrap_or_default();

    // Voice mode goes on in the middle of a turn: that turn is neither spoken nor saved.
    hook("UserPromptSubmit", prompt("PROMPT-MID"));
    c.ok("voice.session", json!({"session": "s1", "on": true}));
    hook("Stop", stop("Reply of the middle turn."));
    assert_eq!(transcript(), "");

    // Prompt A with voice on, off in that turn, Stop. Turn B runs with voice off.
    hook("UserPromptSubmit", prompt("PROMPT-A"));
    c.next_event("voice.stop");
    c.ok("voice.session", json!({"session": "s1", "on": false}));
    hook("Stop", stop("Reply A."));
    hook("UserPromptSubmit", prompt("PROMPT-B"));
    hook("Stop", stop("Reply B."));
    // Voice mode goes on in turn C. The text of A must not pair with the reply of C.
    hook("UserPromptSubmit", prompt("PROMPT-C"));
    c.ok("voice.session", json!({"session": "s1", "on": true}));
    hook("Stop", stop("Reply C."));
    assert_eq!(transcript(), "", "no turn is saved yet");

    // The next turn is spoken and saved with its own prompt.
    hook("UserPromptSubmit", prompt("PROMPT-D"));
    c.next_event("voice.stop");
    hook("Stop", stop("Reply D."));
    assert_eq!(c.next_event("voice.speak")["text"], "Reply D.");
    let md = transcript();
    assert!(md.contains("PROMPT-D") && md.contains("Reply D."), "{md}");
    assert!(
        !md.contains("PROMPT-A") && !md.contains("Reply C.") && !md.contains("MID"),
        "{md}"
    );

    // A forced second Stop of the same turn is still saved and not spoken.
    hook(
        "Stop",
        json!({"session_id": "s1", "stop_hook_active": true, "last_assistant_message": "Forced."})
            .to_string(),
    );
    assert!(transcript().contains("Forced."));

    // A voice-off with a stored prompt, then on again, forgets the prompt.
    hook("UserPromptSubmit", prompt("PROMPT-E"));
    c.next_event("voice.stop");
    c.ok("voice.session", json!({"session": "s1", "on": false}));
    c.ok("voice.session", json!({"session": "s1", "on": true}));
    hook("Stop", stop("Reply E."));
    assert!(!transcript().contains("Reply E."));
}

#[test]
fn outside_the_app_the_bridge_serves_only_a_prompt_and_a_stop_of_a_session_with_voice_mode_on() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let sessions = home.join("voice/sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(sessions.join("s1"), "").unwrap();
    let gone = dir.0.join("none.sock");
    // No core runs. A call to it would log one line.
    for (event, stdin) in [
        ("SessionStart", r#"{"session_id":"s1"}"#),
        ("Notification", r#"{"session_id":"s1"}"#),
        ("PostToolUse", r#"{"session_id":"s1"}"#),
        ("Stop", r#"{"session_id":"s2"}"#),
        ("Stop", "{}"),
        ("Stop", "{not json"),
    ] {
        let (out, took) = bridge(event, stdin, None, Some(&gone), &home);
        assert_silent_success(&out);
        assert!(took < Duration::from_millis(400), "{event} {took:?}");
    }
    assert!(log_lines(&home).is_empty(), "{:?}", log_lines(&home));
    // The session with voice mode on needs the core: with none, the bridge logs one line.
    let (out, _) = bridge("Stop", r#"{"session_id":"s1"}"#, None, Some(&gone), &home);
    assert_silent_success(&out);
    let lines = log_lines(&home);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("Stop - "), "{lines:?}");
}

#[test]
fn a_forced_stop_saves_the_reply_but_does_not_speak() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let core = start_core(&dir, &home);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["voice"]}));
    let id = c.ok(
        "terminal.create",
        json!({"cwd": std::env::temp_dir(), "argv": ["sleep", "30"]}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    c.ok("voice.mode", json!({"terminal": id, "on": true}));
    let payload = r#"{"session_id":"s2","stop_hook_active":true,"last_assistant_message":"Journal updated."}"#;
    let (out, _) = bridge("Stop", payload, Some(&id), Some(&core.socket.path), &home);
    assert_silent_success(&out);
    assert!(home.join("voice/transcripts/s2.md").exists());
    // A later reply is still spoken, and it is the first voice event.
    let (out, _) = bridge(
        "Stop",
        r#"{"session_id":"s2","last_assistant_message":"Next reply."}"#,
        Some(&id),
        Some(&core.socket.path),
        &home,
    );
    assert_silent_success(&out);
    assert_eq!(c.next_event("voice.speak")["text"], "Next reply.");
}

#[test]
fn voice_dictate_is_unavailable_until_the_dictation_unit() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    let r = c.call("voice.dictate", json!({"action": "stop"}));
    assert_eq!(r["error"]["data"]["kind"], "unavailable");
}

/// The shell command of the hook entry in `plugin/hooks/hooks.json` for `event`.
fn plugin_command(event: &str) -> String {
    let file =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin/hooks/hooks.json");
    let json: Value = serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
    json["hooks"][event][0]["hooks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|h| h["command"].as_str())
        .find(|c| c.contains("agentsi-core hook"))
        .unwrap()
        .to_string()
}

fn run_plugin_command(event: &str, path: &str) -> Output {
    Command::new("sh")
        .arg("-c")
        .arg(plugin_command(event))
        .env("PATH", path)
        .env_remove("AGENTSI_TERMINAL")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn the_plugin_entries_stop_with_0_when_agentsi_core_is_missing_or_too_old() {
    use std::os::unix::fs::PermissionsExt;
    let dir = TempDir::new();
    let bin = dir.0.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let old = bin.join("agentsi-core");
    // An old core has no `hook` command: it prints a usage line and exits 2, which would block a prompt.
    std::fs::write(
        &old,
        "#!/bin/sh\necho 'usage: agentsi-core serve' >&2\nexit 2\n",
    )
    .unwrap();
    std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "Notification",
        "PermissionRequest",
        "PostToolUse",
        "Stop",
    ] {
        let missing = run_plugin_command(event, "/usr/bin:/bin");
        assert_silent_success(&missing);
        let too_old = run_plugin_command(event, &format!("{}:/usr/bin:/bin", bin.display()));
        assert_silent_success(&too_old);
    }
}

#[test]
fn a_notification_of_an_unknown_type_leaves_the_state_and_a_stale_prompt_is_not_reused() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    let core = start_core(&dir, &home);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["terminal"]}));
    let id = c.ok(
        "terminal.create",
        json!({"cwd": std::env::temp_dir(), "argv": ["sleep", "30"]}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    let run = |event: &str, payload: &str| {
        let (out, _) = bridge(event, payload, Some(&id), Some(&core.socket.path), &home);
        assert_silent_success(&out);
    };
    run("UserPromptSubmit", r#"{"prompt":"first prompt"}"#);
    assert_eq!(c.next_event("terminal.state")["state"], "working");
    run(
        "Notification",
        r#"{"notification_type":"auth_success","message":"signed in"}"#,
    );
    run("Notification", r#"{"message":"no type"}"#);
    run("Stop", r#"{"session_id":"s3"}"#);
    // The next event on the stream is the Stop state: the notifications changed nothing.
    assert_eq!(c.next_event("terminal.state")["state"], "done");
    // Voice mode turns on in the middle of the next turn: the transcript has that turn's prompt.
    run("UserPromptSubmit", r#"{"prompt":"second prompt"}"#);
    c.ok("voice.mode", json!({"terminal": id, "on": true}));
    run(
        "Stop",
        r#"{"session_id":"s3","last_assistant_message":"Reply two."}"#,
    );
    let md = std::fs::read_to_string(home.join("voice/transcripts/s3.md")).unwrap();
    assert!(
        md.contains("second prompt") && !md.contains("first prompt"),
        "{md}"
    );
}
