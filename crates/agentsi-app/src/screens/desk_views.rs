//! The views of the desk conversation other than the decisions view.
//!
//! An object view keeps only the id that Claude gave. It reads the stored object from the core
//! and reads it again on `data.changed` of its kind: `minutes` for priorities, `stories` for a
//! story and a diff. A button that writes asks a Confirm first. A button that would run a
//! story or spend money only opens the story screen.
//!
//! Spec: `docs/specs/agentsi-desktop-conversation-views.md`.

use std::collections::HashMap;

use agentsi_proto::desk_api::{MinutesDetail, MinutesGet, MinutesGetParams, PriorityInfo};
use agentsi_proto::desk_session_api::View;
use agentsi_proto::marks_api::{Mark, MarkEntry, MarkSet, MarkSetParams, MarkSetResult};
use agentsi_proto::story_api::{
    StoryGet, StoryGetResult, StoryIdParams, StoryReview, StoryReviewResult,
};
use agentsi_ui::components::{Button, Card, neutral_tag};
use agentsi_ui::tokens::{WorkState, control, line, radius, size, space, surface, text, weight};
use gpui_kit::component::diff::{Diff, DiffFile, DiffParseOptions, DiffState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Confirm, DeskConversation, Row};
use crate::link::CallError;
use crate::nav::{Nav, ScreenRequest};
use crate::screens::parts::{Flash, Live, Load, mark_tag, mono, role_label};
use crate::screens::stories::story_tag;
use crate::screens::story::DIFF_LINES;

/// The stored objects of the object views, by id.
#[derive(Default)]
pub(super) struct Objects {
    pub(super) minutes: HashMap<String, Load<MinutesDetail>>,
    pub(super) stories: HashMap<String, Load<StoryGetResult>>,
    pub(super) diffs: HashMap<String, Load<DiffShown>>,
}

/// The diff of a story as the view draws it: at most `DIFF_LINES` lines.
pub(super) struct DiffShown {
    pub(super) story: StoryReviewResult,
    /// The parsed patch. `None` when the patch does not parse: then the view draws the lines.
    pub(super) state: Option<Entity<DiffState>>,
    pub(super) lines: Vec<String>,
    /// The lines after `DIFF_LINES`.
    pub(super) more: usize,
}

/// The views that a read covers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Reads {
    All,
    /// The priorities views: `data.changed` of kind `minutes`.
    Minutes,
    /// The story and diff views: `data.changed` of kind `stories`.
    Stories,
}

impl Reads {
    fn covers(self, view: &View) -> bool {
        matches!(
            (self, view),
            (Reads::All, _)
                | (Reads::Minutes, View::Priorities { .. })
                | (Reads::Stories, View::Story { .. } | View::Diff { .. })
        )
    }
}

impl DeskConversation {
    // ---- Reading ----------------------------------------------------------------------

