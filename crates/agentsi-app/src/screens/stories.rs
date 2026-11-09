//! The stories of a product (U23) as a list, archived ones included. The list takes
//! focus as one element: the arrow keys move a cursor and `Enter` opens the story of
//! the cursor, as on the minutes screen.

use agentsi_proto::story_api::{Story, StoryList, StoryListParams};
use agentsi_ui::components::{ListRow, Tag, neutral_tag, status_tag};
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{Status, control, line, size, space, surface, text, weight};
use gpui_kit::*;

use super::minutes::cursor_after;
use super::parts::{Load, first_screen_ready, notice};
use crate::model::CoreState;
use crate::nav::{Nav, ScreenRequest};

pub struct StoriesScreen {
    product: String,
    core: Entity<CoreState>,
    data: Load<Vec<Story>>,
    scroll: UniformListScrollHandle,
    cursor: usize,
    focus: FocusHandle,
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
        };
        screen.reload(cx);
        screen
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
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
                ),
        )
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
}
