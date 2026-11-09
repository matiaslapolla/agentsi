//! One minutes: the header with the check result, the figures, and tabs for the
//! priorities, the disagreements, the decisions, the answer of each role and the
//! usage. The marks and the decision status come from the store tables.
//!
//! The owner marks a priority with a button, or with `A`, `C` and `R` on the priority
//! under the cursor (`N` flips its "new" flag). A button archives or restores the
//! minutes. The palette has an entry for each action.
//!
//! The header has a "Listen" button when the minutes have a call, and a "Make the call" button
//! when they have none. "Make the call" runs `call.render` and reads `call.get` each second to
//! show the progress that `meeting.render` reports, as the Call screen does.

use std::time::Duration;

use agentsi_proto::call_api::{
    CallGet, CallGetParams, CallGetResult, CallRender, CallRenderParams,
};
use agentsi_proto::desk_api::{MinutesDetail, MinutesGet, MinutesGetParams, PriorityInfo};
use agentsi_proto::marks_api::{
    ArchiveArchive, ArchiveKind, ArchiveParams, ArchiveRestore, Mark, MarkEntry, MarkSet,
    MarkSetParams,
};
use agentsi_ui::components::{Button, Card, Force, neutral_tag, status_tag};
use agentsi_ui::tokens::{
    Status, control, font, line, radius, size, space, status, surface, text, weight,
};
use gpui_kit::component::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::minutes::cursor_after;
use super::parts::{
    CallInfo, Flash, Live, Load, Writes, column, decision_tag, first_screen_ready, flash_line,
    mark_tag, minutes_status, mono, notice, render_name, role_label, stat, stats, write_through,
};
use crate::format;
use crate::model::CoreState;
use crate::nav::{Nav, ScreenRequest};

/// How often the screen reads a call that renders.
const POLL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Priorities,
    Disagreements,
    Decisions,
    Roles,
    Usage,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Priorities => "Priorities",
            Tab::Disagreements => "Disagreements",
            Tab::Decisions => "Decisions",
            Tab::Roles => "Roles",
            Tab::Usage => "Usage",
        }
    }

    /// The tabs that a minutes shows, with the count of each. Disagreements and roles
    /// are left out when empty, as the web page does.
    fn of(m: &MinutesDetail) -> Vec<(Tab, usize)> {
        let mut tabs = vec![(Tab::Priorities, m.priorities.len())];
        if !m.disagreements.is_empty() {
            tabs.push((Tab::Disagreements, m.disagreements.len()));
        }
        tabs.push((Tab::Decisions, m.decisions.len()));
        if !m.roles.is_empty() {
            tabs.push((Tab::Roles, m.roles.len()));
        }
        tabs.push((Tab::Usage, m.usage.len()));
        tabs
    }
}

pub struct MinuteScreen {
    id: String,
    core: Entity<CoreState>,
    data: Load<Box<MinutesDetail>>,
    tab: Tab,
    scroll: ScrollHandle,
    /// The priority under the cursor, as a position in the list.
    selected: usize,
    focus: FocusHandle,
    /// False while the dock hides this tab.
    shown: bool,
    /// The order of use among the minutes screens, for the palette actions.
    stamp: u64,
    busy: bool,
    flash: Option<Flash>,
    /// What `call.get` said. `None` until it answers.
    call: Option<CallInfo>,
    /// The name of the render that this screen started and that has no reply yet.
    render: Option<String>,
    poll: Option<Task<()>>,
}

impl Writes for MinuteScreen {
    fn state(&mut self) -> (&mut bool, &mut Option<Flash>) {
        (&mut self.busy, &mut self.flash)
    }

    fn core(&self) -> &Entity<CoreState> {
        &self.core
    }
}

