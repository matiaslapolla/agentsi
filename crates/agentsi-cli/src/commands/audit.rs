//! `agentsi audit`: the report as JSON, or as the Spanish text of today.

use super::meet::fixed;
use super::{Fail, resolve_product};
use crate::args::{self, Kind};
use crate::core::Core;
use crate::json;
use agentsi_proto::agency_api::{AuditParams, AuditReport, DataAudit};

pub const USAGE: &str = "agentsi audit [--json] [--product <id>]";

pub fn run(argv: &[String]) -> Result<i32, Fail> {
    let a =
        args::parse(argv, &[("product", Kind::Text), ("json", Kind::Flag)]).map_err(Fail::Error)?;
    let mut core = Core::connect()?;
    let product = resolve_product(&mut core, a.text("product"))?;
    let report = core.call::<DataAudit>(&AuditParams {
        product: product.product,
    })?;
    println!(
        "{}",
        if a.flag("json") {
            json::pretty(&report)
        } else {
            render(&report)
        }
    );
    Ok(0)
}

fn pct(n: Option<f64>) -> String {
    n.map_or("—".into(), |n| format!("{}%", (n * 100.0 + 0.5).floor()))
}

fn num(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        fixed(n, 2)
    }
}

fn rounds(n: Option<f64>) -> String {
    n.map_or("—".into(), num)
}

fn reason(class: &str) -> &'static str {
    match class {
        "unresolved_source" => "fuentes que no resuelven",
        "invented_source" => "fuentes inventadas",
        "missing_objective" => "prioridades sin objetivo válido",
        "timeout" => "timeouts",
        _ => "\"sin datos\" repetido",
    }
}

pub fn render(r: &AuditReport) -> String {
    let mut out: Vec<String> = vec![format!("Auditoría de {}", r.product), String::new()];

    out.push("Configuración".into());
    if r.config.valid {
        out.push("  roster válido".into());
    } else {
        out.push(format!("  {} problema(s):", r.config.failures.len()));
    }
    out.extend(r.config.failures.iter().map(|f| format!("    - {f}")));
    for o in &r.config.overdue_objectives {
        out.push(format!(
            "  objetivo vencido: {} ({}) {}",
            o.id, o.due, o.text
        ));
    }

    let m = &r.meetings;
    out.push(String::new());
    out.push(format!("Reuniones (últimas {})", m.count));
    if m.count == 0 {
        out.push("  sin reuniones en el registro".into());
    } else {
        out.push(format!(
            "  check a la primera: {} · reparadas: {} · síntesis: {} del costo · fuentes inventadas: {}",
            pct(m.first_pass_rate),
            pct(m.repair_rate),
            pct(m.synthesis_cost_share),
            m.invented_sources
        ));
        for a in m.agents.iter().filter(|a| a.meetings > 0) {
            out.push(format!(
                "  {}: {}/{} respuestas, {} errores ({} timeouts), {} fuentes sin resolver y {} \"sin datos\" por reunión, costo medio {} USD",
                a.agent,
                a.answers,
                a.meetings,
                a.errors,
                a.timeouts,
                num(a.unresolved_sources_per_meeting),
                num(a.sin_datos_per_meeting),
                fixed(a.mean_cost_usd, 2)
            ));
        }
    }

    match &r.last_repairs {
        None => out.push("  rondas de reparación de las últimas 4 reuniones: [no data]".into()),
        Some(last) => {
            out.push("  rondas de reparación de las últimas 4 reuniones:".into());
            out.extend(
                last.iter()
                    .map(|l| format!("    {}: {}", l.minutes_id, l.rounds)),
            );
        }
    }

    let (baseline, since) = (&r.repairs.baseline, &r.repairs.since);
    out.push(format!(
        "  rondas de reparación por reunión, de las actas: primeras {}: {} · después ({}): {}",
        baseline.minutes.len(),
        rounds(baseline.per_meeting),
        since.minutes.len(),
        rounds(since.per_meeting)
    ));

    out.push(String::new());
    out.push("Marcas por agente".into());
    if r.marks.0.is_empty() {
        out.push("  sin prioridades".into());
    }
    for (agent, c) in &r.marks.0 {
        out.push(format!(
            "  {agent}: {} aceptadas, {} cambiadas, {} rechazadas, {} sin marcar",
            c.accepted, c.changed, c.rejected, c.unmarked
        ));
    }

    out.push(String::new());
    out.push("Memoria".into());
    for k in &r.memory {
        out.push(format!(
            "  {}: {} activos, {} retirados, {} lecciones, {} nunca útiles, {}/{} caracteres",
            k.agent, k.active, k.retired, k.lessons, k.never_helpful, k.chars, k.max_chars
        ));
    }

    out.push(String::new());
    out.push("Reflexiones".into());
    let last = r
        .reflections
        .last
        .as_ref()
        .map_or("ninguna".to_string(), |l| {
            format!("{} ({})", l.id, l.created_at)
        });
    out.push(format!(
        "  última: {last} · propuestas pendientes: {} · revertidas: {}",
        r.reflections.pending_proposals, r.reflections.reverted
    ));

    out.push(String::new());
    out.push("Listo para reflexionar".into());
    if r.ready_to_reflect.is_empty() {
        out.push("  no hay evidencia suficiente".into());
    }
    for a in &r.ready_to_reflect {
        out.push(match &a.failure_class {
            Some(class) => format!("  {}: {} reuniones con {}", a.agent, a.count, reason(class)),
            None => format!("  {}: {} prioridades rechazadas", a.agent, a.count),
        });
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_fixed_rounds_a_half_up_as_javascript_does() {
        assert_eq!(fixed(0.125, 2), "0.13");
        assert_eq!(fixed(0.375, 2), "0.38");
        assert_eq!(fixed(1.0, 2), "1.00");
        assert_eq!(fixed(0.2, 2), "0.20");
        assert_eq!(fixed(1.005, 2), "1.00");
    }

    #[test]
    fn a_rate_prints_as_a_rounded_percent() {
        assert_eq!(pct(None), "—");
        assert_eq!(pct(Some(0.5)), "50%");
        assert_eq!(pct(Some(0.3333333333333333)), "33%");
        assert_eq!(pct(Some(2.0 / 3.0)), "67%");
        assert_eq!(num(2.0), "2");
        assert_eq!(num(0.75), "0.75");
    }
}
