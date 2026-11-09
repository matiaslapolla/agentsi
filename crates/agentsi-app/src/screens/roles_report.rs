//! The brief and the audit sections of the Roles screen. They read what `roles.brief` and
//! `data.audit` give, as the sections of the old web page did, and they write nothing.
//! The functions that make the text are plain, so a test checks each figure.

use agentsi_proto::agency_api::AuditReport;
use agentsi_proto::roles_api::BriefView;
use agentsi_ui::components::{neutral_tag, status_tag};
use agentsi_ui::tokens::{Status, line, size, space, text, weight};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::parts::{Load, mono, role_label, section, stat, stats};
use crate::format::usd;

fn percent(n: Option<f64>) -> String {
    n.map_or("—".into(), |n| format!("{}%", (n * 100.0).round()))
}

/// A number with at most 2 decimals.
fn rounded(n: f64) -> String {
    format!("{}", (n * 100.0).round() / 100.0)
}

fn rounds(n: Option<f64>) -> String {
    n.map_or("—".into(), rounded)
}

/// The figures above the tables of the audit, in the order of the web page.
pub fn audit_figures(r: &AuditReport) -> Vec<(String, String)> {
    let m = &r.meetings;
    vec![
        ("Meetings".into(), m.count.to_string()),
        ("First pass".into(), percent(m.first_pass_rate)),
        ("Repaired".into(), percent(m.repair_rate)),
        (
            format!("Repair rounds, first {}", r.repairs.baseline.minutes.len()),
            rounds(r.repairs.baseline.per_meeting),
        ),
        (
            format!("Repair rounds, since ({})", r.repairs.since.minutes.len()),
            rounds(r.repairs.since.per_meeting),
        ),
        (
            "Cost of the synthesis".into(),
            percent(m.synthesis_cost_share),
        ),
        ("Invented sources".into(), m.invented_sources.to_string()),
    ]
}

fn reason(class: Option<&str>, why: &str) -> String {
    if why == "rejected_priorities" {
        return "rejected priorities".into();
    }
    match class.unwrap_or_default() {
        "unresolved_source" => "unresolved sources".into(),
        "invented_source" => "invented sources".into(),
        "missing_objective" => "priorities with no valid objective".into(),
        "timeout" => "timeouts".into(),
        "sin_datos" => "repeated \"sin datos\"".into(),
        other => other.to_string(),
    }
}

/// One line for each agent that has evidence enough to reflect.
pub fn ready_lines(r: &AuditReport) -> Vec<String> {
    r.ready_to_reflect
        .iter()
        .map(|a| {
            format!(
                "{}: {} {}",
                a.agent,
                a.count,
                reason(a.failure_class.as_deref(), &a.reason)
            )
        })
        .collect()
}

/// The agents that attended a meeting, with their numbers.
pub fn meeting_rows(r: &AuditReport) -> Vec<[String; 7]> {
    r.meetings
        .agents
        .iter()
        .filter(|a| a.meetings > 0)
        .map(|a| {
            [
                a.agent.clone(),
                format!("{}/{}", a.answers, a.meetings),
                a.errors.to_string(),
                a.timeouts.to_string(),
                rounded(a.unresolved_sources_per_meeting),
                rounded(a.sin_datos_per_meeting),
                usd(a.mean_cost_usd),
            ]
        })
        .collect()
}

pub fn mark_rows(r: &AuditReport) -> Vec<[String; 5]> {
    r.marks
        .0
        .iter()
        .map(|(agent, c)| {
            [
                agent.clone(),
                c.accepted.to_string(),
                c.changed.to_string(),
                c.rejected.to_string(),
                c.unmarked.to_string(),
            ]
        })
        .collect()
}

pub fn memory_rows(r: &AuditReport) -> Vec<[String; 6]> {
    r.memory
        .iter()
        .map(|x| {
            [
                x.agent.clone(),
                x.active.to_string(),
                x.retired.to_string(),
                x.lessons.to_string(),
                x.never_helpful.to_string(),
                format!("{}/{}", x.chars, x.max_chars),
            ]
        })
        .collect()
}