impl MinuteScreen {
    pub fn new(id: String, core: Entity<CoreState>, cx: &mut Context<Self>) -> Self {
        let mut screen = MinuteScreen {
            id,
            core,
            data: Load::Loading,
            tab: Tab::Priorities,
            scroll: ScrollHandle::new(),
            selected: 0,
            focus: cx.focus_handle().tab_stop(true),
            stamp: 0,
            shown: true,
            busy: false,
            flash: None,
            call: None,
            render: None,
            poll: None,
        };
        screen.stamp = Live::tick(cx);
        let me = cx.weak_entity();
        Live::add_minute(cx, me);
        screen.reload(cx);
        screen
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub fn stamp(&self) -> u64 {
        self.stamp
    }

    fn touch(&mut self, cx: &mut Context<Self>) {
        self.stamp = Live::tick(cx);
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    pub fn show(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.shown = shown;
        if shown {
            self.touch(cx);
        }
    }

    fn detail(&self) -> Option<&MinutesDetail> {
        match &self.data {
            Load::Ready(m) => Some(m),
            _ => None,
        }
    }

    fn fail(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.flash = Some(Flash::Failed(message.into()));
        cx.notify();
    }

    /// The priority under the cursor, when the Priorities tab is open.
    fn target(&mut self, cx: &mut Context<Self>) -> Option<PriorityInfo> {
        if self.tab != Tab::Priorities {
            self.fail("Open the Priorities tab to mark a priority.", cx);
            return None;
        }
        let found = self.detail().and_then(|m| m.priorities.get(self.selected));
        match found {
            Some(p) => Some(p.clone()),
            None => {
                self.fail("This minutes has no priority to mark.", cx);
                None
            }
        }
    }

    fn set_mark(&mut self, idx: u32, mark: Mark, is_new: bool, cx: &mut Context<Self>) {
        // The call sends its request when it is made, so the check comes before it.
        if self.busy {
            return;
        }
        let call = self.core.read(cx).link.call::<MarkSet>(&MarkSetParams {
            entries: vec![MarkEntry {
                minutes_id: self.id.clone(),
                idx,
                mark,
                is_new,
            }],
            // The owner may change a mark, as `agentsi mark <id> <idx> <mark>` does.
            unmarked_only: false,
        });
        let done = format!(
            "Priority {} marked {}{}.",
            idx + 1,
            mark.as_str(),
            if is_new { " (new)" } else { "" }
        );
        write_through(self, call, done, cx);
    }

    /// Marks the priority under the cursor. It keeps the "new" flag that the priority has.
    pub fn mark_selected(&mut self, mark: Mark, cx: &mut Context<Self>) {
        self.touch(cx);
        if let Some(p) = self.target(cx) {
            self.set_mark(p.idx, mark, p.is_new, cx);
        }
    }

    /// Flips the "new" flag of the priority under the cursor. The priority needs a mark.
    pub fn toggle_new_selected(&mut self, cx: &mut Context<Self>) {
        self.touch(cx);
        let Some(p) = self.target(cx) else { return };
        match p.mark.as_deref().and_then(mark_of) {
            Some(mark) => self.set_mark(p.idx, mark, !p.is_new, cx),
            None => self.fail("Mark the priority first. Then flag it as new.", cx),
        }
    }

    /// Archives or restores these minutes. The record stays in the store.
    pub fn archive_minutes(&mut self, archive: bool, cx: &mut Context<Self>) {
        self.touch(cx);
        let Some(product) = self.detail().map(|m| m.product.clone()) else {
            return;
        };
        if self.busy {
            return;
        }
        let params = ArchiveParams {
            product,
            kind: ArchiveKind::Minutes,
            id: Some(self.id.clone()),
        };
        let link = &self.core.read(cx).link;
        let done = if archive {
            format!("Archived {}. It is out of the lists.", self.id)
        } else {
            format!("Restored {}.", self.id)
        };
        if archive {
            let call = link.call::<ArchiveArchive>(&params);
            write_through(self, call, done, cx);
        } else {
            let call = link.call::<ArchiveRestore>(&params);
            write_through(self, call, done, cx);
        }
    }

    fn key_down(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.modified() || self.tab != Tab::Priorities {
            return;
        }
        let len = self.detail().map_or(0, |m| m.priorities.len());
        self.touch(cx);
        if let Some(to) = cursor_after(&k.key, self.selected, len) {
            self.selected = to;
        } else {
            match k.key.as_str() {
                "a" => self.mark_selected(Mark::Accepted, cx),
                "c" => self.mark_selected(Mark::Changed, cx),
                "r" => self.mark_selected(Mark::Rejected, cx),
                "n" => self.toggle_new_selected(cx),
                _ => return,
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<MinutesGet>(&MinutesGetParams {
                id: self.id.clone(),
            });
        cx.spawn(async move |this, cx| {
            let result = call.await.map(Box::new).map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.data.settle(result);
                let len = this.detail().map_or(0, |m| m.priorities.len());
                this.selected = this.selected.min(len.saturating_sub(1));
                first_screen_ready();
                this.read_call(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Reads whether the minutes have a call. The product comes from the minutes.
    fn read_call(&mut self, cx: &mut Context<Self>) {
        let Some(product) = self.detail().map(|m| m.product.clone()) else {
            return;
        };
        let call = self.core.read(cx).link.call::<CallGet>(&CallGetParams {
            product,
            minutes_id: self.id.clone(),
        });
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| this.call_loaded(result, cx));
        })
        .detach();
    }

    /// Keeps what `call.get` said, and reads again each second while a render runs.
    fn call_loaded(&mut self, result: Result<CallGetResult, String>, cx: &mut Context<Self>) {
        // A call that cannot be read is a call to make: the render reports the real error.
        self.call = Some(result.as_ref().map(CallInfo::from).unwrap_or_default());
        let rendering = self.call.as_ref().is_some_and(|c| c.render.is_some());
        self.poll = (rendering || self.render.is_some()).then(|| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(POLL).await;
                let _ = this.update(cx, |this, cx| {
                    this.poll = None;
                    this.read_call(cx);
                });
            })
        });
        cx.notify();
    }

    /// Opens the Call screen of these minutes.
    pub fn listen(&mut self, cx: &mut Context<Self>) {
        self.touch(cx);
        if let Some(product) = self.detail().map(|m| m.product.clone()) {
            Nav::open(cx, &self.core, ScreenRequest::call(&product, &self.id));
        }
    }

    /// Renders the call of these minutes with the voices of the core. It takes minutes.
    pub fn make_call(&mut self, cx: &mut Context<Self>) {
        self.touch(cx);
        let Some(product) = self.detail().map(|m| m.product.clone()) else {
            return;
        };
        let rendering = self.call.as_ref().is_some_and(|c| c.render.is_some());
        if self.render.is_some() || rendering {
            return;
        }
        let name = render_name();
        self.render = Some(name.clone());
        self.flash = None;
        let call = self
            .core
            .read(cx)
            .link
            .call::<CallRender>(&CallRenderParams {
                product,
                minutes_id: self.id.clone(),
                render: Some(name),
            });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| {
                this.render = None;
                this.flash = Some(match result {
                    Ok(_) => Flash::Done("The call is ready.".into()),
                    Err(e) => Flash::Failed(e.to_string()),
                });
                this.read_call(cx);
            });
        })
        .detach();
        self.read_call(cx);
        cx.notify();
    }
}