    /// Reads the objects of each view that `which` covers, one read for each object.
    pub(super) fn read_objects(&mut self, which: Reads, cx: &mut Context<Self>) {
        let mut seen = Vec::new();
        let views: Vec<View> = self
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::View(v) if which.covers(v) && !seen.contains(v) => {
                    seen.push(v.clone());
                    Some(v.clone())
                }
                _ => None,
            })
            .collect();
        for v in views {
            self.read_view(&v, cx);
        }
    }

    /// Reads the object of one view. A view that holds its data reads nothing.
    pub(super) fn read_view(&mut self, view: &View, cx: &mut Context<Self>) {
        let link = self.core.read(cx).link.clone();
        match view {
            View::Decisions { .. } => {}
            View::Priorities { minutes_id } => {
                let id = minutes_id.clone();
                self.objects
                    .minutes
                    .entry(id.clone())
                    .or_insert(Load::Loading);
                let call = link.call::<MinutesGet>(&MinutesGetParams { id: id.clone() });
                cx.spawn(async move |this, cx| {
                    let result = call.await;
                    let _ = this.update(cx, |this, cx| this.minutes_done(id, result, cx));
                })
                .detach();
            }
            View::Story { id } => {
                let id = id.clone();
                self.objects
                    .stories
                    .entry(id.clone())
                    .or_insert(Load::Loading);
                let call = link.call::<StoryGet>(&StoryIdParams {
                    product: self.product.clone(),
                    id: id.clone(),
                });
                cx.spawn(async move |this, cx| {
                    let result = call.await;
                    let _ = this.update(cx, |this, cx| this.story_done(id, result, cx));
                })
                .detach();
            }
            View::Diff { story_id } => {
                let id = story_id.clone();
                self.objects
                    .diffs
                    .entry(id.clone())
                    .or_insert(Load::Loading);
                let call = link.call::<StoryReview>(&StoryIdParams {
                    product: self.product.clone(),
                    id: id.clone(),
                });
                cx.spawn(async move |this, cx| {
                    let result = call.await;
                    let _ = this.update(cx, |this, cx| this.review_done(id, result, cx));
                })
                .detach();
            }
        }
    }

    pub(crate) fn minutes_done(
        &mut self,
        id: String,
        result: Result<MinutesDetail, CallError>,
        cx: &mut Context<Self>,
    ) {
        self.objects
            .minutes
            .entry(id)
            .or_insert(Load::Loading)
            .settle(result.map_err(|e| e.to_string()));
        self.views_changed(cx);
    }

    pub(crate) fn story_done(
        &mut self,
        id: String,
        result: Result<StoryGetResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        self.objects
            .stories
            .entry(id)
            .or_insert(Load::Loading)
            .settle(result.map_err(|e| e.to_string()));
        self.views_changed(cx);
    }

    pub(crate) fn review_done(
        &mut self,
        id: String,
        result: Result<StoryReviewResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        let old = self.objects.diffs.remove(&id);
        let state = match &old {
            Some(Load::Ready(d)) => d.state.clone(),
            _ => None,
        };
        let shown = result.map_err(|e| e.to_string()).map(|story| {
            let all: Vec<&str> = story.diff.lines().collect();
            let more = all.len().saturating_sub(DIFF_LINES);
            let lines: Vec<String> = all[..all.len().min(DIFF_LINES)]
                .iter()
                .map(|l| l.to_string())
                .collect();
            let mut patch = lines.join("\n");
            patch.push('\n');
            let options = DiffParseOptions::new().truncated_hunks(more > 0);
            let state = match DiffFile::parse_with(&patch, options) {
                Ok(files) if !files.is_empty() => Some(match state {
                    Some(s) => {
                        s.update(cx, |s, cx| s.set_files(files, cx));
                        s
                    }
                    None => cx.new(|cx| DiffState::new(files, cx)),
                }),
                _ => None,
            };
            DiffShown {
                story,
                state,
                lines,
                more,
            }
        });
        let mut entry = old.unwrap_or(Load::Loading);
        entry.settle(shown);
        self.objects.diffs.insert(id, entry);
        self.views_changed(cx);
    }

    /// The rows of the views changed height.
    fn views_changed(&mut self, cx: &mut Context<Self>) {
        let n = self.rows.len();
        self.scroller.update(cx, |s, cx| {
            s.remeasure_items(0..n, cx);
        });
        cx.notify();
    }

    // ---- Writing ----------------------------------------------------------------------

    /// Asks the Confirm of a mark on a priority of a priorities view.
    pub fn ask_mark(&mut self, minutes_id: &str, idx: u32, mark: Mark, cx: &mut Context<Self>) {
        self.confirm = Some(Confirm::Mark(minutes_id.to_string(), idx, mark));
        cx.notify();
    }

    /// Sets the mark with `unmarked_only`: a priority that the owner marked elsewhere since
    /// the view read it is refused, not changed.
    pub(super) fn set_mark(
        &mut self,
        minutes_id: String,
        idx: u32,
        mark: Mark,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.flash = None;
        let call = self.core.read(cx).link.call::<MarkSet>(&MarkSetParams {
            entries: vec![MarkEntry {
                minutes_id: minutes_id.clone(),
                idx,
                mark,
                is_new: false,
            }],
            unmarked_only: true,
        });
        let core = self.core.clone();
        cx.spawn(async move |this, cx| {
            let result = call.await;
            let _ = this.update(cx, |this, cx| {
                this.mark_done(minutes_id, idx, mark, result, cx)
            });
            cx.update(|cx| Live::reload_core(&core, cx));
        })
        .detach();
    }

    pub(crate) fn mark_done(
        &mut self,
        minutes_id: String,
        idx: u32,
        mark: Mark,
        result: Result<MarkSetResult, CallError>,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        self.flash = Some(match result {
            Ok(_) => Flash::Done(format!(
                "Marked priority {} of {minutes_id} as {}.",
                idx + 1,
                mark.as_str()
            )),
            Err(e) => Flash::Failed(e.to_string()),
        });
        // The view shows the stored mark. The owner needs no new message for it.
        self.read_view(&View::Priorities { minutes_id }, cx);
        cx.notify();
    }

    /// Opens the story screen. A view never runs a story itself.
    pub fn open_story(&mut self, id: &str, cx: &mut Context<Self>) {
        Nav::open(cx, &self.core, ScreenRequest::story(&self.product, id));
    }

    // ---- Drawing ----------------------------------------------------------------------

    pub(super) fn view_element(
        &self,
        row: usize,
        view: &View,
        weak: &WeakEntity<Self>,
    ) -> AnyElement {
        match view {
            // The conversation draws the decisions view itself.
            View::Decisions { .. } => div().into_any_element(),
            View::Priorities { minutes_id } => self.priorities_view(row, minutes_id, weak),
            View::Story { id } => self.story_view(row, id, weak),
            View::Diff { story_id } => self.diff_view(row, story_id, weak),
        }
    }

    fn priorities_view(&self, row: usize, minutes_id: &str, weak: &WeakEntity<Self>) -> AnyElement {
        let m = match self.objects.minutes.get(minutes_id) {
            Some(Load::Ready(m)) => m,
            other => return pending_line(other, &format!("the minutes {minutes_id}")),
        };
        div()
            .flex()
            .flex_col()
            .gap(space(2))
            .child(view_title(
                "Priorities",
                &format!("{} · {}", m.id, m.question),
            ))
            .when(m.priorities.is_empty(), |el| {
                el.child(muted("These minutes have no priorities."))
            })
            .children(
                m.priorities
                    .iter()
                    .map(|p| self.priority_card(row, &m.id, p, weak)),
            )
            .into_any_element()
    }

    fn priority_card(
        &self,
        row: usize,
        minutes_id: &str,
        p: &PriorityInfo,
        weak: &WeakEntity<Self>,
    ) -> AnyElement {
        let key = format!("deskv-{row}-p{}", p.idx);
        let mut card = Card::new(SharedString::from(key.clone()))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(space(2))
                    .child(neutral_tag("Priority"))
                    .child(mono(format!(
                        "{} · {}",
                        p.idx + 1,
                        role_label(p.owner_role.as_deref().unwrap_or("pm"))
                    )))
                    .child(mark_tag(p.mark.as_deref())),
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
        if p.mark.is_some() {
            return card.into_any_element();
        }
        let busy = self.busy;
        let asked = match &self.confirm {
            Some(Confirm::Mark(m, idx, mark)) if m == minutes_id && *idx == p.idx => Some(*mark),
            _ => None,
        };
        if let Some(mark) = asked {
            card = card
                .child(muted(&format!(
                    "Mark priority {} as {}?",
                    p.idx + 1,
                    mark_label(mark)
                )))
                .child(confirm_buttons(&key, busy, weak));
            return card.into_any_element();
        }
        let ask = |mark: Mark| {
            let (weak, id, idx) = (weak.clone(), minutes_id.to_string(), p.idx);
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let _ = weak.update(cx, |this, cx| this.ask_mark(&id, idx, mark, cx));
            }
        };
        card.child(
            div()
                .flex()
                .flex_wrap()
                .gap(space(2))
                .child(
                    Button::new(SharedString::from(format!("{key}-accept")))
                        .primary()
                        .small()
                        .label("Accept")
                        .disabled(busy)
                        .on_click(ask(Mark::Accepted)),
                )
                .child(
                    Button::new(SharedString::from(format!("{key}-change")))
                        .secondary()
                        .small()
                        .label("Accept with changes")
                        .disabled(busy)
                        .on_click(ask(Mark::Changed)),
                )
                .child(
                    Button::new(SharedString::from(format!("{key}-reject")))
                        .secondary()
                        .small()
                        .label("Reject")
                        .disabled(busy)
                        .on_click(ask(Mark::Rejected)),
                ),
        )
        .into_any_element()
    }

    fn story_view(&self, row: usize, id: &str, weak: &WeakEntity<Self>) -> AnyElement {
        let got = match self.objects.stories.get(id) {
            Some(Load::Ready(g)) => g,
            other => return pending_line(other, &format!("the story {id}")),
        };
        let s = &got.story;
        Card::new(SharedString::from(format!("deskv-{row}-story")))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(space(2))
                    .child(neutral_tag(if s.kind == "task" { "Task" } else { "Story" }))
                    .child(mono(s.id.clone()))
                    .child(story_tag(s))
                    .when(got.archived, |el| el.child(neutral_tag("archived"))),
            )
            .child(
                div()
                    .text_size(size::md())
                    .font_weight(weight::SEMIBOLD)
                    .text_color(text::primary())
                    .child(s.title.clone()),
            )
            .when(!s.i_want.is_empty(), |el| {
                el.child(muted(&format!("I want {}", s.i_want)))
            })
            .child(
                div()
                    .text_size(size::sm())
                    .font_weight(weight::SEMIBOLD)
                    .text_color(text::secondary())
                    .child(format!("Criteria ({})", s.criteria.len())),
            )
            .children(s.criteria.iter().map(|c| {
                div()
                    .text_size(size::sm())
                    .text_color(text::secondary())
                    .child(format!(
                        "Given {}, when {}, then {}.",
                        c.given, c.when, c.then
                    ))
            }))
            .child(open_button(row, &s.id, weak))
            .into_any_element()
    }

    fn diff_view(&self, row: usize, story_id: &str, weak: &WeakEntity<Self>) -> AnyElement {
        let d = match self.objects.diffs.get(story_id) {
            Some(Load::Ready(d)) => d,
            other => return pending_line(other, &format!("the diff of {story_id}")),
        };
        let body = if d.lines.is_empty() {
            muted("The branch of this story has no diff.").into_any_element()
        } else {
            match &d.state {
                Some(state) => div()
                    .h(control::DESK_VIEW_HEIGHT)
                    .rounded(radius::md())
                    .border_1()
                    .border_color(line::border())
                    .overflow_hidden()
                    .child(Diff::new(state).size_full())
                    .into_any_element(),
                None => div()
                    .flex()
                    .flex_col()
                    .p(space(2))
                    .rounded(radius::md())
                    .bg(surface::raised())
                    .children(
                        d.lines
                            .iter()
                            .take(DIFF_TEXT_LINES)
                            .map(|l| mono(l.clone())),
                    )
                    .into_any_element(),
            }
        };
        div()
            .flex()
            .flex_col()
            .gap(space(2))
            .child(view_title(
                "Diff",
                &format!("{} · {}", d.story.story.id, d.story.story.title),
            ))
            .children(
                d.story
                    .stat
                    .lines()
                    .last()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| mono(l.trim().to_string())),
            )
            .child(body)
            .when(d.more > 0, |el| {
                el.child(muted(&format!(
                    "{} more lines. Open the story for the whole diff.",
                    d.more
                )))
            })
            .child(open_button(row, &d.story.story.id, weak))
            .into_any_element()
    }
}

