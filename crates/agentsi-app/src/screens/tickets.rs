//! The tickets of a product: Open, Waiting, and Done of the last 14 days. Dropped shows only
//! with its filter. The list takes focus as one element: the arrow keys move a cursor and
//! `Enter` opens the ticket of the cursor. An add field at the top makes a ticket.

use agentsi_proto::ticket_api::{
    Author, AuthorBy, Ticket, TicketAdd, TicketAddParams, TicketKind, TicketList, TicketListParams,
    TicketState,
};
use agentsi_ui::components::{Button, ListRow};
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{control, line, size, space, surface, text, weight};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::minutes::cursor_after;
use super::parts::{Flash, Load, first_screen_ready, flash_line, notice};
use super::ticket::{kind_tag, owner, state_tag};
use crate::format;
use crate::model::CoreState;
use crate::nav::{Nav, ScreenRequest};

/// A done ticket stays on the list this long.
const DONE_DAYS: i64 = 14;
const DAY: i64 = 86_400;
/// The most tickets that one read returns.
const LIMIT: u32 = 200;

/// Which states the list shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StateFilter {
    /// Open, Waiting, and Done of the last 14 days.
    #[default]
    Active,
    Only(TicketState),
}

impl StateFilter {
    fn next(self) -> StateFilter {
        match self {
            StateFilter::Active => StateFilter::Only(TicketState::Open),
            StateFilter::Only(TicketState::Open) => StateFilter::Only(TicketState::Waiting),
            StateFilter::Only(TicketState::Waiting) => StateFilter::Only(TicketState::Done),
            StateFilter::Only(TicketState::Done) => StateFilter::Only(TicketState::Dropped),
            StateFilter::Only(TicketState::Dropped) => StateFilter::Active,
        }
    }

