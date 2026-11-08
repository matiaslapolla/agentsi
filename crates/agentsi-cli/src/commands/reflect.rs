//! `agentsi reflect` and `agentsi reflect revert`. The text is the text of the
//! TypeScript command, which is in Spanish.

use super::{Fail, interrupt, resolve_product};
use crate::args::{self, Kind};
use crate::core::Core;
use crate::json;
use agentsi_proto::reflect_api::{
    ProposalCandidate, ReflectRevert, ReflectRevertParams, ReflectRun, ReflectRunParams,
    ReflectionEntry, to_fixed,
};
use serde::Serialize;

pub const USAGE: &str = "agentsi reflect [--memory-only] [--json] [--product <id>]\n       agentsi reflect revert <reflection-id> [--json] [--product <id>]";

fn indent(text: &str) -> Vec<String> {
    text.trim_end()
        .split('\n')
        .map(|l| format!("    {l}"))
        .collect()
}

fn render_reflection(e: &ReflectionEntry) -> String {
    let mut out = vec![match &e.reverts {
        Some(target) => format!("Reversión {} de {target} en {}", e.id, e.product),
        None => format!("Reflexión {} en {}", e.id, e.product),
    }];
    if e.reverts.is_none() {
        let mut per_agent: Vec<(&str, usize)> = Vec::new();
        for s in &e.signals {
            match per_agent.iter_mut().find(|(a, _)| *a == s.agent) {
                Some((_, n)) => *n += 1,
                None => per_agent.push((&s.agent, 1)),
            }
        }
        let detail = if per_agent.is_empty() {
            String::new()
        } else {
            let parts: Vec<String> = per_agent.iter().map(|(a, n)| format!("{a} {n}")).collect();
            format!(" ({})", parts.join(", "))
        };
        out.push(format!("  señales: {}{detail}", e.signals.len()));
    }
    for m in &e.memory {
        let mut parts = vec![
            format!("{} nuevos", m.added.len()),
            format!("{} retirados", m.retired.len()),
            format!("{} útiles", m.helpful.len()),
            format!("{} dañinos", m.harmful.len()),
        ];
        if let Some(restored) = m.restored.as_ref().filter(|r| !r.is_empty()) {
            parts.push(format!("{} restaurados", restored.len()));
        }
        out.push(format!("  {}: {}", m.agent, parts.join(", ")));
        if !m.diff.is_empty() {
            out.extend(indent(&m.diff));
        }
        for d in &m.dropped {
            out.push(format!("    descartado: {d}"));
        }
    }
    if e.memory.is_empty() {
        out.push("  sin cambios de memoria".into());
    }
    out.join("\n")
}

fn outcome_text(outcome: &str) -> &'static str {
    match outcome {
        "proposed" => "propuesta",
        "insufficient_evidence" => "evidencia insuficiente para proponer",
        "deferred" => "queda para la próxima reflexión (otro agente tuvo la propuesta)",
        _ => "la propuesta falló",
    }
}

fn render_proposals(e: Option<&ReflectionEntry>, candidates: &[ProposalCandidate]) -> String {
    if candidates.is_empty() {
        return "Propuestas de prompt: ningún agente pasó el umbral.".into();
    }
    let mut out = vec!["Propuestas de prompt:".to_string()];
    for c in candidates {
        let why = match &c.failure_class {
            Some(class) => format!("{class} en {} reuniones", c.count),
            None => format!("{} prioridades rechazadas", c.count),
        };
        let note = c.note.as_ref().map_or(String::new(), |n| format!(" — {n}"));
        out.push(format!(
            "  {} ({why}, {} casos, {} held-out): {}{note}",
            c.agent,
            c.cases,
            c.held_out,
            outcome_text(&c.outcome)
        ));
    }
    let Some(p) = e.and_then(|e| e.proposal.as_ref()) else {
        return out.join("\n");
    };
    let decision = p
        .decision_id
        .map_or("null".to_string(), |id| id.to_string());
    let state = if p.status == "pending" {
        format!("decisión pendiente #{decision}")
    } else {
        "descartada".into()
    };
    out.push(String::new());
    out.push(format!("{} → {}: {state}", p.agent, p.target));
    if !p.rationale.is_empty() {
        out.push(format!("  causa: {}", p.rationale));
    }
    if !p.expected_effect.is_empty() {
        out.push(format!("  efecto esperado: {}", p.expected_effect));
    }
    if !p.diff.is_empty() {
        out.extend(indent(&p.diff));
    }
    out.extend(p.replay.table().iter().map(|l| format!("  {l}")));
    if !p.replay.verdict.worse.is_empty() {
        out.push(format!("  empeora: {}", p.replay.verdict.worse.join(", ")));
    }
    out.push(format!(
        "  costo de propuesta y replay: ${} de ${}",
        to_fixed(p.replay.cost_usd, 3),
        json::pretty(&p.replay.budget_usd)
    ));
    if p.status == "pending" {
        out.push(format!(
            "  Aprobar: agentsi decide {decision} approve (o reject)."
        ));
    }
    out.join("\n")
}