pub fn reflection_line(r: &AuditReport) -> String {
    let s = &r.reflections;
    format!(
        "Last: {} · {} pending proposals · {} reverted",
        s.last.as_ref().map_or("none", |l| l.id.as_str()),
        s.pending_proposals,
        s.reverted
    )
}

/// One line for each objective of the brief. `today` is `YYYY-MM-DD`.
pub fn objective_lines(brief: &BriefView, today: &str) -> Vec<String> {
    brief
        .objectives
        .iter()
        .map(|o| {
            let overdue = o.due.len() == 10 && o.due.as_str() < today;
            format!(
                "{} · {} · signal: {} · due {}{}",
                o.id,
                o.text,
                o.signal,
                o.due,
                if overdue { " · overdue" } else { "" }
            )
        })
        .collect()
}

/// What the brief says that an agent is for.
pub fn why_of<'a>(brief: &'a BriefView, agent: &str) -> Option<&'a str> {
    brief
        .roles
        .get(agent)
        .map(String::as_str)
        .filter(|s| !s.is_empty())
}

fn label(s: &str) -> Div {
    div()
        .text_size(size::xs())
        .font_weight(weight::MEDIUM)
        .text_color(text::tertiary())
        .child(s.to_string())
}

fn body(s: impl Into<SharedString>) -> Div {
    div()
        .text_size(size::sm())
        .text_color(text::secondary())
        .child(s.into())
}

fn block() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space(3))
        .p(space(4))
        .rounded(agentsi_ui::tokens::radius::lg())
        .border_1()
        .border_color(line::border())
        .bg(agentsi_ui::tokens::surface::raised())
}

/// A table of text. The first column holds the agent.
fn table<const N: usize>(head: [&str; N], rows: Vec<[String; N]>) -> Div {
    let cell = |s: String, first: bool| {
        let c = div().flex_1().text_size(size::sm());
        if first {
            c.text_color(text::primary()).child(s)
        } else {
            c.text_right().text_color(text::secondary()).child(s)
        }
    };
    div()
        .flex()
        .flex_col()
        .gap(space(1))
        .child(
            div()
                .flex()
                .gap(space(3))
                .pb(space(1))
                .border_b_1()
                .border_color(line::border())
                .children(head.iter().enumerate().map(|(i, h)| {
                    let c = div().flex_1();
                    if i == 0 {
                        c.child(label(h))
                    } else {
                        c.text_right().child(label(h))
                    }
                })),
        )
        .children(rows.into_iter().map(|row| {
            div().flex().gap(space(3)).children(
                row.into_iter()
                    .enumerate()
                    .map(|(i, s)| cell(if i == 0 { role_label(&s) } else { s }, i == 0)),
            )
        }))
}

/// The brief when it has loaded and the product has one.
pub fn ready_brief(brief: &Load<Option<BriefView>>) -> Option<&BriefView> {
    match brief {
        Load::Ready(b) => b.as_ref(),
        _ => None,
    }
}

/// The line of a Brief section that has no brief to show.
pub fn brief_text(brief: &Load<Option<BriefView>>) -> String {
    match brief {
        Load::Loading => "Loading the brief.".into(),
        Load::Failed(e) => format!("The brief did not load: {e}"),
        _ => "This product has no brief yet. Run /agentsi setup inside the product.".into(),
    }
}

/// The Brief section. `today` is `YYYY-MM-DD`.
pub fn brief_section(brief: &Load<Option<BriefView>>, today: &str) -> Div {
    let title = |b: Option<&BriefView>| {
        section("Brief", None).children(
            b.and_then(|b| b.updated_at.as_deref())
                .map(|d| mono(d.get(..10).unwrap_or(d).to_string())),
        )
    };
    let content = match brief {
        Load::Loading | Load::Failed(_) | Load::Ready(None) => {
            block().child(body(brief_text(brief)))
        }
        Load::Ready(Some(b)) => block()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space(2))
                    .child(label("What it is"))
                    .child(body(b.what.clone()))
                    .child(label("Who it is for"))
                    .child(body(b.audience.clone()))
                    .child(label("Stage"))
                    .child(div().flex().child(neutral_tag(b.stage.clone()))),
            )
            .child(label("Objectives"))
            .child(list(&objective_lines(b, today), "No objectives."))
            .child(label("North stars"))
            .child(list(&b.north_stars, "None."))
            .child(label("Out of scope"))
            .child(list(&b.out_of_scope, "Nothing declared.")),
    };
    div()
        .flex()
        .flex_col()
        .gap(space(3))
        .child(title(ready_brief(brief)))
        .child(content)
}

