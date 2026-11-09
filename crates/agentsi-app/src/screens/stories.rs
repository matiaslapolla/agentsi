//! The stories of a product (U23) as a list, archived ones included. The list takes
//! focus as one element: the arrow keys move a cursor and `Enter` opens the story of
//! the cursor, as on the minutes screen.

use agentsi_proto::story_api::{
    ProductParams, Story, StoryList, StoryListParams, StoryNew, StoryNewParams, StorySuggest,
};
use agentsi_ui::components::{Button, ListRow, Tag, neutral_tag, status_tag};
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{Status, control, line, size, space, surface, text, weight};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::minutes::cursor_after;
use super::form::{Form, FormEvent};
use super::parts::{Flash, Load, first_screen_ready, flash_line, notice};
use crate::model::CoreState;
use crate::nav::{Nav, ScreenRequest};

pub struct StoriesScreen {
    product: String,
    core: Entity<CoreState>,
    data: Load<Vec<Story>>,
    scroll: UniformListScrollHandle,
    cursor: usize,
    focus: FocusHandle,
    /// Set by the menu and the palette: the next draw opens the form "New story".
    want_form: bool,
    form: Option<(Entity<Form>, Subscription)>,
    /// The owner clicked "Suggest" and the screen waits for the Confirm.
    suggest_confirm: bool,
    /// A model call runs.
    busy: bool,
    flash: Option<Flash>,
}

/// The tag of a story: its session state while it has a session, else its status or approval.
pub fn story_tag(s: &Story) -> Tag {
    if let Some(session) = &s.session {
        let status = match session.state.as_str() {
            "ready" => Status::Ok,
            "failed" => Status::Danger,
            _ => Status::Info,
        };
        return status_tag(format!("session {}", session.state), status);
    }
    match (s.status.as_str(), s.approval.as_str()) {
        ("implemented", _) => status_tag("implemented", Status::Ok),
        ("discarded", _) => neutral_tag("discarded"),
        (_, "approved") => status_tag("approved", Status::Info),
        (_, "changes_requested") => status_tag("changes requested", Status::Warn),
        (_, approval) => neutral_tag(approval.to_string()),
    }
}

/// The line under the title of a row: id, type, owner and the archive mark.
pub fn story_meta(s: &Story) -> String {
    let mut parts = vec![
        s.id.clone(),
        s.kind.clone(),
        format!("owner {}", s.owner_role),
    ];
    if s.archived == Some(true) {
        parts.push("archived".into());
    }
    parts.join(" · ")
}

impl StoriesScreen {
    pub fn new(product: String, core: Entity<CoreState>, cx: &mut Context<Self>) -> Self {
        let mut screen = StoriesScreen {
            product,
            core,
            data: Load::Loading,
            scroll: UniformListScrollHandle::new(),
            cursor: 0,
            focus: cx.focus_handle().tab_stop(true),
            want_form: false,
            form: None,
            suggest_confirm: false,
            busy: false,
            flash: None,
        };
        screen.reload(cx);
        screen
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    /// Opens the form "New story" on the next draw.
    pub fn new_story(&mut self, cx: &mut Context<Self>) {
        self.want_form = true;
        cx.notify();
    }

    fn close_form(&mut self, cx: &mut Context<Self>) {
        self.want_form = false;
        self.form = None;
        cx.notify();
    }

    fn ensure_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.want_form || self.form.is_some() {
            return;
        }
        let form = cx.new(|cx| {
            Form::new("stories-new", "New story", "Shape the story", cx)
                .field("Text", "Describe the story. The PM shapes it.", true, window, cx)
                .pick("Type", &["The PM picks", "Story", "Task"])
                .confirm("The PM calls the model, which costs money.")
        });
        let sub = cx.subscribe(&form, |this, form, e: &FormEvent, cx| match e {
            FormEvent::Submit => this.submit_form(&form, cx),
            FormEvent::Cancel => this.close_form(cx),
        });
        form.update(cx, |f, cx| f.focus_first(window, cx));
        self.form = Some((form, sub));
    }

    fn submit_form(&mut self, form: &Entity<Form>, cx: &mut Context<Self>) {
        let (text, kind) = {
            let f = form.read(cx);
            let kind = match f.picked(0) {
                "Story" => Some("story".to_string()),
                "Task" => Some("task".to_string()),
                _ => None,
            };
            (f.value(0, cx), kind)
        };
        let call = self.core.read(cx).link.call::<StoryNew>(&StoryNewParams {
            product: self.product.clone(),
            text,
            kind,
            minutes: None,
        });
        self.close_form(cx);
        self.propose(call, cx);
    }

