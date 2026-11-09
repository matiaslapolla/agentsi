//! The memory of a product (U25): the items of each agent with their helpful and
//! harmful counts, and the reflections with their changes and their proposals.
//!
//! Retire and Revert ask first: the screen shows what will change, and Confirm writes
//! it. A pending proposal has Approve and Reject, which call `decision.decide`. There
//! is no text field, so a retired item gets a fixed reason. The screen reads again
//! after each write.

use agentsi_proto::marks_api::{DecideParams, DecideResult, DecisionDecide, Outcome, Verdict};
use agentsi_proto::reflect_api::{
    AgentMemory, MemoryChange, MemoryEntry, MemoryList, MemoryListParams, MemoryListResult,
    MemoryRetire, MemoryRetireParams, Proposal, ReflectCancel, ReflectCancelParams, ReflectRevert,
    ReflectRevertParams, ReflectRun, ReflectRunParams, ReflectionEntry, Signal,
};
use agentsi_ui::components::{Button, Card, Force, neutral_tag, status_tag};
use agentsi_ui::tokens::{Status, line, radius, size, space, surface, text, weight};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::minutes::cursor_after;
use super::parts::{
    Flash, Load, column, first_screen_ready, flash_line, mono, notice, role_label, section,
};
use crate::format::date_of;
use crate::model::CoreState;

/// The reason of an item that the owner retires here.
const RETIRED_HERE: &str = "Retired by the owner in the app.";

/// A write that waits for the owner to confirm it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Pending {
    Retire { agent: String, id: String },
    Revert(String),
    Reflect,
}

pub struct MemoryScreen {
    product: String,
    core: Entity<CoreState>,
    data: Load<MemoryListResult>,
    scroll: ScrollHandle,
    /// The active item under the cursor, over all the agents.
    selected: usize,
    focus: FocusHandle,
    /// False while the dock hides this tab.
    shown: bool,
    pending: Option<Pending>,
    /// A write waits for the core. The screen starts no second one.
    busy: bool,
    flash: Option<Flash>,
    /// The box "Memory only" of Reflect.
    memory_only: bool,
}

fn active_of(a: &AgentMemory) -> impl Iterator<Item = &MemoryEntry> {
    a.items.iter().filter(|i| i.retired_at.is_none())
}

