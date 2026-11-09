//! The desk conversation: the right column of the front desk. The owner asks Claude about the
//! product. A reply streams in as Markdown. A tool use or a desk note is one muted line.
//! A view that Claude shows is a native part that reads the stored objects from the core.
//!
//! The conversation holds no decision text that Claude wrote. The decisions view keeps the ids
//! and reads the rows with `decision.get`. It reads them again after `data.changed`, so a
//! decision that the owner settles in the `/agentsi` pane changes here too. Approve and Reject
//! ask a Confirm, then call `decision.decide`. Claude has no tool that writes.
//!
//! Spec: `docs/specs/agentsi-desktop-conversation.md`.

use std::collections::HashMap;

use agentsi_proto::Event as _;
use agentsi_proto::desk_session_api::{
    DeskAuth, DeskAuthParams, DeskAuthResult, DeskDelta, DeskEntry, DeskHistory, DeskHistoryParams,
    DeskHistoryResult, DeskItem, DeskItemEvent, DeskProductParams, DeskReset, DeskResetResult,
    DeskSend, DeskSendParams, DeskState, DeskStateEvent, DeskStop, View,
};
use agentsi_proto::events::{DataChanged, Lagged};
use agentsi_proto::marks_api::{
    DecideParams, DecideResult, DecisionDecide, DecisionGet, DecisionGetParams, DecisionInfo,
    Verdict,
};
use agentsi_proto::rpc::Notification;
use agentsi_ui::components::{Button, Card, StateMark, neutral_tag};
use agentsi_ui::tokens::{WorkState, font, line, radius, size, space, surface, text, weight};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::message_scroller::{MessageScroller, MessageScrollerState};
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::memory::decision_flash;
use super::parts::{Flash, Live, Load, flash_line, mono, notice};
use crate::format;
use crate::link::CallError;
use crate::model::{CoreEvent, CoreState, DeskNotice};

/// The text of the first suggestion in an empty conversation.
const ASK_PENDING: &str = "What waits for me?";

/// One row of the conversation.
enum Row {
    Owner(String),
    /// `seq` is `None` while the reply streams: the `reply` item has not come yet.
    Reply {
        seq: Option<i64>,
        text: Entity<TextViewState>,
    },
    Tool(String),
    View(View),
    Note(String),
    End {
        cost_usd: f64,
        duration_ms: u64,
        error: Option<String>,
    },
    Error(String),
}

/// What waits for the Confirm of the owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Confirm {
    Decide(i64, Verdict),
    NewConversation,
}

/// An event that came before the history: the conversation applies it after the history.
enum Early {
    State(DeskState, Option<String>),
    Delta(String),
    Item(i64, DeskItem),
}

pub struct DeskConversation {
    product: String,
    core: Entity<CoreState>,
    history: Load<()>,
    state: DeskState,
    /// What the state needs: the auth message or the failure.
    message: Option<String>,
    rows: Vec<Row>,
    /// The index in `rows` of the reply that streams.
    open: Option<usize>,
    last_seq: i64,
    early: Vec<Early>,
    scroller: Entity<MessageScrollerState>,
    input: Option<Entity<InputState>>,
    /// The next draw focuses the input.
    want_focus: bool,
    /// The next draw empties the input: the core took the message.
    clear_input: bool,
    /// The decisions of the views, as the core stored them.
    decisions: HashMap<i64, DecisionInfo>,
    decisions_read: bool,
    confirm: Option<Confirm>,
    /// A write or a login check waits for the core.
    busy: bool,
    /// `desk.send` waits for the core.
    sending: bool,
    flash: Option<Flash>,
    _subs: Vec<Subscription>,
}

impl DeskConversation {
    pub fn new(product: String, core: Entity<CoreState>, cx: &mut Context<Self>) -> Self {
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let notices = cx.subscribe(&core, |this, _, e: &DeskNotice, cx| this.take(&e.0, cx));
        // A host that came back may have sent events that this view missed.
        let up = cx.subscribe(&core, |this, _, e: &CoreEvent, cx| {
            if *e == CoreEvent::Up {
                this.reload(cx);
            }
        });
        let mut me = DeskConversation {
            product,
            core,
            history: Load::Loading,
            state: DeskState::Stopped,
            message: None,
            rows: Vec::new(),
            open: None,
            last_seq: 0,
            early: Vec::new(),
            scroller,
            input: None,
            want_focus: false,
            clear_input: false,
            decisions: HashMap::new(),
            decisions_read: false,
            confirm: None,
            busy: false,
            sending: false,
            flash: None,
            _subs: vec![notices, up],
        };
        me.reload(cx);
        me
    }

    /// The input has the focus, so the letter keys of the front desk must do nothing.
    pub fn input_focused(&self, window: &Window, cx: &App) -> bool {
        self.input
            .as_ref()
            .is_some_and(|i| i.read(cx).focus_handle(cx).is_focused(window))
    }

    /// The palette entry "Ask the front desk": the next draw focuses the input.
    pub fn ask(&mut self, cx: &mut Context<Self>) {
        self.want_focus = true;
        cx.notify();
    }

    // ---- Reading ----------------------------------------------------------------------

