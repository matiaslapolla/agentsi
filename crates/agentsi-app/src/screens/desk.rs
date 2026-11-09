//! The front desk of a product: its state, the pending decisions, the objectives, the
//! usage and the last minutes. The owner approves, rejects or archives a pending
//! decision with a button, or with `A`, `R` and `X` on the decision under the cursor.
//! The palette has an entry for each of the three. A write goes through the core, and
//! then each screen reads its data again.

use std::cell::RefCell;
use std::rc::Rc;

use agentsi_proto::desk_api::{DeskResult, MinutesRow, ProductDesk, ProductDeskParams};
use agentsi_proto::knowledge_api::{KbIndexParams, KnowledgeIndex};
use agentsi_proto::marks_api::{
    ArchiveArchive, ArchiveKind, ArchiveParams, ArchiveRestore, ArchivedDecision, DecideParams,
    DecideResult, DecisionAdd, DecisionAddParams, DecisionArchived, DecisionArchivedParams,
    DecisionDecide, Mark, MarkEntry, MarkPending, MarkPendingParams, MarkSet, MarkSetParams,
    PendingDecisionItem, PendingItem, PendingPriority, Verdict,
};
use agentsi_ui::components::{Button, Card, Force, ListRow, neutral_tag, status_tag};
use agentsi_ui::tokens::{Status, control, line, radius, size, space, surface, text, weight};
use gpui_kit::component::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::memory::decision_flash;
use super::minutes::cursor_after;
use super::parts::{
    Flash, Live, Load, Writes, column, first_screen_ready, flash_line, minutes_meta,
    minutes_status, mono, notice, role_label, section, stat, stats, write_through,
    write_through_with,
};
use super::form::{Form, FormEvent};
use crate::format;
use crate::link::CallError;
use crate::model::CoreState;
use crate::nav::{Nav, ScreenRequest};

pub struct DeskScreen {
    pub(super) conversation: Entity<super::desk_conversation::DeskConversation>,
    tickets: Entity<super::desk_tickets::DeskTickets>,
    product: String,
    core: Entity<CoreState>,
    data: Load<DeskResult>,
    scroll: ScrollHandle,
    /// The pending decision under the cursor.
    selected: usize,
    focus: FocusHandle,
    /// False while the dock hides this tab.
    shown: bool,
    /// The order of use among the front desks, for the palette actions.
    stamp: u64,
    /// A write waits for the core. The screen starts no second one.
    busy: bool,
    flash: Option<Flash>,
    /// What waits for the owner, from `mark.pending`, in its order.
    items: Vec<PendingItem>,
    /// The action of the decision that the owner approved last. "Run in Claude" runs it.
    run: Rc<RefCell<Option<String>>>,
    /// Set by the palette: the next draw opens the form "New decision".
    want_form: bool,
    form: Option<(Entity<Form>, Subscription)>,
    /// The archived decisions, while the owner shows them.
    archived: Option<Vec<ArchivedDecision>>,
}

/// `write_through` shows one fixed line for a reply, so a reply that did not do what the
/// owner asked comes back as an error.
fn applied(id: i64, reply: &DecideResult) -> Result<(), CallError> {
    match decision_flash(id, reply) {
        Flash::Done(_) => Ok(()),
        Flash::Failed(line) => Err(CallError::Link(line)),
    }
}

impl DeskScreen {
    pub fn new(product: String, core: Entity<CoreState>, cx: &mut Context<Self>) -> Self {
        let tickets = {
            let (product, core) = (product.clone(), core.clone());
            cx.new(|cx| super::desk_tickets::DeskTickets::new(product, core, cx))
        };
        let conversation = {
            let (product, core) = (product.clone(), core.clone());
            cx.new(|cx| super::desk_conversation::DeskConversation::new(product, core, cx))
        };
        let mut screen = DeskScreen {
            conversation,
            tickets,
            product,
            core,
            data: Load::Loading,
            scroll: ScrollHandle::new(),
            selected: 0,
            focus: cx.focus_handle().tab_stop(true),
            stamp: 0,
            shown: true,
            busy: false,
            flash: None,
            items: Vec::new(),
            run: Rc::default(),
            want_form: false,
            form: None,
            archived: None,
        };
        screen.stamp = Live::tick(cx);
        let me = cx.weak_entity();
        Live::add_desk(cx, me);
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

    /// The palette entry "Ask the front desk": the input of the conversation takes the focus.
    pub fn ask(&mut self, cx: &mut Context<Self>) {
        self.touch(cx);
        self.conversation.update(cx, |c, cx| c.ask(cx));
    }

    /// The decision under the cursor, if the item there is a decision.
    fn decision_at(&self) -> Option<&PendingDecisionItem> {
        match self.items.get(self.selected) {
            Some(PendingItem::Decision(d)) => Some(d),
            _ => None,
        }
    }

    /// Approves or rejects the decision under the cursor.
    pub fn decide_selected(&mut self, verdict: Verdict, cx: &mut Context<Self>) {
        self.touch(cx);
        if let Some(id) = self.decision_at().map(|d| d.id) {
            self.decide(id, verdict, cx);
        }
    }

    /// Archives the decision under the cursor.
    pub fn archive_selected(&mut self, cx: &mut Context<Self>) {
        self.touch(cx);
        if let Some(id) = self.decision_at().map(|d| d.id) {
            self.archive(id, cx);
        }
    }

    /// Marks the priority under the cursor. `is_new` flags a priority that the owner did not have
    /// in mind, and counts as accepted.
    pub fn mark_selected(&mut self, mark: Mark, is_new: bool, cx: &mut Context<Self>) {
        self.touch(cx);
        if let Some(PendingItem::Priority(p)) = self.items.get(self.selected).cloned() {
            self.mark(&p, mark, is_new, cx);
        }
    }

    pub fn mark(&mut self, p: &PendingPriority, mark: Mark, is_new: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let call = self.core.read(cx).link.call::<MarkSet>(&MarkSetParams {
            entries: vec![MarkEntry {
                minutes_id: p.minutes_id.clone(),
                idx: p.idx,
                mark,
                is_new,
            }],
            unmarked_only: true,
        });
        let word = if is_new { "new" } else { mark.as_str() };
        write_through(
            self,
            call,
            format!("Priority {} of {} marked {word}.", p.idx + 1, p.minutes_id),
            cx,
        );
    }

    /// `knowledge.index` for the product. The line shows the repos and the time.
    pub fn update_index(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let call = self
            .core
            .read(cx)
            .link
            .call::<KnowledgeIndex>(&KbIndexParams {
                product: self.product.clone(),
                force: false,
            });
        write_through_with(
            self,
            call,
            |r| {
                format!(
                    "Index updated: {} repos in {:.1} s.",
                    r.repos.len(),
                    r.ms as f64 / 1000.0
                )
            },
            cx,
        );
    }

    /// Opens the form "New decision" on the next draw.
    pub fn new_decision(&mut self, cx: &mut Context<Self>) {
        self.touch(cx);
        self.want_form = true;
        cx.notify();
    }

    fn close_form(&mut self, cx: &mut Context<Self>) {
        self.want_form = false;
        self.form = None;
        cx.notify();
    }

    fn submit_form(&mut self, form: &Entity<Form>, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let (text, why, action) = {
            let f = form.read(cx);
            (f.value(0, cx), f.value(1, cx), f.value(2, cx))
        };
        let call = self
            .core
            .read(cx)
            .link
            .call::<DecisionAdd>(&DecisionAddParams {
                product: self.product.clone(),
                text,
                why,
                action: Some(action).filter(|a| !a.is_empty()),
            });
        write_through_with(self, call, |r| format!("Decision {} added.", r.id), cx);
        self.close_form(cx);
    }

    fn ensure_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.want_form || self.form.is_some() {
            return;
        }
        let form = cx.new(|cx| {
            Form::new("desk-new-decision", "New decision", "Add the decision", cx)
                .field("Decision", "What should the owner decide?", true, window, cx)
                .field("Why", "Why it matters", true, window, cx)
                .field("Action (optional)", "What to run after the approval", false, window, cx)
        });
        let sub = cx.subscribe(&form, |this, form, e: &FormEvent, cx| match e {
            FormEvent::Submit => this.submit_form(&form, cx),
            FormEvent::Cancel => this.close_form(cx),
        });
        form.update(cx, |f, cx| f.focus_first(window, cx));
        self.form = Some((form, sub));
    }

