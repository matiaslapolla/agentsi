//! One ticket: its fields, its body as Markdown, its links, and the actions that its state
//! permits. The same rules and the same core calls serve the ticket menu (`Op`, `allowed`).

use agentsi_proto::ticket_api::{
    Author, AuthorBy, Link, LinkType, PromoteTo, Ticket, TicketGet, TicketGetParams, TicketKind,
    TicketMove, TicketMoveParams, TicketPromote, TicketPromoteParams, TicketState, TicketUpdate,
    TicketUpdateParams,
};
use agentsi_ui::components::{Button, Tag, neutral_tag, status_tag};
use agentsi_ui::tokens::{Status, line, radius, size, space, surface, text, weight};
use futures::FutureExt as _;
use futures::future::LocalBoxFuture;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::parts::{Flash, Load, column, first_screen_ready, flash_line, mono, notice, section};
use crate::menus::{self, Target};
use crate::model::{CoreState, ProductRef};
use crate::nav::{Nav, ScreenRequest};

/// The owner as a writer. The core fills in the name of the owner block.
pub fn owner() -> Author {
    Author {
        by: AuthorBy::Owner,
        id: String::new(),
    }
}

/// What a ticket menu or a button asks of the state of a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Done,
    Wait,
    Drop,
    Reopen,
    PromoteStory,
    PromoteDecision,
}

/// The move table of the spec: Wait only from `open`, Reopen from each other state, and the
/// rest only from an open state.
pub fn allowed(state: TicketState, act: Act) -> bool {
    match act {
        Act::Wait => state == TicketState::Open,
        Act::Reopen => state != TicketState::Open,
        Act::Done | Act::Drop | Act::PromoteStory | Act::PromoteDecision => state.is_open(),
    }
}

/// A write that the owner asks of one ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op {
    Move(TicketState, Option<String>),
    Promote(PromoteTo),
}

/// Sends `op` to the core of `core` as the owner. The result is the line to show.
pub fn run(
    core: &Entity<CoreState>,
    cx: &App,
    product: &str,
    id: &str,
    op: Op,
) -> LocalBoxFuture<'static, Result<String, String>> {
    let link = core.read(cx).link.clone();
    let (product, id) = (product.to_string(), id.to_string());
    match op {
        Op::Move(state, waiting_on) => {
            let call = link.call::<TicketMove>(&TicketMoveParams {
                product,
                id,
                state,
                waiting_on,
                by: owner(),
            });
            async move {
                call.await
                    .map(|r| format!("{} is {}.", r.ticket.id, r.ticket.state.as_str()))
                    .map_err(|e| e.to_string())
            }
            .boxed_local()
        }
        Op::Promote(to) => {
            let call = link.call::<TicketPromote>(&TicketPromoteParams {
                product,
                id,
                to,
                by: owner(),
            });
            async move {
                call.await
                    .map(|r| {
                        let made = match (&r.story, r.decision) {
                            (Some(s), _) => format!("story {}", s.id),
                            (_, Some(d)) => format!("decision {d}"),
                            _ => "a record".into(),
                        };
                        format!("{} became {made}.", r.ticket.id)
                    })
                    .map_err(|e| e.to_string())
            }
            .boxed_local()
        }
    }
}

pub fn kind_tag(kind: TicketKind) -> Tag {
    neutral_tag(kind.as_str())
}

/// The tag of the state of a ticket, with the wait target while it waits.
pub fn state_tag(t: &Ticket) -> Tag {
    match t.state {
        TicketState::Open => status_tag("open", Status::Info),
        TicketState::Waiting => status_tag(
            match &t.waiting_on {
                Some(w) => format!("waiting on {w}"),
                None => "waiting".into(),
            },
            Status::Warn,
        ),
        TicketState::Done => status_tag("done", Status::Ok),
        TicketState::Dropped => neutral_tag("dropped"),
    }
}

/// `owner Matias` or `session 3f2a`.
pub fn who(a: &Author) -> String {
    if a.id.is_empty() {
        a.by.as_str().to_string()
    } else {
        format!("{} {}", a.by.as_str(), a.id)
    }
}

/// What a click on a link does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkAction {
    Screen(ScreenRequest),
    Url(String),
    Copy(String),
}

