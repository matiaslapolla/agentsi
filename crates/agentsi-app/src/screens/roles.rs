//! The roles of a product (U21): the roster, the specialists, the overlays, and the
//! validate, apply and reset actions. `roles.apply` and `roles.reset` plan first. The
//! screen shows the plan, and a second click on Confirm writes it. There is no text
//! field: Apply takes the path of the stage folder from the clipboard.

use agentsi_proto::agency_api::{AuditParams, AuditReport, DataAudit};
use agentsi_proto::roles_api::{
    ApplyPlan, BriefView, ChangeStatus, ResetPlan, RoleSource, RolesApply, RolesApplyParams,
    RolesBrief, RolesBriefParams, RolesList, RolesListParams, RolesListResult, RolesReset,
    RolesResetParams, RolesValidate, RolesValidateParams, RolesValidateResult, RosterEntry,
};
use agentsi_ui::components::{Button, Card, neutral_tag, status_tag};
use agentsi_ui::tokens::{Status, line, radius, size, space, surface, text, weight};
use gpui_kit::component::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::parts::{Load, column, first_screen_ready, mono, notice, role_label, section};
use super::roles_report;
use crate::model::CoreState;

/// A plan that waits for the owner to confirm it.
#[derive(Clone)]
enum Pending {
    Reset(RolesResetParams, ResetPlan),
    Apply(RolesApplyParams, ApplyPlan),
}

pub struct RolesScreen {
    product: String,
    core: Entity<CoreState>,
    data: Load<RolesListResult>,
    /// The brief and the audit load on their own. A failure of one leaves the roster on screen.
    brief: Load<Option<BriefView>>,
    audit: Load<AuditReport>,
    validation: Option<Result<RolesValidateResult, String>>,
    pending: Option<Pending>,
    /// The result of the last action, or its error.
    message: Option<Result<String, String>>,
    scroll: ScrollHandle,
}

/// The lines that describe a reset plan, as `agentsi reset` prints them.
pub fn reset_lines(plan: &ResetPlan) -> Vec<String> {
    let mut lines = Vec::new();
    lines.extend(plan.removals.iter().map(|r| format!("Delete {r}")));
    lines.extend(plan.edits.iter().map(|r| format!("Edit {r}")));
    if plan.roster_before != plan.roster_after {
        lines.push(format!(
            "Roster: {} → {}",
            plan.roster_before.join(", "),
            plan.roster_after.join(", ")
        ));
    }
    lines
}

/// The lines that describe an apply plan.
pub fn apply_lines(plan: &ApplyPlan) -> Vec<String> {
    let mut lines = vec![format!(
        "Roster: {} → {}",
        if plan.roster_before.is_empty() {
            "(none)".to_string()
        } else {
            plan.roster_before.join(", ")
        },
        plan.roster_after.join(", ")
    )];
    if plan.stories_changed {
        let on = |s: &Option<serde_json::Value>| if s.is_some() { "on" } else { "off" };
        lines.push(format!(
            "Stories: {} → {}",
            on(&plan.stories_before),
            on(&plan.stories_after)
        ));
    }
    if plan.changes.is_empty() {
        lines.push("The brief and the overlays do not change.".into());
    }
    lines.extend(plan.changes.iter().map(|c| {
        let verb = match c.status {
            ChangeStatus::New => "New",
            ChangeStatus::Changed => "Changed",
            ChangeStatus::Removed => "Removed",
        };
        format!("{verb} {}", c.path)
    }));
    lines
}

impl RolesScreen {
    pub fn new(product: String, core: Entity<CoreState>, cx: &mut Context<Self>) -> Self {
        let mut screen = RolesScreen {
            product,
            core,
            data: Load::Loading,
            brief: Load::Loading,
            audit: Load::Loading,
            validation: None,
            pending: None,
            message: None,
            scroll: ScrollHandle::new(),
        };
        screen.reload(cx);
        screen
    }

