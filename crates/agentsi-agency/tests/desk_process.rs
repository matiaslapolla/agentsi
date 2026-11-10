//! The desk process with a fake `claude`: the auth check, two owner messages to one process,
//! the stream lines, and a crash. No test runs the real `claude`.

use agentsi_agency::desk::args::{DeskCall, desk_args, user_line};
use agentsi_agency::desk::auth::{AUTH_TIMEOUT, check};
use agentsi_agency::desk::process::{DeskProcess, ProcEvent};
use agentsi_agency::desk::stream::{Out, read_line};
use agentsi_proto::desk_session_api::{AuthState, DeskItem};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A fake `claude`: `auth status` prints `$dir/auth`, else it answers each stdin line.
const FAKE: &str = r#"#!/bin/sh
dir="$(dirname "$0")"
if [ "$1" = auth ]; then cat "$dir/auth"; exit 0; fi
echo "$$" >> "$dir/pids"
printf '%s\n' "$@" > "$dir/args"
echo '{"type":"system","subtype":"init"}'
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$dir/stdin"
  case "$line" in *crash*) echo 'it broke' >&2; exit 3;; esac
  echo '{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"Hi"}}}'
  echo '{"type":"assistant","message":{"content":[{"type":"text","text":"Hi"},{"type":"tool_use","name":"Read","input":{"file_path":"a.md"}}]}}'
  echo '{"type":"result","is_error":false,"total_cost_usd":0.01,"duration_ms":5}'
done
"#;

fn fake(dir: &Path, auth: &str) -> PathBuf {
    let file = dir.join("claude");
    std::fs::write(&file, FAKE).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.join("auth"), auth).unwrap();
    file
}

/// The outputs of one reply, up to its `end` item.
fn reply(p: &mut DeskProcess) -> Vec<Out> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    while Instant::now() < deadline {
        match p.next(Duration::from_millis(50)) {
            Some(ProcEvent::Line(l)) => {
                let items = read_line(&l);
                let end = items
                    .iter()
                    .any(|o| matches!(o, Out::Item(DeskItem::End { .. })));
                out.extend(items);
                if end {
                    return out;
                }
            }
            Some(ProcEvent::Closed { stderr }) => panic!("closed: {stderr}"),
            None => {}
        }
    }
    panic!("no end item: {out:?}");
}

#[test]
fn the_auth_check_reads_the_fake_status() {
    let dir = tempfile::tempdir().unwrap();
    let program = fake(dir.path(), r#"{"loggedIn": true}"#);
    assert_eq!(
        check(program.to_str().unwrap(), AUTH_TIMEOUT).state,
        AuthState::Ready
    );
    std::fs::write(dir.path().join("auth"), r#"{"loggedIn": false}"#).unwrap();
    let out = check(program.to_str().unwrap(), AUTH_TIMEOUT);
    assert_eq!(out.state, AuthState::SignedOut);
    assert!(out.message.unwrap().contains("claude auth login"));
}

#[test]
fn a_slow_auth_check_stops_at_its_limit() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("claude");
    std::fs::write(&file, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    let started = Instant::now();
    let out = check(file.to_str().unwrap(), Duration::from_millis(300));
    assert_eq!(out.state, AuthState::Unknown);
    assert!(out.message.unwrap().contains("did not answer"));
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn two_owner_messages_go_to_the_same_process() {
    let dir = tempfile::tempdir().unwrap();
    let program = fake(dir.path(), "{}");
    let call = DeskCall {
        model: "sonnet".into(),
        system: "sys".into(),
        agentsi: "agentsi".into(),
        socket: "/s".into(),
        session: "1b4e28ba-2fa1-41d2-883f-0016d3cca427".into(),
        ..DeskCall::default()
    };
    let mut p = DeskProcess::start(
        program.to_str().unwrap(),
        &desk_args(&call),
        dir.path(),
        &[],
    )
    .unwrap();
    p.send(&user_line("one")).unwrap();
    let first = reply(&mut p);
    assert_eq!(first[0], Out::Delta("Hi".into()));
    assert_eq!(
        first[1..],
        [
            Out::Item(DeskItem::Reply { text: "Hi".into() }),
            Out::Item(DeskItem::Tool {
                line: "Read: a.md".into()
            }),
            Out::Item(DeskItem::End {
                cost_usd: 0.01,
                duration_ms: 5,
                error: None
            }),
        ]
    );
    p.send(&user_line("two")).unwrap();
    reply(&mut p);
    let pids = std::fs::read_to_string(dir.path().join("pids")).unwrap();
    assert_eq!(pids.lines().collect::<Vec<_>>(), [p.pid().to_string()]);
    let stdin = std::fs::read_to_string(dir.path().join("stdin")).unwrap();
    assert_eq!(stdin.lines().count(), 2);
    assert!(stdin.contains("\"content\":\"two\""));
    let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
    assert!(args.contains("--session-id\n1b4e28ba-2fa1-41d2-883f-0016d3cca427"));
    assert!(!args.contains("--bare"));
}

#[test]
fn a_crash_closes_the_stream_with_the_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let program = fake(dir.path(), "{}");
    let mut p = DeskProcess::start(program.to_str().unwrap(), &[], dir.path(), &[]).unwrap();
    p.send(&user_line("crash now")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline, "no close");
        if let Some(ProcEvent::Closed { stderr }) = p.next(Duration::from_millis(50)) {
            assert_eq!(stderr.trim(), "it broke");
            break;
        }
    }
    assert_eq!(p.next(Duration::from_millis(10)), None);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(p.exit_code(), Some(3));
}

#[test]
fn a_spawn_waits_while_the_program_file_is_busy() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("prog");
    let mut open = std::fs::File::create(&file).unwrap();
    open.write_all(b"#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    // An open write handle makes exec fail with ETXTBSY until it closes.
    let busy = std::process::Command::new(&file).spawn().unwrap_err();
    assert_eq!(busy.raw_os_error(), Some(libc::ETXTBSY), "{busy}");
    let closer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        drop(open);
    });
    let mut child = agentsi_agency::desk::process::spawn(&mut std::process::Command::new(&file))
        .expect("the spawn tries again");
    assert!(child.wait().unwrap().success());
    closer.join().unwrap();
}
