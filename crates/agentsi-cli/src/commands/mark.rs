//! `agentsi mark`: marks of priorities, the list of what is pending, and the
//! terminal wizard.

use super::wizard::{self, Deps};
use super::{Fail, resolve_product};
use crate::args::{self, Kind};
use crate::core::Core;
use crate::json;
use agentsi_proto::marks_api::{
    DecideParams, DecideResult, DecisionDecide, Mark, MarkEntry, MarkPending, MarkPendingParams,
    MarkSet, MarkSetParams, PendingItem, Verdict,
};
use std::io::IsTerminal;

pub const USAGE: &str = "agentsi mark <minutes-id> <idx> accepted|changed|rejected [--new]\n       agentsi mark --set <minutes-id>:<idx>:<mark>[:new] [--set ...]\n       agentsi mark --pending [--json] [--product <id>]\n       agentsi mark [--product <id>]   (wizard, needs a terminal)";

fn mark_of(text: &str) -> Option<Mark> {
    match text {
        "accepted" => Some(Mark::Accepted),
        "changed" => Some(Mark::Changed),
        "rejected" => Some(Mark::Rejected),
        _ => None,
    }
}

fn is_index(text: &str) -> Option<u32> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

/// `<minutes-id>:<idx>:<mark>[:new]`. A minutes id holds no colon.
fn parse_set(spec: &str) -> Result<MarkEntry, Fail> {
    let bad = || {
        Fail::Usage(format!(
            "Bad --set \"{spec}\". Use <minutes-id>:<idx>:accepted|changed|rejected[:new]"
        ))
    };
    let parts: Vec<&str> = spec.split(':').collect();
    let (id, idx, verdict, flag) = match parts.as_slice() {
        [id, idx, verdict] => (*id, *idx, *verdict, None),
        [id, idx, verdict, flag] => (*id, *idx, *verdict, Some(*flag)),
        _ => return Err(bad()),
    };
    let (Some(idx), Some(mark)) = (is_index(idx), mark_of(verdict)) else {
        return Err(bad());
    };
    if id.is_empty() || flag.is_some_and(|f| f != "new") {
        return Err(bad());
    }
    Ok(MarkEntry {
        minutes_id: id.to_string(),
        idx,
        mark,
        is_new: flag.is_some(),
    })
}

struct CoreDeps<'a>(&'a mut Core);

impl Deps for CoreDeps<'_> {
    fn apply_marks(&mut self, entries: &[MarkEntry]) -> Result<(), String> {
        self.0
            .call::<MarkSet>(&MarkSetParams {
                entries: entries.to_vec(),
                unmarked_only: true,
            })
            .map(|_| ())
    }

    fn decide(&mut self, id: i64, status: Verdict) -> Result<DecideResult, String> {
        let done = self.0.call::<DecisionDecide>(&DecideParams {
            decision: id,
            status,
        })?;
        match &done.refused {
            Some(reason) => Err(reason.clone()),
            None => Ok(done),
        }
    }
}

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let (a, positionals) = args::parse_positional(
        argv,
        &[
            ("new", Kind::Flag),
            ("set", Kind::Many),
            ("pending", Kind::Flag),
            ("json", Kind::Flag),
            ("product", Kind::Text),
        ],
    )
    .map_err(Fail::Error)?;
    let usage = || Fail::Usage(USAGE.into());

    if !a.many("set").is_empty() {
        if !positionals.is_empty() || a.flag("pending") || a.flag("new") {
            return Err(usage());
        }
        let entries = a
            .many("set")
            .iter()
            .map(|s| parse_set(s))
            .collect::<Result<Vec<_>, _>>()?;
        let count = entries.len();
        let mut core = Core::connect()?;
        core.call::<MarkSet>(&MarkSetParams {
            entries,
            unmarked_only: true,
        })?;
        println!(
            "Marked {count} {}.",
            if count == 1 { "priority" } else { "priorities" }
        );
        return Ok(0);
    }

    if !positionals.is_empty() {
        let [id, idx, verdict] = positionals.as_slice() else {
            return Err(usage());
        };
        let (Some(index), Some(mark)) = (is_index(idx), mark_of(verdict)) else {
            return Err(usage());
        };
        if a.flag("pending") {
            return Err(usage());
        }
        let is_new = a.flag("new");
        let mut core = Core::connect()?;
        core.call::<MarkSet>(&MarkSetParams {
            entries: vec![MarkEntry {
                minutes_id: id.clone(),
                idx: index,
                mark,
                is_new,
            }],
            unmarked_only: false,
        })?;
        println!(
            "Marked {id} #{idx} {verdict}{}.",
            if is_new { " (new)" } else { "" }
        );
        return Ok(0);
    }

    if a.flag("new") || (a.flag("json") && !a.flag("pending")) {
        return Err(usage());
    }
    let stdin = std::io::stdin();
    if !a.flag("pending") && !stdin.is_terminal() {
        return Err(usage());
    }
    let mut core = Core::connect()?;
    let product = resolve_product(&mut core, a.text("product"))?.product;
    let pending = core.call::<MarkPending>(&MarkPendingParams { product })?;
    if let Some(error) = &pending.decisions_error {
        eprintln!(
            "decisions_error: {}",
            error.lines().next().unwrap_or_default()
        );
    }
    if a.flag("pending") {
        if a.flag("json") {
            println!("{}", json::pretty(&pending.items));
        } else if pending.items.is_empty() {
            println!("Nothing pending.");
        } else {
            for (n, item) in pending.items.iter().enumerate() {
                println!("{}. {}", n + 1, wizard::describe(item).join("\n"));
            }
        }
        return Ok(0);
    }
    let items: &[PendingItem] = &pending.items;
    Ok(wizard::run(
        items,
        stdin.lock(),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
        &mut CoreDeps(&mut core),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_set_reads_the_optional_new_flag_and_rejects_bad_specs() {
        let ok = parse_set("m1:2:changed:new").unwrap();
        assert_eq!((ok.idx, ok.mark, ok.is_new), (2, Mark::Changed, true));
        assert!(!parse_set("m1:0:accepted").unwrap().is_new);
        for bad in [
            "m1:0",
            "m1:x:accepted",
            "m1:0:maybe",
            "m1:0:accepted:old",
            ":0:accepted",
            "m1:0:accepted:new:extra",
            "m1:-1:accepted",
        ] {
            assert!(parse_set(bad).is_err(), "{bad}");
        }
    }
}