fn mark_of(text: &str) -> Option<Mark> {
    match text {
        "accepted" => Some(Mark::Accepted),
        "changed" => Some(Mark::Changed),
        "rejected" => Some(Mark::Rejected),
        _ => None,
    }
}

fn body(s: impl Into<SharedString>) -> Div {
    div()
        .text_size(size::base())
        .text_color(text::secondary())
        .child(s.into())
}

fn small(s: impl Into<SharedString>) -> Div {
    div()
        .text_size(size::sm())
        .text_color(text::tertiary())
        .child(s.into())
}

fn list(title: &str, items: &[String]) -> Option<Div> {
    (!items.is_empty()).then(|| {
        div()
            .flex()
            .flex_col()
            .gap(space(1))
            .child(
                div()
                    .text_size(size::xs())
                    .font_weight(weight::MEDIUM)
                    .text_color(text::tertiary())
                    .child(title.to_string()),
            )
            .children(items.iter().map(|i| body(format!("• {i}"))))
    })
}

fn priorities(
    m: &MinutesDetail,
    selected: usize,
    busy: bool,
    cx: &mut Context<MinuteScreen>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space(3))
        .when(m.priorities.is_empty(), |el| {
            el.child(small("This minutes has no priorities."))
        })
        .children(m.priorities.iter().enumerate().map(|(pos, p)| {
            // A button moves the cursor to its priority, then acts on it.
            let act = |button: Button, mark: Mark| {
                let idx = p.idx;
                let is_new = p.is_new;
                button.on_click(cx.listener(move |this, _, _, cx| {
                    this.selected = pos;
                    this.touch(cx);
                    this.set_mark(idx, mark, is_new, cx);
                }))
            };
            let marked = p.mark.as_deref().and_then(mark_of);
            // The current mark has the filled look. The others stay quiet.
            let button = |id: &'static str, label: &'static str, mark: Mark| {
                let b = Button::new((id, p.idx as usize))
                    .small()
                    .label(label)
                    .disabled(busy);
                act(
                    if marked == Some(mark) {
                        b.secondary()
                    } else {
                        b.ghost()
                    },
                    mark,
                )
            };
            let flag = {
                let (idx, is_new) = (p.idx, p.is_new);
                let b = Button::new(("priority-new", p.idx as usize))
                    .small()
                    .label("New")
                    .disabled(busy || marked.is_none());
                (if is_new { b.secondary() } else { b.ghost() }).on_click(cx.listener(
                    move |this, _, _, cx| {
                        this.selected = pos;
                        this.touch(cx);
                        if let Some(mark) = marked {
                            this.set_mark(idx, mark, !is_new, cx);
                        }
                    },
                ))
            };
            Card::new(("priority", p.idx as usize))
                .force(if pos == selected {
                    Force::Hover
                } else {
                    Force::None
                })
                .child(
                    div()
                        .flex()
                        .items_start()
                        .justify_between()
                        .gap(space(3))
                        .child(
                            div()
                                .flex()
                                .items_start()
                                .gap(space(3))
                                .child(mono(format!("{}", p.idx + 1)))
                                .child(
                                    div()
                                        .text_size(size::md())
                                        .font_weight(weight::SEMIBOLD)
                                        .text_color(text::primary())
                                        .child(p.title.clone()),
                                ),
                        )
                        .child(mark_tag(p.mark.as_deref())),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(space(2))
                        .when_some(p.owner_role.as_deref(), |el, r| {
                            el.child(neutral_tag(role_label(r)))
                        })
                        .when_some(p.objective.as_deref(), |el, o| {
                            el.child(status_tag(format!("objective {o}"), Status::Info))
                        })
                        .when(p.is_new, |el| el.child(status_tag("new", Status::Info))),
                )
                .child(body(p.why.clone()))
                .when(!p.sources.is_empty(), |el| {
                    el.child(
                        div().flex().flex_wrap().gap(space(2)).children(
                            p.sources
                                .iter()
                                .map(|s| neutral_tag(format!("{} {}", s.kind, s.reference))),
                        ),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(space(2))
                        .child(button("priority-accept", "Accept", Mark::Accepted))
                        .child(button("priority-change", "Change", Mark::Changed))
                        .child(button("priority-reject", "Reject", Mark::Rejected))
                        .child(flag),
                )
        }))
}

fn disagreements(m: &MinutesDetail) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space(6))
        .children(m.disagreements.iter().map(|d| {
            div()
                .flex()
                .flex_col()
                .gap(space(3))
                .child(
                    div()
                        .text_size(size::md())
                        .font_weight(weight::SEMIBOLD)
                        .text_color(text::primary())
                        .child(d.topic.clone()),
                )
                .children(d.positions.iter().map(|p| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(space(1))
                        .p(space(3))
                        .rounded(radius::md())
                        .bg(surface::raised())
                        .child(neutral_tag(role_label(&p.role)))
                        .child(body(p.view.clone()))
                }))
        }))
}