fn list(lines: &[String], empty: &str) -> Div {
    if lines.is_empty() {
        return body(empty.to_string());
    }
    div()
        .flex()
        .flex_col()
        .gap(space(1))
        .children(lines.iter().map(|l| body(l.clone())))
}

/// The Audit section.
pub fn audit_section(audit: &Load<AuditReport>) -> Div {
    let head = section("Audit", None);
    let r = match audit {
        Load::Ready(r) => r,
        other => {
            let line = match other {
                Load::Failed(e) => format!("The audit did not load: {e}"),
                _ => "Loading the audit.".into(),
            };
            return div()
                .flex()
                .flex_col()
                .gap(space(3))
                .child(head)
                .child(block().child(body(line)));
        }
    };
    let ready = ready_lines(r);
    let meetings = meeting_rows(r);
    let marks = mark_rows(r);
    div()
        .flex()
        .flex_col()
        .gap(space(3))
        .child(head.child(mono(r.generated_at.get(..10).unwrap_or_default().to_string())))
        .child(
            block()
                .child(if ready.is_empty() {
                    body("Ready to reflect: not enough evidence.")
                } else {
                    div()
                        .flex()
                        .flex_col()
                        .gap(space(1))
                        .child(status_tag("Ready to reflect", Status::Warn))
                        .children(ready.iter().map(|l| body(l.clone())))
                        .child(body(
                            "Run /agentsi reflect to curate the memory and propose a prompt change.",
                        ))
                })
                .child(label("Configuration"))
                .child(if r.config.valid {
                    div().flex().child(status_tag("roster is valid", Status::Ok))
                } else {
                    list(&r.config.failures, "")
                })
                .children(r.config.overdue_objectives.iter().map(|o| {
                    body(format!("Overdue objective: {} ({}) {}", o.id, o.due, o.text))
                }))
                .child(label(&format!("Meetings (last {})", r.meetings.count)))
                .child(stats(
                    audit_figures(r)
                        .into_iter()
                        .map(|(l, v)| stat(&l, v))
                        .collect(),
                ))
                .when(!meetings.is_empty(), |el| {
                    el.child(table(
                        [
                            "Agent", "Answers", "Errors", "Timeouts", "Unresolved", "No data",
                            "Mean cost",
                        ],
                        meetings,
                    ))
                    .child(body("Unresolved and no data are averages for each meeting."))
                })
                .child(label("Marks of each agent"))
                .child(if marks.is_empty() {
                    body("No priorities.")
                } else {
                    table(
                        ["Agent", "Accepted", "Changed", "Rejected", "Unmarked"],
                        marks,
                    )
                })
                .child(label("Memory"))
                .child(table(
                    ["Agent", "Active", "Retired", "Lessons", "Never helpful", "Characters"],
                    memory_rows(r),
                ))
                .child(label("Reflections"))
                .child(body(reflection_line(r))),
        )
}

#[cfg(test)]
pub(crate) mod tests {
    // No `use super::*`: it brings in the `test` module of `gpui_kit`, which hides `#[test]`.
    use super::{
        audit_figures, mark_rows, meeting_rows, memory_rows, objective_lines, ready_lines,
        reflection_line, why_of,
    };
    use agentsi_proto::agency_api::AuditReport;
    use agentsi_proto::roles_api::BriefView;

