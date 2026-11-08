//! `agentsi meet`: runs a meeting. The progress goes to stderr, from the events of
//! a second connection, because the first one waits for the reply.

use super::{Fail, interrupt, resolve_product};
use crate::args::{self, Kind};
use crate::core::Core;
use agentsi_proto::core_api::{Subscribe, SubscribeParams, Topic};
use agentsi_proto::meeting_api::{MeetingProgressParams, MeetingStart, MeetingStartParams};
pub use agentsi_proto::reflect_api::to_fixed as fixed;
use agentsi_proto::{Event, meeting_api::MeetingProgress};
use serde_json::Value;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const USAGE: &str =
    "agentsi meet [--product <id>] --question \"<q>\" [--roles pm,dev] [--audio]";

const WEB: &str = "http://127.0.0.1:3361";
/// How long the last events may take after the reply.
const LAST_EVENTS: Duration = Duration::from_secs(5);

/// The stderr line of one event, as `progress` of the TypeScript CLI prints it.
fn progress_line(e: &MeetingProgressParams) -> Option<String> {
    let stage = e.stage.as_str();
    let who = e.role.as_deref().unwrap_or(stage);
    match (stage, e.phase.as_str()) {
        ("start" | "done", _) | ("check", "start") => None,
        ("warning", _) => e.note.clone(),
        (_, "start") => {
            let note = e
                .note
                .as_ref()
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            Some(format!("[{stage}] {who} started{note}"))
        }
        ("index" | "check", _) => {
            Some(format!("[{stage}] {}", e.note.as_deref().unwrap_or("done")))
        }
        _ => {
            let secs = format!("{}s", fixed(e.seconds.unwrap_or(0.0), 1));
            Some(match &e.error {
                Some(error) => format!("[{stage}] {who} FAILED after {secs}: {error}"),
                None => format!(
                    "[{stage}] {who} finished in {secs}, ${}, input {} tokens",
                    fixed(e.cost_usd.unwrap_or(0.0), 4),
                    e.input_tokens.unwrap_or(0)
                ),
            })
        }
    }
}

/// Prints the progress of the meeting `name` until its `done` event. The topics
/// are on before this returns, so no event of the meeting is lost.
fn follow(name: String) -> Result<JoinHandle<()>, String> {
    let mut events = Core::connect()?;
    events.call::<Subscribe>(&SubscribeParams {
        topics: vec![Topic::Meeting],
    })?;
    Ok(std::thread::spawn(move || {
        while let Ok(n) = events.next_event() {
            if n.method != MeetingProgress::NAME {
                continue;
            }
            let Ok(e) = serde_json::from_value::<MeetingProgressParams>(n.params) else {
                continue;
            };
            if e.meeting != name {
                continue;
            }
            if let Some(line) = progress_line(&e) {
                eprintln!("{line}");
            }
            if e.stage == "done" {
                return;
            }
        }
    }))
}