    /// `story.suggest`: the first click asks for the Confirm, the second one calls the PM.
    pub fn suggest(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if !self.suggest_confirm {
            self.suggest_confirm = true;
            cx.notify();
            return;
        }
        self.run_suggest(cx);
    }

    /// Calls the PM at once: the Confirm came from the menu or from the click above.
    pub fn run_suggest(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.suggest_confirm = false;
        let call = self
            .core
            .read(cx)
            .link
            .call::<StorySuggest>(&ProductParams {
                product: self.product.clone(),
            });
        self.propose(call, cx);
    }

    fn propose(
        &mut self,
        call: impl std::future::Future<
            Output = Result<
                agentsi_proto::story_api::StoryProposeResult,
                crate::link::CallError,
            >,
        > + 'static,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        self.flash = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                this.flash = Some(match result {
                    Ok(r) => {
                        let ids: Vec<_> = r.stories.iter().map(|s| s.id.as_str()).collect();
                        Flash::Done(format!("Proposed: {}.", ids.join(", ")))
                    }
                    Err(e) => Flash::Failed(e.to_string()),
                });
                this.reload(cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let call = self.core.read(cx).link.call::<StoryList>(&StoryListParams {
            product: self.product.clone(),
            include_archived: true,
        });
        cx.spawn(async move |this, cx| {
            let result = call.await.map(|r| r.stories).map_err(|e| e.to_string());
            let _ = this.update(cx, |this, cx| {
                this.data.settle(result);
                if let Load::Ready(rows) = &this.data {
                    this.cursor = this.cursor.min(rows.len().saturating_sub(1));
                }
                first_screen_ready();
                cx.notify();
            });
        })
        .detach();
    }

    fn rows(&self) -> &[Story] {
        match &self.data {
            Load::Ready(rows) => rows,
            _ => &[],
        }
    }

    fn open(&self, row: usize, cx: &App) {
        if let Some(s) = self.rows().get(row) {
            Nav::open(cx, &self.core, ScreenRequest::story(&self.product, &s.id));
        }
    }

    fn key_down(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.modified() {
            return;
        }
        if k.key == "enter" {
            self.open(self.cursor, cx);
            cx.stop_propagation();
        } else if let Some(to) = cursor_after(&k.key, self.cursor, self.rows().len()) {
            self.cursor = to;
            self.scroll.scroll_to_item(to, ScrollStrategy::Nearest);
            cx.stop_propagation();
            cx.notify();
        }
    }
}