/// The lines that a diff with no parsed patch draws as text.
const DIFF_TEXT_LINES: usize = 40;

fn mark_label(mark: Mark) -> &'static str {
    match mark {
        Mark::Accepted => "accepted",
        Mark::Changed => "accepted with changes",
        Mark::Rejected => "rejected",
    }
}

fn muted(s: &str) -> Div {
    div()
        .text_size(size::sm())
        .text_color(text::secondary())
        .child(s.to_string())
}

/// The line of a view whose object has not come, or did not load.
fn pending_line<T>(load: Option<&Load<T>>, what: &str) -> AnyElement {
    match load {
        Some(Load::Failed(e)) => div()
            .text_size(size::sm())
            .text_color(WorkState::Failed.color())
            .child(format!("Cannot read {what}: {e}"))
            .into_any_element(),
        _ => div()
            .text_size(size::sm())
            .text_color(text::tertiary())
            .child(format!("Reading {what}…"))
            .into_any_element(),
    }
}

fn view_title(kind: &str, title: &str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(space(2))
        .min_w_0()
        .child(neutral_tag(kind.to_string()))
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(size::sm())
                .font_weight(weight::SEMIBOLD)
                .text_color(text::primary())
                .child(title.to_string()),
        )
}

fn open_button(row: usize, id: &str, weak: &WeakEntity<DeskConversation>) -> impl IntoElement {
    let (weak, id) = (weak.clone(), id.to_string());
    div().flex().child(
        Button::new(SharedString::from(format!("deskv-{row}-open")))
            .secondary()
            .small()
            .label("Open the story")
            .on_click(move |_, _, cx| {
                let _ = weak.update(cx, |this, cx| this.open_story(&id, cx));
            }),
    )
}

