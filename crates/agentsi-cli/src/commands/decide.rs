//! `agentsi decide <id> approve|reject`.

use super::Fail;
use crate::core::Core;
use agentsi_proto::marks_api::{DecideParams, DecisionDecide, Outcome, Verdict};
use agentsi_proto::rpc::ErrorKind;

pub const USAGE: &str = "agentsi decide <decision-id> approve|reject";

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let (Some(id), Some(verdict)) = (argv.first(), argv.get(1)) else {
        return Err(Fail::Usage(USAGE.into()));
    };
    let status = match verdict.as_str() {
        "approve" => Verdict::Approved,
        "reject" => Verdict::Rejected,
        _ => return Err(Fail::Usage(USAGE.into())),
    };
    let number = id
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| id.parse::<i64>().ok())
        .flatten()
        .ok_or_else(|| Fail::Usage(USAGE.into()))?;
    let mut core = Core::connect()?;
    let done = match core.call_kind::<DecisionDecide>(&DecideParams {
        decision: number,
        status,
    }) {
        Ok(done) => done,
        // TypeScript prints this refusal as it is and returns 1.
        Err((Some(ErrorKind::Conflict), text)) => {
            eprintln!("{text}");
            return Ok(1);
        }
        Err((_, text)) => return Err(Fail::Error(text)),
    };
    if let Some(reason) = done.refused {
        return Err(Fail::Error(reason));
    }
    if let Some(error) = &done.commit_error {
        eprintln!("Commit skipped: {error}");
    }
    if done.status == Outcome::Stale {
        eprintln!(
            "Decision {id} not applied: the target changed since the proposal, so it is now stale."
        );
        return Ok(1);
    }
    let name = match done.status {
        Outcome::Approved => "approved",
        _ => "rejected",
    };
    match &done.minutes_id {
        Some(m) => println!("Decision {id} {name} ({m})."),
        None => println!("Decision {id} {name}."),
    }
    if done.status == Outcome::Approved
        && let Some(action) = done.action.as_deref().filter(|a| !a.is_empty())
    {
        println!("Action: {action}");
    }
    Ok(0)
}