impl Render for StoriesScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_form(window, cx);
        let form = self.form.as_ref().map(|(f, _)| f.clone());
        let flash = self.flash.as_ref().map(flash_line);
        let (busy, confirming) = (self.busy, self.suggest_confirm);
        let root = div().size_full().flex().flex_col().bg(surface::panel());
        let count = self.rows().len();
        let open = self
            .rows()
            .iter()
            .filter(|s| s.status == "not_implemented" && s.archived != Some(true))
            .count();
        let body = match &self.data {
            Load::Loading => notice("Reading the stories…", None).into_any_element(),
            Load::Failed(e) => notice(
                e.clone(),
                Some("Stories mode needs a stories block in agentsi.config.json.".into()),
            )
            .into_any_element(),
            Load::Ready(rows) if rows.is_empty() => notice(
                "No stories for this product yet.",
                Some("Propose one with agentsi story new \"<text>\".".into()),
            )
            .into_any_element(),
            Load::Ready(_) => {
                let focused = self.focus.is_focused(window);
                uniform_list(
                    "stories-list",
                    count,
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .filter_map(|i| {
                                let s = this.rows().get(i)?;
                                let (id, product) = (s.id.clone(), this.product.clone());
                                let row = ListRow::new(("stories-row", i), s.title.clone())
                                    .meta(story_meta(s))
                                    .selected(this.cursor == i)
                                    .focused(this.cursor == i && focused)
                                    .trailing(story_tag(s))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.cursor = i;
                                        window.focus(&this.focus, cx);
                                        Nav::open(
                                            cx,
                                            &this.core,
                                            ScreenRequest::story(&product, &id),
                                        );
                                        cx.notify();
                                    }));
                                Some(div().pad_x(space(1)).child(row))
                            })
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll)
                .flex_1()
                .p(space(1))
                .into_any_element()
            }
        };
        root.child(
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
                        .child("Stories"),
                )
                .child(
                    div()
                        .text_size(size::sm())
                        .text_color(text::tertiary())
                        .child(format!("{count} stories · {open} open")),
                )
                .child(div().flex_1())
                .child(
                    Button::new("stories-new")
                        .secondary()
                        .small()
                        .label("New story")
                        .on_click(cx.listener(|this, _, _, cx| this.new_story(cx))),
                )
                .child(
                    Button::new("stories-suggest")
                        .secondary()
                        .small()
                        .label(if confirming { "Confirm" } else { "Suggest" })
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| this.suggest(cx))),
                ),
        )
        .when(confirming, |el| {
            el.child(
                div()
                    .pad_x(space(4))
                    .pt(space(2))
                    .text_size(size::sm())
                    .text_color(Status::Warn.color())
                    .child("The PM proposes up to 3 stories. It calls the model, which costs money. Click Confirm to go on."),
            )
        })
        .when_some(form, |el, f| el.child(div().p(space(4)).child(f)))
        .when_some(flash, |el, f| el.child(div().p(space(4)).child(f)))
        .child(
            div()
                .id("stories-body")
                .track_focus(&self.focus)
                .key_context("StoriesList")
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
    use super::{story_meta, story_tag};
    use agentsi_proto::story_api::Story;

    fn story() -> Story {
        serde_json::from_value(serde_json::json!({
            "id": "S001", "type": "story", "slug": "a", "title": "A", "owner_role": "dev",
            "status": "not_implemented", "approval": "approved", "proposed_by": "pm",
            "created_at": "2026-10-01T10:00:00.000Z", "updated_at": "2026-10-01T10:00:00.000Z"
        }))
        .unwrap()
    }

    #[test]
    fn a_row_shows_the_id_the_owner_and_the_archive_mark() {
        let mut s = story();
        assert_eq!(story_meta(&s), "S001 · story · owner dev");
        s.archived = Some(true);
        assert_eq!(story_meta(&s), "S001 · story · owner dev · archived");
        let _ = story_tag(&s);
    }

    fn request(other: &mut std::os::unix::net::UnixStream) -> Option<serde_json::Value> {
        other.set_nonblocking(true).unwrap();
        let f = agentsi_proto::frame::read_frame(other).ok().flatten()?;
        serde_json::from_slice(&f.payload).ok()
    }

    fn screen(
        cx: &mut gpui::TestAppContext,
    ) -> (
        gpui::Entity<super::StoriesScreen>,
        std::os::unix::net::UnixStream,
        &mut gpui::VisualTestContext,
    ) {
        use gpui_kit::AppContext as _;
        cx.update(agentsi_ui::init);
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| {
            crate::model::CoreState::new(std::sync::Arc::new(crate::link::Link::for_tests(end)))
        });
        let (screen, cx) =
            cx.add_window_view(|_, cx| super::StoriesScreen::new("demo".into(), core, cx));
        assert_eq!(request(&mut other).unwrap()["method"], "story.list");
        (screen, other, cx)
    }

    #[gpui::test]
    fn new_story_takes_a_text_and_a_type_and_asks_one_confirm_before_the_call(
        cx: &mut gpui::TestAppContext,
    ) {
        let (screen, mut other, cx) = screen(cx);
        screen.update(cx, |s, cx| s.new_story(cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("form:stories-new").is_some());
        let form = screen.read_with(cx, |s, _| s.form.as_ref().unwrap().0.clone());
        form.update_in(cx, |f, w, cx| {
            f.set_value(0, "Add a login", w, cx);
            f.select(0, 2, cx);
            f.submit(cx);
        });
        assert!(request(&mut other).is_none(), "the first click only asks");
        form.update(cx, |f, cx| f.submit(cx));
        let r = request(&mut other).expect("story.new");
        assert_eq!(r["method"], "story.new");
        assert_eq!(
            r["params"],
            serde_json::json!({"product": "demo", "text": "Add a login", "type": "task"})
        );
        assert!(screen.read_with(cx, |s, _| s.form.is_none()));
    }

    #[gpui::test]
    fn suggest_asks_one_confirm_then_calls_story_suggest(cx: &mut gpui::TestAppContext) {
        let (screen, mut other, cx) = screen(cx);
        screen.update(cx, |s, cx| s.suggest(cx));
        assert!(request(&mut other).is_none());
        screen.update(cx, |s, cx| s.suggest(cx));
        let r = request(&mut other).expect("story.suggest");
        assert_eq!(r["method"], "story.suggest");
        assert_eq!(r["params"], serde_json::json!({"product": "demo"}));
    }
}