fn confirm_buttons(key: &str, busy: bool, weak: &WeakEntity<DeskConversation>) -> Div {
    let call = |f: fn(&mut DeskConversation, &mut Context<DeskConversation>)| {
        let weak = weak.clone();
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            let _ = weak.update(cx, f);
        }
    };
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
        )
}

#[cfg(test)]
mod tests {
    use agentsi_proto::desk_api::MinutesDetail;
    use agentsi_proto::events::DataChanged;
    use agentsi_proto::marks_api::{Mark, MarkSetResult};
    use agentsi_proto::rpc::Notification;
    use agentsi_proto::story_api::{StoryGetResult, StoryReviewResult};
    use gpui::{TestAppContext, VisualTestContext};
    use gpui_kit::Entity;
    use serde_json::{Value, json};

    // No `use super::*`: see the tests of the conversation.
    use super::super::tests::{draw, entry, history, methods_sent, setup};
    use super::{DeskConversation, Load};
    use crate::link::CallError;
    use crate::nav::{Nav, ScreenRequest};
    use crate::screens::parts::Flash;

    pub(in crate::screens) fn minutes(mark: Option<&str>) -> MinutesDetail {
        serde_json::from_value(json!({
            "id": "m1", "product": "demo", "scope": null, "question": "What now?",
            "status": "valid", "created_at": null, "archived": false, "heard": false,
            "check": null,
            "priorities": [
                {"idx": 0, "title": "Ship the beta", "why": "Users wait.", "owner_role": "pm",
                 "objective": null, "mark": mark, "is_new": false, "sources": []},
                {"idx": 1, "title": "Fix the login", "why": "It fails.", "owner_role": "dev",
                 "objective": null, "mark": "rejected", "is_new": false, "sources": []}
            ],
            "disagreements": [], "decisions": [], "roles": [], "usage": [],
            "usage_totals": {"rows": 0, "input_tokens": 0, "output_tokens": 0,
                             "cache_read": 0, "cache_write": 0, "cost_usd": 0.0}
        }))
        .unwrap()
    }