fn decisions(m: &MinutesDetail) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space(3))
        .when(m.decisions.is_empty(), |el| {
            el.child(small("This minutes proposes no decisions."))
        })
        .children(m.decisions.iter().enumerate().map(|(i, d)| {
            Card::new(("decision", i))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .justify_between()
                        .gap(space(3))
                        .child(
                            div()
                                .text_size(size::md())
                                .font_weight(weight::SEMIBOLD)
                                .text_color(text::primary())
                                .child(d.text.clone()),
                        )
                        .child(decision_tag(&d.status, d.id.is_some())),
                )
                .when_some(d.why.as_deref(), |el, w| el.child(body(w.to_string())))
                .when_some(d.action.as_deref(), |el, a| {
                    el.child(small(format!("Action: {a}")))
                })
                .when_some(d.id.filter(|_| d.status == "pending"), |el, id| {
                    el.child(mono(format!("agentsi decide {id}")))
                })
        }))
}

fn roles(m: &MinutesDetail) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(space(3))
        .children(m.roles.iter().enumerate().map(|(i, r)| {
            Card::new(("role", i))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(space(3))
                        .child(
                            div()
                                .text_size(size::md())
                                .font_weight(weight::SEMIBOLD)
                                .text_color(text::primary())
                                .child(role_label(&r.role)),
                        )
                        .when_some(r.model.as_deref(), |el, model| {
                            el.child(mono(model.to_string()))
                        }),
                )
                .child(body(if r.answer.is_empty() {
                    "No answer.".to_string()
                } else {
                    r.answer.clone()
                }))
                .children(list("Concerns", &r.concerns))
                .children(list("Notes for its memory", &r.memory_notes))
        }))
}

fn cell(s: impl Into<SharedString>) -> Div {
    div()
        .w(control::NUM_COLUMN)
        .flex_none()
        .text_right()
        .font_family(font::MONO)
        .text_size(size::sm())
        .text_color(text::secondary())
        .child(s.into())
}

fn usage(m: &MinutesDetail) -> Div {
    let head = |s: &str| {
        div()
            .w(control::NUM_COLUMN)
            .flex_none()
            .text_right()
            .text_size(size::xs())
            .font_weight(weight::MEDIUM)
            .text_color(text::tertiary())
            .child(s.to_string())
    };
    let name = |role: SharedString, model: SharedString| {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .gap(space(2))
            .items_baseline()
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::primary())
                    .child(role),
            )
            .child(mono(model))
    };
    let row = || {
        div()
            .flex()
            .items_baseline()
            .gap(space(3))
            .py(space(2))
            .border_b_1()
            .border_color(line::border())
    };
    if m.usage.is_empty() {
        return div().child(small("No usage is stored for this minutes."));
    }
    let t = &m.usage_totals;
    div()
        .flex()
        .flex_col()
        .p(space(4))
        .rounded(radius::lg())
        .bg(surface::raised())
        .child(
            row()
                .child(
                    div()
                        .flex_1()
                        .text_size(size::xs())
                        .font_weight(weight::MEDIUM)
                        .text_color(text::tertiary())
                        .child("Role"),
                )
                .child(head("In"))
                .child(head("Out"))
                .child(head("Cache read"))
                .child(head("Cache write"))
                .child(head("Cost")),
        )
        .children(m.usage.iter().map(|u| {
            row()
                .child(name(role_label(&u.role).into(), u.model.clone().into()))
                .child(cell(format::compact(u.input_tokens)))
                .child(cell(format::compact(u.output_tokens)))
                .child(cell(format::compact(u.cache_read)))
                .child(cell(format::compact(u.cache_write)))
                .child(cell(format::usd(u.cost_usd)))
        }))
        .child(
            row()
                .child(
                    div()
                        .flex_1()
                        .text_size(size::sm())
                        .font_weight(weight::SEMIBOLD)
                        .text_color(text::primary())
                        .child("Total"),
                )
                .child(cell(format::compact(t.input_tokens)))
                .child(cell(format::compact(t.output_tokens)))
                .child(cell(format::compact(t.cache_read)))
                .child(cell(format::compact(t.cache_write)))
                .child(cell(format::usd(t.cost_usd))),
        )
}