pub fn link_action(product: &str, link: &Link) -> LinkAction {
    let r = link.reference.as_str();
    match link.kind {
        LinkType::Story => LinkAction::Screen(ScreenRequest::story(product, r)),
        LinkType::Minutes => LinkAction::Screen(ScreenRequest::minute(product, r)),
        LinkType::Priority => LinkAction::Screen(ScreenRequest::minute(
            product,
            r.split_once('#').map_or(r, |(id, _)| id),
        )),
        LinkType::Ticket => LinkAction::Screen(ScreenRequest::ticket(product, r)),
        LinkType::Decision => LinkAction::Screen(ScreenRequest::front_desk(product)),
        LinkType::Url => LinkAction::Url(r.to_string()),
        LinkType::Commit | LinkType::Path | LinkType::Issue => LinkAction::Copy(r.to_string()),
    }
}

/// The owner edits these fields.
struct Edit {
    title: Entity<InputState>,
    body: Entity<TextareaState>,
    kind: TicketKind,
    assignee: Option<String>,
}

pub struct TicketScreen {
    product: String,
    id: String,
    core: Entity<CoreState>,
    data: Load<Ticket>,
    page: Option<Entity<TextViewState>>,
    edit: Option<Edit>,
    /// The line "Waiting on" after a click on Wait.
    waiting: Option<Entity<InputState>>,
    confirm_story: bool,
    busy: bool,
    flash: Option<Flash>,
    scroll: ScrollHandle,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl TicketScreen {
    pub fn new(
        product: String,
        id: String,
        core: Entity<CoreState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut screen = TicketScreen {
            product,
            id,
            core,
            data: Load::Loading,
            page: None,
            edit: None,
            waiting: None,
            confirm_story: false,
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

    pub fn ticket(&self) -> Option<&Ticket> {
        match &self.data {
            Load::Ready(t) => Some(t),
            _ => None,
        }
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let call = self.core.read(cx).link.call::<TicketGet>(&TicketGetParams {
            product: self.product.clone(),
            id: self.id.clone(),
        });
        cx.spawn(async move |this, cx| {
            let result = call.await.map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(r) => {
                        match &this.page {
                            Some(page) => {
                                page.update(cx, |p, cx| p.set_text(&r.markdown_body(), cx))
                            }
                            None => {
                                this.page = Some(
                                    cx.new(|cx| TextViewState::markdown(&r.markdown_body(), cx)),
                                )
                            }
                        }
                        this.data.settle(Ok(r.ticket));
                    }
                    Err(e) => this.data.settle(Err(e)),
                }
                first_screen_ready();
                cx.notify();
            });
        })
        .detach();
    }

    /// Sends one move or promote. A refusal shows its message and its hint.
    pub fn apply(&mut self, op: Op, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let done = run(&self.core, cx, &self.product, &self.id, op);
        self.send(done, cx);
    }