    pub(crate) const AUDIT: &str = r#"{
      "product": "demo", "generated_at": "2026-11-07T10:00:00.000Z",
      "config": {"valid": false, "failures": ["The roster is empty."],
                 "overdue_objectives": [{"id": "o1", "due": "2026-10-01", "text": "Ship it"}]},
      "meetings": {"count": 4, "first_pass_rate": 0.75, "repair_rate": 0.25, "synthesis_cost_share": 0.4,
        "invented_sources": 2,
        "agents": [
          {"agent": "pm", "meetings": 4, "answers": 3, "errors": 1, "timeouts": 0,
           "unresolved_sources_per_meeting": 0.5, "sin_datos_per_meeting": 0.25, "mean_cost_usd": 0.1234},
          {"agent": "qa", "meetings": 0, "answers": 0, "errors": 0, "timeouts": 0,
           "unresolved_sources_per_meeting": 0.0, "sin_datos_per_meeting": 0.0, "mean_cost_usd": 0.0}]},
      "repairs": {"baseline": {"minutes": ["a", "b"], "repaired": 1, "per_meeting": 0.5},
                  "since": {"minutes": ["c"], "repaired": 0, "per_meeting": null}},
      "last_repairs": null,
      "marks": {"pm": {"accepted": 3, "changed": 1, "rejected": 2, "unmarked": 0}},
      "memory": [{"agent": "pm", "active": 2, "retired": 1, "lessons": 1, "never_helpful": 0, "chars": 120, "max_chars": 4000}],
      "reflections": {"last": {"id": "r-1", "created_at": "2026-11-01T00:00:00.000Z"}, "pending_proposals": 1, "reverted": 0},
      "ready_to_reflect": [
        {"agent": "dev", "reason": "failure_class", "failure_class": "invented_source", "count": 3},
        {"agent": "qa", "reason": "rejected_priorities", "failure_class": null, "count": 4}]
    }"#;

    fn audit() -> AuditReport {
        serde_json::from_str(AUDIT).unwrap()
    }

    #[test]
    fn the_audit_figures_match_the_web_tiles() {
        assert_eq!(
            audit_figures(&audit()),
            [
                ("Meetings".to_string(), "4".to_string()),
                ("First pass".into(), "75%".into()),
                ("Repaired".into(), "25%".into()),
                ("Repair rounds, first 2".into(), "0.5".into()),
                ("Repair rounds, since (1)".into(), "—".into()),
                ("Cost of the synthesis".into(), "40%".into()),
                ("Invented sources".into(), "2".into()),
            ]
        );
    }

    #[test]
    fn the_ready_lines_name_the_agent_the_count_and_the_reason() {
        assert_eq!(
            ready_lines(&audit()),
            ["dev: 3 invented sources", "qa: 4 rejected priorities"]
        );
    }

    #[test]
    fn the_tables_list_each_agent_with_its_numbers() {
        let r = audit();
        assert_eq!(
            meeting_rows(&r),
            [["pm", "3/4", "1", "0", "0.5", "0.25", "$0.12"]]
        );
        assert_eq!(mark_rows(&r), [["pm", "3", "1", "2", "0"]]);
        assert_eq!(memory_rows(&r), [["pm", "2", "1", "1", "0", "120/4000"]]);
        assert_eq!(
            reflection_line(&r),
            "Last: r-1 · 1 pending proposals · 0 reverted"
        );
    }

    #[test]
    fn the_brief_lines_hold_the_product_the_objectives_and_the_limits() {
        let brief: BriefView = serde_json::from_value(serde_json::json!({
            "what": "A mail filter", "audience": "Small teams", "stage": "building",
            "objectives": [{"id": "o1", "text": "Ship the filter", "signal": "10 users", "due": "2020-01-01"}],
            "north_stars": ["Calm inbox"], "out_of_scope": [],
            "roles": {"pm": "Ranks the work."}, "updated_at": "2026-10-30T10:00:00.000Z"
        }))
        .unwrap();
        assert_eq!(
            objective_lines(&brief, "2026-11-07"),
            ["o1 · Ship the filter · signal: 10 users · due 2020-01-01 · overdue"]
        );
        assert_eq!(
            objective_lines(&brief, "2019-01-01"),
            ["o1 · Ship the filter · signal: 10 users · due 2020-01-01"]
        );
        assert_eq!(why_of(&brief, "pm"), Some("Ranks the work."));
        assert_eq!(why_of(&brief, "dev"), None);
    }
}