    /// Shows or hides the archived decisions.
    pub fn toggle_archived(&mut self, cx: &mut Context<Self>) {
        self.touch(cx);
        if self.archived.take().is_none() {
            self.load_archived(cx);
        }
        cx.notify();
    }

    fn load_archived(&mut self, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<DecisionArchived>(&DecisionArchivedParams {
                product: self.product.clone(),
            });
        self.archived.get_or_insert_default();
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(r) => this.archived = Some(r.decisions),
                    Err(e) => {
                        this.archived = None;
                        this.flash = Some(Flash::Failed(e.to_string()));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn restore(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let call = self
            .core
            .read(cx)
            .link
            .call::<ArchiveRestore>(&ArchiveParams {
                product: self.product.clone(),
                kind: ArchiveKind::Decision,
                id: Some(id.to_string()),
            });
        write_through(self, call, format!("Decision {id} restored."), cx);
    }

    /// Opens a Claude pane with the action of the decision that the owner approved last.
    pub fn run_in_claude(&mut self, cx: &mut Context<Self>) {
        self.touch(cx);
        let action = self.run.borrow().clone();
        if let Some(text) = action {
            crate::nav::Runs::run(cx, &self.core, &self.product, &text);
        }
    }

    pub fn decide(&mut self, id: i64, verdict: Verdict, cx: &mut Context<Self>) {
        // The call sends its request when it is made, so the check comes before it.
        if self.busy {
            return;
        }
        let call = self
            .core
            .read(cx)
            .link
            .call::<DecisionDecide>(&DecideParams {
                decision: id,
                status: verdict,
            });
        let checked = async move {
            let reply = call.await?;
            applied(id, &reply)?;
            Ok(reply)
        };
        let run = self.run.clone();
        *run.borrow_mut() = None;
        write_through_with(
            self,
            checked,
            move |reply| {
                if verdict == Verdict::Approved {
                    *run.borrow_mut() = reply.action.clone().filter(|a| !a.is_empty());
                }
                format!("Decision {id} {}.", verdict.as_str())
            },
            cx,
        );
    }

    pub fn archive(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let call = self
            .core
            .read(cx)
            .link
            .call::<ArchiveArchive>(&ArchiveParams {
                product: self.product.clone(),
                kind: ArchiveKind::Decision,
                id: Some(id.to_string()),
            });
        write_through(
            self,
            call,
            format!("Decision {id} archived. Restore it with agentsi restore {id}."),
            cx,
        );
    }

    fn key_down(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.modified() {
            return;
        }
        self.touch(cx);
        if let Some(to) = cursor_after(&k.key, self.selected, self.items.len()) {
            self.selected = to;
        } else {
            let priority = matches!(self.items.get(self.selected), Some(PendingItem::Priority(_)));
            match (k.key.as_str(), priority) {
                ("a", true) => self.mark_selected(Mark::Accepted, false, cx),
                ("c", true) => self.mark_selected(Mark::Changed, false, cx),
                ("r", true) => self.mark_selected(Mark::Rejected, false, cx),
                ("n", true) => self.mark_selected(Mark::Accepted, true, cx),
                ("a", false) => self.decide_selected(Verdict::Approved, cx),
                ("r", false) => self.decide_selected(Verdict::Rejected, cx),
                ("x", false) => self.archive_selected(cx),
                ("enter", _) => {
                    let minutes = match self.items.get(self.selected) {
                        Some(PendingItem::Priority(p)) => Some(p.minutes_id.clone()),
                        Some(PendingItem::Decision(d)) => d.minutes_id.clone(),
                        None => None,
                    };
                    if let Some(m) = minutes {
                        Nav::open(cx, &self.core, ScreenRequest::minute(&self.product, &m));
                    }
                }
                _ => return,
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Reads the front desk again. The screen keeps what it shows until the reply comes.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<ProductDesk>(&ProductDeskParams {
                product: self.product.clone(),
            });
        let pending = self
            .core
            .read(cx)
            .link
            .call::<MarkPending>(&MarkPendingParams {
                product: self.product.clone(),
            });
        // The desk asks for its own data first.
        self.tickets.update(cx, |t, cx| t.reload(cx));
        if self.archived.is_some() {
            self.load_archived(cx);
        }
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let pending = pending.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.data.settle(result);
                match pending {
                    Ok(p) => this.items = p.items,
                    Err(e) => this.flash = Some(Flash::Failed(e.to_string())),
                }
                this.selected = this.selected.min(this.items.len().saturating_sub(1));
                first_screen_ready();
                cx.notify();
            });
        })
        .detach();
    }
}