    fn send(
        &mut self,
        done: LocalBoxFuture<'static, Result<String, String>>,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        self.flash = None;
        self.confirm_story = false;
        self.waiting = None;
        cx.spawn(async move |this, cx| {
            let result = done.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.flash = Some(match result {
                    Ok(m) => Flash::Done(m),
                    Err(e) => Flash::Failed(e),
                });
                this.reload(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Starts an action. Wait asks for its target, and Promote to story asks one Confirm.
    pub fn act(&mut self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.ticket() else { return };
        if !allowed(t.state, act) {
            return;
        }
        self.flash = None;
        match act {
            Act::Done => self.apply(Op::Move(TicketState::Done, None), cx),
            Act::Drop => self.apply(Op::Move(TicketState::Dropped, None), cx),
            Act::Reopen => self.apply(Op::Move(TicketState::Open, None), cx),
            Act::PromoteDecision => self.apply(Op::Promote(PromoteTo::Decision), cx),
            Act::PromoteStory => {
                self.confirm_story = true;
                cx.notify();
            }
            Act::Wait => {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder("Waiting on…"));
                self._subscriptions.push(cx.subscribe(
                    &input,
                    |this, input, e: &InputEvent, cx| {
                        if let InputEvent::PressEnter { .. } = e {
                            let target = input.read(cx).value().trim().to_string();
                            this.wait_on(target, cx);
                        }
                    },
                ));
                let focus = input.focus_handle(cx);
                window.defer(cx, move |window, cx| window.focus(&focus, cx));
                self.waiting = Some(input);
                cx.notify();
            }
        }
    }

    fn wait_on(&mut self, target: String, cx: &mut Context<Self>) {
        if target.is_empty() {
            self.flash = Some(Flash::Failed(
                "Write who or what the ticket waits on.".into(),
            ));
            cx.notify();
        } else {
            self.apply(Op::Move(TicketState::Waiting, Some(target)), cx);
        }
    }

    pub fn start_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.ticket().cloned() else {
            return;
        };
        let title = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_value(t.title.clone(), window, cx);
            s
        });
        let body = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx);
            s.set_value(t.body.clone(), window, cx);
            s
        });
        self.edit = Some(Edit {
            title,
            body,
            kind: t.kind,
            assignee: t.assignee.clone().filter(|a| a == "owner" || a == "claude"),
        });
        cx.notify();
    }

    fn save_edit(&mut self, cx: &mut Context<Self>) {
        let Some(e) = self.edit.take() else { return };
        let Some(t) = self.ticket() else { return };
        let title = e.title.read(cx).value().trim().to_string();
        if title.is_empty() {
            self.flash = Some(Flash::Failed("The title cannot be empty.".into()));
            self.edit = Some(e);
            cx.notify();
            return;
        }
        let body = e.body.read(cx).value().to_string();
        let params = TicketUpdateParams {
            product: self.product.clone(),
            id: self.id.clone(),
            title: (title != t.title).then_some(title),
            body: (body != t.body).then_some(body),
            kind: (e.kind != t.kind).then_some(e.kind),
            assignee: (e.assignee != t.assignee).then(|| e.assignee.clone().unwrap_or_default()),
            links: None,
            scope: None,
            objective: None,
            by: owner(),
        };
        let call = self.core.read(cx).link.call::<TicketUpdate>(&params);
        let id = self.id.clone();
        self.send(
            async move {
                call.await
                    .map(|_| format!("Saved {id}."))
                    .map_err(|e| e.to_string())
            }
            .boxed_local(),
            cx,
        );
    }

    fn open_link(&self, link: &Link, cx: &mut Context<Self>) {
        match link_action(&self.product, link) {
            LinkAction::Screen(r) => Nav::open(cx, &self.core, r),
            LinkAction::Url(u) => cx.open_url(&u),
            LinkAction::Copy(c) => cx.write_to_clipboard(ClipboardItem::new_string(c)),
        }
    }
}

/// The text of the Markdown view: the body, or a line when there is none.
trait BodyText {
    fn markdown_body(&self) -> String;
}

impl BodyText for agentsi_proto::ticket_api::TicketGetResult {
    fn markdown_body(&self) -> String {
        if self.ticket.body.trim().is_empty() {
            "_No text._".into()
        } else {
            self.ticket.body.clone()
        }
    }
}

fn act_label(act: Act) -> &'static str {
    match act {
        Act::Done => "Done",
        Act::Wait => "Wait",
        Act::Drop => "Drop",
        Act::Reopen => "Reopen",
        Act::PromoteStory => "Promote to story",
        Act::PromoteDecision => "Promote to decision",
    }
}

fn field(name: &str, value: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .gap(space(3))
        .text_size(size::sm())
        .child(
            div()
                .w(space(24))
                .flex_none()
                .text_color(text::tertiary())
                .child(name.to_string()),
        )
        .child(
            div()
                .min_w_0()
                .text_color(text::secondary())
                .child(value.into()),
        )
}

impl TicketScreen {
    fn actions(&self, t: &Ticket, cx: &mut Context<Self>) -> Div {
        let busy = self.busy;
        let buttons = [
            Act::Done,
            Act::Wait,
            Act::Drop,
            Act::Reopen,
            Act::PromoteStory,
            Act::PromoteDecision,
        ];
        div()
            .flex()
            .flex_wrap()
            .gap(space(2))
            .children(
                buttons
                    .into_iter()
                    .filter(|a| allowed(t.state, *a))
                    .map(|a| {
                        let b = Button::new(("ticket-act", a as usize))
                            .small()
                            .label(act_label(a));
                        let b = match a {
                            Act::Done => b.primary(),
                            Act::Drop => b.danger(),
                            _ => b.secondary(),
                        };
                        b.disabled(busy).on_click(
                            cx.listener(move |this, _, window, cx| this.act(a, window, cx)),
                        )
                    }),
            )
            .child(
                Button::new("ticket-edit")
                    .small()
                    .secondary()
                    .label("Edit")
                    .disabled(busy || self.edit.is_some())
                    .on_click(cx.listener(|this, _, window, cx| this.start_edit(window, cx))),
            )
    }