    fn story_json() -> Value {
        json!({
            "id": "S001", "type": "story", "slug": "login", "title": "Add a login",
            "i_want": "to sign in", "so_that": "my data is mine",
            "criteria": [{"given": "a user", "when": "they sign in", "then": "they see the home",
                          "test": "tests/login.rs", "checks": "the home shows"}],
            "owner_role": "dev", "status": "not_implemented", "approval": "approved",
            "proposed_by": "pm", "created_at": "2026-11-01", "updated_at": "2026-11-02"
        })
    }

    pub(in crate::screens) fn story() -> StoryGetResult {
        serde_json::from_value(json!({
            "story": story_json(), "markdown": "# S001", "actions": ["run"], "log": "",
            "archived": false
        }))
        .unwrap()
    }

    pub(in crate::screens) fn review(diff: &str) -> StoryReviewResult {
        serde_json::from_value(json!({
            "story": story_json(), "stat": " src/a.rs | 2 +-\n 1 file changed",
            "diff": diff, "repo": "/src/demo"
        }))
        .unwrap()
    }

    pub(in crate::screens) const PATCH: &str = "diff --git a/src/a.rs b/src/a.rs\n\
        index 1111111..2222222 100644\n--- a/src/a.rs\n+++ b/src/a.rs\n\
        @@ -1,2 +1,2 @@\n fn main() {\n-    old();\n+    new();\n";