    fn label(self) -> String {
        match self {
            StateFilter::Active => "State: active".into(),
            StateFilter::Only(s) => format!("State: {}", s.as_str()),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filters {
    pub kind: Option<TicketKind>,
    pub state: StateFilter,
    pub assignee: Option<String>,
    pub author: Option<AuthorBy>,
}

fn next_kind(k: Option<TicketKind>) -> Option<TicketKind> {
    match k {
        None => Some(TicketKind::Note),
        Some(TicketKind::Note) => Some(TicketKind::Todo),
        Some(TicketKind::Todo) => Some(TicketKind::Question),
        Some(TicketKind::Question) => None,
    }
}

fn next_assignee(a: &Option<String>) -> Option<String> {
    match a.as_deref() {
        None => Some("owner".into()),
        Some("owner") => Some("claude".into()),
        _ => None,
    }
}

fn next_author(a: Option<AuthorBy>) -> Option<AuthorBy> {
    match a {
        None => Some(AuthorBy::Owner),
        Some(AuthorBy::Owner) => Some(AuthorBy::Agent),
        Some(AuthorBy::Agent) => Some(AuthorBy::Session),
        Some(AuthorBy::Session) => None,
    }
}

/// The groups of the list, each with the indexes of its tickets, in the order of the list.
/// A group with no ticket is left out.
pub fn visible(tickets: &[Ticket], f: &Filters, now: i64) -> Vec<(TicketState, Vec<usize>)> {
    TicketState::ALL
        .into_iter()
        .filter_map(|state| {
            let rows: Vec<usize> = tickets
                .iter()
                .enumerate()
                .filter(|(_, t)| t.state == state)
                .filter(|(_, t)| match f.state {
                    StateFilter::Active => match state {
                        TicketState::Dropped => false,
                        TicketState::Done => t
                            .closed_at
                            .as_deref()
                            .or(Some(&t.updated_at))
                            .and_then(format::unix_secs)
                            .is_none_or(|c| c >= now - DONE_DAYS * DAY),
                        _ => true,
                    },
                    StateFilter::Only(only) => only == state,
                })
                .filter(|(_, t)| f.kind.is_none_or(|k| t.kind == k))
                .filter(|(_, t)| {
                    f.assignee
                        .as_ref()
                        .is_none_or(|a| t.assignee.as_ref() == Some(a))
                })
                .filter(|(_, t)| f.author.is_none_or(|a| t.author.by == a))
                .map(|(i, _)| i)
                .collect();
            (!rows.is_empty()).then_some((state, rows))
        })
        .collect()
}

/// The line under the title of a row: id, author, assignee, link count and age.
pub fn meta(t: &Ticket, now: i64) -> String {
    let mut parts = vec![
        t.id.clone(),
        format!("by {}", super::ticket::who(&t.author)),
    ];
    if let Some(a) = &t.assignee {
        parts.push(format!("for {a}"));
    }
    if !t.links.is_empty() {
        parts.push(format!(
            "{} link{}",
            t.links.len(),
            if t.links.len() == 1 { "" } else { "s" }
        ));
    }
    parts.push(format::ago(&t.updated_at, now));
    parts.join(" · ")
}

fn group_name(s: TicketState) -> &'static str {
    match s {
        TicketState::Open => "Open",
        TicketState::Waiting => "Waiting",
        TicketState::Done => "Done",
        TicketState::Dropped => "Dropped",
    }
}

pub struct TicketsScreen {
    product: String,
    core: Entity<CoreState>,
    data: Load<Vec<Ticket>>,
    filters: Filters,
    cursor: usize,
    add_kind: TicketKind,
    input: Option<Entity<InputState>>,
    busy: bool,
    flash: Option<Flash>,
    scroll: ScrollHandle,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl TicketsScreen {
    pub fn new(product: String, core: Entity<CoreState>, cx: &mut Context<Self>) -> Self {
        let mut screen = TicketsScreen {
            product,
            core,
            data: Load::Loading,
            filters: Filters::default(),
            cursor: 0,
            add_kind: TicketKind::Note,
            input: None,
            busy: false,
            flash: None,
            scroll: ScrollHandle::new(),
            focus: cx.focus_handle().tab_stop(true),
            _subscriptions: Vec::new(),
        };
        screen.reload(cx);
        screen
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let call = self
            .core
            .read(cx)
            .link
            .call::<TicketList>(&TicketListParams {
                product: self.product.clone(),
                states: Some(TicketState::ALL.to_vec()),
                limit: Some(LIMIT),
                ..TicketListParams::default()
            });
        cx.spawn(async move |this, cx| {
            let result = call.await.map(|r| r.tickets).map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.data.settle(result);
                this.cursor = this.cursor.min(this.flat().len().saturating_sub(1));
                first_screen_ready();
                cx.notify();
            });
        })
        .detach();
    }

    fn tickets(&self) -> &[Ticket] {
        match &self.data {
            Load::Ready(rows) => rows,
            _ => &[],
        }
    }

    fn groups(&self) -> Vec<(TicketState, Vec<usize>)> {
        visible(self.tickets(), &self.filters, format::now_secs())
    }

    /// The tickets in the order that the cursor walks.
    fn flat(&self) -> Vec<usize> {
        self.groups()
            .into_iter()
            .flat_map(|(_, rows)| rows)
            .collect()
    }

    fn open(&self, t: &Ticket, cx: &App) {
        Nav::open(cx, &self.core, ScreenRequest::ticket(&self.product, &t.id));
    }

    fn input(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        if let Some(input) = &self.input {
            return input.clone();
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("A new ticket…"));
        self._subscriptions.push(cx.subscribe_in(
            &input,
            window,
            |this, input, e: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = e {
                    let title = input.read(cx).value().trim().to_string();
                    this.add(title, window, cx);
                }
            },
        ));
        self.input = Some(input.clone());
        input
    }