    fn links(&self, t: &Ticket, cx: &mut Context<Self>) -> Option<Div> {
        if t.links.is_empty() {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_wrap()
                .gap(space(2))
                .children(t.links.iter().enumerate().map(|(i, l)| {
                    let link = l.clone();
                    Button::new(("ticket-link", i))
                        .small()
                        .ghost()
                        .label(format!("{} {}", l.kind.as_str(), l.reference))
                        .on_click(cx.listener(move |this, _, _, cx| this.open_link(&link, cx)))
                })),
        )
    }

    fn edit_form(&self, e: &Edit, cx: &mut Context<Self>) -> Div {
        let kinds = TicketKind::ALL.into_iter().enumerate().map(|(i, k)| {
            let b = Button::new(("ticket-edit-kind", i))
                .small()
                .label(k.as_str());
            let b = if e.kind == k {
                b.primary()
            } else {
                b.secondary()
            };
            b.on_click(cx.listener(move |this, _, _, cx| {
                if let Some(e) = &mut this.edit {
                    e.kind = k;
                }
                cx.notify();
            }))
        });
        let who = [
            ("Unassigned", None),
            ("owner", Some("owner")),
            ("claude", Some("claude")),
        ];
        let assignees = who.into_iter().enumerate().map(|(i, (label, a))| {
            let a = a.map(String::from);
            let b = Button::new(("ticket-edit-who", i)).small().label(label);
            let b = if e.assignee == a {
                b.primary()
            } else {
                b.secondary()
            };
            b.on_click(cx.listener(move |this, _, _, cx| {
                if let Some(e) = &mut this.edit {
                    e.assignee = a.clone();
                }
                cx.notify();
            }))
        });
        div()
            .flex()
            .flex_col()
            .gap(space(3))
            .p(space(4))
            .rounded(radius::lg())
            .border_1()
            .border_color(line::border())
            .bg(surface::raised())
            .child(Input::new(&e.title))
            .child(Textarea::new(&e.body).h(space(40)))
            .child(div().flex().gap(space(2)).children(kinds))
            .child(div().flex().gap(space(2)).children(assignees))
            .child(
                div()
                    .flex()
                    .gap(space(2))
                    .child(
                        Button::new("ticket-save")
                            .small()
                            .primary()
                            .label("Save")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.save_edit(cx))),
                    )
                    .child(
                        Button::new("ticket-edit-cancel")
                            .small()
                            .secondary()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.edit = None;
                                cx.notify();
                            })),
                    ),
            )
    }

    fn story_confirm(&self, t: &Ticket, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(space(3))
            .p(space(4))
            .rounded(radius::lg())
            .border_1()
            .border_color(line::border())
            .bg(surface::raised())
            .child(section(&format!("Promote {} to a story", t.id), None))
            .child(mono(
                "The PM shapes a story from the title and the text, which costs money. The ticket becomes done.",
            ))
            .child(
                div()
                    .flex()
                    .gap(space(2))
                    .child(
                        Button::new("ticket-story-confirm")
                            .small()
                            .danger()
                            .label("Confirm")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.apply(Op::Promote(PromoteTo::Story), cx)
                            })),
                    )
                    .child(
                        Button::new("ticket-story-cancel")
                            .small()
                            .secondary()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_story = false;
                                cx.notify();
                            })),
                    ),
            )
    }
}