/// The words of the call state next to the call button.
fn call_progress(call: &CallInfo, own_render: bool) -> Option<String> {
    match (call.render, own_render) {
        (Some((done, of)), _) => Some(format!("Making the call: segment {done} of {of}…")),
        (None, true) => Some("Making the call…".into()),
        (None, false) => None,
    }
}

/// "Listen" with a call, "Make the call" with none, and the progress of a render. Nothing
/// shows until `call.get` has answered.
fn call_controls(
    call: Option<&CallInfo>,
    own_render: bool,
    cx: &mut Context<MinuteScreen>,
) -> Vec<AnyElement> {
    let Some(call) = call else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if call.audio {
        out.push(
            Button::new("minute-listen")
                .secondary()
                .small()
                .icon(IconName::Play)
                .label("Listen")
                .on_click(cx.listener(|this, _, _, cx| this.listen(cx)))
                .into_any_element(),
        );
    }
    let progress = call_progress(call, own_render);
    if !call.audio && progress.is_none() {
        out.push(
            Button::new("minute-make-call")
                .secondary()
                .small()
                .label("Make the call")
                .on_click(cx.listener(|this, _, _, cx| this.make_call(cx)))
                .into_any_element(),
        );
    }
    if let Some(line) = progress {
        out.push(small(line).into_any_element());
    }
    out
}