fn revert(argv: &[String]) -> Result<i32, Fail> {
    let (a, positionals) =
        args::parse_positional(argv, &[("product", Kind::Text), ("json", Kind::Flag)])
            .map_err(Fail::Error)?;
    let [id] = positionals.as_slice() else {
        return Err(Fail::Usage(USAGE.into()));
    };
    let mut core = Core::connect()?;
    let product = resolve_product(&mut core, a.text("product"))?.product;
    let done = core.call::<ReflectRevert>(&ReflectRevertParams {
        product,
        reflection: id.clone(),
    })?;
    if a.flag("json") {
        println!("{}", json::pretty(&done.entry));
    } else {
        println!("{}", render_reflection(&done.entry));
    }
    Ok(0)
}

#[derive(Serialize)]
struct RunJson<'a> {
    entry: Option<&'a ReflectionEntry>,
    candidates: &'a [ProposalCandidate],
}

/// A cancel that came after the write finds nothing to stop, so the reply tells what
/// was saved.
fn stopped_by_signal<T, E>(signaled: bool, reply: &Result<T, E>) -> bool {
    signaled && reply.is_err()
}

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    if argv.first().is_some_and(|a| a == "revert") {
        return revert(&argv[1..]);
    }
    let (a, positionals) = args::parse_positional(
        argv,
        &[
            ("product", Kind::Text),
            ("json", Kind::Flag),
            ("memory-only", Kind::Flag),
        ],
    )
    .map_err(Fail::Error)?;
    if !positionals.is_empty() {
        return Err(Fail::Usage(USAGE.into()));
    }
    let memory_only = a.flag("memory-only");
    let mut core = Core::connect()?;
    let product = resolve_product(&mut core, a.text("product"))?.product;
    interrupt::cancel_reflection_on_signal(&product);
    let reply = core.call::<ReflectRun>(&ReflectRunParams {
        product,
        memory_only,
    });
    if stopped_by_signal(interrupt::happened(), &reply) {
        return Ok(interrupt::EXIT_CODE);
    }
    let done = reply?;
    for warning in &done.warnings {
        eprintln!("{warning}");
    }
    if a.flag("json") {
        println!(
            "{}",
            json::pretty(&RunJson {
                entry: done.entry.as_ref(),
                candidates: &done.candidates,
            })
        );
        return Ok(0);
    }
    match &done.entry {
        Some(e) => println!("{}", render_reflection(e)),
        None => {
            println!("No hay señales ni memoria nueva desde la última reflexión: nada que curar.")
        }
    }
    if !memory_only {
        println!(
            "\n{}",
            render_proposals(done.entry.as_ref(), &done.candidates)
        );
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::stopped_by_signal;

    #[test]
    fn a_signal_after_the_write_still_prints_what_was_saved() {
        assert!(stopped_by_signal::<(), _>(true, &Err("canceled")));
        assert!(!stopped_by_signal::<_, ()>(true, &Ok(())));
        assert!(!stopped_by_signal::<(), _>(false, &Err("failed")));
    }
}