    /// Reads the stored conversation and the state. The rows stay until the reply comes.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<DeskHistory>(&DeskHistoryParams {
                product: self.product.clone(),
                before: None,
                limit: None,
            });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| this.history_done(result, cx));
        })
        .detach();
    }

    pub(crate) fn history_done(
        &mut self,
        result: Result<DeskHistoryResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        let r = match result {
            Ok(r) => r,
            Err(e) => {
                self.history = Load::Failed(e.to_string());
                cx.notify();
                return;
            }
        };
        self.rows.clear();
        self.open = None;
        self.last_seq = 0;
        for DeskEntry { seq, item, .. } in r.items {
            self.last_seq = self.last_seq.max(seq);
            let row = self.row_of(seq, item, cx);
            self.rows.push(row);
        }
        self.state = r.state;
        self.message = r.message;
        self.history = Load::Ready(());
        let n = self.rows.len();
        self.scroller.update(cx, |s, cx| s.reset(n, cx));
        for early in std::mem::take(&mut self.early) {
            match early {
                Early::State(state, message) => self.set_state(state, message, cx),
                Early::Delta(text) => self.delta(&text, cx),
                Early::Item(seq, item) => self.item(seq, item, cx),
            }
        }
        self.read_decisions(cx);
        cx.notify();
    }

    /// Reads the decisions of each decisions view.
    fn read_decisions(&mut self, cx: &mut Context<Self>) {
        let mut ids: Vec<i64> = self
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::View(View::Decisions { ids }) => Some(ids.iter().copied()),
                _ => None,
            })
            .flatten()
            .collect();
        ids.sort_unstable();
        ids.dedup();
        if ids.is_empty() {
            return;
        }
        let call = self
            .core
            .read(cx)
            .link
            .call::<DecisionGet>(&DecisionGetParams {
                product: self.product.clone(),
                ids,
            });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| this.decisions_done(result, cx));
        })
        .detach();
    }

    pub(crate) fn decisions_done(
        &mut self,
        result: Result<agentsi_proto::marks_api::DecisionGetResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(r) => {
                self.decisions = r.decisions.into_iter().map(|d| (d.id, d)).collect();
                self.decisions_read = true;
            }
            Err(e) => self.flash = Some(Flash::Failed(format!("The decisions did not load: {e}"))),
        }
        // The rows of the views changed height.
        let n = self.rows.len();
        self.scroller.update(cx, |s, cx| {
            s.remeasure_items(0..n, cx);
        });
        cx.notify();
    }

    // ---- Events -----------------------------------------------------------------------

    /// One event of the core. It acts only on an event of this product.
    pub(crate) fn take(&mut self, n: &Notification, cx: &mut Context<Self>) {
        if n.method == Lagged::NAME {
            self.reload(cx);
            return;
        }
        let product = n.params.get("product").and_then(|p| p.as_str());
        if product.is_some_and(|p| p != self.product) {
            return;
        }
        if n.method == DataChanged::NAME {
            match n.params.get("kind").and_then(|k| k.as_str()) {
                Some("decision" | "minutes") => self.read_decisions(cx),
                // A clear deleted the desk rows of the product.
                Some("clear") => self.reload(cx),
                _ => {}
            }
            return;
        }
        if let Some(p) = n.parse::<DeskStateEvent>() {
            self.when_loaded(Early::State(p.state, p.message), cx);
        } else if let Some(p) = n.parse::<DeskDelta>() {
            self.when_loaded(Early::Delta(p.text), cx);
        } else if let Some(p) = n.parse::<DeskItemEvent>() {
            self.when_loaded(Early::Item(p.seq, p.item), cx);
        }
    }

    /// Applies the event now, or after the history when the history has not come.
    fn when_loaded(&mut self, e: Early, cx: &mut Context<Self>) {
        if !matches!(self.history, Load::Ready(())) {
            self.early.push(e);
            return;
        }
        match e {
            Early::State(state, message) => self.set_state(state, message, cx),
            Early::Delta(text) => self.delta(&text, cx),
            Early::Item(seq, item) => self.item(seq, item, cx),
        }
    }

    fn set_state(&mut self, state: DeskState, message: Option<String>, cx: &mut Context<Self>) {
        self.state = state;
        self.message = message;
        cx.notify();
    }

    fn push(&mut self, row: Row, cx: &mut Context<Self>) {
        self.rows.push(row);
        self.scroller.update(cx, |s, cx| {
            s.append(1, cx);
        });
    }

    fn remeasure(&mut self, i: usize, cx: &mut Context<Self>) {
        self.scroller.update(cx, |s, cx| {
            s.remeasure_items(i..i + 1, cx);
        });
    }

    /// One text delta of the reply that streams.
    pub(crate) fn delta(&mut self, text: &str, cx: &mut Context<Self>) {
        let i = match self.open {
            Some(i) => i,
            None => {
                let state = cx.new(|cx| TextViewState::markdown("", cx));
                self.push(
                    Row::Reply {
                        seq: None,
                        text: state,
                    },
                    cx,
                );
                let i = self.rows.len() - 1;
                self.open = Some(i);
                i
            }
        };
        if let Row::Reply { text: state, .. } = &self.rows[i] {
            state.update(cx, |s, cx| s.push_str(text, cx));
        }
        self.remeasure(i, cx);
        cx.notify();
    }

    /// One stored item. The streamed text of a reply gives way to the text of its item.
    pub(crate) fn item(&mut self, seq: i64, item: DeskItem, cx: &mut Context<Self>) {
        if seq <= self.last_seq {
            return;
        }
        self.last_seq = seq;
        match item {
            DeskItem::Reply { text } => {
                if let Some(i) = self.open.take()
                    && let Row::Reply {
                        seq: s,
                        text: state,
                    } = &mut self.rows[i]
                {
                    *s = Some(seq);
                    state.update(cx, |st, cx| st.set_text(&text, cx));
                    self.remeasure(i, cx);
                } else {
                    let row = self.row_of(seq, DeskItem::Reply { text }, cx);
                    self.push(row, cx);
                }
            }
            other => {
                // A reply that never got its item stays as it streamed.
                self.open = None;
                let is_view = matches!(other, DeskItem::View { .. });
                let row = self.row_of(seq, other, cx);
                self.push(row, cx);
                if is_view {
                    self.read_decisions(cx);
                }
            }
        }
        cx.notify();
    }

    fn row_of(&self, seq: i64, item: DeskItem, cx: &mut Context<Self>) -> Row {
        match item {
            DeskItem::Owner { text } => Row::Owner(text),
            DeskItem::Reply { text } => Row::Reply {
                seq: Some(seq),
                text: cx.new(|cx| TextViewState::markdown(&text, cx)),
            },
            DeskItem::Tool { line } => Row::Tool(line),
            DeskItem::View { view } => Row::View(view),
            DeskItem::Note { text } => Row::Note(text),
            DeskItem::End {
                cost_usd,
                duration_ms,
                error,
            } => Row::End {
                cost_usd,
                duration_ms,
                error,
            },
            DeskItem::Error { message } => Row::Error(message),
        }
    }

    // ---- Writing ----------------------------------------------------------------------

    /// The owner can send a message: no reply runs, and the login is not the problem.
    pub fn can_send(&self) -> bool {
        matches!(self.history, Load::Ready(()))
            && matches!(
                self.state,
                DeskState::Stopped | DeskState::Idle | DeskState::Failed
            )
            && !self.sending
    }

    /// Sends one owner message. The core stores it and sends it back as a `desk.item`.
    pub fn send(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = text.trim().to_string();
        if text.is_empty() || !self.can_send() {
            return;
        }
        self.sending = true;
        self.flash = None;
        let call = self.core.read(cx).link.call::<DeskSend>(&DeskSendParams {
            product: self.product.clone(),
            text,
        });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| {
                this.sending = false;
                match result {
                    Ok(_) => this.clear_input = true,
                    Err(e) => this.flash = Some(Flash::Failed(e.to_string())),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Stops the reply. The next owner message resumes the session.
    pub fn stop(&mut self, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<DeskStop>(&DeskProductParams {
                product: self.product.clone(),
            });
        cx.spawn(async move |_, _| {
            let _ = call.await;
        })
        .detach();
    }

    /// "Check again": runs the login check of the core, then reads the state.
    pub fn recheck(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.flash = None;
        let call = self
            .core
            .read(cx)
            .link
            .call::<DeskAuth>(&DeskAuthParams { recheck: true });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| this.recheck_done(result, cx));
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn recheck_done(
        &mut self,
        result: Result<DeskAuthResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        match result {
            Ok(r)
                if r.message.is_none()
                    && r.state == agentsi_proto::desk_session_api::AuthState::Ready =>
            {
                self.reload(cx)
            }
            Ok(r) => {
                self.state = DeskState::Auth;
                self.message = r.message.or(self.message.take());
            }
            Err(e) => self.flash = Some(Flash::Failed(e.to_string())),
        }
        cx.notify();
    }

    /// Asks the Confirm of Approve or Reject on a decision of a view.
    pub fn ask_decide(&mut self, id: i64, verdict: Verdict, cx: &mut Context<Self>) {
        self.confirm = Some(Confirm::Decide(id, verdict));
        cx.notify();
    }

    pub fn ask_reset(&mut self, cx: &mut Context<Self>) {
        self.confirm = Some(Confirm::NewConversation);
        cx.notify();
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        self.confirm = None;
        cx.notify();
    }

    /// The owner clicked Confirm.
    pub fn confirmed(&mut self, cx: &mut Context<Self>) {
        match self.confirm.take() {
            Some(Confirm::Decide(id, verdict)) => self.decide(id, verdict, cx),
            Some(Confirm::NewConversation) => self.reset(cx),
            None => {}
        }
        cx.notify();
    }

    fn decide(&mut self, id: i64, verdict: Verdict, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.flash = None;
        let call = self
            .core
            .read(cx)
            .link
            .call::<DecisionDecide>(&DecideParams {
                decision: id,
                status: verdict,
            });
        let core = self.core.clone();
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| this.decide_done(id, result, cx));
            // The left column of the front desk reads the new data too.
            cx.update(|cx| Live::reload_core(&core, cx));
        })
        .detach();
    }

    pub(crate) fn decide_done(
        &mut self,
        id: i64,
        result: Result<DecideResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        self.flash = Some(match result {
            Ok(reply) => decision_flash(id, &reply),
            Err(e) => Flash::Failed(e.to_string()),
        });
        // The view shows the stored status. The owner needs no new message for it.
        self.read_decisions(cx);
        cx.notify();
    }

    fn reset(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let call = self
            .core
            .read(cx)
            .link
            .call::<DeskReset>(&DeskProductParams {
                product: self.product.clone(),
            });
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| this.reset_done(result, cx));
        })
        .detach();
    }

    pub(crate) fn reset_done(
        &mut self,
        result: Result<DeskResetResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        match result {
            Ok(_) => {
                self.decisions.clear();
                self.decisions_read = false;
                self.flash = None;
                self.reload(cx);
            }
            Err(e) => self.flash = Some(Flash::Failed(e.to_string())),
        }
        cx.notify();
    }

    fn ensure_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.input.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Ask the front desk"));
            let sub = cx.subscribe(&input, |this, source, e: &InputEvent, cx| {
                if let InputEvent::PressEnter { .. } = e {
                    let text = source.read(cx).value().to_string();
                    this.send(&text, cx);
                }
            });
            self._subs.push(sub);
            self.input = Some(input);
        }
        let Some(input) = self.input.clone() else {
            return;
        };
        if self.clear_input {
            self.clear_input = false;
            input.update(cx, |s, cx| s.set_value("", window, cx));
        }
        if self.want_focus && self.can_send() {
            self.want_focus = false;
            let focus = input.read(cx).focus_handle(cx);
            window.defer(cx, move |window, cx| window.focus(&focus, cx));
        }
    }

    // ---- Drawing ----------------------------------------------------------------------

    fn row_element(&self, i: usize, weak: &WeakEntity<Self>) -> AnyElement {
        match &self.rows[i] {
            Row::Owner(t) => div()
                .flex()
                .justify_end()
                .child(
                    div()
                        .max_w(relative(0.85))
                        .px(space(3))
                        .py(space(2))
                        .rounded(radius::lg())
                        .border_1()
                        .border_color(line::border())
                        .bg(surface::raised())
                        .text_size(size::base())
                        .text_color(text::primary())
                        .child(t.clone()),
                )
                .into_any_element(),
            Row::Reply { text, .. } => div()
                .min_w_0()
                .text_size(size::base())
                .text_color(text::primary())
                .child(TextView::new(text).selectable(true))
                .into_any_element(),
            Row::Tool(l) => div()
                .font_family(font::MONO)
                .text_size(size::xs())
                .text_color(text::tertiary())
                .truncate()
                .child(l.clone())
                .into_any_element(),
            Row::Note(t) => div()
                .text_size(size::xs())
                .text_color(text::tertiary())
                .child(t.clone())
                .into_any_element(),
            Row::End {
                cost_usd,
                duration_ms,
                error,
            } => match error {
                Some(e) => div()
                    .text_size(size::sm())
                    .text_color(WorkState::Failed.color())
                    .child(format!("The reply failed: {e}"))
                    .into_any_element(),
                None => mono(format!(
                    "{} · {:.1} s",
                    format::usd(*cost_usd),
                    *duration_ms as f64 / 1000.0
                ))
                .into_any_element(),
            },
            Row::Error(m) => div()
                .text_size(size::sm())
                .text_color(WorkState::Failed.color())
                .child(m.clone())
                .into_any_element(),
            Row::View(View::Decisions { ids }) => div()
                .flex()
                .flex_col()
                .gap(space(3))
                .children(
                    ids.iter()
                        .enumerate()
                        .map(|(n, id)| self.decision_card(i, n, *id, weak)),
                )
                .into_any_element(),
        }
    }

    /// One decision of a view, as the core stores it.
    fn decision_card(&self, row: usize, n: usize, id: i64, weak: &WeakEntity<Self>) -> AnyElement {
        let key = format!("deskc-{row}-{n}");
        let Some(d) = self.decisions.get(&id) else {
            let line = if self.decisions_read {
                format!("Decision {id} is not stored.")
            } else {
                format!("Reading decision {id}…")
            };
            return div()
                .text_size(size::sm())
                .text_color(text::tertiary())
                .child(line)
                .into_any_element();
        };
        let status = d.status.clone().unwrap_or_else(|| "pending".into());
        let open = status == "pending" && !d.archived;
        let kind = if d.minutes_id.is_some() {
            "Decision · minutes"
        } else {
            "Decision"
        };
        let mut card = Card::new(SharedString::from(key.clone()))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(space(2))
                    .child(neutral_tag(kind))
                    .child(mono(format!("#{id}")))
                    .child(super::parts::decision_tag(&status, true))
                    .when(d.archived, |el| el.child(neutral_tag("archived"))),
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
        if !open {
            return card.into_any_element();
        }
        let call = |f: fn(&mut DeskConversation, &mut Context<DeskConversation>)| {
            let weak = weak.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, f);
            }
        };
        let busy = self.busy;
        match self.confirm {
            Some(Confirm::Decide(want, verdict)) if want == id => card
                .child(
                    div()
                        .text_size(size::sm())
                        .text_color(text::secondary())
                        .child(format!(
                            "{} decision {id}?",
                            if verdict == Verdict::Approved {
                                "Approve"
                            } else {
                                "Reject"
                            }
                        )),
                )
                .child(
                    div()
                        .flex()
                        .gap(space(2))
                        .child(
                            Button::new(SharedString::from(format!("{key}-confirm")))
                                .primary()
                                .small()
                                .label("Confirm")
                                .disabled(busy)
                                .on_click(call(|t, cx| t.confirmed(cx))),
                        )
                        .child(
                            Button::new(SharedString::from(format!("{key}-cancel")))
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(call(|t, cx| t.cancel(cx))),
                        ),
                )
                .into_any_element(),
            _ => {
                let ask = |verdict: Verdict| {
                    let weak = weak.clone();
                    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        let _ = weak.update(cx, |this, cx| this.ask_decide(id, verdict, cx));
                    }
                };
                card.child(
                    div()
                        .flex()
                        .gap(space(2))
                        .child(
                            Button::new(SharedString::from(format!("{key}-approve")))
                                .primary()
                                .small()
                                .label("Approve")
                                .disabled(busy)
                                .on_click(ask(Verdict::Approved)),
                        )
                        .child(
                            Button::new(SharedString::from(format!("{key}-reject")))
                                .secondary()
                                .small()
                                .label("Reject")
                                .disabled(busy)
                                .on_click(ask(Verdict::Rejected)),
                        ),
                )
                .into_any_element()
            }
        }
    }

    fn mark(&self) -> Option<StateMark> {
        match self.state {
            DeskState::Working => Some(StateMark::new(WorkState::Working)),
            DeskState::Starting => {
                Some(StateMark::new(WorkState::Working).reason("starting Claude"))
            }
            DeskState::Auth => Some(StateMark::new(WorkState::Blocked).reason("sign in")),
            DeskState::Failed => Some(StateMark::new(WorkState::Failed)),
            DeskState::Stopped | DeskState::Idle => None,
        }
    }

    fn confirm_bar(&self, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(space(2))
            .p(space(3))
            .rounded(radius::md())
            .border_1()
            .border_color(line::border())
            .bg(surface::raised())
            .child(
                div()
                    .text_size(size::sm())
                    .text_color(text::secondary())
                    .child("Delete this conversation and start a new one?"),
            )
            .child(
                div()
                    .flex()
                    .gap(space(2))
                    .child(
                        Button::new("deskc-reset-confirm")
                            .danger()
                            .small()
                            .label("Confirm")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.confirmed(cx))),
                    )
                    .child(
                        Button::new("deskc-reset-cancel")
                            .ghost()
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx))),
                    ),
            )
    }
}