    fn views_history(cx: &mut VisualTestContext, conv: &Entity<DeskConversation>) {
        conv.update(cx, |c, cx| {
            c.history_done(
                Ok(history(
                    "idle",
                    None,
                    json!([
                        entry(1, json!({"kind": "owner", "text": "Show me"})),
                        entry(2, json!({"kind": "view", "view": {"kind": "priorities", "minutes_id": "m1"}})),
                        entry(3, json!({"kind": "view", "view": {"kind": "story", "id": "S001"}})),
                        entry(4, json!({"kind": "view", "view": {"kind": "diff", "story_id": "S001"}})),
                    ]),
                )),
                cx,
            )
        });
    }

    fn changed(kind: &str) -> Notification {
        Notification::of::<DataChanged>(
            &serde_json::from_value(json!({"product": "demo", "kind": kind})).unwrap(),
        )
    }

    #[gpui::test]
    fn the_priorities_story_and_diff_views_read_their_objects_and_draw_them(
        cx: &mut TestAppContext,
    ) {
        let (conv, mut other, cx) = setup(cx);
        methods_sent(&mut other);
        views_history(cx, &conv);
        let sent = methods_sent(&mut other);
        assert_eq!(
            sent,
            [
                ("minutes.get".to_string(), json!({"id": "m1"})),
                (
                    "story.get".to_string(),
                    json!({"product": "demo", "id": "S001"})
                ),
                (
                    "story.review".to_string(),
                    json!({"product": "demo", "id": "S001"})
                ),
            ]
        );
        // Before the objects come, each view draws a reading line.
        draw(cx);
        conv.update(cx, |c, cx| {
            c.minutes_done("m1".into(), Ok(minutes(None)), cx);
            c.story_done("S001".into(), Ok(story()), cx);
            c.review_done("S001".into(), Ok(review(PATCH)), cx);
        });
        draw(cx);
        conv.read_with(cx, |c, _| {
            assert!(matches!(c.objects.minutes["m1"], Load::Ready(_)));
            assert!(matches!(c.objects.stories["S001"], Load::Ready(_)));
            let Load::Ready(d) = &c.objects.diffs["S001"] else {
                panic!("the diff is not ready")
            };
            assert!(d.state.is_some(), "the patch parses");
            assert_eq!(d.more, 0);
        });

        // A failed read is a line. It does not stop the other views.
        conv.update(cx, |c, cx| {
            c.story_done("S001".into(), Err(CallError::Link("gone".into())), cx)
        });
        draw(cx);
    }

    #[gpui::test]
    fn a_long_diff_shows_the_line_limit_of_the_story_screen(cx: &mut TestAppContext) {
        let (conv, mut other, cx) = setup(cx);
        views_history(cx, &conv);
        methods_sent(&mut other);
        let mut long = String::from(PATCH);
        for n in 0..500 {
            long.push_str(&format!("+line {n}\n"));
        }
        conv.update(cx, |c, cx| {
            c.review_done("S001".into(), Ok(review(&long)), cx)
        });
        draw(cx);
        conv.read_with(cx, |c, _| {
            let Load::Ready(d) = &c.objects.diffs["S001"] else {
                panic!("the diff is not ready")
            };
            assert_eq!(d.lines.len(), crate::screens::story::DIFF_LINES);
            assert_eq!(
                d.more,
                long.lines().count() - crate::screens::story::DIFF_LINES
            );
        });
        // An empty branch is a line, not an error.
        conv.update(cx, |c, cx| c.review_done("S001".into(), Ok(review("")), cx));
        draw(cx);
    }

