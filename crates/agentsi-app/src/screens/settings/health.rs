//! The Health section: one row for each check of `core.doctor`, and "Check again". A run
//! takes up to 15 seconds for each tool, so the screen shows the old rows while it waits.
//! The Data section holds "Back up now" (`data.backup`) in `data.rs`.

use agentsi_proto::core_api::Empty;
use agentsi_proto::doctor::{Level, Status as Check};
use agentsi_proto::ops_api::{CoreDoctor, CoreDoctorResult, DoctorCheck};
use agentsi_ui::components::{Button, neutral_tag, status_tag};
use agentsi_ui::tokens::{Status, line, size, space, text, weight};
use gpui_kit::*;

use super::SettingsScreen;
use crate::screens::parts::{Load, mono, section};

#[derive(Default)]
pub(super) struct HealthState {
    pub(super) checks: Option<Load<Vec<DoctorCheck>>>,
    pub(super) running: bool,
}

fn level_word(l: Level) -> &'static str {
    match l {
        Level::Required => "required",
        Level::Stories => "stories",
        Level::Optional => "optional",
    }
}

fn row(c: &DoctorCheck) -> Div {
    let tag = match c.status {
        Check::Ok => status_tag("ok", Status::Ok),
        Check::Missing if c.level == Level::Optional => neutral_tag("missing"),
        Check::Missing => status_tag("missing", Status::Warn),
        Check::Fail => status_tag("fail", Status::Danger),
    };
    let detail = if c.detail.is_empty() {
        c.blocks.clone()
    } else if c.blocks.is_empty() || c.status == Check::Ok {
        c.detail.clone()
    } else {
        format!("{} · blocks {}", c.detail, c.blocks)
    };
    let name = c.name.clone();
    div()
        .debug_selector(move || format!("health-row:{name}"))
        .flex()
        .items_start()
        .gap(space(3))
        .py(space(2))
        .border_b_1()
        .border_color(line::border())
        .child(
            div()
                .w(space(40))
                .flex_none()
                .text_size(size::sm())
                .font_weight(weight::MEDIUM)
                .text_color(text::primary())
                .child(c.name.clone()),
        )
        .child(div().w(space(20)).flex_none().child(tag))
        .child(div().w(space(20)).flex_none().child(mono(level_word(c.level))))
        .child(div().flex_1().min_w_0().child(mono(detail)))
}

impl SettingsScreen {
    /// Runs `core.doctor`. A second run waits for the first.
    pub fn check_health(&mut self, cx: &mut Context<Self>) {
        if self.health.running {
            return;
        }
        self.health.running = true;
        if self.health.checks.is_none() {
            self.health.checks = Some(Load::Loading);
        }
        let call = self.core.read(cx).link.call::<CoreDoctor>(&Empty {});
        cx.spawn(async move |this, cx| {
            let result = call.await.map(|r: CoreDoctorResult| r.checks);
            let _ = this.update(cx, |this, cx| this.finish_health(result.map_err(|e| e.to_string()), cx));
        })
        .detach();
        cx.notify();
    }

    pub(super) fn finish_health(
        &mut self,
        result: Result<Vec<DoctorCheck>, String>,
        cx: &mut Context<Self>,
    ) {
        self.health.running = false;
        match &mut self.health.checks {
            Some(load) => load.settle(result),
            slot => {
                let mut load = Load::Loading;
                load.settle(result);
                *slot = Some(load);
            }
        }
        cx.notify();
    }