    /// Reads the roster again. The screen keeps what it shows until the reply comes.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let call = self.core.read(cx).link.call::<RolesList>(&RolesListParams {
            product: self.product.clone(),
            agent: None,
        });
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.data.settle(result);
                first_screen_ready();
                cx.notify();
            });
        })
        .detach();
        let link = self.core.read(cx).link.clone();
        let product = self.product.clone();
        let brief = link.call::<RolesBrief>(&RolesBriefParams {
            product: product.clone(),
        });
        let audit = link.call::<DataAudit>(&AuditParams { product });
        cx.spawn(async move |this, cx| {
            let brief = brief.await.map(|r| r.brief).map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.brief.settle(brief);
                cx.notify();
            });
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let audit = audit.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.audit.settle(audit);
                cx.notify();
            });
        })
        .detach();
    }

    fn validate(&mut self, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<RolesValidate>(&RolesValidateParams {
                product: self.product.clone(),
            });
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.validation = Some(result);
                cx.notify();
            });
        })
        .detach();
    }

    /// Plans a reset of `agent`, or of the whole product, and waits for the confirm.
    fn plan_reset(&mut self, agent: Option<String>, cx: &mut Context<Self>) {
        let params = RolesResetParams {
            product: self.product.clone(),
            agent,
            memory: false,
            yes: false,
        };
        let call = self.core.read(cx).link.call::<RolesReset>(&params);
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.message = None;
                match result {
                    Ok(r) if r.plan.noop => {
                        this.pending = None;
                        this.message = Some(Ok("Nothing to reset.".into()));
                    }
                    Ok(r) => this.pending = Some(Pending::Reset(params, r.plan)),
                    Err(e) => this.message = Some(Err(e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Plans an apply of the stage folder whose path is on the clipboard.
    fn plan_apply(&mut self, cx: &mut Context<Self>) {
        let path = cx
            .read_from_clipboard()
            .and_then(|c| c.text())
            .map(|t| t.trim().to_string())
            .unwrap_or_default();
        let host = self.core.read(cx).remote.clone();
        if path.is_empty() {
            self.message = Some(Err(match &host {
                Some(h) => format!("Copy the path of the stage folder on {h} first."),
                None => "Copy the path of the stage folder first.".into(),
            }));
            cx.notify();
            return;
        }
        let params = RolesApplyParams {
            product: self.product.clone(),
            stage_dir: path,
            yes: false,
            register: None,
        };
        let call = self.core.read(cx).link.call::<RolesApply>(&params);
        cx.spawn(async move |this, cx| {
            // The core of the host reads the path, so a folder of this device fails there.
            let result = call.await.map_err(|e| match &host {
                Some(h) => format!("{e} The stage folder must be on {h}."),
                None => e.to_string(),
            });
            let _ = this.update(cx, |this, cx| {
                this.message = None;
                match result {
                    Ok(r) => match r.plan {
                        Some(plan) => this.pending = Some(Pending::Apply(params, plan)),
                        None => {
                            this.pending = None;
                            this.message = Some(Err(format!(
                                "The stage is not valid: {}",
                                r.failures.join(" · ")
                            )));
                        }
                    },
                    Err(e) => this.message = Some(Err(e)),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Writes the pending plan.
    fn confirm(&mut self, cx: &mut Context<Self>) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let link = self.core.read(cx).link.clone();
        let (done, commit) = match pending {
            Pending::Reset(mut p, _) => {
                p.yes = true;
                let call = link.call::<RolesReset>(&p);
                (
                    cx.background_spawn(async move {
                        call.await.map_err(|e| e.to_string()).and_then(|r| {
                            if r.applied {
                                Ok(r.commit)
                            } else {
                                Err("The data changed since the plan. Nothing was written."
                                    .to_string())
                            }
                        })
                    }),
                    "Reset",
                )
            }
            Pending::Apply(mut p, _) => {
                p.yes = true;
                let call = link.call::<RolesApply>(&p);
                (
                    cx.background_spawn(async move {
                        call.await.map_err(|e| e.to_string()).and_then(|r| {
                            if r.applied {
                                Ok(r.commit)
                            } else {
                                Err("The data changed since the plan. Nothing was written."
                                    .to_string())
                            }
                        })
                    }),
                    "Applied",
                )
            }
        };
        cx.spawn(async move |this, cx| {
            let result = done.await;
            let _ = this.update(cx, |this, cx| {
                this.message = Some(result.map(|c| {
                    let c = c.unwrap_or_default();
                    match (c.committed, c.error) {
                        (_, Some(e)) => format!(
                            "{commit}. The commit was skipped: {}",
                            e.lines().next().unwrap_or_default()
                        ),
                        (true, None) => format!("{commit} and committed."),
                        (false, None) => format!("{commit}. No new commit."),
                    }
                }));
                this.validation = None;
                this.reload(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        cx.notify();
    }
}

fn source_tag(source: Option<RoleSource>) -> Option<impl IntoElement> {
    source.map(|s| match s {
        RoleSource::Global => neutral_tag("global"),
        RoleSource::Overlay => status_tag("overlay", Status::Info),
        RoleSource::Custom => status_tag("custom", Status::Warn),
    })
}

fn line_list(lines: &[String]) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space(1))
        .children(lines.iter().map(|l| mono(l.clone())))
}

impl RolesScreen {
    fn agent_card(&self, i: usize, e: &RosterEntry, cx: &mut Context<Self>) -> Card {
        let id = e.id.clone();
        let model = match (&e.model, &e.effort) {
            (Some(m), Some(x)) => Some(format!("{m} · {x}")),
            (Some(m), None) => Some(m.clone()),
            _ => None,
        };
        Card::new(("roles-agent", i))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space(2))
                    .child(
                        div()
                            .text_size(size::md())
                            .font_weight(weight::SEMIBOLD)
                            .text_color(text::primary())
                            .child(role_label(&e.id)),
                    )
                    .child(mono(e.id.clone()))
                    .children(source_tag(e.source))
                    .when(e.id.contains('.'), |el| el.child(neutral_tag("specialist")))
                    .children(model.map(mono)),
            )
            .when_some(e.error.clone(), |el, error| {
                el.child(
                    div()
                        .text_size(size::sm())
                        .text_color(text::secondary())
                        .child(error),
                )
            })
            .when_some(
                roles_report::ready_brief(&self.brief)
                    .and_then(|b| roles_report::why_of(b, &e.id))
                    .map(String::from),
                |el, why| {
                    el.child(
                        div()
                            .text_size(size::sm())
                            .text_color(text::secondary())
                            .child(why),
                    )
                },
            )
            .child(
                div().flex().justify_end().child(
                    Button::new(("roles-reset", i))
                        .ghost()
                        .small()
                        .icon(IconName::RotateCw)
                        .label("Reset to the global role")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.plan_reset(Some(id.clone()), cx);
                        })),
                ),
            )
    }

    fn pending_card(&self, cx: &mut Context<Self>) -> Option<Div> {
        let pending = self.pending.as_ref()?;
        let (title, lines) = match pending {
            Pending::Reset(p, plan) => (
                match &p.agent {
                    Some(a) => format!("Reset {a} to the global role"),
                    None => "Reset the whole product to the global roles".to_string(),
                },
                reset_lines(plan),
            ),
            Pending::Apply(_, plan) => ("Apply the stage folder".to_string(), apply_lines(plan)),
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap(space(3))
                .p(space(4))
                .rounded(radius::lg())
                .border_1()
                .border_color(line::border())
                .bg(surface::raised())
                .child(section(&title, None))
                .child(line_list(&lines))
                .child(
                    div()
                        .flex()
                        .gap(space(2))
                        .child(
                            Button::new("roles-confirm")
                                .danger()
                                .small()
                                .label("Confirm")
                                .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                        )
                        .child(
                            Button::new("roles-cancel")
                                .secondary()
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                        ),
                ),
        )
    }

    fn validation_block(&self) -> Option<Div> {
        let v = self.validation.as_ref()?;
        let body = match v {
            Err(e) => vec![mono(format!("Validate failed: {e}"))],
            Ok(r) if r.valid => vec![mono(format!("The roster is valid ({} agents).", r.agents))],
            Ok(r) => r.failures.iter().map(|f| mono(f.clone())).collect(),
        };
        Some(
            div()
                .flex()
                .flex_col()
                .gap(space(1))
                .child(section(
                    "Validation",
                    v.as_ref().ok().map(|r| r.failures.len()).filter(|n| *n > 0),
                ))
                .children(body),
        )
    }
}

impl Render for RolesScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().id("roles-screen").size_full().bg(surface::panel());
        let r = match &self.data {
            Load::Loading => {
                return root.child(notice("Reading the roles…", None));
            }
            Load::Failed(e) => {
                return root.child(notice(
                    format!("The roles did not load: {e}"),
                    Some(super::parts::reload_hint(cx)),
                ));
            }
            Load::Ready(r) => r.clone(),
        };
        let roster = r.roster.clone().unwrap_or_default();
        let (specialists, roles): (Vec<_>, Vec<_>) =
            roster.iter().cloned().partition(|e| e.id.contains('.'));
        let stray: Vec<String> = r
            .overlays
            .iter()
            .filter(|o| !roster.iter().any(|e| &e.id == *o))
            .cloned()
            .collect();

        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(space(3))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(space(1))
                    .child(
                        div()
                            .text_size(size::xl())
                            .font_weight(weight::SEMIBOLD)
                            .text_color(text::primary())
                            .child("Roles"),
                    )
                    .child(mono(format!(
                        "{} · {} agents · {} global roles",
                        r.product,
                        roster.len(),
                        r.globals.len()
                    ))),
            )
            .child(
                div()
                    .flex()
                    .gap(space(2))
                    .child(
                        Button::new("roles-validate")
                            .secondary()
                            .small()
                            .label("Validate")
                            .on_click(cx.listener(|this, _, _, cx| this.validate(cx))),
                    )
                    .child(
                        Button::new("roles-apply")
                            .secondary()
                            .small()
                            .label(match &self.core.read(cx).remote {
                                Some(h) => {
                                    format!("Apply stage from the clipboard (a path on {h})")
                                }
                                None => "Apply stage from the clipboard".to_string(),
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.plan_apply(cx))),
                    )
                    .child(
                        Button::new("roles-reset-all")
                            .danger()
                            .small()
                            .label("Reset all")
                            .on_click(cx.listener(|this, _, _, cx| this.plan_reset(None, cx))),
                    ),
            );

        let message = self.message.as_ref().map(|m| match m {
            Ok(text) => mono(text.clone()),
            Err(e) => mono(format!("Error: {e}")),
        });

        let group = |title: &str,
                     items: Vec<RosterEntry>,
                     offset: usize,
                     empty: &str,
                     cx: &mut Context<Self>| {
            div()
                .flex()
                .flex_col()
                .gap(space(3))
                .child(section(title, Some(items.len())))
                .when(items.is_empty(), |el| {
                    el.child(
                        div()
                            .text_size(size::sm())
                            .text_color(text::tertiary())
                            .child(empty.to_string()),
                    )
                })
                .children(
                    items
                        .iter()
                        .enumerate()
                        .map(|(i, e)| self.agent_card(offset + i, e, cx))
                        .collect::<Vec<_>>(),
                )
        };
        let roles_len = roles.len();
        let roles_block = group(
            "Roster",
            roles,
            0,
            "This product has no agents in its roster.",
            cx,
        );
        let specialists_block = group(
            "Specialists",
            specialists,
            roles_len,
            "No specialists. A specialist is a role with an overlay, such as dev.backend.",
            cx,
        );

        let overlays_block = (!stray.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(space(2))
                .child(section("Overlays outside the roster", Some(stray.len())))
                .children(stray.iter().map(|o| mono(format!("agents/{o}.md"))))
        });

        root.track_scroll(&self.scroll).overflow_y_scroll().child(
            column()
                .child(header)
                .children(message)
                .children(self.pending_card(cx))
                .children(self.validation_block())
                .child(roles_block)
                .child(specialists_block)
                .children(overlays_block)
                .child(roles_report::brief_section(
                    &self.brief,
                    &crate::format::date_from_secs(crate::format::now_secs()),
                ))
                .child(roles_report::audit_section(&self.audit)),
        )
    }
}

#[cfg(test)]
mod tests {
    // No `use super::*`: it brings in the `test` module of `gpui_kit`, which hides `#[test]`.
    use super::{
        Load, Pending, ResetPlan, RolesResetParams, RolesScreen, RolesValidateResult, reset_lines,
    };
    use crate::link::Link;
    use crate::model::CoreState;
    use gpui::TestAppContext;
    use gpui_kit::AppContext as _;
    use std::sync::Arc;

    const LIST: &str = r#"{
        "product": "demo",
        "roster": [
          {"id": "pm", "role": "pm", "source": "global", "model": "opus", "effort": "high", "error": null},
          {"id": "dev.api", "role": "dev", "source": "overlay", "model": "sonnet", "effort": null, "error": null},
          {"id": "ghost", "role": null, "source": null, "model": null, "effort": null, "error": "Unknown role \"ghost\""}],
        "overlays": ["dev.api", "old"],
        "globals": ["pm", "dev"],
        "warnings": []
    }"#;

    fn screen(cx: &mut TestAppContext, change: fn(&mut RolesScreen)) {
        cx.update(agentsi_ui::init);
        let (end, _other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| RolesScreen::new("demo".into(), core, cx));
        screen.update(cx, |s, cx| {
            s.data = Load::Ready(serde_json::from_str(LIST).unwrap());
            change(s);
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn plan() -> ResetPlan {
        ResetPlan {
            product: "demo".into(),
            agent: Some("dev.api".into()),
            removals: vec!["agents/dev.api.md".into()],
            edits: vec!["brief.json".into()],
            roster_before: vec!["pm".into(), "dev.api".into()],
            roster_after: vec!["pm".into()],
            noop: false,
        }
    }

    fn params() -> RolesResetParams {
        RolesResetParams {
            product: "demo".into(),
            agent: Some("dev.api".into()),
            memory: false,
            yes: false,
        }
    }

    fn invalid() -> RolesValidateResult {
        RolesValidateResult {
            product: "demo".into(),
            valid: false,
            failures: vec!["The roster is empty.".into()],
            agents: 0,
            warnings: vec![],
        }
    }

    fn busy(s: &mut RolesScreen) {
        s.validation = Some(Ok(invalid()));
        s.message = Some(Err("no".into()));
        s.pending = Some(Pending::Reset(params(), plan()));
    }

    fn reported(s: &mut RolesScreen) {
        s.brief = Load::Ready(Some(
            serde_json::from_value(serde_json::json!({
                "what": "A mail filter", "audience": "Small teams", "stage": "building",
                "objectives": [{"id": "o1", "text": "Ship", "signal": "10 users", "due": "2020-01-01"}],
                "north_stars": ["Calm inbox"], "out_of_scope": [],
                "roles": {"pm": "Ranks the work."}, "updated_at": "2026-10-30T10:00:00.000Z"
            }))
            .unwrap(),
        ));
        s.audit = Load::Ready(serde_json::from_str(super::roles_report::tests::AUDIT).unwrap());
    }

    #[gpui::test]
    fn the_brief_and_the_audit_draw_with_data_with_none_and_with_errors(cx: &mut TestAppContext) {
        screen(cx, reported);
        screen(cx, |s| {
            reported(s);
            s.brief = Load::Ready(None);
        });
        screen(cx, |s| {
            s.brief = Load::Failed("no brief".into());
            s.audit = Load::Failed("no audit".into());
        });
        screen(cx, |s| {
            s.brief = Load::Loading;
            s.audit = Load::Loading;
        });
    }

    /// B8: the brief says "no brief yet" only after its own reply, whatever the audit does.
    #[gpui::test]
    fn the_brief_and_the_audit_settle_each_on_its_own(cx: &mut TestAppContext) {
        cx.update(agentsi_ui::init);
        let (end, _other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| RolesScreen::new("demo".into(), core, cx));
        screen.update(cx, |s, _| {
            assert!(matches!(s.brief, Load::Loading));
            assert!(matches!(s.audit, Load::Loading));
            s.brief.settle(Ok(None));
            assert!(matches!(s.brief, Load::Ready(None)));
            assert!(matches!(s.audit, Load::Loading));
        });
        assert!(
            super::roles_report::brief_text(&Load::Loading).contains("Loading"),
            "a brief that is still read is not a missing brief"
        );
    }

    #[gpui::test]
    fn the_roles_draw_in_each_state(cx: &mut TestAppContext) {
        screen(cx, |_| {});
        screen(cx, busy);
        screen(cx, |s| s.data = Load::Failed("no core".into()));
        screen(cx, |s| s.data = Load::Loading);
    }

    #[test]
    fn a_reset_plan_reads_as_lines() {
        let plan = ResetPlan {
            product: "demo".into(),
            agent: None,
            removals: vec!["brief.json".into(), "agents/qa.md".into()],
            edits: vec![],
            roster_before: vec!["pm".into(), "qa".into()],
            roster_after: vec!["pm".into()],
            noop: false,
        };
        assert_eq!(
            reset_lines(&plan),
            [
                "Delete brief.json",
                "Delete agents/qa.md",
                "Roster: pm, qa → pm"
            ]
        );
    }
}