impl Writes for DeskScreen {
    fn state(&mut self) -> (&mut bool, &mut Option<Flash>) {
        (&mut self.busy, &mut self.flash)
    }

    fn core(&self) -> &Entity<CoreState> {
        &self.core
    }
}

fn priority_card(
    i: usize,
    p: &PendingPriority,
    selected: bool,
    busy: bool,
    cx: &mut Context<DeskScreen>,
) -> Card {
    let card = Card::new(("desk-priority", i))
        .force(if selected { Force::Hover } else { Force::None })
        .child(
            div()
                .flex()
                .items_center()
                .gap(space(2))
                .child(neutral_tag("Priority"))
                .child(mono(format!("{} · {}", p.minutes_id, role_label(&p.owner_role)))),
        )
        .child(
            div()
                .text_size(size::md())
                .font_weight(weight::SEMIBOLD)
                .text_color(text::primary())
                .child(p.title.clone()),
        )
        .child(
            div()
                .text_size(size::sm())
                .text_color(text::secondary())
                .child(p.why.clone()),
        );
    let act = |button: Button, mark: Mark, is_new: bool| {
        let p = p.clone();
        button.disabled(busy).on_click(cx.listener(move |this, _, _, cx| {
            this.selected = i;
            this.touch(cx);
            this.mark(&p, mark, is_new, cx);
        }))
    };
    card.child(
        div()
            .flex()
            .items_center()
            .gap(space(2))
            .child(act(
                Button::new(("desk-accept", i)).primary().small().label("Accept"),
                Mark::Accepted,
                false,
            ))
            .child(act(
                Button::new(("desk-change", i)).secondary().small().label("Change"),
                Mark::Changed,
                false,
            ))
            .child(act(
                Button::new(("desk-reject-priority", i)).secondary().small().label("Reject"),
                Mark::Rejected,
                false,
            ))
            .child(act(
                Button::new(("desk-new", i)).ghost().small().label("New"),
                Mark::Accepted,
                true,
            )),
    )
}

fn decision_card(
    i: usize,
    d: &PendingDecisionItem,
    product: &str,
    selected: bool,
    busy: bool,
    cx: &mut Context<DeskScreen>,
) -> Card {
    let kind = if d.minutes_id.is_some() { "Decision · minutes" } else { "Decision" };
    let mut card = Card::new(("desk-decision", i))
        // The accent border marks the decision under the cursor.
        .force(if selected { Force::Hover } else { Force::None })
        .child(
            div()
                .flex()
                .items_center()
                .gap(space(2))
                .child(neutral_tag(kind))
                .child(mono(format!("#{}", d.id))),
        )
        .child(
            div()
                .text_size(size::md())
                .font_weight(weight::SEMIBOLD)
                .text_color(text::primary())
                .child(d.text.clone()),
        );
    if let Some(why) = &d.why {
        card = card.child(
            div()
                .text_size(size::sm())
                .text_color(text::secondary())
                .child(why.clone()),
        );
    }
    if let Some(action) = &d.action {
        card = card.child(
            div()
                .text_size(size::sm())
                .text_color(text::tertiary())
                .child(format!("Action: {action}")),
        );
    }
    let open = d.minutes_id.clone().map(|m| {
        let product = product.to_string();
        Button::new(("desk-decision-open", i))
            .ghost()
            .small()
            .icon(IconName::ArrowRight)
            .label(format!("Open {m}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                Nav::open(cx, &this.core, ScreenRequest::minute(&product, &m));
            }))
    });
    let id = d.id;
    // A button moves the cursor to its decision, then acts on it.
    let act = |button: Button, f: fn(&mut DeskScreen, i64, &mut Context<DeskScreen>)| {
        button.on_click(cx.listener(move |this, _, _, cx| {
            this.selected = i;
            this.touch(cx);
            f(this, id, cx);
        }))
    };
    card.child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(space(3))
            .child(mono(format!("agentsi decide {}", d.id)))
            .children(open),
    )
    .child(
        div()
            .flex()
            .items_center()
            .gap(space(2))
            .child(act(
                Button::new(("desk-approve", i))
                    .primary()
                    .small()
                    .label("Approve")
                    .disabled(busy),
                |this, id, cx| this.decide(id, Verdict::Approved, cx),
            ))
            .child(act(
                Button::new(("desk-reject", i))
                    .secondary()
                    .small()
                    .label("Reject")
                    .disabled(busy),
                |this, id, cx| this.decide(id, Verdict::Rejected, cx),
            ))
            .child(act(
                Button::new(("desk-archive", i))
                    .ghost()
                    .small()
                    .label("Archive")
                    .disabled(busy),
                |this, id, cx| this.archive(id, cx),
            )),
    )
}

fn minutes_row(i: usize, m: &MinutesRow, product: &str, cx: &mut Context<DeskScreen>) -> ListRow {
    let (id, product) = (m.id.clone(), product.to_string());
    ListRow::new(("desk-minutes", i), m.question.clone())
        .meta(minutes_meta(m))
        .trailing(minutes_status(m.status.as_deref()))
        .on_click(cx.listener(move |this, _, _, cx| {
            Nav::open(cx, &this.core, ScreenRequest::minute(&product, &id));
        }))
}

