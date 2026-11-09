use super::{Fail, resolve_product};
use crate::args::{self, Args, Kind};
use crate::core::Core;
use crate::json;
use agentsi_proto::roles_api::{
    ApplyPlan, ChangeStatus, Registration, RolesApply, RolesApplyParams, RolesList,
    RolesListParams, RolesValidate, RolesValidateParams,
};
use serde_json::{Value, json};

pub const USAGE: &str = "agentsi roles [agent] [--json] [--product <id>]\n       agentsi roles validate [--json] [--product <id>]\n       agentsi roles apply <staging-dir> [--yes] [--register --product <id> --name <n> --path <abs>]";

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    match argv.split_first() {
        Some((sub, rest)) if sub == "validate" => validate(rest),
        Some((sub, rest)) if sub == "apply" => apply(rest),
        _ => list(argv),
    }
}

fn parse(argv: &[String], options: &[(&'static str, Kind)]) -> Result<(Args, Vec<String>), Fail> {
    args::parse_positional(argv, options).map_err(Fail::Error)
}

fn list(argv: &[String]) -> Result<i32, Fail> {
    let (a, positionals) = parse(argv, &[("product", Kind::Text), ("json", Kind::Flag)])?;
    if positionals.len() > 1 {
        return Err(Fail::Usage(USAGE.into()));
    }
    let mut core = Core::connect()?;
    let product = resolve_product(&mut core, a.text("product"))?;
    let r = core.call::<RolesList>(&RolesListParams {
        product: product.product,
        agent: positionals.first().cloned(),
    })?;
    for w in &r.warnings {
        eprintln!("{w}");
    }
    if let Some(def) = r.agent {
        if a.flag("json") {
            println!("{}", json::pretty(&def));
        } else {
            let source = format!("{:?}", def.source).to_lowercase();
            let effort = def
                .effort
                .as_ref()
                .map_or(String::new(), |e| format!(", esfuerzo {e}"));
            println!(
                "# {} (rol {}, {source}, {}{effort})\n\n{}",
                def.id, def.role, def.model, def.body
            );
        }
        return Ok(0);
    }
    let entries = r.roster.unwrap_or_default();
    if a.flag("json") {
        println!(
            "{}",
            json::pretty(&json!({ "product": r.product, "roster": entries }))
        );
        return Ok(0);
    }
    let width = |pick: &dyn Fn(&agentsi_proto::roles_api::RosterEntry) -> String| {
        entries
            .iter()
            .map(|e| pick(e).chars().count())
            .max()
            .unwrap_or(0)
    };
    let source = |e: &agentsi_proto::roles_api::RosterEntry| {
        e.source
            .map_or(String::new(), |s| format!("{s:?}").to_lowercase())
    };
    let id_w = width(&|e| e.id.clone());
    let role_w = width(&|e| e.role.clone().unwrap_or_default());
    let src_w = width(&source);
    println!("Roster de {}:", r.product);
    for e in &entries {
        let detail = match &e.error {
            Some(error) => format!("ERROR {error}"),
            None => format!(
                "{:role_w$}  {:src_w$}  {}{}",
                e.role.clone().unwrap_or_default(),
                source(e),
                e.model.clone().unwrap_or_default(),
                e.effort
                    .as_ref()
                    .map_or(String::new(), |x| format!("  esfuerzo {x}"))
            ),
        };
        println!("  {:id_w$}  {detail}", e.id);
    }
    Ok(0)
}

fn validate(argv: &[String]) -> Result<i32, Fail> {
    let a =
        args::parse(argv, &[("product", Kind::Text), ("json", Kind::Flag)]).map_err(Fail::Error)?;
    let mut core = Core::connect()?;
    let product = resolve_product(&mut core, a.text("product"))?;
    let r = core.call::<RolesValidate>(&RolesValidateParams {
        product: product.product,
    })?;
    for w in &r.warnings {
        eprintln!("{w}");
    }
    if a.flag("json") {
        println!(
            "{}",
            json::pretty(
                &json!({ "product": r.product, "valid": r.valid, "failures": r.failures })
            )
        );
    } else if r.valid {
        println!("{}: el roster es válido ({} agentes).", r.product, r.agents);
    } else {
        eprintln!("{}: {} problema(s):", r.product, r.failures.len());
        for f in &r.failures {
            eprintln!("  - {f}");
        }
    }
    Ok(if r.valid { 0 } else { 1 })
}

fn stories_label(block: &Option<Value>) -> String {
    let Some(s) = block else {
        return "apagado".into();
    };
    let text = |k: &str| {
        s.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let gate: Vec<String> = s
        .get("gate")
        .and_then(Value::as_array)
        .map(|g| {
            g.iter()
                .map(|c| format!("\"{}\"", c.as_str().unwrap_or_default()))
                .collect()
        })
        .unwrap_or_default();
    let repo = match s
        .get("repo")
        .and_then(Value::as_str)
        .filter(|r| !r.is_empty())
    {
        Some(r) => format!(", repo {r}"),
        None => String::new(),
    };
    format!("test \"{}\", gate {}{repo}", text("test"), gate.join(", "))
}

fn render_plan(plan: &ApplyPlan) -> String {
    let id = &plan.product;
    let head = if plan.register {
        "Se registra y se configura"
    } else {
        "Se configura"
    };
    let before = if plan.roster_before.is_empty() {
        "(ninguno)".to_string()
    } else {
        plan.roster_before.join(", ")
    };
    let mut lines = vec![
        format!("{head} {id}."),
        String::new(),
        format!("Roster: {before} -> {}", plan.roster_after.join(", ")),
    ];
    let added: Vec<&str> = plan
        .roster_after
        .iter()
        .filter(|a| !plan.roster_before.contains(a))
        .map(String::as_str)
        .collect();
    let removed: Vec<&str> = plan
        .roster_before
        .iter()
        .filter(|a| !plan.roster_after.contains(a))
        .map(String::as_str)
        .collect();
    if !added.is_empty() {
        lines.push(format!("  agregados: {}", added.join(", ")));
    }
    if !removed.is_empty() {
        lines.push(format!("  quitados: {}", removed.join(", ")));
    }
    if plan.stories_changed {
        lines.push(format!(
            "Stories: {} -> {}",
            stories_label(&plan.stories_before),
            stories_label(&plan.stories_after)
        ));
    }
    lines.push(String::new());
    lines.push(if plan.changes.is_empty() {
        "Base de datos (brief y overlays): sin cambios.".into()
    } else {
        "Base de datos (brief y overlays):".into()
    });
    for c in &plan.changes {
        let status = match c.status {
            ChangeStatus::New => "nuevo",
            ChangeStatus::Changed => "cambiado",
            ChangeStatus::Removed => "eliminado",
        };
        lines.push(format!("  {status}  {id}/{}", c.path));
        if !c.diff.is_empty() {
            lines.extend(c.diff.split('\n').map(|l| format!("    {l}")));
        }
    }
    lines.join("\n")
}

fn apply(argv: &[String]) -> Result<i32, Fail> {
    let (a, positionals) = parse(
        argv,
        &[
            ("product", Kind::Text),
            ("register", Kind::Flag),
            ("name", Kind::Text),
            ("path", Kind::Text),
            ("yes", Kind::Flag),
        ],
    )?;
    let [dir] = positionals.as_slice() else {
        return Err(Fail::Usage(USAGE.into()));
    };
    let mut core = Core::connect()?;
    let register = a.flag("register");
    let (product, registration) = if register {
        let Some(id) = a.text("product").filter(|p| !p.is_empty()) else {
            return Err(Fail::Usage(
                "--register needs --product <id>, --name and --path.".into(),
            ));
        };
        let registration = Registration {
            name: a.text("name").unwrap_or_default().into(),
            path: a.text("path").unwrap_or_default().into(),
        };
        (id.to_string(), Some(registration))
    } else {
        (resolve_product(&mut core, a.text("product"))?.product, None)
    };
    let stage = std::path::absolute(dir).map_err(|e| e.to_string())?;
    let mut params = RolesApplyParams {
        product,
        stage_dir: Some(stage.to_string_lossy().into_owned()),
        stage: None,
        yes: false,
        register: registration,
    };
    let planned = core.call::<RolesApply>(&params)?;
    for w in &planned.warnings {
        eprintln!("{w}");
    }
    let Some(plan) = planned.plan else {
        eprintln!("La etapa no es válida:");
        for f in &planned.failures {
            eprintln!("  - {f}");
        }
        return Ok(1);
    };
    println!("{}", render_plan(&plan));
    if !a.flag("yes") {
        println!("\nSimulacro: no se escribió nada. Repetí con --yes para aplicar.");
        return Ok(0);
    }
    params.yes = true;
    let done = core.call::<RolesApply>(&params)?;
    if !done.applied {
        return Err(Fail::Error(
            "the stage changed since the plan, and nothing was written. Run it again.".into(),
        ));
    }
    let commit = done.commit.unwrap_or_default();
    println!(
        "\n{}",
        if commit.committed {
            "Aplicado y commiteado."
        } else {
            "Aplicado, sin commit nuevo."
        }
    );
    if let Some(error) = commit.error {
        eprintln!("Commit skipped: {error}");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stories_block_prints_its_test_gate_and_repo() {
        assert_eq!(stories_label(&None), "apagado");
        let block = json!({"repo": "/r", "test": "t {files}", "gate": ["a", "b"], "budget_usd": 2});
        assert_eq!(
            stories_label(&Some(block)),
            "test \"t {files}\", gate \"a\", \"b\", repo /r"
        );
    }
}