impl Render for DeskConversation {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_input(window, cx);
        let working = matches!(self.state, DeskState::Working | DeskState::Starting);
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(space(2))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space(2))
                    .min_w_0()
                    .child(
                        div()
                            .text_size(size::md())
                            .font_weight(weight::SEMIBOLD)
                            .text_color(text::primary())
                            .child("Ask the front desk"),
                    )
                    .children(self.mark()),
            )
            .child(
                div()
                    .flex()
                    .gap(space(1))
                    .when(working, |el| {
                        el.child(
                            Button::new("deskc-stop")
                                .ghost()
                                .small()
                                .label("Stop")
                                .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                        )
                    })
                    .child(
                        Button::new("deskc-new")
                            .ghost()
                            .small()
                            .label("New conversation")
                            .disabled(self.busy || self.rows.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.ask_reset(cx))),
                    ),
            );

        let weak = cx.weak_entity();
        let body = match &self.history {
            Load::Loading => notice("Reading the conversation…", None).into_any_element(),
            Load::Failed(e) => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(space(3))
                .p(space(4))
                .child(
                    div()
                        .text_size(size::sm())
                        .text_color(text::secondary())
                        .child(format!("The conversation did not load: {e}")),
                )
                .child(
                    Button::new("deskc-retry")
                        .secondary()
                        .small()
                        .label("Try again")
                        .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                )
                .into_any_element(),
            Load::Ready(()) if self.rows.is_empty() => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(space(3))
                .p(space(4))
                .child(
                    div()
                        .text_size(size::sm())
                        .text_color(text::tertiary())
                        .child("Ask Claude about this product. Claude reads. You decide."),
                )
                .when(self.can_send(), |el| {
                    el.child(
                        Button::new("deskc-suggest")
                            .secondary()
                            .small()
                            .label(ASK_PENDING)
                            .on_click(cx.listener(|this, _, _, cx| this.send(ASK_PENDING, cx))),
                    )
                })
                .into_any_element(),
            Load::Ready(()) => {
                let w = weak.clone();
                MessageScroller::new("deskc-list", self.scroller.clone(), move |i, _, cx| {
                    w.read_with(cx, |this, _| this.row_element(i, &w))
                        .unwrap_or_else(|_| div().into_any_element())
                })
                .size_full()
                .px(space(3))
                .into_any_element()
            }
        };

        let footer = if self.state == DeskState::Auth {
            let message = self
                .message
                .clone()
                .unwrap_or_else(|| "The login of Claude Code is not usable.".into());
            div()
                .flex()
                .flex_col()
                .gap(space(2))
                .child(
                    div()
                        .text_size(size::sm())
                        .text_color(text::secondary())
                        .child(message),
                )
                .child(
                    div().flex().child(
                        Button::new("deskc-check-again")
                            .secondary()
                            .small()
                            .label("Check again")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.recheck(cx))),
                    ),
                )
        } else {
            let can_send = self.can_send();
            div()
                .flex()
                .flex_col()
                .gap(space(2))
                .when(self.state == DeskState::Failed, |el| {
                    el.child(
                        div()
                            .text_size(size::sm())
                            .text_color(WorkState::Failed.color())
                            .child(
                                self.message
                                    .clone()
                                    .unwrap_or_else(|| "The desk session failed.".into()),
                            ),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(space(2))
                        .children(self.input.as_ref().map(|i| {
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(i).disabled(!can_send))
                        }))
                        .child(
                            Button::new("deskc-send")
                                .primary()
                                .small()
                                .label("Send")
                                .disabled(!can_send)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let text = this
                                        .input
                                        .as_ref()
                                        .map(|i| i.read(cx).value().to_string())
                                        .unwrap_or_default();
                                    this.send(&text, cx);
                                })),
                        ),
                )
        };

        div()
            .id("desk-conversation")
            .flex()
            .flex_col()
            .size_full()
            .gap(space(3))
            .p(space(3))
            .bg(surface::panel())
            .child(header)
            .when(self.confirm == Some(Confirm::NewConversation), |el| {
                el.child(self.confirm_bar(cx))
            })
            .child(div().flex_1().min_h_0().child(body))
            .children(self.flash.as_ref().map(flash_line))
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::net::UnixStream;
    use std::sync::Arc;

    use agentsi_proto::desk_session_api::{DeskDeltaParams, DeskItemParams, DeskStateParams};
    use agentsi_proto::marks_api::{DecisionGetResult, Outcome};
    use gpui::{TestAppContext, VisualTestContext};
    use serde_json::{Value, json};

    // No `use super::*`: it would bring the glob of `gpui_kit` and its `test` macro, and
    // the `#[test]` that `#[gpui::test]` writes would expand again and again.
    use super::{
        CallError, DecideResult, DecisionInfo, DeskAuthResult, DeskConversation, DeskDelta,
        DeskHistoryResult, DeskItem, DeskItemEvent, DeskResetResult, DeskState, DeskStateEvent,
        Flash, Notification, Row, Verdict,
    };
    use crate::link::Link;
    use crate::model::CoreState;
    use crate::model::two_cores::received;
    use agentsi_proto::events::DataChanged;
    use gpui_kit::{AppContext as _, Entity};

    impl DeskConversation {
        /// One word for each row, with the seq of a reply: `reply:3`, `reply:-` while it streams.
        fn kinds(&self) -> Vec<String> {
            self.rows
                .iter()
                .map(|r| match r {
                    Row::Owner(_) => "owner".into(),
                    Row::Reply { seq, .. } => {
                        format!("reply:{}", seq.map_or("-".into(), |s| s.to_string()))
                    }
                    Row::Tool(_) => "tool".into(),
                    Row::View(_) => "view".into(),
                    Row::Note(_) => "note".into(),
                    Row::End { .. } => "end".into(),
                    Row::Error(_) => "error".into(),
                })
                .collect()
        }
    }

    fn setup(
        cx: &mut TestAppContext,
    ) -> (Entity<DeskConversation>, UnixStream, &mut VisualTestContext) {
        cx.update(agentsi_ui::init);
        let (end, other) = UnixStream::pair().unwrap();
        other.set_nonblocking(true).unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (conv, cx) = cx.add_window_view(|_, cx| DeskConversation::new("demo".into(), core, cx));
        (conv, other, cx)
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn methods_sent(other: &mut UnixStream) -> Vec<(String, Value)> {
        received(other)
            .into_iter()
            .map(|r| {
                (
                    r["method"].as_str().unwrap_or_default().to_string(),
                    r["params"].clone(),
                )
            })
            .collect()
    }

    fn history(state: &str, message: Option<&str>, items: Value) -> DeskHistoryResult {
        serde_json::from_value(json!({
            "session": "s1", "state": state, "message": message, "items": items
        }))
        .unwrap()
    }

    fn full_items() -> Vec<Value> {
        let items = [
            json!({"kind": "owner", "text": "Hi"}),
            json!({"kind": "reply", "text": "Hello **there**"}),
            json!({"kind": "tool", "line": "Read: docs/a.md"}),
            json!({"kind": "note", "text": "The owner approved decision 14."}),
            json!({"kind": "end", "cost_usd": 0.01, "duration_ms": 1200}),
            json!({"kind": "error", "message": "The desk session stopped"}),
        ];
        items
            .into_iter()
            .zip(1..)
            .map(|(i, seq)| entry(seq, i))
            .collect()
    }

    fn entry(seq: i64, item: Value) -> Value {
        json!({"seq": seq, "at": "2026-11-09T10:00:00.000Z", "item": item})
    }

    fn note<E: agentsi_proto::Event>(params: &E::Params) -> Notification {
        Notification::of::<E>(params)
    }

    fn item_event(seq: i64, item: DeskItem) -> Notification {
        note::<DeskItemEvent>(&DeskItemParams {
            product: "demo".into(),
            seq,
            at: "2026-11-09T10:00:00.000Z".into(),
            item,
        })
    }

    fn delta_event(text: &str) -> Notification {
        note::<DeskDelta>(&DeskDeltaParams {
            product: "demo".into(),
            text: text.into(),
        })
    }

    fn decision(id: i64, status: &str) -> DecisionInfo {
        DecisionInfo {
            id,
            kind: "minutes".into(),
            minutes_id: Some("m1".into()),
            text: "Ship it".into(),
            why: Some("The release is ready.".into()),
            action: Some("git tag v1".into()),
            status: Some(status.into()),
            archived: false,
        }
    }

    #[gpui::test]
    fn opening_the_conversation_reads_the_history_and_the_state(cx: &mut TestAppContext) {
        let (conv, mut other, cx) = setup(cx);
        let sent = methods_sent(&mut other);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "desk.history");
        assert_eq!(sent[0].1, json!({"product": "demo"}));
        draw(cx);
        conv.update(cx, |c, cx| {
            c.history_done(Ok(history("idle", None, Value::Array(full_items()))), cx)
        });
        draw(cx);
        conv.read_with(cx, |c, _| {
            assert_eq!(
                c.kinds(),
                ["owner", "reply:2", "tool", "note", "end", "error"]
            );
            assert!(c.can_send());
        });
    }

    #[gpui::test]
    fn a_reply_streams_into_one_row_and_its_item_closes_it(cx: &mut TestAppContext) {
        let (conv, _other, cx) = setup(cx);
        conv.update(cx, |c, cx| {
            c.history_done(Ok(history("idle", None, json!([]))), cx);
            c.take(&item_event(1, DeskItem::Owner { text: "Q".into() }), cx);
            c.take(&delta_event("Hel"), cx);
            c.take(&delta_event("lo"), cx);
        });
        draw(cx);
        conv.read_with(cx, |c, _| assert_eq!(c.kinds(), ["owner", "reply:-"]));
        conv.update(cx, |c, cx| {
            c.take(
                &item_event(
                    2,
                    DeskItem::Reply {
                        text: "Hello".into(),
                    },
                ),
                cx,
            );
            // A second text block of the same reply opens its own row.
            c.take(&delta_event("More"), cx);
            c.take(
                &item_event(
                    3,
                    DeskItem::Reply {
                        text: "More".into(),
                    },
                ),
                cx,
            );
            c.take(
                &item_event(
                    4,
                    DeskItem::End {
                        cost_usd: 0.0,
                        duration_ms: 5,
                        error: None,
                    },
                ),
                cx,
            );
            // An event that the core sent twice changes nothing.
            c.take(
                &item_event(
                    4,
                    DeskItem::End {
                        cost_usd: 0.0,
                        duration_ms: 5,
                        error: None,
                    },
                ),
                cx,
            );
        });
        draw(cx);
        conv.read_with(cx, |c, _| {
            assert_eq!(c.kinds(), ["owner", "reply:2", "reply:3", "end"]);
            assert!(c.open.is_none());
        });
    }

    #[gpui::test]
    fn a_reply_without_a_delta_is_a_row_and_an_event_of_another_product_is_ignored(
        cx: &mut TestAppContext,
    ) {
        let (conv, _other, cx) = setup(cx);
        conv.update(cx, |c, cx| {
            c.history_done(Ok(history("idle", None, json!([]))), cx);
            c.take(&item_event(1, DeskItem::Reply { text: "Hi".into() }), cx);
            let mut other = delta_event("x");
            other.params["product"] = json!("other");
            c.take(&other, cx);
        });
        conv.read_with(cx, |c, _| assert_eq!(c.kinds(), ["reply:1"]));
    }

    #[gpui::test]
    fn events_before_the_history_wait_for_it_and_do_not_repeat_its_items(cx: &mut TestAppContext) {
        let (conv, _other, cx) = setup(cx);
        conv.update(cx, |c, cx| {
            c.take(&item_event(2, DeskItem::Reply { text: "Hi".into() }), cx);
            c.take(&item_event(3, DeskItem::Note { text: "n".into() }), cx);
            c.take(
                &note::<DeskStateEvent>(&DeskStateParams {
                    product: "demo".into(),
                    state: DeskState::Working,
                    message: None,
                }),
                cx,
            );
        });
        conv.read_with(cx, |c, _| assert!(c.rows.is_empty()));
        conv.update(cx, |c, cx| {
            c.history_done(
                Ok(history(
                    "idle",
                    None,
                    json!([
                        entry(1, json!({"kind": "owner", "text": "Q"})),
                        entry(2, json!({"kind": "reply", "text": "Hi"})),
                    ]),
                )),
                cx,
            )
        });
        conv.read_with(cx, |c, _| {
            assert_eq!(c.kinds(), ["owner", "reply:2", "note"]);
            assert_eq!(c.state, DeskState::Working);
        });
    }

    #[gpui::test]
    fn the_input_waits_while_a_reply_runs_and_a_second_send_is_refused(cx: &mut TestAppContext) {
        let (conv, mut other, cx) = setup(cx);
        methods_sent(&mut other);
        conv.update(cx, |c, cx| {
            c.history_done(Ok(history("working", None, json!([]))), cx);
            c.send("Another question", cx);
        });
        draw(cx);
        assert!(
            methods_sent(&mut other).is_empty(),
            "no send while a reply runs"
        );
        conv.update(cx, |c, cx| {
            c.take(
                &note::<DeskStateEvent>(&DeskStateParams {
                    product: "demo".into(),
                    state: DeskState::Idle,
                    message: None,
                }),
                cx,
            );
            c.send("  What waits for me?  ", cx);
            // The first message has no reply yet, so a second one waits.
            c.send("Again", cx);
        });
        let sent = methods_sent(&mut other);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "desk.send");
        assert_eq!(
            sent[0].1,
            json!({"product": "demo", "text": "What waits for me?"})
        );
    }

    #[gpui::test]
    fn each_auth_state_shows_its_message_and_check_again_without_a_desk_send(
        cx: &mut TestAppContext,
    ) {
        let (conv, mut other, cx) = setup(cx);
        methods_sent(&mut other);
        for (state, message) in [
            (
                agentsi_proto::desk_session_api::AuthState::SignedOut,
                "Claude Code is not signed in. Run `claude auth login` in a terminal, then select Check again.",
            ),
            (
                agentsi_proto::desk_session_api::AuthState::Missing,
                "AgentSI cannot find the `claude` program on the PATH.",
            ),
            (
                agentsi_proto::desk_session_api::AuthState::NotRead,
                "Claude Code did not use your login. A newer version can run `claude -p` in bare mode.",
            ),
            (
                agentsi_proto::desk_session_api::AuthState::Unknown,
                "claude: unexpected output",
            ),
        ] {
            conv.update(cx, |c, cx| {
                c.history_done(Ok(history("auth", Some(message), json!([]))), cx);
                c.send("hello", cx);
            });
            draw(cx);
            conv.read_with(cx, |c, _| {
                assert_eq!(c.state, DeskState::Auth);
                assert_eq!(c.message.as_deref(), Some(message));
                assert!(!c.can_send());
            });
            // Check again asks the core to run the check again, and reads no stale answer.
            conv.update(cx, |c, cx| c.recheck(cx));
            let sent = methods_sent(&mut other);
            assert_eq!(sent.len(), 1, "{state:?}: {sent:?}");
            assert_eq!(sent[0].0, "desk.auth");
            assert_eq!(sent[0].1, json!({"recheck": true}));
            // The check still fails: the message stays and no desk process starts.
            conv.update(cx, |c, cx| {
                c.recheck_done(
                    Ok(DeskAuthResult {
                        state,
                        message: Some(message.into()),
                    }),
                    cx,
                )
            });
            conv.read_with(cx, |c, _| assert_eq!(c.state, DeskState::Auth));
        }
        // A login that works again reads the state, which shows the input.
        conv.update(cx, |c, cx| {
            c.recheck_done(
                Ok(DeskAuthResult {
                    state: agentsi_proto::desk_session_api::AuthState::Ready,
                    message: None,
                }),
                cx,
            )
        });
        assert_eq!(methods_sent(&mut other)[0].0, "desk.history");
    }

    fn view_history(cx: &mut VisualTestContext, conv: &Entity<DeskConversation>) {
        conv.update(cx, |c, cx| {
            c.history_done(
                Ok(history(
                    "idle",
                    None,
                    json!([
                        entry(1, json!({"kind": "owner", "text": "What waits for me?"})),
                        entry(
                            2,
                            json!({"kind": "view", "view": {"kind": "decisions", "ids": [7]}})
                        ),
                    ]),
                )),
                cx,
            )
        });
    }

    #[gpui::test]
    fn the_decisions_view_reads_the_stored_decision_and_approve_asks_a_confirm_first(
        cx: &mut TestAppContext,
    ) {
        let (conv, mut other, cx) = setup(cx);
        methods_sent(&mut other);
        view_history(cx, &conv);
        let sent = methods_sent(&mut other);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "decision.get");
        assert_eq!(sent[0].1, json!({"product": "demo", "ids": [7]}));
        conv.update(cx, |c, cx| {
            c.decisions_done(
                Ok(DecisionGetResult {
                    decisions: vec![decision(7, "pending")],
                }),
                cx,
            )
        });
        draw(cx);

        // Approve only asks. It writes nothing.
        conv.update(cx, |c, cx| c.ask_decide(7, Verdict::Approved, cx));
        draw(cx);
        assert!(methods_sent(&mut other).is_empty());
        // Cancel drops the question.
        conv.update(cx, |c, cx| c.cancel(cx));
        conv.update(cx, |c, cx| c.confirmed(cx));
        assert!(methods_sent(&mut other).is_empty());

        conv.update(cx, |c, cx| c.ask_decide(7, Verdict::Approved, cx));
        conv.update(cx, |c, cx| c.confirmed(cx));
        let sent = methods_sent(&mut other);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "decision.decide");
        assert_eq!(sent[0].1, json!({"decision": 7, "status": "approved"}));

        // The core accepted. The view reads the stored decision again. No owner message is sent.
        conv.update(cx, |c, cx| {
            c.decide_done(
                7,
                Ok(DecideResult {
                    product: "demo".into(),
                    minutes_id: Some("m1".into()),
                    status: Outcome::Approved,
                    action: Some("git tag v1".into()),
                    commit_error: None,
                    refused: None,
                }),
                cx,
            )
        });
        let sent = methods_sent(&mut other);
        assert_eq!(
            sent.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(),
            ["decision.get"]
        );
        conv.update(cx, |c, cx| {
            c.decisions_done(
                Ok(DecisionGetResult {
                    decisions: vec![decision(7, "approved")],
                }),
                cx,
            )
        });
        draw(cx);
        conv.read_with(cx, |c, _| {
            assert_eq!(c.decisions[&7].status.as_deref(), Some("approved"));
            assert!(matches!(c.flash, Some(Flash::Done(_))));
            assert!(!c.busy);
        });
        assert!(methods_sent(&mut other).is_empty());
    }

    #[gpui::test]
    fn a_decision_that_the_pane_settles_changes_the_view_after_data_changed(
        cx: &mut TestAppContext,
    ) {
        let (conv, mut other, cx) = setup(cx);
        methods_sent(&mut other);
        view_history(cx, &conv);
        methods_sent(&mut other);
        let changed = |kind: &str, product: &str| {
            Notification::of::<DataChanged>(
                &serde_json::from_value(json!({"product": product, "kind": kind})).unwrap(),
            )
        };
        // Another kind or another product does not read the decisions.
        conv.update(cx, |c, cx| {
            c.take(&changed("stories", "demo"), cx);
            c.take(&changed("decision", "other"), cx);
        });
        assert!(methods_sent(&mut other).is_empty());
        conv.update(cx, |c, cx| c.take(&changed("decision", "demo"), cx));
        assert_eq!(methods_sent(&mut other)[0].0, "decision.get");
        // The desk note of the pane write shows as a line, and it is not a new turn.
        conv.update(cx, |c, cx| {
            c.take(
                &item_event(
                    3,
                    DeskItem::Note {
                        text: "The owner approved decision 7.".into(),
                    },
                ),
                cx,
            )
        });
        conv.read_with(cx, |c, _| assert_eq!(c.kinds().last().unwrap(), "note"));
        assert!(methods_sent(&mut other).is_empty());
    }

    #[gpui::test]
    fn a_decision_that_is_gone_or_a_failed_write_shows_a_line_and_changes_nothing(
        cx: &mut TestAppContext,
    ) {
        let (conv, mut other, cx) = setup(cx);
        methods_sent(&mut other);
        view_history(cx, &conv);
        conv.update(cx, |c, cx| {
            c.decisions_done(Ok(DecisionGetResult { decisions: vec![] }), cx)
        });
        draw(cx);
        conv.read_with(cx, |c, _| {
            assert!(c.decisions.is_empty() && c.decisions_read)
        });
        conv.update(cx, |c, cx| {
            c.decide_done(
                7,
                Err(CallError::Link("the link to the core broke".into())),
                cx,
            )
        });
        conv.read_with(cx, |c, _| {
            assert!(matches!(c.flash, Some(Flash::Failed(_))))
        });
    }

    #[gpui::test]
    fn new_conversation_asks_a_confirm_then_resets_and_reads_the_empty_history(
        cx: &mut TestAppContext,
    ) {
        let (conv, mut other, cx) = setup(cx);
        methods_sent(&mut other);
        view_history(cx, &conv);
        methods_sent(&mut other);
        conv.update(cx, |c, cx| c.ask_reset(cx));
        draw(cx);
        assert!(methods_sent(&mut other).is_empty());
        conv.update(cx, |c, cx| c.confirmed(cx));
        assert_eq!(methods_sent(&mut other)[0].0, "desk.reset");
        conv.update(cx, |c, cx| {
            c.reset_done(
                Ok(DeskResetResult {
                    session: "s2".into(),
                }),
                cx,
            )
        });
        assert_eq!(methods_sent(&mut other)[0].0, "desk.history");
        conv.update(cx, |c, cx| {
            c.history_done(Ok(history("stopped", None, json!([]))), cx)
        });
        draw(cx);
        conv.read_with(cx, |c, _| {
            assert!(c.rows.is_empty() && c.decisions.is_empty())
        });
    }

    #[gpui::test]
    fn the_input_takes_the_focus_and_the_letter_keys_of_the_desk_wait(cx: &mut TestAppContext) {
        use crate::screens::desk::DeskScreen;
        cx.update(agentsi_ui::init);
        let (end, mut other) = UnixStream::pair().unwrap();
        other.set_nonblocking(true).unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (desk, cx) = cx.add_window_view(|_, cx| DeskScreen::new("demo".into(), core, cx));
        draw(cx);
        desk.update(cx, |d, cx| d.ask(cx));
        draw(cx);
        draw(cx);
        let focused = desk.read_with(cx, |d, _| d.conversation.clone());
        // The history has not come, so the input cannot take the focus yet.
        conv_ready(cx, &focused);
        desk.update(cx, |d, cx| d.ask(cx));
        draw(cx);
        draw(cx);
        assert!(cx.update(|w, cx| focused.read(cx).input_focused(w, cx)));
        methods_sent(&mut other);
        cx.simulate_keystrokes("a x r");
        assert!(
            methods_sent(&mut other)
                .iter()
                .all(|(m, _)| m != "decision.decide" && m != "archive.archive"),
            "a letter in the input must not act on a decision"
        );
    }

    fn conv_ready(cx: &mut VisualTestContext, conv: &Entity<DeskConversation>) {
        conv.update(cx, |c, cx| {
            c.history_done(Ok(history("idle", None, json!([]))), cx)
        });
    }
}