    pub(super) fn health_section(&mut self, cx: &mut Context<Self>) -> Div {
        let running = self.health.running;
        let body = match &self.health.checks {
            None => div().child(mono("Not checked yet.")),
            Some(Load::Loading) => div().child(mono("Checking the tools…")),
            Some(Load::Failed(e)) => div()
                .text_size(size::sm())
                .text_color(Status::Danger.color())
                .child(format!("The checks did not run: {e}")),
            Some(Load::Ready(checks)) => div().flex().flex_col().children(checks.iter().map(row)),
        };
        div()
            .debug_selector(|| "health-section".into())
            .flex()
            .flex_col()
            .gap(space(3))
            .child(section(
                "Health",
                match &self.health.checks {
                    Some(Load::Ready(c)) => Some(c.len()),
                    _ => None,
                },
            ))
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::secondary())
                    .child("The tools that AgentSI needs, checked on the host of this core. A check can take a few seconds."),
            )
            .child(body)
            .child(
                div().child(
                    Button::new("health-check")
                        .secondary()
                        .small()
                        .label(if running { "Checking…" } else { "Check again" })
                        .disabled(running)
                        .on_click(cx.listener(|this, _, _, cx| this.check_health(cx))),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use agentsi_proto::ops_api::{DataBackupResult, DoctorCheck};
    use gpui::{Modifiers, TestAppContext, VisualTestContext};
    use serde_json::Value;

    use crate::link::CallError;
    use crate::screens::settings::tests::support::{Setup, setup};

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    /// The methods that the screen sent since the last read.
    fn methods(t: &mut Setup) -> Vec<String> {
        t.core_end.set_nonblocking(true).unwrap();
        let mut out = Vec::new();
        while let Some(f) = agentsi_proto::frame::read_frame(&mut t.core_end)
            .ok()
            .flatten()
        {
            let v: Value = serde_json::from_slice(&f.payload).unwrap();
            out.push(v["method"].as_str().unwrap().to_string());
        }
        out
    }

    fn click(t: &Setup, cx: &mut VisualTestContext, selector: &'static str) {
        t.screen.update(cx, |s, _| s.scroll.scroll_to_bottom());
        draw(cx);
        let b = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is drawn"));
        cx.simulate_click(b.center(), Modifiers::none());
        draw(cx);
    }

    fn check(name: &str, status: &str) -> DoctorCheck {
        serde_json::from_value(serde_json::json!({
            "name": name, "level": "required", "status": status,
            "detail": "x", "blocks": "meetings"
        }))
        .unwrap()
    }

    #[gpui::test]
    fn the_health_section_runs_core_doctor_shows_each_row_and_checks_again(
        cx: &mut TestAppContext,
    ) {
        let (mut t, cx) = setup(cx, None);
        assert!(methods(&mut t).contains(&"core.doctor".to_string()));
        t.screen.update(cx, |s, cx| {
            s.finish_health(Ok(vec![check("core", "ok"), check("git", "missing")]), cx)
        });
        t.screen.update(cx, |s, _| s.scroll.scroll_to_bottom());
        draw(cx);
        assert!(cx.debug_bounds("health-row:core").is_some());
        assert!(cx.debug_bounds("health-row:git").is_some());

        click(&t, cx, "button:health-check");
        assert_eq!(methods(&mut t), ["core.doctor"]);
        // The old rows stay while the run waits.
        assert!(cx.debug_bounds("health-row:core").is_some());
    }

    #[gpui::test]
    fn back_up_now_calls_data_backup_and_shows_both_paths(cx: &mut TestAppContext) {
        let (mut t, cx) = setup(cx, None);
        methods(&mut t);
        click(&t, cx, "button:backup-now");
        assert_eq!(methods(&mut t), ["data.backup"]);
        let done = DataBackupResult {
            file: "/d/backups/agentsi-2026-11-09.db".into(),
            registry: Some("/d/backups/agentsi.config-2026-11-09.json".into()),
            removed: vec![],
        };
        t.screen.update(cx, |s, cx| s.finish_backup(Ok(done), cx));
        t.screen.update(cx, |s, _| s.scroll.scroll_to_bottom());
        draw(cx);
        assert!(cx.debug_bounds("backup-outcome").is_some());
        let lines = t.screen.read_with(cx, |s, _| s.clear.backup.clone());
        let lines = lines.unwrap().unwrap();
        assert!(lines[0].starts_with("Backup: /d/backups/"));
        assert!(lines[1].starts_with("Registry: "));

        t.screen.update(cx, |s, cx| {
            s.finish_backup(Err(CallError::Link("closed".into())), cx)
        });
        assert!(t.screen.read_with(cx, |s, _| s.clear.backup.clone()).unwrap().is_err());
    }
}