fn wait_for(thread: JoinHandle<()>) {
    let deadline = Instant::now() + LAST_EVENTS;
    while !thread.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let a = args::parse(
        argv,
        &[
            ("product", Kind::Text),
            ("question", Kind::Text),
            ("roles", Kind::Text),
            ("audio", Kind::Flag),
        ],
    )
    .map_err(Fail::Error)?;
    let Some(question) = a.text("question").filter(|q| !q.is_empty()) else {
        return Err(Fail::Usage("Missing --question".into()));
    };
    let mut core = Core::connect()?;
    let product = resolve_product(&mut core, a.text("product"))?;
    let roles: Option<Vec<String>> = a.text("roles").map(|r| {
        r.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect()
    });
    let started = Instant::now();
    let name = interrupt::name();
    let follower = follow(name.clone())?;
    interrupt::cancel_on_signal(&name);
    let reply = core.call::<MeetingStart>(&MeetingStartParams {
        product: product.product.clone(),
        scope: product.scope,
        question: question.into(),
        roles,
        meeting: Some(name),
        frozen: None,
    });
    wait_for(follower);
    if interrupt::happened() {
        return Ok(interrupt::EXIT_CODE);
    }
    let res = reply?;
    let m = &res.minutes;

    if a.flag("audio") {
        eprintln!("[audio] rendering");
        match super::render::render_call(&mut core, &product.product, text(m, "id")) {
            Ok(dir) => eprintln!("[audio] done: {dir}"),
            Err(e) => eprintln!("WARNING: audio failed, minutes were kept: {e}"),
        }
        if interrupt::happened() {
            return Ok(interrupt::EXIT_CODE);
        }
    }

    let (id, product, status) = (text(m, "id"), text(m, "product"), text(m, "status"));
    let check = m.get("check").cloned().unwrap_or(Value::Null);
    let repaired = check.get("repaired").and_then(Value::as_bool) == Some(true);
    let passed = check.get("passed").and_then(Value::as_bool) == Some(true);
    println!("Minutes: {id}");
    println!(
        "Status: {status}{}",
        if repaired { " (repaired)" } else { "" }
    );
    if !passed {
        let failures = check.get("failures").and_then(Value::as_array);
        for f in failures.map_or(&[][..], Vec::as_slice) {
            println!("  check failure: {}", f.as_str().unwrap_or_default());
        }
    }
    println!("Top priorities:");
    let priorities = m.get("priorities").and_then(Value::as_array);
    let top = priorities.map_or(&[][..], Vec::as_slice).iter().take(3);
    for (i, p) in top.enumerate() {
        println!(
            "  {}. {} ({})",
            i + 1,
            text(p, "title"),
            text(p, "owner_role")
        );
    }
    println!(
        "Cost: ${} in {}s",
        fixed(res.total_cost_usd, 4),
        fixed(started.elapsed().as_secs_f64(), 0)
    );
    println!("Web: {WEB}/p/{product}/m/{id}");
    println!("Call: {WEB}/p/{product}/m/{id}/call");
    Ok(if status == "valid" { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(stage: &str, phase: &str) -> MeetingProgressParams {
        MeetingProgressParams {
            stage: stage.into(),
            phase: phase.into(),
            ..MeetingProgressParams::default()
        }
    }

    #[test]
    fn fixed_rounds_a_half_up_as_javascript_does() {
        assert_eq!(fixed(0.03125, 4), "0.0313");
        assert_eq!(fixed(0.0312, 4), "0.0312");
        assert_eq!(fixed(1.005, 2), "1.00");
        assert_eq!(fixed(0.0, 4), "0.0000");
        assert_eq!(fixed(9.99996, 4), "10.0000");
        assert_eq!(fixed(12.5, 0), "13");
        assert_eq!(fixed(2.25, 1), "2.3");
    }

    #[test]
    fn each_stage_prints_the_line_of_the_typescript_cli() {
        assert_eq!(
            progress_line(&event("index", "start")).unwrap(),
            "[index] index started"
        );
        assert_eq!(
            progress_line(&event("index", "end")).unwrap(),
            "[index] done"
        );
        assert_eq!(progress_line(&event("check", "start")), None);
        assert_eq!(progress_line(&event("start", "start")), None);
        let role = MeetingProgressParams {
            role: Some("dev".into()),
            seconds: Some(1.25),
            cost_usd: Some(0.5),
            input_tokens: Some(9),
            ..event("role", "end")
        };
        assert_eq!(
            progress_line(&role).unwrap(),
            "[role] dev finished in 1.3s, $0.5000, input 9 tokens"
        );
        let failed = MeetingProgressParams {
            error: Some("boom".into()),
            ..role
        };
        assert_eq!(
            progress_line(&failed).unwrap(),
            "[role] dev FAILED after 1.3s: boom"
        );
        let repair = MeetingProgressParams {
            role: Some("synthesis-repair".into()),
            note: Some("repair".into()),
            ..event("synthesis", "start")
        };
        assert_eq!(
            progress_line(&repair).unwrap(),
            "[synthesis] synthesis-repair started (repair)"
        );
    }
}
