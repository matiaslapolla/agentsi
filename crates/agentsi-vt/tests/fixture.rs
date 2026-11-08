//! Feeds the bytes of a real Claude Code session (`tests/fixtures/claude-status.raw`) to a terminal.
//! The session ran `claude --model haiku` in an empty folder. A script typed a prompt that makes Claude ask
//! to run `touch`, and then pressed Esc. See `tests/fixtures/README.md`.

use agentsi_vt::{AgentState, BlockedKind, Event, Options, Terminal};
use std::path::PathBuf;

fn fixture() -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/claude-status.raw");
    std::fs::read(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Feeds the fixture in chunks of `chunk` bytes and returns all events.
fn run(chunk: usize) -> (Terminal, Vec<Event>) {
    let mut t = Terminal::new(Options {
        cols: 120,
        rows: 40,
        ..Options::default()
    })
    .unwrap();
    let mut events = Vec::new();
    for piece in fixture().chunks(chunk) {
        t.write(piece);
        events.extend(t.take_events());
    }
    (t, events)
}

fn states(events: &[Event]) -> Vec<AgentState> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Status(r) => Some(r.state.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_fixture_gives_idle_working_and_blocked_in_that_order() {
    // One chunk, one OSC split across chunks (7 bytes), and byte by byte must agree.
    for chunk in [usize::MAX, 4096, 7, 1] {
        let (t, events) = run(chunk);
        let seen = states(&events);
        let blocked = AgentState::Blocked {
            kind: BlockedKind::Permission,
            msg: "approve Bash: touch probe-file.txt".into(),
        };
        assert_eq!(
            seen,
            [
                AgentState::Idle,
                AgentState::Working,
                blocked,
                AgentState::Idle
            ],
            "chunk size {chunk}"
        );
        assert_eq!(t.agent_state(), Some(&AgentState::Idle));
        let first_app = events.iter().find_map(|e| match e {
            Event::Status(r) => Some(r.app.clone()),
            _ => None,
        });
        assert_eq!(first_app.as_deref(), Some("claude-code"));
    }
}

#[test]
fn the_status_reply_comes_before_the_device_attributes_reply_in_a_real_session() {
    // Claude Code sends `OSC 7501 ; ?` and then `CSI c`.
    let (_, events) = run(usize::MAX);
    let all: Vec<u8> = events
        .iter()
        .filter_map(|e| match e {
            Event::Reply(b) => Some(b.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    let at = |needle: &[u8]| all.windows(needle.len()).position(|w| w == needle);
    let status = at(b"\x1b]7501;?").unwrap_or_else(|| panic!("no status reply in {all:?}"));
    let attrs = at(b"\x1b[?62;22c").unwrap_or_else(|| panic!("no DA1 reply in {all:?}"));
    assert!(status < attrs, "{all:?}");
}

#[test]
fn the_fixture_ends_on_the_idle_screen_after_the_denied_permission() {
    let (mut t, events) = run(usize::MAX);
    let screen = t.grid().unwrap().text_lines().join("\n");
    assert!(
        screen.contains("Interrupted · What should Claude do instead?"),
        "{screen}"
    );
    assert!(
        screen.contains("Run this shell command and nothing else: touch probe-file.txt"),
        "{screen}"
    );
    assert!(screen.contains("manual mode on"), "{screen}");
    let titles: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::Title(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(titles.last(), Some(&"Probe-file.txt touch command"));
}

#[test]
fn the_fixture_holds_no_private_data() {
    let text = String::from_utf8_lossy(&fixture()).into_owned();
    for word in ["matiasdev", "desk-workers", "agentsi", "@", "sk-", "token"] {
        assert!(!text.contains(word), "the fixture contains {word:?}");
    }
}