impl MemoryScreen {
    pub fn new(product: String, core: Entity<CoreState>, cx: &mut Context<Self>) -> Self {
        let mut screen = MemoryScreen {
            product,
            core,
            data: Load::Loading,
            scroll: ScrollHandle::new(),
            selected: 0,
            focus: cx.focus_handle().tab_stop(true),
            shown: true,
            pending: None,
            busy: false,
            flash: None,
            memory_only: false,
        };
        screen.reload(cx);
        screen
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub fn show(&mut self, shown: bool) {
        self.shown = shown;
    }

    /// The active items in the order of the screen, with their agents.
    fn rows(&self) -> Vec<(String, String)> {
        let Load::Ready(d) = &self.data else {
            return Vec::new();
        };
        d.agents
            .iter()
            .flat_map(|a| active_of(a).map(|i| (a.agent.clone(), i.id.clone())))
            .collect()
    }

    /// Reads the memory again. The screen keeps what it shows until the reply comes.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<MemoryList>(&MemoryListParams {
                product: self.product.clone(),
            });
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.data.settle(result);
                this.selected = this.selected.min(this.rows().len().saturating_sub(1));
                first_screen_ready();
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks to retire the item under the cursor. A hidden tab does nothing: a palette
    /// action must not reach a screen that the owner does not see.
    pub fn retire_selected(&mut self, cx: &mut Context<Self>) {
        if !self.shown {
            return;
        }
        if let Some((agent, id)) = self.rows().get(self.selected).cloned() {
            self.pending = Some(Pending::Retire { agent, id });
            cx.notify();
        }
    }

    fn ask_revert(&mut self, id: String, cx: &mut Context<Self>) {
        self.pending = Some(Pending::Revert(id));
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        cx.notify();
    }

    /// "Reflect": shows the Confirm, because each agent calls the model.
    pub fn ask_reflect(&mut self, cx: &mut Context<Self>) {
        if !self.reflecting(cx) {
            self.pending = Some(Pending::Reflect);
            cx.notify();
        }
    }

    fn reflecting(&self, cx: &App) -> bool {
        self.core.read(cx).reflecting.contains(&self.product)
    }

    fn run_reflect(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let product = self.product.clone();
        core.update(cx, |s, cx| {
            if s.set_reflecting(&product, true) {
                cx.notify();
            }
        });
        let call = core.read(cx).link.call::<ReflectRun>(&ReflectRunParams {
            product: product.clone(),
            memory_only: self.memory_only,
        });
        self.flash = None;
        cx.spawn(async move |this, cx| {
            let result = call.await;
            core.update(cx, |s, cx| {
                if s.set_reflecting(&product, false) {
                    cx.notify();
                }
            });
            let _ = this.update(cx, |this, cx| {
                this.flash = Some(match result {
                    Ok(r) if r.entry.is_some() => Flash::Done("Reflected.".into()),
                    Ok(_) => Flash::Done("Nothing to reflect on.".into()),
                    Err(e) => Flash::Failed(e.to_string()),
                });
                this.reload(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Stops the reflection of the product that runs.
    pub fn cancel_reflect(&mut self, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<ReflectCancel>(&ReflectCancelParams {
                product: self.product.clone(),
            });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| {
                this.flash = Some(match result {
                    Ok(r) if r.canceled => Flash::Done("The reflection stops.".into()),
                    Ok(_) => Flash::Done("No reflection runs.".into()),
                    Err(e) => Flash::Failed(e.to_string()),
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Sends one write, shows its outcome and reads the screen again.
    fn write<R: 'static>(
        &mut self,
        call: impl std::future::Future<Output = Result<R, crate::link::CallError>> + 'static,
        outcome: impl FnOnce(R) -> Flash + 'static,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        self.flash = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| {
                let ok = result.is_ok();
                this.flash = Some(match result {
                    Ok(reply) => outcome(reply),
                    Err(e) => Flash::Failed(e.to_string()),
                });
                // After a write that the core took, the screen stays busy until it has the new data.
                this.busy = ok;
                this.reload(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        // The call sends its request when it is made, so the check comes before it.
        if self.busy {
            return;
        }
        let Some(pending) = self.pending.take() else {
            return;
        };
        let link = self.core.read(cx).link.clone();
        match pending {
            Pending::Reflect => self.run_reflect(cx),
            Pending::Retire { agent, id } => {
                let done = format!("Item {id} retired.");
                let call = link.call::<MemoryRetire>(&MemoryRetireParams {
                    product: self.product.clone(),
                    agent,
                    id,
                    why: RETIRED_HERE.into(),
                });
                self.write(call, |_| Flash::Done(done), cx);
            }
            Pending::Revert(reflection) => {
                let done = format!("Reflection {reflection} reverted.");
                let call = link.call::<ReflectRevert>(&ReflectRevertParams {
                    product: self.product.clone(),
                    reflection,
                });
                self.write(call, |_| Flash::Done(done), cx);
            }
        }
    }

    fn decide(&mut self, decision: i64, verdict: Verdict, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let call = self
            .core
            .read(cx)
            .link
            .call::<DecisionDecide>(&DecideParams {
                decision,
                status: verdict,
            });
        self.write(call, move |r| decision_flash(decision, &r), cx);
    }

    /// Asks to revert the newest reflection that can be reverted. A hidden tab does nothing.
    pub fn revert_latest(&mut self, cx: &mut Context<Self>) {
        if !self.shown {
            return;
        }
        let Load::Ready(d) = &self.data else {
            return;
        };
        if let Some(e) = d.reflections.iter().find(|e| e.can_revert()) {
            let id = e.id.clone();
            self.ask_revert(id, cx);
        }
    }

    /// Approves or rejects the newest pending proposal. A hidden tab does nothing.
    pub fn decide_pending(&mut self, verdict: Verdict, cx: &mut Context<Self>) {
        if !self.shown {
            return;
        }
        let Load::Ready(d) = &self.data else {
            return;
        };
        let pending = d.reflections.iter().filter_map(|e| e.proposal.as_ref());
        let id = pending
            .filter(|p| p.status == "pending")
            .find_map(|p| p.decision_id);
        if let Some(id) = id {
            self.decide(id, verdict, cx);
        }
    }

    /// What the owner confirms: the text of an item, or what a revert changes.
    fn pending_lines(&self) -> Vec<String> {
        let (Load::Ready(d), Some(pending)) = (&self.data, &self.pending) else {
            return Vec::new();
        };
        match pending {
            Pending::Retire { id, .. } => d
                .agents
                .iter()
                .flat_map(|a| &a.items)
                .filter(|i| i.id == *id)
                .map(|i| i.text.clone())
                .collect(),
            Pending::Reflect => Vec::new(),
            Pending::Revert(id) => d
                .reflections
                .iter()
                .filter(|e| e.id == *id)
                .flat_map(|e| &e.memory)
                .map(revert_line)
                .collect(),
        }
    }

    fn key_down(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.modified() {
            return;
        }
        if let Some(to) = cursor_after(&k.key, self.selected, self.rows().len()) {
            self.selected = to;
        } else {
            match k.key.as_str() {
                "x" => self.retire_selected(cx),
                "enter" if self.pending.is_some() => self.confirm(cx),
                "escape" if self.pending.is_some() => self.cancel(cx),
                _ => return,
            }
        }
        cx.stop_propagation();
        cx.notify();
    }
}

/// The line of a write on a decision. A proposal whose target changed applies nothing.
pub fn decision_flash(id: i64, reply: &DecideResult) -> Flash {
    if let Some(reason) = &reply.refused {
        return Flash::Failed(reason.clone());
    }
    if let Some(error) = &reply.commit_error {
        return Flash::Failed(format!(
            "Decision {id} approved. The commit was skipped: {error}"
        ));
    }
    match reply.status {
        Outcome::Approved => Flash::Done(format!("Decision {id} approved.")),
        Outcome::Rejected => Flash::Done(format!("Decision {id} rejected.")),
        Outcome::Stale => Flash::Failed(format!(
            "Decision {id} not applied: the target changed since the proposal, so it is now stale."
        )),
    }
}

/// What a revert changes in the memory of one agent.
fn revert_line(m: &MemoryChange) -> String {
    let mut parts = Vec::new();
    if !m.added.is_empty() {
        parts.push(format!("retires {}", m.added.join(", ")));
    }
    if !m.retired.is_empty() {
        parts.push(format!("restores {}", m.retired.join(", ")));
    }
    let counts = m.helpful.len() + m.harmful.len();
    if counts > 0 {
        parts.push(format!("undoes {counts} counts"));
    }
    format!("{}: {}", role_label(&m.agent), parts.join(", "))
}

fn small(s: impl Into<SharedString>) -> Div {
    div()
        .text_size(size::sm())
        .text_color(text::secondary())
        .child(s.into())
}

fn item_tags(i: &MemoryEntry) -> Div {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(space(2))
        .child(mono(date_of(&i.created_at)))
        .when(i.kind == "lesson", |el| {
            el.child(status_tag("lesson", Status::Info))
        })
        .when(i.helpful > 0, |el| {
            el.child(status_tag(format!("helpful ×{}", i.helpful), Status::Ok))
        })
        .when(i.harmful > 0, |el| {
            el.child(status_tag(
                format!("harmful ×{}", i.harmful),
                Status::Danger,
            ))
        })
        .child(mono(i.source.clone()))
}

fn item_text(i: &MemoryEntry) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(space(1))
        .child(
            div().flex().gap(space(2)).child(mono(i.id.clone())).child(
                div()
                    .text_size(size::base())
                    .text_color(text::primary())
                    .child(i.text.clone()),
            ),
        )
        .child(item_tags(i))
}

fn retired_row(i: &MemoryEntry) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space(1))
        .child(item_text(i))
        .when_some(i.retired_why.clone(), |el, why| {
            el.child(small(format!("Retired: {why}")))
        })
}

fn code_block(body: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .p(space(3))
        .rounded(radius::md())
        .border_1()
        .border_color(line::border())
        .bg(surface::raised())
        .children(body.trim_end().lines().map(|l| mono(l.to_string())))
}

fn change_line(m: &MemoryChange) -> String {
    let mut parts = vec![
        format!("{} new", m.added.len()),
        format!("{} retired", m.retired.len()),
        format!("{} helpful", m.helpful.len()),
        format!("{} harmful", m.harmful.len()),
    ];
    if let Some(restored) = m.restored.as_ref().filter(|r| !r.is_empty()) {
        parts.push(format!("{} restored", restored.len()));
    }
    format!("{}: {}", role_label(&m.agent), parts.join(", "))
}

fn signal_line(s: &Signal) -> String {
    format!(
        "{} · {} · {} · {}",
        role_label(&s.agent),
        s.kind,
        s.detail,
        s.reference
    )
}

fn proposal_tag(status: &str) -> impl IntoElement {
    let kind = match status {
        "pending" => Status::Warn,
        "approved" => Status::Ok,
        "rejected" | "discarded" | "stale" => Status::Danger,
        _ => Status::Info,
    };
    status_tag(status.to_string(), kind)
}

impl MemoryScreen {
    fn agent_block(
        &self,
        a: &AgentMemory,
        first: usize,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let active: Vec<&MemoryEntry> = active_of(a).collect();
        let retired: Vec<&MemoryEntry> =
            a.items.iter().filter(|i| i.retired_at.is_some()).collect();
        let rows: Vec<Card> = active
            .iter()
            .enumerate()
            .map(|(n, i)| {
                let row = first + n;
                let selected = row == self.selected;
                let (agent, id) = (a.agent.clone(), i.id.clone());
                Card::new(("memory-item", row))
                    // The accent border marks the item under the cursor.
                    .force(if selected { Force::Hover } else { Force::None })
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .justify_between()
                            .gap(space(3))
                            .child(item_text(i))
                            .child(
                                Button::new(("memory-retire", row))
                                    .ghost()
                                    .small()
                                    .label("Retire")
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.selected = row;
                                        this.pending = Some(Pending::Retire {
                                            agent: agent.clone(),
                                            id: id.clone(),
                                        });
                                        cx.notify();
                                    })),
                            ),
                    )
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(section(&role_label(&a.agent), Some(active.len())))
            .when(active.is_empty() && retired.is_empty(), |el| {
                el.child(small("This agent has no memory yet."))
            })
            .children(rows)
            .when(!retired.is_empty(), |el| {
                el.child(small(format!("Retired ({})", retired.len())))
                    .children(retired.iter().map(|i| retired_row(i)))
            })
    }

    fn proposal_block(&self, i: usize, p: &Proposal, busy: bool, cx: &mut Context<Self>) -> Div {
        let decision = p.decision_id.filter(|_| p.status == "pending");
        div()
            .flex()
            .flex_col()
            .gap(space(2))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space(2))
                    .child(
                        div()
                            .text_size(size::base())
                            .font_weight(weight::SEMIBOLD)
                            .text_color(text::primary())
                            .child(format!("Prompt of {}", role_label(&p.agent))),
                    )
                    .child(mono(p.target.clone()))
                    .child(proposal_tag(&p.status)),
            )
            .when(!p.rationale.is_empty(), |el| {
                el.child(small(p.rationale.clone()))
            })
            .when(!p.expected_effect.is_empty(), |el| {
                el.child(small(format!("Expected: {}", p.expected_effect)))
            })
            .when(!p.diff.is_empty(), |el| el.child(code_block(&p.diff)))
            .children(p.replay.table().into_iter().map(mono))
            .when_some(decision, |el, id| {
                el.child(
                    div()
                        .flex()
                        .gap(space(2))
                        .child(
                            Button::new(("memory-approve", i))
                                .primary()
                                .small()
                                .label("Approve")
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.decide(id, Verdict::Approved, cx);
                                })),
                        )
                        .child(
                            Button::new(("memory-reject", i))
                                .secondary()
                                .small()
                                .label("Reject")
                                .disabled(busy)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.decide(id, Verdict::Rejected, cx);
                                })),
                        ),
                )
            })
    }

    fn reflection_card(
        &self,
        i: usize,
        e: &ReflectionEntry,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Card {
        let id = e.id.clone();
        let head = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(space(3))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space(2))
                    .child(mono(e.id.clone()))
                    .child(mono(date_of(&e.created_at)))
                    .when_some(e.reverts.clone(), |el, target| {
                        el.child(neutral_tag(format!("reverts {target}")))
                    })
                    .when(e.status == "reverted", |el| {
                        el.child(status_tag("reverted", Status::Warn))
                    }),
            )
            .when(e.can_revert(), |el| {
                el.child(
                    Button::new(("memory-revert", i))
                        .ghost()
                        .small()
                        .label("Revert")
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.ask_revert(id.clone(), cx);
                        })),
                )
            });
        let mut card = Card::new(("memory-reflection", i)).child(head);
        if e.reverts.is_none() {
            card = card.child(small(format!("{} signals", e.signals.len())));
            for s in &e.signals {
                card = card.child(mono(signal_line(s)));
            }
        }
        for m in &e.memory {
            card = card.child(small(change_line(m)));
            if !m.diff.is_empty() {
                card = card.child(code_block(&m.diff));
            }
        }
        if let Some(p) = &e.proposal {
            card = card.child(self.proposal_block(i, p, busy, cx));
        }
        card
    }

    fn reflect_row(&self, cx: &mut Context<Self>) -> Div {
        let running = self.reflecting(cx);
        div()
            .flex()
            .items_center()
            .gap(space(3))
            .child(
                Button::new("memory-reflect")
                    .secondary()
                    .small()
                    .label("Reflect")
                    .disabled(running || self.busy)
                    .on_click(cx.listener(|this, _, _, cx| this.ask_reflect(cx))),
            )
            .child(
                Checkbox::new("memory-only")
                    .label("Memory only")
                    .checked(self.memory_only)
                    .on_click(cx.listener(|this, on: &bool, _, cx| {
                        this.memory_only = *on;
                        cx.notify();
                    })),
            )
            .when(running, |el| {
                el.child(
                    Button::new("memory-cancel-reflect")
                        .danger()
                        .small()
                        .label("Cancel the reflection")
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_reflect(cx))),
                )
            })
    }

    fn pending_card(&self, cx: &mut Context<Self>) -> Option<Div> {
        let (title, body) = match self.pending.as_ref()? {
            Pending::Retire { agent, id } => (
                format!("Retire item {id} of {}", role_label(agent)),
                "Meetings and reflections stop using this item. The screen keeps it under Retired."
                    .to_string(),
            ),
            Pending::Reflect => (
                format!("Reflect on {}", self.product),
                if self.memory_only {
                    "Each agent calls the model, which costs money. It curates the memory only and makes no prompt proposal."
                } else {
                    "Each agent calls the model, which costs money. It curates the memory and may propose a prompt change."
                }
                .to_string(),
            ),
            Pending::Revert(id) => (
                format!("Revert reflection {id}"),
                "This retires what it added and marks it harmful, restores what it retired and undoes its counts."
                    .to_string(),
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
                .children(self.pending_lines().into_iter().map(mono))
                .child(small(body))
                .child(
                    div()
                        .flex()
                        .gap(space(2))
                        .child(
                            Button::new("memory-confirm")
                                .danger()
                                .small()
                                .label("Confirm")
                                .disabled(self.busy)
                                .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                        )
                        .child(
                            Button::new("memory-cancel")
                                .secondary()
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                        ),
                ),
        )
    }
}