    pub fn add(&mut self, title: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if title.is_empty() {
            self.flash = Some(Flash::Failed("Write the title of the ticket first.".into()));
            cx.notify();
            return;
        }
        let call = self.core.read(cx).link.call::<TicketAdd>(&TicketAddParams {
            product: self.product.clone(),
            kind: self.add_kind,
            title,
            body: None,
            assignee: None,
            links: None,
            scope: None,
            objective: None,
            author: Author { ..owner() },
        });
        self.busy = true;
        self.flash = None;
        cx.spawn_in(window, async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(r) => {
                        this.flash = Some(Flash::Done(format!("Added {}.", r.ticket.id)));
                        if let Some(input) = &this.input {
                            input.update(cx, |i, cx| i.set_value("", window, cx));
                        }
                    }
                    Err(e) => this.flash = Some(Flash::Failed(e)),
                }
                this.reload(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn key_down(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.modified() {
            return;
        }
        let flat = self.flat();
        if k.key == "enter" {
            if let Some(t) = flat.get(self.cursor).and_then(|i| self.tickets().get(*i)) {
                self.open(t, cx);
            }
            cx.stop_propagation();
        } else if let Some(to) = cursor_after(&k.key, self.cursor, flat.len()) {
            self.cursor = to;
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn filter_button(
        &self,
        id: &'static str,
        label: String,
        on: bool,
        cx: &mut Context<Self>,
        change: impl Fn(&mut Filters) + 'static,
    ) -> Button {
        let b = Button::new(id).small().label(label);
        let b = if on { b.primary() } else { b.ghost() };
        b.on_click(cx.listener(move |this, _, _, cx| {
            change(&mut this.filters);
            this.cursor = 0;
            cx.notify();
        }))
    }
}

impl Render for TicketsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let input = self.input(window, cx);
        let now = format::now_secs();
        let groups = self.groups();
        let focused = self.focus.is_focused(window);
        let f = self.filters.clone();
        let total = self.tickets().iter().filter(|t| t.state.is_open()).count();
        let mut at = 0usize;
        let mut list = div().flex().flex_col().gap(space(1)).p(space(1));
        for (state, rows) in groups {
            list = list.child(
                div()
                    .pad_x(space(3))
                    .pt(space(3))
                    .text_size(size::sm())
                    .font_weight(weight::SEMIBOLD)
                    .text_color(text::tertiary())
                    .child(format!("{} · {}", group_name(state), rows.len())),
            );
            for i in rows {
                let Some(t) = self.tickets().get(i).cloned() else {
                    continue;
                };
                let row_at = at;
                at += 1;
                let id = t.id.clone();
                let row = ListRow::new(("tickets-row", row_at), t.title.clone())
                    .meta(meta(&t, now))
                    .selected(self.cursor == row_at)
                    .focused(self.cursor == row_at && focused)
                    .trailing(
                        div()
                            .flex()
                            .gap(space(2))
                            .child(kind_tag(t.kind))
                            .child(state_tag(&t)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.cursor = row_at;
                        window.focus(&this.focus, cx);
                        Nav::open(cx, &this.core, ScreenRequest::ticket(&this.product, &id));
                        cx.notify();
                    }));
                list = list.child(div().pad_x(space(1)).child(row));
            }
        }
        let body = match &self.data {
            Load::Loading => notice("Reading the tickets…", None).into_any_element(),
            Load::Failed(e) => notice(e.clone(), None).into_any_element(),
            Load::Ready(_) if at == 0 => notice(
                "No tickets here.",
                Some("Write one in the field above, or ask a Claude pane to add one.".into()),
            )
            .into_any_element(),
            Load::Ready(_) => div()
                .id("tickets-scroll")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(list)
                .into_any_element(),
        };
        let kinds: Vec<_> = TicketKind::ALL
            .into_iter()
            .enumerate()
            .map(|(i, k)| {
                let b = Button::new(("tickets-add-kind", i))
                    .small()
                    .label(k.as_str());
                let b = if self.add_kind == k {
                    b.primary()
                } else {
                    b.ghost()
                };
                b.on_click(cx.listener(move |this, _, _, cx| {
                    this.add_kind = k;
                    cx.notify();
                }))
            })
            .collect();
        let filter_row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(space(2))
            .child(self.filter_button(
                "tickets-f-kind",
                format!("Kind: {}", f.kind.map_or("any", TicketKind::as_str)),
                f.kind.is_some(),
                cx,
                |f| f.kind = next_kind(f.kind),
            ))
            .child(self.filter_button(
                "tickets-f-state",
                f.state.label(),
                f.state != StateFilter::Active,
                cx,
                |f| f.state = f.state.next(),
            ))
            .child(self.filter_button(
                "tickets-f-assignee",
                format!("For: {}", f.assignee.as_deref().unwrap_or("anyone")),
                f.assignee.is_some(),
                cx,
                |f| f.assignee = next_assignee(&f.assignee),
            ))
            .child(self.filter_button(
                "tickets-f-author",
                format!("By: {}", f.author.map_or("anyone", AuthorBy::as_str)),
                f.author.is_some(),
                cx,
                |f| f.author = next_author(f.author),
            ));
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(surface::panel())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(space(3))
                    .h(control::TOOLBAR_HEIGHT)
                    .flex_none()
                    .pad_x(space(4))
                    .border_b_1()
                    .border_color(line::border())
                    .child(
                        div()
                            .text_size(size::md())
                            .font_weight(weight::SEMIBOLD)
                            .text_color(text::primary())
                            .child("Tickets"),
                    )
                    .child(
                        div()
                            .text_size(size::sm())
                            .text_color(text::tertiary())
                            .child(format!("{total} open")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .gap(space(2))
                    .p(space(4))
                    .border_b_1()
                    .border_color(line::border())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(space(2))
                            .child(div().flex_1().min_w_0().child(Input::new(&input)))
                            .children(kinds)
                            .child(
                                Button::new("tickets-add")
                                    .small()
                                    .primary()
                                    .label("Add")
                                    .disabled(self.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        let title = this
                                            .input
                                            .as_ref()
                                            .map(|i| i.read(cx).value().trim().to_string())
                                            .unwrap_or_default();
                                        this.add(title, window, cx);
                                    })),
                            ),
                    )
                    .child(filter_row)
                    .when_some(self.flash.as_ref(), |el, f| el.child(flash_line(f))),
            )
            .child(
                div()
                    .id("tickets-body")
                    .track_focus(&self.focus)
                    .key_context("TicketsList")
                    .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.key_down(e, cx)))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `super` brings `gpui::test` in as `test`, and `#[test]` then loops.
    use super::{
        AuthorBy, Filters, StateFilter, Ticket, TicketKind, TicketState, format, meta, visible,
    };
    use serde_json::json;

    fn ticket(id: &str, state: &str, kind: &str, closed: Option<&str>, by: &str) -> Ticket {
        serde_json::from_value(json!({
            "product": "demo", "id": id, "scope": null, "kind": kind, "state": state,
            "title": format!("Title {id}"), "body": "", "waiting_on": null, "assignee": null,
            "author": {"by": by, "id": ""}, "updated_by": {"by": by, "id": ""},
            "links": [], "objective": null,
            "created_at": "2026-11-01T10:00:00.000Z", "updated_at": "2026-11-09T10:00:00.000Z",
            "closed_at": closed
        }))
        .unwrap()
    }

    fn now() -> i64 {
        format::unix_secs("2026-11-09T12:00:00.000Z").unwrap()
    }

    fn all() -> Vec<Ticket> {
        vec![
            ticket("C001", "open", "todo", None, "owner"),
            ticket("C002", "waiting", "question", None, "session"),
            ticket(
                "C003",
                "done",
                "note",
                Some("2026-11-05T10:00:00.000Z"),
                "owner",
            ),
            ticket(
                "C004",
                "done",
                "note",
                Some("2026-10-01T10:00:00.000Z"),
                "owner",
            ),
            ticket(
                "C005",
                "dropped",
                "note",
                Some("2026-11-08T10:00:00.000Z"),
                "owner",
            ),
        ]
    }

    #[test]
    fn the_default_list_hides_old_done_and_dropped_tickets() {
        let groups = visible(&all(), &Filters::default(), now());
        assert_eq!(
            groups,
            vec![
                (TicketState::Open, vec![0]),
                (TicketState::Waiting, vec![1]),
                (TicketState::Done, vec![2]),
            ]
        );
    }

    #[test]
    fn a_state_filter_shows_that_state_only_and_dropped_has_its_filter() {
        let f = |state| Filters {
            state: StateFilter::Only(state),
            ..Filters::default()
        };
        assert_eq!(
            visible(&all(), &f(TicketState::Dropped), now()),
            vec![(TicketState::Dropped, vec![4])]
        );
        assert_eq!(
            visible(&all(), &f(TicketState::Done), now()),
            vec![(TicketState::Done, vec![2, 3])]
        );
    }

    #[test]
    fn the_filters_by_kind_assignee_and_author_narrow_the_list() {
        let by_kind = Filters {
            kind: Some(TicketKind::Question),
            ..Filters::default()
        };
        assert_eq!(
            visible(&all(), &by_kind, now()),
            vec![(TicketState::Waiting, vec![1])]
        );
        let by_author = Filters {
            author: Some(AuthorBy::Session),
            ..Filters::default()
        };
        assert_eq!(
            visible(&all(), &by_author, now()),
            vec![(TicketState::Waiting, vec![1])]
        );
        let mut tickets = all();
        tickets[0].assignee = Some("claude".into());
        let by_assignee = Filters {
            assignee: Some("claude".into()),
            ..Filters::default()
        };
        assert_eq!(
            visible(&tickets, &by_assignee, now()),
            vec![(TicketState::Open, vec![0])]
        );
    }

    #[test]
    fn a_row_shows_the_id_the_author_the_assignee_the_links_and_the_age() {
        let mut t = ticket("C001", "open", "todo", None, "session");
        t.author.id = "3f2a".into();
        t.assignee = Some("claude".into());
        t.links.push(agentsi_proto::ticket_api::Link {
            kind: agentsi_proto::ticket_api::LinkType::Commit,
            reference: "1a2b".into(),
        });
        let line = meta(&t, now());
        assert!(
            line.starts_with("C001 · by session 3f2a · for claude · 1 link · "),
            "{line}"
        );
    }

    use crate::link::Link;
    use crate::screens::parts::Load;
    use gpui::TestAppContext;
    use gpui_kit::AppContext as _;
    use std::sync::Arc;

    fn sent(other: &mut std::os::unix::net::UnixStream) -> Option<serde_json::Value> {
        other.set_nonblocking(true).unwrap();
        let frame = agentsi_proto::frame::read_frame(other).ok().flatten()?;
        serde_json::from_slice(&frame.payload).ok()
    }

    #[gpui::test]
    fn the_screen_draws_in_each_state_and_adds_a_ticket_as_the_owner(cx: &mut TestAppContext) {
        cx.update(agentsi_ui::init);
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| crate::model::CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) =
            cx.add_window_view(|_, cx| super::TicketsScreen::new("demo".into(), core, cx));
        let read = sent(&mut other).unwrap();
        assert_eq!(read["method"], "ticket.list");
        assert_eq!(read["params"]["product"], "demo");
        for data in [
            Load::Loading,
            Load::Failed("no core".into()),
            Load::Ready(vec![]),
            Load::Ready(all()),
        ] {
            screen.update(cx, |s, cx| {
                s.data = data;
                cx.notify();
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        screen.update(cx, |s, _| s.add_kind = TicketKind::Todo);
        screen.update_in(cx, |s, window, cx| {
            s.add("Try the tracker".into(), window, cx)
        });
        let add = sent(&mut other).expect("a request");
        assert_eq!(add["method"], "ticket.add");
        assert_eq!(add["params"]["kind"], "todo");
        assert_eq!(add["params"]["title"], "Try the tracker");
        assert_eq!(add["params"]["author"]["by"], "owner");
        // A second add waits for the first.
        screen.update_in(cx, |s, window, cx| s.add("Again".into(), window, cx));
        assert!(sent(&mut other).is_none());
        // An empty title sends nothing.
        screen.update(cx, |s, _| s.busy = false);
        screen.update_in(cx, |s, window, cx| s.add(String::new(), window, cx));
        assert!(sent(&mut other).is_none());
    }
}
