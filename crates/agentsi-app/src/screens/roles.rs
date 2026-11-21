//! The roles of a product (U21): the roster, the specialists, the overlays, and the
//! validate, apply and reset actions. `roles.apply` and `roles.reset` plan first. The
//! screen shows the plan, and a second click on Confirm writes it. "Update the brief" opens
//! the setup screen, which applies a new brief.
//!
//! The general roles (`docs/specs/agentsi-desktop-global-roles.md`) sit above the roster:
//! every product can add one. The roster picker lists the builtin roles, then the general
//! roles, then the custom roles of the product. A delete and a "Make general" ask one Confirm.

use agentsi_proto::agency_api::{AuditParams, AuditReport, DataAudit};
use agentsi_proto::method::Method;
use agentsi_proto::roles_api::{
    ApplyPlan, RoleDef, BriefView, ChangeStatus, CommitInfo, GeneralCreate, GeneralDelete,
    GeneralDeleteParams, GeneralEntry, GeneralList, GeneralListParams, GeneralListResult,
    GeneralPromote, GeneralPromoteParams, GeneralRole, GeneralUpdate, ResetPlan, RoleSource,
    RolesBrief, RolesBriefParams, RolesList, RolesListParams, RolesListResult, RolesReset,
    RolesResetParams, RolesRoster, RolesRosterParams, RolesValidate, RolesValidateParams,
    RolesValidateResult, RosterEntry,
};
use agentsi_ui::components::{Button, Card, neutral_tag, status_tag};
use agentsi_ui::tokens::{Status, line, radius, size, space, surface, text, weight};
use gpui_kit::component::IconName;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::ScreenSpec;
use super::form::{Form, FormEvent};
use super::parts::{Load, column, first_screen_ready, mono, notice, role_label, section};
use super::roles_report;
use crate::model::CoreState;
use crate::nav::Nav;

/// A plan that waits for the owner to confirm it.
#[derive(Clone)]
enum Pending {
    Reset(RolesResetParams, ResetPlan),
    /// Deletes a general role.
    DeleteGeneral(String),
    /// Moves a custom role of the product to a general role.
    Promote(String),
}

/// What the form of a general role writes.
#[derive(Clone, Debug, PartialEq, Eq)]
enum GeneralForm {
    New,
    Edit(GeneralRole),
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
    /// The resolved definition of one agent, in a panel: the agent and its definition or error.
    definition: Option<(String, Result<RoleDef, String>)>,
    general: Load<GeneralListResult>,
    /// The form that the next draw makes: it needs the window.
    want_form: Option<GeneralForm>,
    form: Option<(Entity<Form>, GeneralForm, Subscription)>,
}