impl Render for TicketScreen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let menu = self.ticket().map(|t| {
            Target::Ticket(
                ProductRef {
                    host: self.core.read(cx).remote.clone(),
                    id: self.product.clone(),
                },
                t.id.clone(),
                t.state,
            )
        });
        let root = div()
            .id("ticket-screen")
            .track_focus(&self.focus)
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |_, e: &MouseDownEvent, _, cx| {
                    if let Some(target) = &menu {
                        menus::request(cx, target.clone(), e.position);
                    }
                }),
            )
            .size_full()
            .bg(surface::panel());
        let t = match &self.data {
            Load::Loading => return root.child(notice("Reading the ticket…", None)),
            Load::Failed(e) => {
                return root.child(notice(
                    e.clone(),
                    Some("The ticket may be gone. Open the tickets of the product.".into()),
                ));
            }
            Load::Ready(t) => t.clone(),
        };
        let waiting = self.waiting.clone();
        let body = if let Some(page) = &self.page
            && self.edit.is_none()
        {
            Some(TextView::new(page).selectable(true).into_any_element())
        } else {
            None
        };
        let edit = self.edit.as_ref().map(|e| self.edit_form(e, cx));
        root.child(
            div()
                .id("ticket-scroll")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(
                    column()
                        .py(space(8))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(space(2))
                                .child(
                                    div()
                                        .text_size(size::xl())
                                        .font_weight(weight::SEMIBOLD)
                                        .text_color(text::primary())
                                        .child(t.title.clone()),
                                )
                                .child(mono(t.id.clone()))
                                .child(
                                    div()
                                        .flex()
                                        .gap(space(2))
                                        .child(kind_tag(t.kind))
                                        .child(state_tag(&t)),
                                ),
                        )
                        .child(self.actions(&t, cx))
                        .when_some(waiting, |el, input| {
                            el.child(
                                div()
                                    .flex()
                                    .gap(space(2))
                                    .child(div().flex_1().child(Input::new(&input)))
                                    .child(
                                        Button::new("ticket-wait-go")
                                            .small()
                                            .primary()
                                            .label("Wait")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                let target =
                                                    input.read(cx).value().trim().to_string();
                                                this.wait_on(target, cx);
                                            })),
                                    ),
                            )
                        })
                        .when(self.confirm_story, |el| {
                            el.child(self.story_confirm(&t, cx))
                        })
                        .when_some(self.flash.as_ref(), |el, f| el.child(flash_line(f)))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(space(1))
                                .child(field(
                                    "Assignee",
                                    t.assignee.clone().unwrap_or_else(|| "none".into()),
                                ))
                                .child(field("Author", who(&t.author)))
                                .child(field("Last write", who(&t.updated_by)))
                                .when_some(t.scope.clone(), |el, s| el.child(field("Scope", s)))
                                .when_some(t.objective.clone(), |el, o| {
                                    el.child(field("Objective", o))
                                })
                                .child(field("Created", t.created_at.clone()))
                                .child(field("Updated", t.updated_at.clone()))
                                .when_some(t.closed_at.clone(), |el, c| {
                                    el.child(field("Closed", c))
                                }),
                        )
                        .when_some(self.links(&t, cx), |el, links| {
                            el.child(section("Links", Some(t.links.len()))).child(links)
                        })
                        .when_some(edit, |el, form| el.child(form))
                        .when_some(body, |el, body| {
                            el.child(section("Text", None)).child(div().child(body))
                        }),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `super` brings `gpui::test` in as `test`, and `#[test]` then loops.
    use super::{
        Act, Author, AuthorBy, Link, LinkAction, LinkType, Op, PromoteTo, ScreenRequest,
        TicketState, allowed, link_action, owner, who,
    };

    fn link(kind: LinkType, r: &str) -> Link {
        Link {
            kind,
            reference: r.into(),
        }
    }

    #[test]
    fn the_move_table_decides_which_actions_are_on() {
        use Act::*;
        let on = |s| {
            [Done, Wait, Drop, Reopen, PromoteStory, PromoteDecision]
                .into_iter()
                .filter(|a| allowed(s, *a))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            on(TicketState::Open),
            [Done, Wait, Drop, PromoteStory, PromoteDecision]
        );
        assert_eq!(
            on(TicketState::Waiting),
            [Done, Drop, Reopen, PromoteStory, PromoteDecision]
        );
        assert_eq!(on(TicketState::Done), [Reopen]);
        assert_eq!(on(TicketState::Dropped), [Reopen]);
    }

    #[test]
    fn a_link_opens_its_screen_the_browser_or_the_clipboard() {
        assert_eq!(
            link_action("demo", &link(LinkType::Story, "S014")),
            LinkAction::Screen(ScreenRequest::story("demo", "S014"))
        );
        assert_eq!(
            link_action("demo", &link(LinkType::Priority, "m1#2")),
            LinkAction::Screen(ScreenRequest::minute("demo", "m1"))
        );
        assert_eq!(
            link_action("demo", &link(LinkType::Ticket, "C007")),
            LinkAction::Screen(ScreenRequest::ticket("demo", "C007"))
        );
        assert_eq!(
            link_action("demo", &link(LinkType::Decision, "42")),
            LinkAction::Screen(ScreenRequest::front_desk("demo"))
        );
        assert_eq!(
            link_action("demo", &link(LinkType::Url, "https://x.dev")),
            LinkAction::Url("https://x.dev".into())
        );
        assert_eq!(
            link_action("demo", &link(LinkType::Commit, "1a2b3c4")),
            LinkAction::Copy("1a2b3c4".into())
        );
    }

    #[test]
    fn a_writer_shows_its_kind_and_its_id() {
        assert_eq!(who(&owner()), "owner");
        let s = Author {
            by: AuthorBy::Session,
            id: "3f2a".into(),
        };
        assert_eq!(who(&s), "session 3f2a");
    }

    use crate::link::Link as CoreLink;
    use crate::screens::parts::Load;
    use agentsi_proto::ticket_api::Ticket;
    use gpui::TestAppContext;
    use gpui_kit::AppContext as _;
    use std::sync::Arc;

    fn sent(other: &mut std::os::unix::net::UnixStream) -> Option<serde_json::Value> {
        other.set_nonblocking(true).unwrap();
        let frame = agentsi_proto::frame::read_frame(other).ok().flatten()?;
        serde_json::from_slice(&frame.payload).ok()
    }

    fn ticket(state: &str) -> Ticket {
        serde_json::from_value(serde_json::json!({
            "product": "demo", "id": "C001", "scope": null, "kind": "todo", "state": state,
            "title": "Install the bundle", "body": "Steps\n\n- one", "waiting_on": null,
            "assignee": "claude", "author": {"by": "session", "id": "3f2a"},
            "updated_by": {"by": "owner", "id": "Matias"},
            "links": [{"type": "story", "ref": "S014"}], "objective": null,
            "created_at": "2026-11-09T10:00:00.000Z", "updated_at": "2026-11-09T11:00:00.000Z",
            "closed_at": null
        }))
        .unwrap()
    }

    #[gpui::test]
    fn the_screen_draws_each_state_and_its_actions_send_the_writes_as_the_owner(
        cx: &mut TestAppContext,
    ) {
        cx.update(agentsi_ui::init);
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| crate::model::CoreState::new(Arc::new(CoreLink::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| {
            super::TicketScreen::new("demo".into(), "C001".into(), core, cx)
        });
        let read = sent(&mut other).unwrap();
        assert_eq!(read["method"], "ticket.get");
        assert_eq!(read["params"]["id"], "C001");
        for data in [
            Load::Loading,
            Load::Failed("gone".into()),
            Load::Ready(ticket("open")),
            Load::Ready(ticket("waiting")),
            Load::Ready(ticket("done")),
        ] {
            screen.update(cx, |s, cx| {
                s.data = data;
                cx.notify();
            });
            cx.update(|window, cx| window.draw(cx).clear(cx));
        }
        screen.update(cx, |s, _| s.data = Load::Ready(ticket("open")));
        // Promote to story spends money: it asks and sends nothing before the Confirm.
        screen.update_in(cx, |s, window, cx| s.act(Act::PromoteStory, window, cx));
        assert!(sent(&mut other).is_none());
        screen.update(cx, |s, cx| s.apply(Op::Promote(PromoteTo::Story), cx));
        let promote = sent(&mut other).expect("a request");
        assert_eq!(promote["method"], "ticket.promote");
        assert_eq!(promote["params"]["to"], "story");
        assert_eq!(promote["params"]["by"]["by"], "owner");
        // Drop is not on for a done ticket: the action sends nothing.
        screen.update(cx, |s, _| {
            s.busy = false;
            s.data = Load::Ready(ticket("done"));
        });
        screen.update_in(cx, |s, window, cx| s.act(Act::Drop, window, cx));
        assert!(sent(&mut other).is_none());
        screen.update_in(cx, |s, window, cx| s.act(Act::Reopen, window, cx));
        let reopen = sent(&mut other).expect("a request");
        assert_eq!(reopen["method"], "ticket.move");
        assert_eq!(reopen["params"]["state"], "open");
    }
}