impl Render for MinuteScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .id("minute")
            .track_focus(&self.focus)
            .key_context("Minute")
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.key_down(e, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.touch(cx)),
            )
            .size_full()
            .bg(surface::panel());
        let m = match &self.data {
            Load::Loading => return root.child(notice("Reading the minutes…", None)),
            Load::Failed(e) => {
                return root.child(notice(
                    format!("These minutes did not load: {e}"),
                    Some(super::parts::reload_hint(cx)),
                ));
            }
            Load::Ready(m) => m,
        };
        let (selected, busy) = (self.selected, self.busy);
        let flash = self.flash.as_ref().map(flash_line);
        let archived = m.archived;
        let tabs = Tab::of(m);
        let tab = if tabs.iter().any(|(t, _)| *t == self.tab) {
            self.tab
        } else {
            Tab::Priorities
        };
        let marked = m.priorities.iter().filter(|p| p.mark.is_some()).count();
        let decided = m.decisions.iter().filter(|d| d.status != "pending").count();

        let header = div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(mono(m.id.clone()).truncate())
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(space(3))
                    .child(minutes_status(m.status.as_deref()))
                    .when_some(m.created_at.as_deref(), |el, at| {
                        el.child(small(format::date_of(at)))
                    })
                    .when_some(m.scope.as_deref(), |el, s| {
                        el.child(neutral_tag(s.to_string()))
                    })
                    .when(m.heard, |el| el.child(neutral_tag("heard")))
                    .when(m.archived, |el| el.child(neutral_tag("archived")))
                    .when_some(m.check.as_ref().filter(|c| c.passed), |el, c| {
                        el.child(status_tag(
                            if c.repaired {
                                "check passed · repaired once"
                            } else {
                                "check passed"
                            },
                            Status::Ok,
                        ))
                    }),
            )
            .child(
                div()
                    .text_size(size::lg())
                    .font_weight(weight::SEMIBOLD)
                    .text_color(text::primary())
                    .child(m.question.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(space(3))
                    .children(call_controls(self.call.as_ref(), self.render.is_some(), cx))
                    .child(
                        Button::new("minute-archive")
                            .ghost()
                            .small()
                            .label(if archived {
                                "Restore these minutes"
                            } else {
                                "Archive these minutes"
                            })
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.archive_minutes(!archived, cx);
                            })),
                    ),
            );

        let failed_check = m.check.as_ref().filter(|c| !c.passed).map(|c| {
            div()
                .flex()
                .flex_col()
                .gap(space(1))
                .p(space(4))
                .rounded(radius::lg())
                .border_1()
                .border_color(status::danger().alpha(status::BORDER_ALPHA))
                .bg(Status::Danger.tint())
                .child(
                    div()
                        .text_size(size::base())
                        .font_weight(weight::MEDIUM)
                        .text_color(status::danger())
                        .child(if c.repaired {
                            "The check failed. The PM repaired it once."
                        } else {
                            "The check failed."
                        }),
                )
                .children(c.failures.iter().map(|f| body(format!("• {f}"))))
        });

        let figures = stats(vec![
            stat(
                "Priorities marked",
                format!("{marked}/{}", m.priorities.len()),
            ),
            stat(
                "Decisions decided",
                format!("{decided}/{}", m.decisions.len()),
            ),
            stat("Disagreements", m.disagreements.len().to_string()),
            stat(
                "Cost",
                if m.usage.is_empty() {
                    "—".to_string()
                } else {
                    format::usd(m.usage_totals.cost_usd)
                },
            ),
        ]);

        let bar = div()
            .flex()
            .flex_wrap()
            .gap(space(2))
            .children(tabs.iter().map(|(t, n)| {
                let t = *t;
                let b = Button::new(("minute-tab", t as usize))
                    .small()
                    .label(format!("{} {n}", t.label()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tab = t;
                        cx.notify();
                    }));
                if t == tab { b.secondary() } else { b.ghost() }
            }));

        let content = match tab {
            Tab::Priorities => priorities(m, selected, busy, cx),
            Tab::Disagreements => disagreements(m),
            Tab::Decisions => decisions(m),
            Tab::Roles => roles(m),
            Tab::Usage => usage(m),
        };

        root.track_scroll(&self.scroll).overflow_y_scroll().child(
            column()
                .child(header)
                .children(flash)
                .children(failed_check)
                .child(figures)
                .child(bar)
                .child(content),
        )
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: it would bring in the `test` macro of `gpui_kit`.
    use super::{Load, MinutesDetail, Tab};
    use serde_json::Value;

    const EMPTY: &str = r#"{
        "id": "m1", "product": "demo", "scope": null, "question": "Q?", "status": "valid",
        "created_at": null, "archived": false, "heard": false, "check": null,
        "priorities": [], "disagreements": [], "decisions": [], "roles": [], "usage": [],
        "usage_totals": {"rows": 0, "input_tokens": 0, "output_tokens": 0,
                         "cache_read": 0, "cache_write": 0, "cost_usd": 0.0}
    }"#;

    fn detail(extra: &str) -> MinutesDetail {
        let mut v: Value = serde_json::from_str(EMPTY).unwrap();
        let extra: Value = serde_json::from_str(extra).unwrap();
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn the_tabs_leave_out_empty_disagreements_and_roles() {
        let tabs: Vec<Tab> = Tab::of(&detail("{}")).into_iter().map(|(t, _)| t).collect();
        assert_eq!(tabs, [Tab::Priorities, Tab::Decisions, Tab::Usage]);
        let m = detail(
            r#"{"disagreements": [{"topic": "T", "positions": []}],
                "roles": [{"role": "pm", "model": null, "answer": "", "concerns": [], "memory_notes": []}]}"#,
        );
        assert_eq!(
            Tab::of(&m),
            [
                (Tab::Priorities, 0),
                (Tab::Disagreements, 1),
                (Tab::Decisions, 0),
                (Tab::Roles, 1),
                (Tab::Usage, 0)
            ]
        );
    }

    #[gpui::test]
    fn one_minutes_draws_each_tab_and_a_failed_check(cx: &mut gpui::TestAppContext) {
        use crate::link::Link;
        use crate::model::CoreState;
        use gpui_kit::AppContext as _;
        cx.update(agentsi_ui::init);
        let (end, _other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(std::sync::Arc::new(Link::for_tests(end))));
        let (screen, cx) =
            cx.add_window_view(|_, cx| super::MinuteScreen::new("m1".into(), core, cx));
        let full = detail(
            r#"{"check": {"passed": false, "repaired": true, "failures": ["f1"]},
                "priorities": [{"idx": 0, "title": "P", "why": "w", "owner_role": "pm", "objective": "o1",
                                "mark": "accepted", "is_new": true, "sources": [{"type": "url", "ref": "https://x"}]}],
                "disagreements": [{"topic": "T", "positions": [{"role": "pm", "view": "yes"}]}],
                "decisions": [{"id": null, "text": "D", "why": null, "action": null, "status": "pending"}],
                "roles": [{"role": "dev.backend", "model": "m", "answer": "a", "concerns": ["c"], "memory_notes": ["n"]}],
                "usage": [{"role": "pm", "model": "m", "input_tokens": 1, "output_tokens": 2,
                           "cache_read": 3, "cache_write": 4, "cost_usd": 0.5}],
                "usage_totals": {"rows": 1, "input_tokens": 1, "output_tokens": 2,
                                 "cache_read": 3, "cache_write": 4, "cost_usd": 0.5}}"#,
        );
        screen.update(cx, |s, cx| {
            s.data = Load::Ready(Box::new(full));
            cx.notify();
        });
        for tab in [
            Tab::Priorities,
            Tab::Disagreements,
            Tab::Decisions,
            Tab::Roles,
            Tab::Usage,
        ] {
            screen.update(cx, |s, cx| {
                s.tab = tab;
                cx.notify();
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
    }

    fn sent(other: &mut std::os::unix::net::UnixStream) -> Option<Value> {
        other.set_nonblocking(true).unwrap();
        let frame = agentsi_proto::frame::read_frame(other).ok().flatten()?;
        serde_json::from_slice(&frame.payload).ok()
    }

    #[gpui::test]
    fn keys_mark_the_priority_under_the_cursor_and_the_new_flag_needs_a_mark(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::link::Link;
        use crate::model::CoreState;
        use gpui_kit::AppContext as _;
        use serde_json::json;
        cx.update(agentsi_ui::init);
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(std::sync::Arc::new(Link::for_tests(end))));
        let (screen, cx) =
            cx.add_window_view(|_, cx| super::MinuteScreen::new("m1".into(), core, cx));
        assert_eq!(sent(&mut other).unwrap()["method"], "minutes.get");
        let two = detail(
            r#"{"priorities": [
                {"idx": 0, "title": "A", "why": "w", "owner_role": "pm", "objective": null,
                 "mark": null, "is_new": false, "sources": []},
                {"idx": 1, "title": "B", "why": "w", "owner_role": "pm", "objective": null,
                 "mark": "accepted", "is_new": false, "sources": []}]}"#,
        );
        screen.update(cx, |s, cx| {
            s.data = Load::Ready(Box::new(two));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.focus_next(cx));
        assert!(cx.update(|window, cx| screen.read(cx).focus_handle().is_focused(window)));

        // "n" on an unmarked priority sends nothing and says why.
        cx.simulate_keystrokes("n");
        assert!(sent(&mut other).is_none());
        let flash = screen.read_with(cx, |s, _| s.flash.clone());
        assert!(
            matches!(flash, Some(super::Flash::Failed(m)) if m.contains("Mark the priority first"))
        );

        cx.simulate_keystrokes("c");
        let request = sent(&mut other).expect("a request");
        assert_eq!(request["method"], "mark.set");
        assert_eq!(
            request["params"],
            json!({"entries": [{"minutes_id": "m1", "idx": 0, "mark": "changed", "is_new": false}],
                   "unmarked_only": false})
        );
        screen.update(cx, |s, _| s.busy = false);
        // Down to the accepted priority, then "n" flips its flag and keeps its mark.
        cx.simulate_keystrokes("down n");
        let request = sent(&mut other).expect("a request");
        assert_eq!(
            request["params"]["entries"],
            json!([{"minutes_id": "m1", "idx": 1, "mark": "accepted", "is_new": true}])
        );
    }

    #[gpui::test]
    fn the_archive_button_archives_and_a_tab_other_than_priorities_marks_nothing(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::link::Link;
        use crate::model::CoreState;
        use gpui_kit::AppContext as _;
        use serde_json::json;
        cx.update(agentsi_ui::init);
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(std::sync::Arc::new(Link::for_tests(end))));
        let (screen, cx) =
            cx.add_window_view(|_, cx| super::MinuteScreen::new("m1".into(), core, cx));
        sent(&mut other);
        let one = detail(
            r#"{"priorities": [{"idx": 0, "title": "A", "why": "w", "owner_role": "pm",
                "objective": null, "mark": null, "is_new": false, "sources": []}]}"#,
        );
        screen.update(cx, |s, cx| {
            s.data = Load::Ready(Box::new(one));
            s.tab = Tab::Decisions;
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.focus_next(cx));
        // The keys of the priorities do nothing on another tab.
        cx.simulate_keystrokes("a");
        assert!(sent(&mut other).is_none());
        screen.update(cx, |s, cx| s.mark_selected(super::Mark::Accepted, cx));
        assert!(sent(&mut other).is_none());
        assert!(screen.read_with(cx, |s, _| matches!(
            &s.flash,
            Some(super::Flash::Failed(m)) if m.contains("Priorities tab")
        )));

        screen.update(cx, |s, cx| s.archive_minutes(true, cx));
        let request = sent(&mut other).expect("a request");
        assert_eq!(request["method"], "archive.archive");
        assert_eq!(
            request["params"],
            json!({"product": "demo", "kind": "minutes", "id": "m1"})
        );
        screen.update(cx, |s, _| s.busy = false);
        screen.update(cx, |s, cx| s.archive_minutes(false, cx));
        assert_eq!(sent(&mut other).unwrap()["method"], "archive.restore");
    }

    #[gpui::test]
    fn a_mark_lands_on_the_core_of_the_screen_when_two_cores_have_the_same_minutes(
        cx: &mut gpui::TestAppContext,
    ) {
        use crate::model::two_cores::{methods, two_cores};
        use gpui_kit::AppContext as _;
        cx.update(agentsi_ui::init);
        let mut t = two_cores(cx);
        let studio = t.cores.of(Some("studio")).unwrap().clone();
        let local = t.cores.local.clone();
        let (_local_screen, cx) =
            cx.add_window_view(|_, cx| super::MinuteScreen::new("m1".into(), local, cx));
        let screen =
            cx.update(|_, cx| cx.new(|cx| super::MinuteScreen::new("m1".into(), studio, cx)));
        assert_eq!(methods(&mut t.local), ["minutes.get"]);
        assert_eq!(methods(&mut t.studio), ["minutes.get"]);
        let one = detail(
            r#"{"priorities": [{"idx": 0, "title": "A", "why": "w", "owner_role": "pm",
                "objective": null, "mark": null, "is_new": false, "sources": []}]}"#,
        );
        screen.update(cx, |s, cx| {
            s.data = Load::Ready(Box::new(one));
            s.mark_selected(super::Mark::Accepted, cx);
        });
        assert_eq!(methods(&mut t.studio), ["mark.set"]);
        assert_eq!(methods(&mut t.local), Vec::<String>::new());
    }

    fn call_get(extra: &str) -> super::CallGetResult {
        let mut v = json!({
            "product": "demo", "minutes_id": "m1", "question": "Q?", "segments": [],
            "timed": true, "heard": false
        });
        v.as_object_mut().unwrap().extend(
            serde_json::from_str::<Value>(extra)
                .unwrap()
                .as_object()
                .unwrap()
                .clone(),
        );
        serde_json::from_value(v).unwrap()
    }

    use serde_json::json;

    /// A minutes screen with its minutes loaded, and the core end of its link.
    fn with_minutes(
        cx: &mut gpui::TestAppContext,
    ) -> (
        gpui::Entity<super::MinuteScreen>,
        &mut gpui::VisualTestContext,
        std::os::unix::net::UnixStream,
        futures::channel::mpsc::UnboundedReceiver<crate::nav::ScreenRequest>,
    ) {
        use crate::link::Link;
        use crate::model::CoreState;
        use gpui_kit::AppContext as _;
        cx.update(agentsi_ui::init);
        let (nav, requests) = crate::nav::Nav::channel();
        cx.update(|cx| cx.set_global(nav));
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(std::sync::Arc::new(Link::for_tests(end))));
        let (screen, cx) =
            cx.add_window_view(|_, cx| super::MinuteScreen::new("m1".into(), core, cx));
        assert_eq!(sent(&mut other).unwrap()["method"], "minutes.get");
        screen.update(cx, |s, cx| {
            s.data = Load::Ready(Box::new(detail("{}")));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (screen, cx, other, requests)
    }

    #[gpui::test]
    fn listen_shows_with_a_call_and_opens_the_call_screen(cx: &mut gpui::TestAppContext) {
        let (screen, cx, mut other, mut requests) = with_minutes(cx);
        // Nothing shows until the core has answered.
        assert!(cx.debug_bounds("button:minute-listen").is_none());
        assert!(cx.debug_bounds("button:minute-make-call").is_none());
        screen.update(cx, |s, cx| {
            s.call_loaded(
                Ok(call_get(r#"{"audio": {"format": "opus", "bytes": 10}}"#)),
                cx,
            )
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("button:minute-make-call").is_none());
        let listen = cx
            .debug_bounds("button:minute-listen")
            .expect("a Listen button");
        cx.simulate_click(listen.center(), gpui::Modifiers::none());
        assert_eq!(
            requests.try_recv().unwrap(),
            crate::nav::ScreenRequest::call("demo", "m1")
        );
        assert!(
            sent(&mut other).is_none(),
            "Listen sends nothing to the core"
        );
    }

    #[gpui::test]
    fn make_the_call_shows_with_no_call_and_runs_call_render(cx: &mut gpui::TestAppContext) {
        let (screen, cx, mut other, _requests) = with_minutes(cx);
        screen.update(cx, |s, cx| s.call_loaded(Ok(call_get("{}")), cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("button:minute-listen").is_none());
        let make = cx
            .debug_bounds("button:minute-make-call")
            .expect("a Make the call button");
        cx.simulate_click(make.center(), gpui::Modifiers::none());
        let request = sent(&mut other).expect("a request");
        assert_eq!(request["method"], "call.render");
        assert_eq!(request["params"]["product"], "demo");
        assert_eq!(request["params"]["minutes_id"], "m1");
        assert!(
            request["params"]["render"]
                .as_str()
                .unwrap()
                .starts_with("app-")
        );
        // It reads the call at once, to show the progress.
        assert_eq!(sent(&mut other).unwrap()["method"], "call.get");
        // A second click while it renders runs nothing.
        screen.update(cx, |s, cx| s.make_call(cx));
        assert!(sent(&mut other).is_none());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("button:minute-make-call").is_none());
    }

    #[test]
    fn a_running_render_shows_its_segments() {
        use super::{CallInfo, call_progress};
        let info = |render| CallInfo {
            audio: false,
            render,
        };
        assert_eq!(
            call_progress(&info(Some((3, 12))), false).as_deref(),
            Some("Making the call: segment 3 of 12…")
        );
        assert_eq!(
            call_progress(&info(None), true).as_deref(),
            Some("Making the call…")
        );
        assert_eq!(call_progress(&info(None), false), None);
    }
}