/// The candidates of the roster picker in three groups: the builtin roles, the general roles,
/// then the custom roles and the specialists of the product.
pub fn picker_groups(
    globals: &[String],
    builtin: &[String],
    overlays: &[String],
) -> [(&'static str, Vec<String>); 3] {
    let builtins: Vec<String> = builtin
        .iter()
        .filter(|b| globals.contains(b))
        .cloned()
        .collect();
    let mut general: Vec<String> = globals
        .iter()
        .filter(|g| !builtin.contains(g))
        .cloned()
        .collect();
    general.sort();
    let mut custom: Vec<String> = overlays
        .iter()
        .filter(|o| !globals.contains(o))
        .cloned()
        .collect();
    custom.sort();
    [("Builtin", builtins), ("General", general), ("Custom", custom)]
}

/// `roster` with `id` taken out, or added at the end.
pub fn toggled(roster: &[String], id: &str) -> Vec<String> {
    if roster.iter().any(|r| r == id) {
        roster.iter().filter(|r| *r != id).cloned().collect()
    } else {
        roster.iter().cloned().chain([id.to_string()]).collect()
    }
}

/// A custom role with no specialty can become a general role.
pub fn can_make_general(e: &RosterEntry) -> bool {
    e.source == Some(RoleSource::Custom) && !e.id.contains('.')
}

/// The line of a write: what it did, and what became of its commit.
fn done_line(what: &str, commit: Option<CommitInfo>) -> String {
    let c = commit.unwrap_or_default();
    match (c.committed, c.error) {
        (_, Some(e)) => format!(
            "{what}. The commit was skipped: {}",
            e.lines().next().unwrap_or_default()
        ),
        (true, None) => format!("{what} and committed."),
        (false, None) => format!("{what}. No new commit."),
    }
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
            definition: None,
            general: Load::Loading,
            want_form: None,
            form: None,
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
        let general = link.call::<GeneralList>(&GeneralListParams {});
        cx.spawn(async move |this, cx| {
            let general = general.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.general.settle(general);
                cx.notify();
            });
        })
        .detach();
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
    fn plan_reset(&mut self, agent: Option<String>, memory: bool, cx: &mut Context<Self>) {
        let params = RolesResetParams {
            product: self.product.clone(),
            agent,
            memory,
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

    /// Reads the resolved definition of `agent` and shows it in a panel. A second click hides it.
    pub fn show_definition(&mut self, agent: String, cx: &mut Context<Self>) {
        if self.definition.as_ref().is_some_and(|(a, _)| *a == agent) {
            self.definition = None;
            cx.notify();
            return;
        }
        let call = self.core.read(cx).link.call::<RolesList>(&RolesListParams {
            product: self.product.clone(),
            agent: Some(agent.clone()),
        });
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string()).and_then(|r| {
                r.agent
                    .ok_or_else(|| "The core sent no definition.".to_string())
            });
            let _ = this.update(cx, |this, cx| {
                this.definition = Some((agent, result));
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
        match pending {
            Pending::Reset(mut p, _) => {
                p.yes = true;
                let call = self.core.read(cx).link.call::<RolesReset>(&p);
                let done = cx.background_spawn(async move {
                    call.await.map_err(|e| e.to_string()).and_then(|r| {
                        if r.applied {
                            Ok(done_line("Reset", r.commit))
                        } else {
                            Err("The data changed since the plan. Nothing was written.".to_string())
                        }
                    })
                });
                cx.spawn(async move |this, cx| {
                    let result = done.await;
                    let _ = this.update(cx, |this, cx| this.written(result, false, cx));
                })
                .detach();
            }
            Pending::DeleteGeneral(id) => self.send::<GeneralDelete>(
                &GeneralDeleteParams { id: id.clone() },
                move |r| done_line(&format!("General role {id} deleted"), Some(r.commit)),
                false,
                cx,
            ),
            Pending::Promote(agent) => self.send::<GeneralPromote>(
                &GeneralPromoteParams {
                    product: self.product.clone(),
                    agent,
                },
                |r| done_line(&format!("{} is a general role", r.role.id), Some(r.commit)),
                false,
                cx,
            ),
        }
    }

    /// Sends one write. Its line or its error shows on the screen, which then reads again. With
    /// `close`, a write that the core accepted closes the form.
    fn send<M: Method>(
        &mut self,
        params: &M::Params,
        done: impl FnOnce(M::Result) -> String + 'static,
        close: bool,
        cx: &mut Context<Self>,
    ) {
        let call = self.core.read(cx).link.call::<M>(params);
        cx.spawn(async move |this, cx| {
            let result = call.await.map(done).map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.written(result, close, cx));
        })
        .detach();
    }

    fn written(&mut self, result: Result<String, String>, close: bool, cx: &mut Context<Self>) {
        if close && result.is_ok() {
            self.close_form(cx);
        }
        self.message = Some(result);
        self.validation = None;
        self.reload(cx);
        cx.notify();
    }

    /// Sets the roster of the product to `roles`.
    fn set_roster(&mut self, roles: Vec<String>, cx: &mut Context<Self>) {
        let product = self.product.clone();
        self.send::<RolesRoster>(
            &RolesRosterParams { product, roles },
            |r| done_line(&format!("Roster: {}", r.roster.join(", ")), Some(r.commit)),
            false,
            cx,
        );
    }

    /// Opens the form of a new general role. The palette entry "Roles: New general role" runs it.
    pub fn new_general(&mut self, cx: &mut Context<Self>) {
        self.open_form(GeneralForm::New, cx);
    }

    fn open_form(&mut self, which: GeneralForm, cx: &mut Context<Self>) {
        if self.form.as_ref().is_some_and(|(_, w, _)| *w == which) {
            return;
        }
        self.form = None;
        self.want_form = Some(which);
        cx.notify();
    }

    fn close_form(&mut self, cx: &mut Context<Self>) {
        self.want_form = None;
        self.form = None;
        cx.notify();
    }

    fn ensure_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(which) = self.want_form.take() else {
            return;
        };
        let form = cx.new(|cx| match &which {
            GeneralForm::New => Form::new("roles-general", "New general role", "Create", cx)
                .field("Id", "designer", true, window, cx)
                .field("Name", "Designer", false, window, cx)
                .field("Description", "What the role looks at, in one line", false, window, cx)
                .area("Definition", "You are the designer of the agency…", true, window, cx),
            GeneralForm::Edit(role) => {
                let mut f = Form::new("roles-general", "Edit the general role", "Save", cx)
                    .field("Name", "Designer", false, window, cx)
                    .field("Description", "What the role looks at, in one line", false, window, cx)
                    .area("Definition", "You are the designer of the agency…", true, window, cx);
                f.set_value(0, &role.name, window, cx);
                f.set_value(1, &role.description, window, cx);
                f.set_area_value(0, &role.body, window, cx);
                f
            }
        });
        let sub = cx.subscribe(&form, |this, form, e: &FormEvent, cx| match e {
            FormEvent::Submit => this.submit_form(&form, cx),
            FormEvent::Cancel => this.close_form(cx),
        });
        form.update(cx, |f, cx| f.focus_first(window, cx));
        self.form = Some((form, which, sub));
    }

    fn submit_form(&mut self, form: &Entity<Form>, cx: &mut Context<Self>) {
        let Some((_, which, _)) = &self.form else {
            return;
        };
        let f = form.read(cx);
        match which.clone() {
            GeneralForm::New => {
                let role = GeneralRole {
                    id: f.value(0, cx),
                    name: f.value(1, cx),
                    description: f.value(2, cx),
                    body: f.area_value(0, cx),
                };
                self.send::<GeneralCreate>(
                    &role,
                    |r| done_line(&format!("General role {} created", r.role.id), Some(r.commit)),
                    true,
                    cx,
                );
            }
            GeneralForm::Edit(before) => {
                let role = GeneralRole {
                    id: before.id,
                    name: f.value(0, cx),
                    description: f.value(1, cx),
                    body: f.area_value(0, cx),
                };
                self.send::<GeneralUpdate>(
                    &role,
                    |r| done_line(&format!("General role {} saved", r.role.id), Some(r.commit)),
                    true,
                    cx,
                );
            }
        }
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
        let definition_of = e.id.clone();
        let promote = can_make_general(e).then(|| e.id.clone());
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
                div().flex().justify_end().gap(space(2)).child(
                    Button::new(("roles-definition", i))
                        .ghost()
                        .small()
                        .icon(IconName::FileText)
                        .label("Definition")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.show_definition(definition_of.clone(), cx);
                        })),
                ).children(promote.map(|agent| {
                    Button::new(("roles-make-general", i))
                        .ghost()
                        .small()
                        .icon(IconName::Globe)
                        .label("Make general")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.pending = Some(Pending::Promote(agent.clone()));
                            cx.notify();
                        }))
                })).child(
                    Button::new(("roles-reset", i))
                        .ghost()
                        .small()
                        .icon(IconName::RotateCw)
                        .label("Reset to the global role")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.plan_reset(Some(id.clone()), false, cx);
                        })),
                ),
            )
    }

    fn general_card(&self, i: usize, g: &GeneralEntry, cx: &mut Context<Self>) -> Card {
        let role = g.role.clone();
        let id = g.role.id.clone();
        let used = if g.used_by.is_empty() {
            "No roster uses it.".to_string()
        } else {
            format!("Used by {}.", g.used_by.join(", "))
        };
        Card::new(("roles-general", i))
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
                            .child(g.role.name.clone()),
                    )
                    .child(mono(g.role.id.clone()))
                    .child(neutral_tag("general")),
            )
            .when(!g.role.description.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(size::sm())
                        .text_color(text::secondary())
                        .child(g.role.description.clone()),
                )
            })
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::tertiary())
                    .child(used),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(space(2))
                    .child(
                        Button::new(("roles-general-edit", i))
                            .ghost()
                            .small()
                            .icon(IconName::FileText)
                            .label("Edit")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_form(GeneralForm::Edit(role.clone()), cx)
                            })),
                    )
                    .child(
                        Button::new(("roles-general-delete", i))
                            .ghost()
                            .small()
                            .icon(IconName::Delete)
                            .label("Delete")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.pending = Some(Pending::DeleteGeneral(id.clone()));
                                cx.notify();
                            })),
                    ),
            )
    }

    fn general_block(&self, cx: &mut Context<Self>) -> Div {
        let count = match &self.general {
            Load::Ready(g) => Some(g.roles.len()),
            _ => None,
        };
        let head = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(space(2))
            .child(section("General roles", count))
            .child(
                Button::new("roles-general-new")
                    .secondary()
                    .small()
                    .icon(IconName::Plus)
                    .label("New general role")
                    .on_click(cx.listener(|this, _, _, cx| this.new_general(cx))),
            );
        let quiet = |t: String| {
            div()
                .text_size(size::sm())
                .text_color(text::tertiary())
                .child(t)
        };
        let body: Vec<AnyElement> = match &self.general {
            Load::Loading => vec![quiet("Reading the general roles…".into()).into_any_element()],
            Load::Failed(e) => {
                vec![quiet(format!("The general roles did not load: {e}")).into_any_element()]
            }
            Load::Ready(g) if g.roles.is_empty() => vec![
                quiet(
                    "No general roles. A general role is made once, and each product can add it."
                        .into(),
                )
                .into_any_element(),
            ],
            Load::Ready(g) => g
                .roles
                .iter()
                .enumerate()
                .map(|(i, e)| self.general_card(i, e, cx).into_any_element())
                .collect(),
        };
        div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(head)
            .children(self.form.as_ref().map(|(f, _, _)| f.clone()))
            .children(body)
    }

    /// The roster picker: a box for each role that the product can list.
    fn picker_block(&self, r: &RolesListResult, cx: &mut Context<Self>) -> Option<Div> {
        let Load::Ready(general) = &self.general else {
            return None;
        };
        let roster: Vec<String> = r
            .roster
            .iter()
            .flatten()
            .map(|e| e.id.clone())
            .collect();
        let mut n = 0usize;
        let groups: Vec<Div> = picker_groups(&r.globals, &general.builtin, &r.overlays)
            .into_iter()
            .filter(|(_, ids)| !ids.is_empty())
            .map(|(title, ids)| {
                let boxes: Vec<Checkbox> = ids
                    .into_iter()
                    .map(|id| {
                        n += 1;
                        let next = toggled(&roster, &id);
                        Checkbox::new(("roles-pick", n))
                            .label(SharedString::from(id.clone()))
                            .checked(roster.contains(&id))
                            .on_click(cx.listener(move |this, _: &bool, _, cx| {
                                this.set_roster(next.clone(), cx)
                            }))
                    })
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .gap(space(2))
                    .child(
                        div()
                            .text_size(size::xs())
                            .font_weight(weight::MEDIUM)
                            .text_color(text::tertiary())
                            .child(title),
                    )
                    .child(div().flex().flex_wrap().gap(space(3)).children(boxes))
            })
            .collect();
        Some(
            div()
                .flex()
                .flex_col()
                .gap(space(3))
                .child(section("Roster picker", None))
                .children(groups),
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
            Pending::DeleteGeneral(id) => (
                format!("Delete the general role {id}"),
                vec![format!("Delete plugin/agents/{id}.md")],
            ),
            Pending::Promote(agent) => (
                format!("Make {agent} a general role"),
                vec![
                    format!("Move agents/{agent}.md of {} to plugin/agents/{agent}.md", self.product),
                    format!("The roster of {} keeps {agent}, and each product can add it.", self.product),
                ],
            ),
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
                .children(match pending {
                    Pending::Reset(p, _) => {
                        let (agent, memory) = (p.agent.clone(), p.memory);
                        Some(
                            div().child(
                                Checkbox::new("roles-reset-memory")
                                    .label("Also delete the memory")
                                    .checked(memory)
                                    .on_click(cx.listener(move |this, on: &bool, _, cx| {
                                        this.plan_reset(agent.clone(), *on, cx)
                                    })),
                            ),
                        )
                    }
                    Pending::DeleteGeneral(_) | Pending::Promote(_) => None,
                })
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

    fn definition_block(&self) -> Option<Stateful<Div>> {
        let (agent, def) = self.definition.as_ref()?;
        let body = match def {
            Ok(d) => {
                let mut lines = vec![format!(
                    "{} · {:?} · {}{}",
                    d.role,
                    d.source,
                    d.model,
                    d.effort
                        .as_ref()
                        .map_or(String::new(), |e| format!(" · {e}"))
                )];
                if let Some(t) = &d.tools {
                    lines.push(format!("Tools: {}", t.join(", ")));
                }
                lines.extend(d.body.lines().map(String::from));
                lines
            }
            Err(e) => vec![format!("The definition did not load: {e}")],
        };
        Some(
            div()
                .id("roles-definition-panel")
                .debug_selector(|| "roles-definition-panel".into())
                .flex()
                .flex_col()
                .gap(space(1))
                .max_h(space(80))
                .overflow_y_scroll()
                .p(space(3))
                .rounded(radius::lg())
                .border_1()
                .border_color(line::border())
                .bg(surface::raised())
                .child(section(
                    &format!("Definition of {}", role_label(agent)),
                    None,
                ))
                .children(body.into_iter().map(mono)),
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_form(window, cx);
        let root = div().id("roles-screen").size_full().bg(surface::panel());
        let waiting = match &self.data {
            Load::Loading => Some("Reading the roles…".to_string()),
            Load::Failed(e) => Some(format!("The roles did not load: {e}")),
            Load::Ready(_) => None,
        };
        if let Some(line) = waiting {
            // The general roles belong to no product: their form stays open without a roster.
            if self.form.is_some() {
                return root
                    .track_scroll(&self.scroll)
                    .overflow_y_scroll()
                    .child(column().child(mono(line)).child(self.general_block(cx)));
            }
            let hint = matches!(self.data, Load::Failed(_)).then(|| super::parts::reload_hint(cx));
            return root.child(notice(line, hint));
        }
        let Load::Ready(r) = &self.data else {
            return root;
        };
        let r = r.clone();
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
                        Button::new("roles-update-brief")
                            .secondary()
                            .small()
                            .label("Update the brief")
                            .on_click(cx.listener(|this, _, _, cx| {
                                Nav::open(cx, &this.core, ScreenSpec::setup(&this.product))
                            })),
                    )
                    .child(
                        Button::new("roles-reset-all")
                            .danger()
                            .small()
                            .label("Reset all")
                            .on_click(
                                cx.listener(|this, _, _, cx| this.plan_reset(None, false, cx)),
                            ),
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
                .children(self.definition_block())
                .child(self.general_block(cx))
                .child(roles_block)
                .children(self.picker_block(&r, cx))
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
        GeneralForm, GeneralRole, Load, Pending, ResetPlan, RolesResetParams, RolesScreen,
        RolesValidateResult, can_make_general, picker_groups, reset_lines, toggled,
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
          {"id": "seo", "role": "seo", "source": "custom", "model": "sonnet", "effort": null, "error": null},
          {"id": "ghost", "role": null, "source": null, "model": null, "effort": null, "error": "Unknown role \"ghost\""}],
        "overlays": ["dev.api", "old", "seo"],
        "globals": ["pm", "dev", "designer"],
        "warnings": []
    }"#;

    const GENERAL: &str = r#"{
        "roles": [
          {"id": "designer", "name": "Designer", "description": "Reads the screens.", "body": "You are the designer.", "used_by": ["demo"]},
          {"id": "writer", "name": "Writer", "description": "", "body": "You write.", "used_by": []}],
        "builtin": ["pm", "social", "dev", "qa", "scout"]
    }"#;

    fn designer() -> GeneralRole {
        GeneralRole {
            id: "designer".into(),
            name: "Designer".into(),
            description: "Reads the screens.".into(),
            body: "You are the designer.".into(),
        }
    }

    fn screen(cx: &mut TestAppContext, change: fn(&mut RolesScreen)) {
        cx.update(agentsi_ui::init);
        let (end, _other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| RolesScreen::new("demo".into(), core, cx));
        screen.update(cx, |s, cx| {
            s.data = Load::Ready(serde_json::from_str(LIST).unwrap());
            s.general = Load::Ready(serde_json::from_str(GENERAL).unwrap());
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

    fn request(other: &mut std::os::unix::net::UnixStream) -> Option<serde_json::Value> {
        other.set_nonblocking(true).unwrap();
        let f = agentsi_proto::frame::read_frame(other).ok().flatten()?;
        serde_json::from_slice(&f.payload).ok()
    }

    #[gpui::test]
    fn a_definition_click_asks_for_the_agent_and_the_memory_box_plans_again(
        cx: &mut TestAppContext,
    ) {
        cx.update(agentsi_ui::init);
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| RolesScreen::new("demo".into(), core, cx));
        while request(&mut other).is_some() {}
        screen.update(cx, |s, cx| s.show_definition("dev.api".into(), cx));
        let r = request(&mut other).expect("roles.list");
        assert_eq!(r["method"], "roles.list");
        assert_eq!(
            r["params"],
            serde_json::json!({"product": "demo", "agent": "dev.api"})
        );

        screen.update(cx, |s, cx| s.plan_reset(Some("dev.api".into()), true, cx));
        let r = request(&mut other).expect("roles.reset");
        assert_eq!(r["method"], "roles.reset");
        assert_eq!(r["params"]["memory"], true);
        assert_eq!(r["params"]["yes"], false);
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_picker_lists_builtin_then_general_then_custom_roles() {
        let groups = picker_groups(
            &ids(&["pm", "dev", "writer", "designer"]),
            &ids(&["pm", "social", "dev", "qa", "scout"]),
            &ids(&["seo", "dev", "dev.api"]),
        );
        assert_eq!(groups[0], ("Builtin", ids(&["pm", "dev"])));
        assert_eq!(groups[1], ("General", ids(&["designer", "writer"])));
        assert_eq!(groups[2], ("Custom", ids(&["dev.api", "seo"])));
        assert_eq!(toggled(&ids(&["pm", "dev"]), "dev"), ids(&["pm"]));
        assert_eq!(toggled(&ids(&["pm"]), "designer"), ids(&["pm", "designer"]));
    }

    #[test]
    fn only_a_custom_role_with_no_specialty_can_be_made_general() {
        let list: super::RolesListResult = serde_json::from_str(LIST).unwrap();
        let can: Vec<&str> = list
            .roster
            .iter()
            .flatten()
            .filter(|e| can_make_general(e))
            .map(|e| e.id.as_str())
            .collect();
        assert_eq!(can, ["seo"]);
    }

    #[gpui::test]
    fn the_general_roles_the_picker_and_their_plans_draw(cx: &mut TestAppContext) {
        screen(cx, |s| s.pending = Some(Pending::DeleteGeneral("writer".into())));
        screen(cx, |s| s.pending = Some(Pending::Promote("seo".into())));
        screen(cx, |s| s.want_form = Some(GeneralForm::New));
        screen(cx, |s| s.want_form = Some(GeneralForm::Edit(designer())));
        screen(cx, |s| s.general = Load::Failed("no core".into()));
        screen(cx, |s| s.general = Load::Loading);
        screen(cx, |s| {
            s.general = Load::Ready(serde_json::from_value(serde_json::json!({"roles": [], "builtin": []})).unwrap())
        });
    }

    fn opened(
        cx: &mut TestAppContext,
    ) -> (
        gpui::Entity<RolesScreen>,
        &mut gpui::VisualTestContext,
        std::os::unix::net::UnixStream,
    ) {
        cx.update(agentsi_ui::init);
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| RolesScreen::new("demo".into(), core, cx));
        let mut reads = Vec::new();
        while let Some(r) = request(&mut other) {
            reads.push(r["method"].as_str().unwrap_or_default().to_string());
        }
        assert!(reads.contains(&"role.general.list".to_string()), "{reads:?}");
        screen.update(cx, |s, cx| {
            s.data = Load::Ready(serde_json::from_str(LIST).unwrap());
            s.general = Load::Ready(serde_json::from_str(GENERAL).unwrap());
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (screen, cx, other)
    }

    #[gpui::test]
    fn the_new_general_role_form_sends_role_general_create(cx: &mut TestAppContext) {
        let (screen, cx, mut other) = opened(cx);
        screen.update(cx, |s, cx| s.new_general(cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let form = screen.read_with(cx, |s, _| s.form.as_ref().map(|(f, _, _)| f.clone()));
        let form = form.expect("the next draw makes the form");
        assert!(!form.read_with(cx, |f, cx| f.ready(cx)), "the id and the body are required");
        form.update_in(cx, |f, window, cx| {
            f.set_value(0, "designer", window, cx);
            f.set_value(1, "Designer", window, cx);
            f.set_value(2, "Reads the screens.", window, cx);
            f.set_area_value(0, "You are the designer.\nYou read the screens.", window, cx);
        });
        form.update(cx, |f, cx| f.submit(cx));
        let r = request(&mut other).expect("role.general.create");
        assert_eq!(r["method"], "role.general.create");
        assert_eq!(
            r["params"],
            serde_json::json!({
                "id": "designer", "name": "Designer", "description": "Reads the screens.",
                "body": "You are the designer.\nYou read the screens."
            })
        );
    }

    #[gpui::test]
    fn the_edit_form_starts_with_the_role_and_sends_role_general_update(cx: &mut TestAppContext) {
        let (screen, cx, mut other) = opened(cx);
        screen.update(cx, |s, cx| s.open_form(GeneralForm::Edit(designer()), cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let form = screen.read_with(cx, |s, _| s.form.as_ref().map(|(f, _, _)| f.clone())).unwrap();
        assert_eq!(form.read_with(cx, |f, cx| f.area_value(0, cx)), "You are the designer.");
        form.update_in(cx, |f, window, cx| f.set_value(0, "UX", window, cx));
        form.update(cx, |f, cx| f.submit(cx));
        let r = request(&mut other).expect("role.general.update");
        assert_eq!(r["method"], "role.general.update");
        assert_eq!(r["params"]["id"], "designer");
        assert_eq!(r["params"]["name"], "UX");
        assert_eq!(r["params"]["body"], "You are the designer.");
    }

    #[gpui::test]
    fn the_picker_make_general_and_delete_send_their_writes(cx: &mut TestAppContext) {
        let (screen, cx, mut other) = opened(cx);
        screen.update(cx, |s, cx| s.set_roster(toggled(&ids(&["pm", "seo"]), "designer"), cx));
        let r = request(&mut other).expect("roles.roster");
        assert_eq!(r["method"], "roles.roster");
        assert_eq!(
            r["params"],
            serde_json::json!({"product": "demo", "roles": ["pm", "seo", "designer"]})
        );

        screen.update(cx, |s, cx| {
            s.pending = Some(Pending::Promote("seo".into()));
            s.confirm(cx);
        });
        let r = request(&mut other).expect("role.general.promote");
        assert_eq!(r["method"], "role.general.promote");
        assert_eq!(r["params"], serde_json::json!({"product": "demo", "agent": "seo"}));

        screen.update(cx, |s, cx| {
            s.pending = Some(Pending::DeleteGeneral("writer".into()));
            s.confirm(cx);
        });
        let r = request(&mut other).expect("role.general.delete");
        assert_eq!(r["method"], "role.general.delete");
        assert_eq!(r["params"], serde_json::json!({"id": "writer"}));
    }
}