impl Render for MemoryScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .id("memory-screen")
            .track_focus(&self.focus)
            .key_context("Memory")
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.key_down(e, cx)))
            .size_full()
            .bg(surface::panel());
        let d = match &self.data {
            Load::Loading => return root.child(notice("Reading the memory…", None)),
            Load::Failed(e) => {
                return root.child(notice(
                    format!("The memory did not load: {e}"),
                    Some(super::parts::reload_hint(cx)),
                ));
            }
            Load::Ready(d) => d.clone(),
        };
        let busy = self.busy;
        let total: usize = d.agents.iter().map(|a| active_of(a).count()).sum();
        let header = div()
            .flex()
            .flex_col()
            .gap(space(1))
            .child(
                div()
                    .text_size(size::xl())
                    .font_weight(weight::SEMIBOLD)
                    .text_color(text::primary())
                    .child("Memory"),
            )
            .child(mono(format!(
                "{} · {} active items · {} reflections",
                self.product,
                total,
                d.reflections.len()
            )));

        let mut first = 0;
        let mut blocks = Vec::new();
        for a in &d.agents {
            blocks.push(self.agent_block(a, first, busy, cx));
            first += active_of(a).count();
        }
        let reflections: Vec<Card> = d
            .reflections
            .iter()
            .enumerate()
            .map(|(i, e)| self.reflection_card(i, e, busy, cx))
            .collect();
        let reflections_block = div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(section("Reflections", Some(reflections.len())))
            .when(reflections.is_empty(), |el| {
                el.child(small(
                    "No reflection yet. Run agentsi reflect when the audit says that evidence is ready.",
                ))
            })
            .children(reflections);

        root.track_scroll(&self.scroll).overflow_y_scroll().child(
            column()
                .child(header)
                .child(self.reflect_row(cx))
                .children(self.flash.as_ref().map(flash_line))
                .children(self.pending_card(cx))
                .children(blocks)
                .child(reflections_block),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DecideResult, Flash, Load, MemoryListResult, MemoryScreen, Outcome, Pending, Signal,
        Verdict, decision_flash, signal_line,
    };
    use crate::link::Link;
    use crate::model::CoreState;
    use gpui::TestAppContext;
    use gpui_kit::{AppContext as _, Entity};
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    const DATA: &str = r##"{
        "agents": [
            {"agent": "pm", "items": []},
            {"agent": "dev", "items": [
                {"id": "m-a2", "text": "A lesson.", "kind": "lesson", "helpful": 2, "harmful": 1,
                 "source": "r-1", "created_at": "2026-11-02T10:00:00.000Z", "retired_at": null, "retired_why": null},
                {"id": "m-a1", "text": "Old.", "kind": "fact", "helpful": 0, "harmful": 0,
                 "source": "m1", "created_at": "2026-11-01T10:00:00.000Z",
                 "retired_at": "2026-11-02T10:00:00.000Z", "retired_why": "Stale."}]},
            {"agent": "qa", "items": [
                {"id": "m-b1", "text": "A fact.", "kind": "fact", "helpful": 0, "harmful": 0,
                 "source": "m1", "created_at": "2026-11-01T10:00:00.000Z", "retired_at": null, "retired_why": null}]}
        ],
        "reflections": [
            {"id": "r-2", "product": "demo", "created_at": "2026-11-03T10:00:00.000Z", "status": "applied",
             "signals": [{"agent": "dev", "kind": "mark_rejected", "ref": "m1", "detail": "#0 A"}],
             "memory": [{"agent": "dev", "added": ["m-a2"], "retired": ["m-a1"], "helpful": [], "harmful": [],
                         "dropped": [], "diff": "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-- old\n+- new\n"}],
             "proposal": {"agent": "dev", "target": "workspace/demo/agents/dev.md", "status": "pending",
                          "decision_id": 9, "rationale": "Why.", "expected_effect": "Less.", "diff": "@@\n+x\n"}},
            {"id": "r-1", "product": "demo", "created_at": "2026-11-01T10:00:00.000Z", "reverts": "r-0",
             "status": "applied", "signals": [], "memory": [], "proposal": null}
        ]
    }"##;

    fn data() -> MemoryListResult {
        serde_json::from_str(DATA).unwrap()
    }

    fn screen(
        cx: &mut TestAppContext,
        state: Load<MemoryListResult>,
    ) -> (
        Entity<MemoryScreen>,
        UnixStream,
        &mut gpui::VisualTestContext,
    ) {
        cx.update(agentsi_ui::init);
        let (end, mut other) = UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, vcx) = cx.add_window_view(|_, cx| MemoryScreen::new("demo".into(), core, cx));
        screen.update(vcx, |s, cx| {
            s.data = state;
            cx.notify();
        });
        vcx.update(|window, cx| window.draw(cx).clear(cx));
        // The list that the screen asked for when it opened.
        assert_eq!(sent(&mut other).unwrap()["method"], "memory.list");
        (screen, other, vcx)
    }

    /// The next request that the screen sent to the core, as JSON, or `None`.
    fn sent(other: &mut UnixStream) -> Option<serde_json::Value> {
        other.set_nonblocking(true).unwrap();
        let frame = agentsi_proto::frame::read_frame(other).ok().flatten()?;
        serde_json::from_slice(&frame.payload).ok()
    }

    #[gpui::test]
    fn the_screen_draws_in_each_state(cx: &mut TestAppContext) {
        screen(cx, Load::Loading);
        screen(cx, Load::Failed("no core".into()));
        screen(cx, Load::Ready(data()));
        let empty = MemoryListResult {
            agents: Vec::new(),
            reflections: Vec::new(),
        };
        screen(cx, Load::Ready(empty));
    }

    #[gpui::test]
    fn the_cursor_walks_the_active_items_of_all_the_agents(cx: &mut TestAppContext) {
        let (s, _other, vcx) = screen(cx, Load::Ready(data()));
        s.update(vcx, |s, _| {
            let rows = s.rows();
            assert_eq!(
                rows,
                [
                    ("dev".to_string(), "m-a2".to_string()),
                    ("qa".to_string(), "m-b1".to_string())
                ]
            );
        });
    }

    #[gpui::test]
    fn retire_asks_first_and_confirm_writes_the_item_under_the_cursor(cx: &mut TestAppContext) {
        let (s, mut other, vcx) = screen(cx, Load::Ready(data()));
        s.update(vcx, |s, cx| {
            s.selected = 1;
            s.retire_selected(cx);
            assert_eq!(
                s.pending,
                Some(Pending::Retire {
                    agent: "qa".into(),
                    id: "m-b1".into()
                })
            );
        });
        assert!(sent(&mut other).is_none(), "nothing is sent before Confirm");
        s.update(vcx, |s, cx| s.confirm(cx));
        let request = sent(&mut other).unwrap();
        assert_eq!(request["method"], "memory.retire");
        assert_eq!(request["params"]["id"], "m-b1");
        assert_eq!(request["params"]["agent"], "qa");
        assert_eq!(request["params"]["why"], super::RETIRED_HERE);
        s.update(vcx, |s, cx| {
            assert!(s.busy && s.pending.is_none());
            s.pending = Some(Pending::Revert("r-2".into()));
            s.confirm(cx);
        });
        assert!(sent(&mut other).is_none(), "a second write waits");
    }

    #[gpui::test]
    fn a_hidden_tab_ignores_the_palette_actions(cx: &mut TestAppContext) {
        let (s, mut other, vcx) = screen(cx, Load::Ready(data()));
        s.update(vcx, |s, cx| {
            s.show(false);
            s.retire_selected(cx);
            s.revert_latest(cx);
            s.decide_pending(Verdict::Approved, cx);
            assert!(s.pending.is_none());
        });
        assert!(sent(&mut other).is_none());
    }

    #[gpui::test]
    fn the_palette_acts_on_the_newest_reflection_that_it_can(cx: &mut TestAppContext) {
        let (s, mut other, vcx) = screen(cx, Load::Ready(data()));
        s.update(vcx, |s, cx| {
            s.revert_latest(cx);
            assert_eq!(s.pending, Some(Pending::Revert("r-2".into())));
            assert_eq!(s.pending_lines(), ["Dev: retires m-a2, restores m-a1"]);
            s.cancel(cx);
            s.decide_pending(Verdict::Rejected, cx);
        });
        let request = sent(&mut other).unwrap();
        assert_eq!(request["method"], "decision.decide");
        assert_eq!(request["params"]["decision"], 9);
        assert_eq!(request["params"]["status"], "rejected");
    }

    #[test]
    fn a_stale_decision_says_that_nothing_was_applied() {
        let reply = |status| DecideResult {
            product: "demo".into(),
            minutes_id: None,
            status,
            action: None,
            commit_error: None,
            refused: None,
        };
        let refused = DecideResult {
            refused: Some("Decision 9 rejected: no proposal.".into()),
            ..reply(Outcome::Rejected)
        };
        assert_eq!(
            decision_flash(9, &refused),
            Flash::Failed("Decision 9 rejected: no proposal.".into())
        );
        let skipped = DecideResult {
            commit_error: Some("hook failed".into()),
            ..reply(Outcome::Approved)
        };
        assert_eq!(
            decision_flash(9, &skipped),
            Flash::Failed("Decision 9 approved. The commit was skipped: hook failed".into())
        );
        assert_eq!(
            decision_flash(9, &reply(Outcome::Stale)),
            Flash::Failed(
                "Decision 9 not applied: the target changed since the proposal, so it is now stale."
                    .into()
            )
        );
        assert_eq!(
            decision_flash(9, &reply(Outcome::Approved)),
            Flash::Done("Decision 9 approved.".into())
        );
        assert_eq!(
            decision_flash(9, &reply(Outcome::Rejected)),
            Flash::Done("Decision 9 rejected.".into())
        );
    }

    #[test]
    fn a_signal_reads_as_one_line() {
        let s = Signal {
            agent: "dev".into(),
            kind: "mark_rejected".into(),
            reference: "m1".into(),
            detail: "#0 A".into(),
        };
        assert_eq!(signal_line(&s), "Dev · mark_rejected · #0 A · m1");
    }

    #[gpui::test]
    fn revert_asks_first_and_a_verdict_goes_to_the_decision(cx: &mut TestAppContext) {
        let (s, mut other, vcx) = screen(cx, Load::Ready(data()));
        s.update(vcx, |s, cx| {
            s.ask_revert("r-2".into(), cx);
            s.confirm(cx);
        });
        let request = sent(&mut other).unwrap();
        assert_eq!(request["method"], "reflect.revert");
        assert_eq!(request["params"]["reflection"], "r-2");
        s.update(vcx, |s, cx| {
            s.busy = false;
            s.decide(9, Verdict::Approved, cx);
        });
        let request = sent(&mut other).unwrap();
        assert_eq!(request["method"], "decision.decide");
        assert_eq!(request["params"]["decision"], 9);
    }

    #[gpui::test]
    fn reflect_asks_one_confirm_then_sends_the_box_and_cancel_calls_reflect_cancel(
        cx: &mut TestAppContext,
    ) {
        use serde_json::json;
        let (screen, mut other, cx) =
            screen(cx, Load::Ready(serde_json::from_str(DATA).unwrap()));
        screen.update(cx, |s, cx| {
            s.memory_only = true;
            s.ask_reflect(cx);
        });
        assert!(sent(&mut other).is_none(), "nothing is sent before Confirm");
        assert_eq!(
            screen.read_with(cx, |s, _| s.pending.clone()),
            Some(Pending::Reflect)
        );
        screen.update(cx, |s, cx| s.confirm(cx));
        let r = sent(&mut other).expect("reflect.run");
        assert_eq!(r["method"], "reflect.run");
        assert_eq!(r["params"], json!({"product": "demo", "memory_only": true}));

        screen.update(cx, |s, cx| s.cancel_reflect(cx));
        let r = sent(&mut other).expect("reflect.cancel");
        assert_eq!(r["method"], "reflect.cancel");
        assert_eq!(r["params"], json!({"product": "demo"}));
    }
}