    #[gpui::test]
    fn accept_on_a_priority_asks_a_confirm_then_sets_the_mark_and_reads_it_again(
        cx: &mut TestAppContext,
    ) {
        let (conv, mut other, cx) = setup(cx);
        views_history(cx, &conv);
        conv.update(cx, |c, cx| {
            c.minutes_done("m1".into(), Ok(minutes(None)), cx)
        });
        methods_sent(&mut other);

        // Accept only asks. It writes nothing.
        conv.update(cx, |c, cx| c.ask_mark("m1", 0, Mark::Accepted, cx));
        draw(cx);
        assert!(methods_sent(&mut other).is_empty());
        conv.update(cx, |c, cx| c.cancel(cx));
        conv.update(cx, |c, cx| c.confirmed(cx));
        assert!(methods_sent(&mut other).is_empty());

        conv.update(cx, |c, cx| c.ask_mark("m1", 0, Mark::Accepted, cx));
        conv.update(cx, |c, cx| c.confirmed(cx));
        let sent = methods_sent(&mut other);
        assert_eq!(
            sent,
            [(
                "mark.set".to_string(),
                json!({"entries": [{"minutes_id": "m1", "idx": 0, "mark": "accepted",
                                    "is_new": false}], "unmarked_only": true})
            )]
        );

        // The core accepted. The view reads the minutes again. No owner message is sent.
        conv.update(cx, |c, cx| {
            c.mark_done(
                "m1".into(),
                0,
                Mark::Accepted,
                Ok(MarkSetResult { marked: 1 }),
                cx,
            )
        });
        let sent = methods_sent(&mut other);
        assert_eq!(
            sent.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(),
            ["minutes.get"]
        );
        conv.update(cx, |c, cx| {
            c.minutes_done("m1".into(), Ok(minutes(Some("accepted"))), cx)
        });
        draw(cx);
        conv.read_with(cx, |c, _| {
            let Load::Ready(m) = &c.objects.minutes["m1"] else {
                panic!("the minutes are not ready")
            };
            assert_eq!(m.priorities[0].mark.as_deref(), Some("accepted"));
            assert!(matches!(c.flash, Some(Flash::Done(_))));
            assert!(!c.busy);
        });
        assert!(methods_sent(&mut other).is_empty());

        // A refusal of the core shows as a line.
        conv.update(cx, |c, cx| {
            c.mark_done(
                "m1".into(),
                0,
                Mark::Rejected,
                Err(CallError::Link("conflict".into())),
                cx,
            )
        });
        conv.read_with(cx, |c, _| {
            assert!(matches!(c.flash, Some(Flash::Failed(_))))
        });
    }

    #[gpui::test]
    fn each_object_view_reads_again_after_data_changed_of_its_kind(cx: &mut TestAppContext) {
        let (conv, mut other, cx) = setup(cx);
        views_history(cx, &conv);
        methods_sent(&mut other);
        conv.update(cx, |c, cx| c.take(&changed("tickets"), cx));
        assert!(methods_sent(&mut other).is_empty());
        conv.update(cx, |c, cx| c.take(&changed("minutes"), cx));
        let names: Vec<String> = methods_sent(&mut other).into_iter().map(|m| m.0).collect();
        assert_eq!(names, ["minutes.get"]);
        conv.update(cx, |c, cx| c.take(&changed("stories"), cx));
        let names: Vec<String> = methods_sent(&mut other).into_iter().map(|m| m.0).collect();
        assert_eq!(names, ["story.get", "story.review"]);
    }

    #[gpui::test]
    fn open_the_story_asks_the_shell_for_the_story_screen(cx: &mut TestAppContext) {
        let (nav, mut requests) = Nav::channel();
        cx.update(|cx| cx.set_global(nav));
        let (conv, _other, cx) = setup(cx);
        conv.update(cx, |c, cx| c.open_story("S001", cx));
        assert_eq!(
            requests.try_recv().unwrap(),
            ScreenRequest::story("demo", "S001")
        );
    }
}