impl Render for DeskScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_form(window, cx);
        let main = self.main(cx);
        div()
            .id("desk")
            .track_focus(&self.focus)
            .key_context("Desk")
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, window, cx| {
                // The letters of the owner go to the input of the conversation.
                if !this.conversation.read(cx).input_focused(window, cx) {
                    this.key_down(e, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.touch(cx)),
            )
            .size_full()
            .bg(surface::panel())
            .flex()
            .flex_row()
            .child(main)
            .child(
                div()
                    .flex_none()
                    .w(control::DESK_CONVERSATION_WIDTH)
                    .h_full()
                    .border_l_1()
                    .border_color(line::border())
                    .child(self.conversation.clone()),
            )
    }
}

impl DeskScreen {
    /// The left column: the state of the product. It stays on screen when the data fails to
    /// load, and the conversation at its right still works.
    fn main(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let column_box = || div().id("desk-main").flex_1().min_w_0().h_full();
        let d = match &self.data {
            Load::Loading => {
                return column_box()
                    .child(notice("Reading the front desk…", None))
                    .into_any_element();
            }
            Load::Failed(e) => {
                return column_box()
                    .child(notice(
                        format!("The front desk did not load: {e}"),
                        Some(super::parts::reload_hint(cx)),
                    ))
                    .into_any_element();
            }
            Load::Ready(d) => d,
        };
        let now = format::now_secs();
        let product = self.product.clone();
        let (selected, busy) = (self.selected, self.busy);
        let flash = self.flash.as_ref().map(flash_line);
        let form = self.form.as_ref().map(|(f, _)| f.clone());
        let archived_shown = self.archived.is_some();
        // "Run in Claude" shows after an approval with an action, next to its outcome line.
        let run = self
            .run
            .borrow()
            .clone()
            .filter(|_| matches!(self.flash, Some(Flash::Done(_))));
        let archived_rows = self.archived.as_ref().map(|list| {
            div()
                .flex()
                .flex_col()
                .gap(space(2))
                .child(section("Archived decisions", Some(list.len())))
                .when(list.is_empty(), |el| {
                    el.child(
                        div()
                            .text_size(size::sm())
                            .text_color(text::tertiary())
                            .child("No decision is archived."),
                    )
                })
                .children(list.iter().enumerate().map(|(i, d)| {
                    let id = d.id;
                    ListRow::new(
                        ("desk-archived-row", i),
                        d.text.clone().unwrap_or_else(|| format!("Decision {id}")),
                    )
                    .meta(format!("#{id}"))
                    .trailing(
                        Button::new(SharedString::from(format!("desk-restore-{id}")))
                            .secondary()
                            .small()
                            .label("Restore")
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| this.restore(id, cx))),
                    )
                }))
        });
        let hint = self
            .core
            .read(cx)
            .product(&self.product)
            .and_then(|p| p.path.clone());

        let header = div()
            .flex()
            .flex_col()
            .gap(space(2))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space(3))
                    .child(
                        div()
                            .text_size(size::xl())
                            .font_weight(weight::SEMIBOLD)
                            .text_color(text::primary())
                            .child(d.product.name.clone()),
                    )
                    .child(mono(d.product.id.clone()))
                    .when_some(d.status.as_deref(), |el, s| {
                        el.child(match s {
                            "active" => status_tag("active", Status::Ok),
                            other => neutral_tag(other.to_string()),
                        })
                    }),
            )
            .when_some(hint, |el, p| el.child(mono(p)))
            .when(!d.roles.is_empty(), |el| {
                el.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(space(2))
                        .children(d.roles.iter().map(|r| neutral_tag(role_label(r)))),
                )
            });

        let mut figures = vec![
            stat("Pending items", self.items.len().to_string()),
            stat("Minutes", d.minutes_total.to_string()),
            stat(
                "Index",
                format!(
                    "{} docs · {} commits · {} issues",
                    d.index.docs, d.index.commits, d.index.issues
                ),
            ),
        ];
        if d.usage.rows > 0 {
            figures.push(stat("Cost", format::usd(d.usage.cost_usd)));
        }

        let index_line = {
            let indexed = d
                .index
                .indexed_at
                .as_deref()
                .map_or("never indexed".to_string(), |at| {
                    format!("indexed {}", format::ago(at, now))
                });
            let commit = d
                .index
                .last_commit
                .as_ref()
                .map_or("no commit indexed".to_string(), |c| {
                    format!("last commit {} {}", c.date, c.title)
                });
            format!("{indexed} · {commit}")
        };

        let decisions = div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(section("Pending items", Some(self.items.len())))
                    .child(
                        div()
                            .flex()
                            .gap(space(2))
                            .child(
                                Button::new("desk-index")
                                    .secondary()
                                    .small()
                                    .label("Update the index")
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, _, cx| this.update_index(cx))),
                            )
                            .child(
                                Button::new("desk-new-decision")
                                    .secondary()
                                    .small()
                                    .label("New decision")
                                    .on_click(cx.listener(|this, _, _, cx| this.new_decision(cx))),
                            )
                            .child(
                                Button::new("desk-archived")
                                    .ghost()
                                    .small()
                                    .label(if archived_shown { "Hide archived" } else { "Show archived" })
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.toggle_archived(cx)),
                                    ),
                            ),
                    ),
            )
            .when_some(form, |el, f| el.child(f))
            .when(self.items.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(size::sm())
                        .text_color(text::tertiary())
                        .child("Nothing waits for your approval."),
                )
            })
            .children(flash)
            .when(run.is_some(), |el| {
                el.child(
                    Button::new("desk-run-claude")
                        .secondary()
                        .small()
                        .icon(IconName::Bot)
                        .label("Run in Claude")
                        .on_click(cx.listener(|this, _, _, cx| this.run_in_claude(cx))),
                )
            })
            .children(self.items.iter().enumerate().map(|(i, item)| match item {
                PendingItem::Priority(p) => priority_card(i, p, i == selected, busy, cx),
                PendingItem::Decision(d) => decision_card(i, d, &product, i == selected, busy, cx),
            }))
            .when_some(archived_rows, |el, rows| el.child(rows));

        let objectives = (!d.objectives.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(space(3))
                .child(section("Objectives", Some(d.objectives.len())))
                .children(d.objectives.iter().map(|o| {
                    div()
                        .flex()
                        .flex_col()
                        .gap(space(1))
                        .p(space(4))
                        .rounded(radius::lg())
                        .border_1()
                        .border_color(line::border())
                        .child(
                            div()
                                .text_size(size::base())
                                .font_weight(weight::MEDIUM)
                                .text_color(text::primary())
                                .child(o.text.clone()),
                        )
                        .child(
                            div()
                                .text_size(size::sm())
                                .text_color(text::secondary())
                                .child(format!("Signal: {}", o.signal)),
                        )
                        .child(mono(format!("{} · due {}", o.id, o.due)))
                }))
        });

        let usage = (d.usage.rows > 0).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(space(3))
                .child(section("Usage", None))
                .child(stats(vec![
                    stat("Tokens in", format::compact(d.usage.input_tokens)),
                    stat("Tokens out", format::compact(d.usage.output_tokens)),
                    stat("Cache read", format::compact(d.usage.cache_read)),
                    stat("Cache write", format::compact(d.usage.cache_write)),
                    stat("Cost", format::usd(d.usage.cost_usd)),
                ]))
        });

        let minutes = div()
            .flex()
            .flex_col()
            .gap(space(3))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(section("Last minutes", Some(d.minutes_total as usize)))
                    .child({
                        let product = product.clone();
                        Button::new("desk-all-minutes")
                            .secondary()
                            .small()
                            .label("All minutes")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                Nav::open(cx, &this.core, ScreenRequest::minutes(&product));
                            }))
                    }),
            )
            .when(d.last_minutes.is_empty(), |el| {
                el.child(
                    div()
                        .text_size(size::sm())
                        .text_color(text::tertiary())
                        .child("No meetings yet. Convene one with agentsi meet."),
                )
            })
            .children(
                d.last_minutes
                    .iter()
                    .enumerate()
                    .map(|(i, m)| minutes_row(i, m, &product, cx)),
            );

        column_box()
            .track_scroll(&self.scroll)
            .overflow_y_scroll()
            .child(
                column()
                    .child(header)
                    .child(stats(figures))
                    .child(decisions)
                    .child(self.tickets.clone())
                    .children(objectives)
                    .children(usage)
                    .child(minutes)
                    .child(mono(index_line)),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{DeskResult, DeskScreen, Live, Load, ScreenRequest};
    use crate::link::Link;
    use crate::model::CoreState;
    use gpui::{TestAppContext, VisualTestContext};
    use gpui_kit::{AppContext as _, Entity};
    use std::sync::Arc;

    const DESK: &str = r#"{
        "product": {"id": "demo", "name": "Demo", "path": "/src/demo"},
        "status": "active", "roles": ["pm", "dev.backend"],
        "index": {"docs": 3, "commits": 20, "issues": 1, "indexed_at": "2026-11-02T10:00:00.000Z",
                  "last_commit": {"date": "2026-11-01", "title": "fix a thing"}},
        "objectives": [{"id": "o1", "text": "Ship", "signal": "a release", "due": "2026-12-01"}],
        "pending_decisions": [
            {"id": 7, "kind": "minutes", "minutes_id": "m1", "reflection_id": null,
             "text": "Ship it", "why": "because", "action": "do it"},
            {"id": 8, "kind": "product", "minutes_id": null, "reflection_id": null,
             "text": "Another", "why": null, "action": null}],
        "minutes_total": 1,
        "last_minutes": [{"id": "m1", "question": "What now?", "status": "valid",
             "created_at": "2026-11-02T10:00:00.000Z", "scope": null,
             "marks": {"accepted": 1, "changed": 0, "rejected": 0, "unmarked": 1}, "heard": true}],
        "usage": {"rows": 2, "input_tokens": 1500, "output_tokens": 20, "cache_read": 0,
                  "cache_write": 0, "cost_usd": 0.5}
    }"#;

    /// Sets the desk data and the pending items: the decisions of `product.desk` as items.
    fn load(s: &mut DeskScreen, d: DeskResult) {
        use agentsi_proto::marks_api::{PendingDecisionItem, PendingItem};
        s.items = d
            .pending_decisions
            .iter()
            .map(|p| {
                PendingItem::Decision(PendingDecisionItem {
                    id: p.id,
                    minutes_id: p.minutes_id.clone(),
                    text: p.text.clone(),
                    why: p.why.clone(),
                    action: p.action.clone(),
                    priorities: vec![],
                })
            })
            .collect();
        s.data = Load::Ready(d);
    }

    fn screen(cx: &mut TestAppContext, data: Load<DeskResult>) -> gpui::Entity<DeskScreen> {
        cx.update(agentsi_ui::init);
        let (end, _other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| DeskScreen::new("demo".into(), core, cx));
        screen.update(cx, |s, cx| {
            s.data = data;
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        screen
    }

    #[gpui::test]
    fn the_desk_draws_in_each_state(cx: &mut TestAppContext) {
        screen(cx, Load::Loading);
        screen(cx, Load::Failed("no core".into()));
        screen(cx, Load::Ready(serde_json::from_str(DESK).unwrap()));
        // A product with nothing: no decisions, objectives, usage or minutes.
        let mut empty: DeskResult = serde_json::from_str(DESK).unwrap();
        empty.pending_decisions.clear();
        empty.objectives.clear();
        empty.last_minutes.clear();
        empty.minutes_total = 0;
        empty.usage.rows = 0;
        empty.roles.clear();
        empty.index.indexed_at = None;
        empty.index.last_commit = None;
        screen(cx, Load::Ready(empty));
    }

    /// The reads of the parts of the desk: the tickets and the desk conversation. A test of
    /// the left column does not count them.
    fn is_part_read(method: &str) -> bool {
        matches!(
            method,
            "ticket.list" | "mark.pending" | "desk.history" | "decision.get"
        )
    }

    /// The next request that the screen sent to the core, as JSON, or `None`. The read of the
    /// tickets part of the desk (`desk_tickets.rs`) is not counted.
    fn sent(other: &mut std::os::unix::net::UnixStream) -> Option<serde_json::Value> {
        other.set_nonblocking(true).unwrap();
        loop {
            let frame = agentsi_proto::frame::read_frame(other).ok().flatten()?;
            let request: serde_json::Value = serde_json::from_slice(&frame.payload).ok()?;
            if !is_part_read(request["method"].as_str().unwrap_or_default()) {
                return Some(request);
            }
        }
    }

    /// The methods that the core received, without the read of the tickets part.
    fn methods(socket: &mut std::os::unix::net::UnixStream) -> Vec<String> {
        crate::model::two_cores::methods(socket)
            .into_iter()
            .filter(|m| !is_part_read(m))
            .collect()
    }

    #[gpui::test]
    fn a_key_sends_the_write_for_the_decision_under_the_cursor_and_a_second_waits(
        cx: &mut TestAppContext,
    ) {
        use agentsi_proto::marks_api::Verdict;
        use serde_json::json;
        cx.update(agentsi_ui::init);
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| DeskScreen::new("demo".into(), core, cx));
        // The new screen asks for its data first.
        assert_eq!(sent(&mut other).unwrap()["method"], "product.desk");
        screen.update(cx, |s, cx| {
            load(s, serde_json::from_str(DESK).unwrap());
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.focus_next(cx));
        assert!(cx.update(|window, cx| screen.read(cx).focus_handle().is_focused(window)));

        cx.simulate_keystrokes("down r");
        let request = sent(&mut other).expect("a request");
        assert_eq!(request["method"], "decision.decide");
        assert_eq!(
            request["params"],
            json!({"decision": 8, "status": "rejected"})
        );
        // The first write has no reply yet, so a second key sends nothing.
        cx.simulate_keystrokes("up a");
        assert!(sent(&mut other).is_none());
        screen.update(cx, |s, _| s.busy = false);
        screen.update(cx, |s, cx| s.decide_selected(Verdict::Approved, cx));
        let request = sent(&mut other).expect("a request");
        assert_eq!(
            request["params"],
            json!({"decision": 7, "status": "approved"})
        );
    }

    #[gpui::test]
    fn x_archives_the_decision_and_a_reflection_decision_is_decided_here(cx: &mut TestAppContext) {
        use serde_json::json;
        cx.update(agentsi_ui::init);
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| DeskScreen::new("demo".into(), core, cx));
        sent(&mut other);
        let mut desk: DeskResult = serde_json::from_str(DESK).unwrap();
        desk.pending_decisions[1].kind = "reflection".into();
        screen.update(cx, |s, cx| {
            load(s, desk);
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.focus_next(cx));

        cx.simulate_keystrokes("x");
        let request = sent(&mut other).expect("a request");
        assert_eq!(request["method"], "archive.archive");
        assert_eq!(
            request["params"],
            json!({"product": "demo", "kind": "decision", "id": "7"})
        );
        screen.update(cx, |s, _| s.busy = false);
        cx.simulate_keystrokes("down a");
        let request = sent(&mut other).expect("a request");
        assert_eq!(request["method"], "decision.decide");
        assert_eq!(
            request["params"],
            json!({"decision": 8, "status": "approved"})
        );
    }

    #[test]
    fn a_decision_that_the_core_did_not_apply_is_an_error_line() {
        use agentsi_proto::marks_api::{DecideResult, Outcome};
        let reply = |status| DecideResult {
            product: "demo".into(),
            minutes_id: None,
            status,
            action: None,
            commit_error: None,
            refused: None,
        };
        assert!(super::applied(8, &reply(Outcome::Approved)).is_ok());
        assert!(super::applied(8, &reply(Outcome::Rejected)).is_ok());
        let stale = super::applied(8, &reply(Outcome::Stale)).unwrap_err();
        assert_eq!(
            stale.to_string(),
            "Decision 8 not applied: the target changed since the proposal, so it is now stale."
        );
        let refused = DecideResult {
            refused: Some("Decision 8 rejected: no proposal.".into()),
            ..reply(Outcome::Rejected)
        };
        let line = super::applied(8, &refused).unwrap_err();
        assert_eq!(line.to_string(), "Decision 8 rejected: no proposal.");
    }

    /// The front desks of product `demo` on this device and on `studio`, with their data.
    fn desks_of_the_same_id(
        cx: &mut TestAppContext,
    ) -> (
        crate::model::two_cores::TwoCores,
        Entity<DeskScreen>,
        Entity<DeskScreen>,
        &mut VisualTestContext,
    ) {
        use crate::model::two_cores::two_cores;
        cx.update(agentsi_ui::init);
        cx.update(crate::actions::bind);
        let mut t = two_cores(cx);
        let (local, studio) = (
            t.cores.of(None).unwrap().clone(),
            t.cores.of(Some("studio")).unwrap().clone(),
        );
        let (screen, cx) = cx.add_window_view(|_, cx| DeskScreen::new("demo".into(), local, cx));
        let remote = cx.update(|_, cx| cx.new(|cx| DeskScreen::new("demo".into(), studio, cx)));
        for s in [&screen, &remote] {
            s.update(cx, |s, cx| {
                load(s, serde_json::from_str(DESK).unwrap());
                cx.notify();
            });
        }
        assert_eq!(methods(&mut t.local), ["product.desk"]);
        assert_eq!(methods(&mut t.studio), ["product.desk"]);
        (t, screen, remote, cx)
    }

    #[gpui::test]
    fn a_decision_lands_on_the_core_of_the_screen_when_two_cores_have_the_same_product(
        cx: &mut TestAppContext,
    ) {
        use crate::model::two_cores::received;
        let (mut t, local, remote, cx) = desks_of_the_same_id(cx);
        // The remote desk is the newest, so the palette action acts on it.
        cx.dispatch_action(crate::actions::ApproveDecision);
        let sent = received(&mut t.studio);
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0]["method"], "decision.decide");
        assert_eq!(methods(&mut t.local), Vec::<String>::new());

        // The owner uses the local desk: now the same action reaches only the local core.
        local.update(cx, |s, cx| s.stamp = Live::tick(cx));
        remote.update(cx, |s, _| s.busy = false);
        cx.dispatch_action(crate::actions::RejectDecision);
        assert_eq!(methods(&mut t.local), ["decision.decide"]);
        assert_eq!(methods(&mut t.studio), Vec::<String>::new());
    }

    #[gpui::test]
    fn a_palette_action_follows_the_tab_that_the_owner_selected(cx: &mut TestAppContext) {
        use crate::model::two_cores::two_cores;
        use crate::screens::{ScreenPanel, ScreenSpec};
        use gpui_kit::component::dock::BasePanel as _;
        cx.update(agentsi_ui::init);
        cx.update(crate::actions::bind);
        let mut t = two_cores(cx);
        let product = |host: Option<&str>| crate::model::ProductRef {
            host: host.map(String::from),
            id: "demo".into(),
        };
        let (local, studio) = (
            t.cores.of(None).unwrap().clone(),
            t.cores.of(Some("studio")).unwrap().clone(),
        );
        let spec = ScreenSpec::desk(&product(None));
        let (first, cx) = cx.add_window_view(|_, cx| ScreenPanel::new(spec, local, cx));
        let spec = ScreenSpec::desk(&product(Some("studio")));
        let second = cx.update(|_, cx| cx.new(|cx| ScreenPanel::new(spec, studio, cx)));
        let desks: Vec<Entity<DeskScreen>> = [&first, &second]
            .iter()
            .map(|p| p.read_with(cx, |p, _| p.desk().unwrap()))
            .collect();
        for d in &desks {
            d.update(cx, |s, _| {
                load(s, serde_json::from_str(DESK).unwrap())
            });
        }
        methods(&mut t.local);
        methods(&mut t.studio);
        // The owner uses the remote desk, then selects a tab that is no desk: no desk is in
        // view, so a palette write reaches no core.
        second.update_in(cx, |p, window, cx| p.set_active(true, window, cx));
        second.update_in(cx, |p, window, cx| p.set_active(false, window, cx));
        cx.dispatch_action(crate::actions::ApproveDecision);
        assert_eq!(methods(&mut t.studio), Vec::<String>::new());
        assert_eq!(methods(&mut t.local), Vec::<String>::new());
        // The remote desk was used last, but the local desk is the one in view.
        first.update_in(cx, |p, window, cx| p.set_active(true, window, cx));
        cx.dispatch_action(crate::actions::ApproveDecision);
        assert_eq!(methods(&mut t.local), ["decision.decide"]);
        assert_eq!(methods(&mut t.studio), Vec::<String>::new());
    }

    #[gpui::test]
    fn a_reload_after_a_write_stays_on_the_core_of_the_write(cx: &mut TestAppContext) {
        let (mut t, _local, _remote, cx) = desks_of_the_same_id(cx);
        let studio = t.cores.of(Some("studio")).unwrap().clone();
        cx.update(|_, cx| Live::reload_core(&studio, cx));
        assert_eq!(methods(&mut t.studio), ["product.desk"]);
        assert_eq!(methods(&mut t.local), Vec::<String>::new());
    }

    #[gpui::test]
    fn a_screen_of_a_host_opens_its_minutes_on_the_same_host(cx: &mut TestAppContext) {
        let (t, _local, remote, cx) = desks_of_the_same_id(cx);
        let (nav, mut requests) = crate::nav::Nav::channel();
        cx.update(|_, cx| cx.set_global(nav));
        remote.update(cx, |s, cx| {
            crate::nav::Nav::open(cx, &s.core, ScreenRequest::minutes("demo"))
        });
        let spec = requests.try_recv().unwrap();
        assert_eq!(spec.host.as_deref(), Some("studio"));
        let panel = cx.update(|_, cx| {
            let core = t.cores.of(spec.host.as_deref()).unwrap().clone();
            cx.new(|cx| crate::screens::ScreenPanel::new(spec.clone(), core, cx))
        });
        let (same, other_host) = panel.read_with(cx, |p, _| {
            (p.is(&spec), p.is(&ScreenRequest::minutes("demo")))
        });
        assert!(same);
        assert!(
            !other_host,
            "the local screen of the same id is another panel"
        );
        assert!(panel.read_with(cx, |p, _| p.follows(Some("studio"))));
        assert!(!panel.read_with(cx, |p, _| p.follows(None)));
    }

    #[gpui::test]
    fn a_palette_action_acts_on_the_front_desk_that_the_owner_used_last(cx: &mut TestAppContext) {
        use serde_json::json;
        cx.update(agentsi_ui::init);
        cx.update(crate::actions::bind);
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| DeskScreen::new("demo".into(), core, cx));
        sent(&mut other);
        screen.update(cx, |s, cx| {
            load(s, serde_json::from_str(DESK).unwrap());
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        // The focus is not on the screen, as after the palette closes on a pane.
        cx.dispatch_action(crate::actions::ApproveDecision);
        let request = sent(&mut other).expect("a request");
        assert_eq!(request["method"], "decision.decide");
        assert_eq!(
            request["params"],
            json!({"decision": 7, "status": "approved"})
        );
    }

    /// A desk with the requests of its first read taken, and two priorities and a decision.
    fn desk_with_items(
        cx: &mut TestAppContext,
    ) -> (
        Entity<DeskScreen>,
        std::os::unix::net::UnixStream,
        &mut VisualTestContext,
    ) {
        use agentsi_proto::marks_api::PendingItem;
        use serde_json::json;
        cx.update(agentsi_ui::init);
        cx.update(crate::actions::bind);
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| DeskScreen::new("demo".into(), core, cx));
        // The desk reads the pending items with its data.
        other.set_nonblocking(true).unwrap();
        let mut asked = Vec::new();
        while let Some(f) = agentsi_proto::frame::read_frame(&mut other).ok().flatten() {
            let v: serde_json::Value = serde_json::from_slice(&f.payload).unwrap();
            asked.push(v["method"].as_str().unwrap().to_string());
        }
        assert!(asked.contains(&"mark.pending".to_string()), "{asked:?}");
        let items: Vec<PendingItem> = serde_json::from_value(json!([
            {"kind": "priority", "minutes_id": "m1", "idx": 0, "title": "Fix it", "why": "w",
             "owner_role": "dev", "objective": null, "sources": []},
            {"kind": "priority", "minutes_id": "m1", "idx": 1, "title": "Ship it", "why": "w",
             "owner_role": "pm", "objective": null, "sources": []},
            {"kind": "decision", "id": 7, "minutes_id": "m1", "text": "Go", "why": "y",
             "action": "do it", "priorities": []}]))
        .unwrap();
        screen.update(cx, |s, cx| {
            load(s, serde_json::from_str(DESK).unwrap());
            s.items = items;
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.focus_next(cx));
        (screen, other, cx)
    }

    #[gpui::test]
    fn the_keys_mark_a_priority_and_decide_a_decision_in_the_order_of_the_core(
        cx: &mut TestAppContext,
    ) {
        use serde_json::json;
        let (screen, mut other, cx) = desk_with_items(cx);
        let mark = |other: &mut std::os::unix::net::UnixStream| sent(other).expect("a request");
        cx.simulate_keystrokes("c");
        let r = mark(&mut other);
        assert_eq!(r["method"], "mark.set");
        assert_eq!(
            r["params"]["entries"],
            json!([{"minutes_id": "m1", "idx": 0, "mark": "changed", "is_new": false}])
        );
        screen.update(cx, |s, _| s.busy = false);
        cx.simulate_keystrokes("down n");
        let r = mark(&mut other);
        assert_eq!(
            r["params"]["entries"],
            json!([{"minutes_id": "m1", "idx": 1, "mark": "accepted", "is_new": true}])
        );
        screen.update(cx, |s, _| s.busy = false);
        cx.simulate_keystrokes("down a");
        let r = mark(&mut other);
        assert_eq!(r["method"], "decision.decide");
        assert_eq!(r["params"], json!({"decision": 7, "status": "approved"}));
    }

    #[gpui::test]
    fn the_buttons_of_the_desk_call_the_index_the_decision_form_and_the_archive(
        cx: &mut TestAppContext,
    ) {
        use agentsi_proto::marks_api::ArchivedDecision;
        use gpui::Modifiers;
        use serde_json::json;
        let (screen, mut other, cx) = desk_with_items(cx);
        let click = |cx: &mut VisualTestContext, sel: &'static str| {
            let b = cx.debug_bounds(sel).unwrap_or_else(|| panic!("{sel}"));
            cx.simulate_click(b.center(), Modifiers::none());
            cx.update(|window, cx| window.draw(cx).clear(cx));
        };
        click(cx, "button:desk-index");
        let r = sent(&mut other).unwrap();
        assert_eq!(r["method"], "knowledge.index");
        assert_eq!(r["params"], json!({"product": "demo", "force": false}));
        screen.update(cx, |s, _| s.busy = false);

        // The form: the button stays off with no text, and Enter in the last field adds.
        click(cx, "button:desk-new-decision");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("form:desk-new-decision").is_some());
        let form = screen.read_with(cx, |s, _| s.form.as_ref().unwrap().0.clone());
        form.update(cx, |f, cx| f.submit(cx));
        assert!(sent(&mut other).is_none(), "an empty form writes nothing");
        form.update_in(cx, |f, w, cx| {
            f.set_value(0, "Ship", w, cx);
            f.set_value(1, "Because", w, cx);
            f.submit(cx);
        });
        let r = sent(&mut other).unwrap();
        assert_eq!(r["method"], "decision.add");
        assert_eq!(
            r["params"],
            json!({"product": "demo", "text": "Ship", "why": "Because"})
        );
        assert!(screen.read_with(cx, |s, _| s.form.is_none()));
        screen.update(cx, |s, _| s.busy = false);

        // Show archived reads the list. Restore sends the id.
        click(cx, "button:desk-archived");
        assert_eq!(sent(&mut other).unwrap()["method"], "decision.archived");
        let archived: ArchivedDecision = serde_json::from_value(json!({
            "id": 3, "kind": "decision", "minutes_id": null, "text": "Old", "why": null,
            "action": null, "status": "pending"}))
        .unwrap();
        screen.update(cx, |s, cx| {
            s.archived = Some(vec![archived]);
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        click(cx, "button:desk-restore-3");
        let r = sent(&mut other).unwrap();
        assert_eq!(r["method"], "archive.restore");
        assert_eq!(
            r["params"],
            json!({"product": "demo", "kind": "decision", "id": "3"})
        );
    }

    #[gpui::test]
    fn run_in_claude_shows_after_an_approval_with_an_action_and_asks_for_a_pane(
        cx: &mut TestAppContext,
    ) {
        use gpui::Modifiers;
        let (screen, _other, cx) = desk_with_items(cx);
        let (runs, mut rx) = crate::nav::Runs::channel();
        cx.update(|_, cx| cx.set_global(runs));
        assert!(cx.debug_bounds("button:desk-run-claude").is_none());
        screen.update(cx, |s, cx| {
            *s.run.borrow_mut() = Some("do it".into());
            s.flash = Some(super::Flash::Done("Decision 7 approved.".into()));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let b = cx.debug_bounds("button:desk-run-claude").expect("the button");
        cx.simulate_click(b.center(), Modifiers::none());
        let run = rx.try_recv().expect("a request");
        assert_eq!((run.product.as_str(), run.text.as_str()), ("demo", "do it"));
    }
}
