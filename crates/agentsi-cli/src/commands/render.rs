//! `agentsi render <minutes-id>`: renders the audio of a meeting. The core speaks the
//! script with Kokoro and writes the call into the data root. The progress goes to
//! stderr, from the events of a second connection, because the first one waits for the
//! reply. `meet --audio` uses `render_call` too.

use super::{Fail, interrupt};
use crate::core::Core;
use agentsi_proto::Event;
use agentsi_proto::call_api::{CallRender, CallRenderParams, MeetingRender, MeetingRenderParams};
use agentsi_proto::core_api::{Subscribe, SubscribeParams, Topic};
use agentsi_proto::desk_api::{MinutesGet, MinutesGetParams};
use agentsi_proto::rpc::ErrorKind;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const USAGE: &str = "agentsi render <minutes-id>";

/// How long the last events may take after the reply.
const LAST_EVENTS: Duration = Duration::from_secs(5);
/// How long the events of a failed render may take: the core sends them before its reply.
const REJECTED_EVENTS: Duration = Duration::from_millis(300);

/// The stderr line of one step. `done` prints nothing: the reply says how it ended.
fn progress_line(e: &MeetingRenderParams) -> Option<String> {
    match e.stage.as_str() {
        "start" => Some(format!("[render] {} segments", e.of)),
        "segment" => Some(format!("[render] segment {} of {}", e.done, e.of)),
        "encode" => Some("[render] writing call.opus".into()),
        _ => None,
    }
}

/// Prints the progress of the render `name` until its `done` event. The topic is on
/// before this returns, so no event of the render is lost.
fn follow(name: String) -> Result<JoinHandle<()>, String> {
    let mut events = Core::connect()?;
    events.call::<Subscribe>(&SubscribeParams {
        topics: vec![Topic::Meeting],
    })?;
    Ok(std::thread::spawn(move || {
        while let Ok(n) = events.next_event() {
            if n.method != MeetingRender::NAME {
                continue;
            }
            let Ok(e) = serde_json::from_value::<MeetingRenderParams>(n.params) else {
                continue;
            };
            if e.render != name {
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

/// Renders the call of `minutes_id` in `product` and gives the folder that holds it.
/// `Ctrl+C` during the render cancels it in the core.
pub fn render_call(core: &mut Core, product: &str, minutes_id: &str) -> Result<String, String> {
    let name = interrupt::name();
    let follower = follow(name.clone())?;
    interrupt::cancel_on_signal(&name);
    let reply = core.call::<CallRender>(&CallRenderParams {
        product: product.into(),
        minutes_id: minutes_id.into(),
        render: Some(name),
    });
    // A render that the core rejects sends no `done` event, so its error does not wait for one.
    let wait = if reply.is_ok() {
        LAST_EVENTS
    } else {
        REJECTED_EVENTS
    };
    let deadline = Instant::now() + wait;
    while !follower.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    reply.map(|r| r.out)
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let Some(id) = argv.first() else {
        return Err(Fail::Usage("Missing <minutes-id>".into()));
    };
    if !valid_id(id) {
        return Err(Fail::Error(format!("Invalid minutes id \"{id}\"")));
    }
    let mut core = Core::connect()?;
    let product = match core.call_kind::<MinutesGet>(&MinutesGetParams { id: id.clone() }) {
        Ok(found) => found.product,
        Err((Some(ErrorKind::NotFound), _)) => {
            return Err(Fail::Error(format!(
                "Minutes \"{id}\" not found in any product"
            )));
        }
        Err((_, message)) => return Err(Fail::Error(message)),
    };
    let out = render_call(&mut core, &product, id);
    if interrupt::happened() {
        return Ok(interrupt::EXIT_CODE);
    }
    println!("{}", out?);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_a_word_with_dots_and_dashes() {
        assert!(valid_id("20261020-1000-a"));
        assert!(valid_id("a.b_c"));
        assert!(!valid_id(""));
        assert!(!valid_id("a b"));
        assert!(!valid_id("a/b"));
    }

    #[test]
    fn each_step_but_the_last_prints_a_line() {
        let step = |stage: &str, done, of| MeetingRenderParams {
            stage: stage.into(),
            done,
            of,
            ..MeetingRenderParams::default()
        };
        assert_eq!(
            progress_line(&step("start", 0, 5)).unwrap(),
            "[render] 5 segments"
        );
        assert_eq!(
            progress_line(&step("segment", 2, 5)).unwrap(),
            "[render] segment 2 of 5"
        );
        assert_eq!(
            progress_line(&step("encode", 5, 5)).unwrap(),
            "[render] writing call.opus"
        );
        assert_eq!(progress_line(&step("done", 5, 5)), None);
    }
}
