use super::{Fail, resolve_product};
use crate::args::{self, Kind};
use crate::core::Core;
use agentsi_proto::roles_api::{ResetPlan, RolesReset, RolesResetParams};
use std::io::{BufRead, IsTerminal, Write};

pub const USAGE: &str = "agentsi reset <agent> [--memory] [--yes] [--product <id>]\n       agentsi reset --all [--memory] [--yes] [--product <id>]";

fn render_plan(plan: &ResetPlan) -> String {
    let id = &plan.product;
    let mut lines = vec![match &plan.agent {
        Some(agent) => format!("Se resetea {agent} en {id} al rol global."),
        None => format!("Se resetea {id} a los roles globales."),
    }];
    for rel in &plan.removals {
        lines.push(format!("  elimina  {id}/{rel}"));
    }
    for rel in &plan.edits {
        lines.push(format!("  edita    {id}/{rel}"));
    }
    if plan.roster_before != plan.roster_after {
        lines.push(format!(
            "  roster   {} -> {}",
            plan.roster_before.join(", "),
            plan.roster_after.join(", ")
        ));
    }
    lines.join("\n")
}

fn confirm() -> bool {
    print!("¿Aplicar? (s/N) ");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(
        answer.trim().to_lowercase().as_str(),
        "s" | "si" | "sí" | "y" | "yes"
    )
}

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let (a, positionals) = args::parse_positional(
        argv,
        &[
            ("all", Kind::Flag),
            ("memory", Kind::Flag),
            ("yes", Kind::Flag),
            ("product", Kind::Text),
        ],
    )
    .map_err(Fail::Error)?;
    let agent = positionals.first().filter(|s| !s.is_empty()).cloned();
    if positionals.len() > 1 || agent.is_some() == a.flag("all") {
        return Err(Fail::Usage(USAGE.into()));
    }
    let mut core = Core::connect()?;
    let product = resolve_product(&mut core, a.text("product"))?;
    let mut params = RolesResetParams {
        product: product.product,
        agent,
        memory: a.flag("memory"),
        yes: false,
    };
    let plan = core.call::<RolesReset>(&params)?.plan;
    if plan.noop {
        println!("Nada que resetear.");
        return Ok(0);
    }
    println!("{}", render_plan(&plan));
    if !a.flag("yes") {
        if !std::io::stdin().is_terminal() {
            eprintln!("\nNo hay terminal para confirmar: repetí con --yes.");
            return Ok(1);
        }
        if !confirm() {
            println!("Cancelado.");
            return Ok(1);
        }
    }
    params.yes = true;
    let done = core.call::<RolesReset>(&params)?;
    if !done.applied {
        return Err(Fail::Error(
            "the data changed since the plan, and nothing was written. Run it again.".into(),
        ));
    }
    let commit = done.commit.unwrap_or_default();
    println!(
        "{}",
        if commit.committed {
            "Reseteado y commiteado."
        } else {
            "Reseteado, sin commit nuevo."
        }
    );
    if let Some(error) = commit.error {
        eprintln!("Commit skipped: {error}");
    }
    Ok(0)
}
