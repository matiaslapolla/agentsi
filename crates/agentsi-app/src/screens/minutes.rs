//! The minutes of a product as a virtual list, read only. The list takes focus as one
//! element. The arrow keys move a cursor, `Enter` opens the minutes of the cursor, and
//! a row has no focus handle of its own.
//!
//! A row whose minutes have a call shows a play mark, and a click on it opens the Call screen.
//! The list has no field for the call, so it asks `call.get` for the rows in view, once for
//! each row and each read of the list.

use std::collections::{HashMap, HashSet};

use agentsi_proto::call_api::{CallGet, CallGetParams};
use agentsi_proto::desk_api::{MinutesList, MinutesListParams, MinutesRow};
use agentsi_ui::bench::Bench;
use agentsi_ui::components::{Button, ListRow};
use agentsi_ui::style::StyleExt as _;
use agentsi_ui::tokens::{control, line, size, space, surface, text, weight};
use gpui_kit::component::IconName;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::parts::{
    CallInfo, Live, Load, first_screen_ready, minutes_meta, minutes_status, notice,
};
use crate::bench::BenchHandle;
use crate::model::CoreState;
use crate::nav::{Nav, ScreenRequest};

/// The rows that a Page Up or a Page Down moves the cursor.
const PAGE: usize = 10;

pub struct MinutesScreen {
    product: String,
    core: Entity<CoreState>,
    data: Load<Vec<MinutesRow>>,
    scroll: UniformListScrollHandle,
    cursor: usize,
    focus: FocusHandle,
    bench: Option<Bench>,
    /// What `call.get` said for each minutes id. A row without an entry has no play mark.
    calls: HashMap<String, CallInfo>,
    /// The ids that `call.get` was asked for since the last read of the list.
    asked: HashSet<String>,
}

/// Where a key moves the cursor in a list of `len` rows, or `None` when the key is not a move.
pub fn cursor_after(key: &str, cursor: usize, len: usize) -> Option<usize> {
    let last = len.checked_sub(1)?;
    Some(match key {
        "down" => (cursor + 1).min(last),
        "up" => cursor.saturating_sub(1),
        "pagedown" => (cursor + PAGE).min(last),
        "pageup" => cursor.saturating_sub(PAGE),
        "home" => 0,
        "end" => last,
        _ => return None,
    })
}

impl MinutesScreen {
    pub fn core(&self) -> &Entity<CoreState> {
        &self.core
    }

    pub fn new(product: String, core: Entity<CoreState>, cx: &mut Context<Self>) -> Self {
        let mut screen = MinutesScreen {
            product,
            core,
            data: Load::Loading,
            scroll: UniformListScrollHandle::new(),
            cursor: 0,
            focus: cx.focus_handle().tab_stop(true),
            bench: cx.try_global::<BenchHandle>().map(|b| b.0.clone()),
            calls: HashMap::new(),
            asked: HashSet::new(),
        };
        let me = cx.weak_entity();
        Live::add_list(cx, me);
        screen.reload(cx);
        screen
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        // A call may have been made or removed: the rows in view ask again.
        self.asked.clear();
        let call = self
            .core
            .read(cx)
            .link
            .call::<MinutesList>(&MinutesListParams {
                product: self.product.clone(),
            });
        cx.spawn(async move |this, cx| {
            let result = call.await.map(|r| r.minutes).map_err(|e| e.to_string());
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

    fn rows(&self) -> &[MinutesRow] {
        match &self.data {
            Load::Ready(rows) => rows,
            _ => &[],
        }
    }

    /// Asks `call.get` for the minutes in `range` that it was not asked for yet.
    fn ask_calls(&mut self, range: std::ops::Range<usize>, cx: &mut Context<Self>) {
        let in_view: Vec<String> = self
            .rows()
            .get(range)
            .unwrap_or_default()
            .iter()
            .map(|m| m.id.clone())
            .collect();
        let ids: Vec<String> = in_view
            .into_iter()
            .filter(|id| self.asked.insert(id.clone()))
            .collect();
        if ids.is_empty() {
            return;
        }
        let link = self.core.read(cx).link.clone();
        // Each call goes out now, and the replies come in any order.
        let calls: Vec<_> = ids
            .into_iter()
            .map(|id| {
                let reply = link.call::<CallGet>(&CallGetParams {
                    product: self.product.clone(),
                    minutes_id: id.clone(),
                });
                (id, reply)
            })
            .collect();
        cx.spawn(async move |this, cx| {
            for (id, reply) in calls {
                let result = reply
                    .await
                    .map(|c| CallInfo::from(&c))
                    .map_err(|e| e.to_string());
                let _ = this.update(cx, |this, cx| this.call_loaded(id, result, cx));
            }
        })
        .detach();
    }

    /// Keeps what `call.get` said for one minutes. A minutes with no call, or with an error, has no mark.
    fn call_loaded(
        &mut self,
        id: String,
        result: Result<CallInfo, String>,
        cx: &mut Context<Self>,
    ) {
        let info = result.unwrap_or_default();
        if self.calls.get(&id) != Some(&info) {
            self.calls.insert(id, info);
            cx.notify();
        }
    }

    fn has_call(&self, id: &str) -> bool {
        self.calls.get(id).is_some_and(|c| c.audio)
    }

    fn open(&self, row: usize, cx: &App) {
        if let Some(m) = self.rows().get(row) {
            Nav::open(cx, &self.core, ScreenRequest::minute(&self.product, &m.id));
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

impl Render for MinutesScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div().size_full().flex().flex_col().bg(surface::panel());
        let count = self.rows().len();
        let body = match &self.data {
            Load::Loading => notice("Reading the minutes…", None).into_any_element(),
            Load::Failed(e) => notice(
                format!("The minutes did not load: {e}"),
                Some(super::parts::reload_hint(cx)),
            )
            .into_any_element(),
            Load::Ready(rows) if rows.is_empty() => notice(
                "No minutes for this product yet.",
                Some("Convene a meeting with agentsi meet.".into()),
            )
            .into_any_element(),
            Load::Ready(_) => {
                if let Some(b) = &self.bench {
                    let max = self.scroll.0.borrow().base_handle.max_offset().y;
                    let range = f32::from(max).max(1.);
                    if let Some(off) = b.scroll_offset(Pixels::from(range)) {
                        self.scroll
                            .0
                            .borrow()
                            .base_handle
                            .set_offset(point(Pixels::ZERO, -off));
                        window.request_animation_frame();
                    }
                }
                let focused = self.focus.is_focused(window);
                uniform_list(
                    "minutes-list",
                    count,
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        this.ask_calls(range.clone(), cx);
                        range
                            .filter_map(|i| {
                                let m = this.rows().get(i)?;
                                let id = m.id.clone();
                                let product = this.product.clone();
                                let play = this.has_call(&m.id).then(|| {
                                    let (core, product, id) =
                                        (this.core.clone(), product.clone(), id.clone());
                                    Button::new(("minutes-play", i))
                                        .ghost()
                                        .small()
                                        .icon(IconName::Play)
                                        .on_click(move |_, _, cx| {
                                            // The row under the mark must not open the minutes too.
                                            cx.stop_propagation();
                                            Nav::open(
                                                cx,
                                                &core,
                                                ScreenRequest::call(&product, &id),
                                            );
                                        })
                                });
                                let trailing = div()
                                    .flex()
                                    .items_center()
                                    .gap(space(2))
                                    .when_some(play, |el, b| el.child(b))
                                    .child(minutes_status(m.status.as_deref()));
                                let row = ListRow::new(("minutes-row", i), m.question.clone())
                                    .meta(minutes_meta(m))
                                    .selected(this.cursor == i)
                                    .focused(this.cursor == i && focused)
                                    .trailing(trailing)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.cursor = i;
                                        window.focus(&this.focus, cx);
                                        Nav::open(
                                            cx,
                                            &this.core,
                                            ScreenRequest::minute(&product, &id),
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
                        .child("Minutes"),
                )
                .child(
                    div()
                        .text_size(size::sm())
                        .text_color(text::tertiary())
                        .child(format!("{count} meetings")),
                ),
        )
        .child(
            div()
                .id("minutes-body")
                .track_focus(&self.focus)
                .key_context("MinutesList")
                .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.key_down(e, cx)))
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(body),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{CallInfo, Load, MinutesRow, MinutesScreen, cursor_after};
    use crate::link::Link;
    use crate::model::CoreState;
    use crate::nav::{Nav, ScreenRequest};
    use gpui::TestAppContext;
    use gpui_kit::AppContext as _;
    use std::sync::Arc;

    #[test]
    fn the_cursor_moves_and_stops_at_the_ends() {
        assert_eq!(cursor_after("down", 0, 3), Some(1));
        assert_eq!(cursor_after("down", 2, 3), Some(2));
        assert_eq!(cursor_after("up", 0, 3), Some(0));
        assert_eq!(cursor_after("up", 2, 3), Some(1));
        assert_eq!(cursor_after("pagedown", 5, 100), Some(15));
        assert_eq!(cursor_after("pagedown", 95, 100), Some(99));
        assert_eq!(cursor_after("pageup", 5, 100), Some(0));
        assert_eq!(cursor_after("home", 40, 100), Some(0));
        assert_eq!(cursor_after("end", 40, 100), Some(99));
    }

    #[test]
    fn a_key_that_is_not_a_move_and_an_empty_list_give_no_cursor() {
        assert_eq!(cursor_after("a", 0, 3), None);
        assert_eq!(cursor_after("enter", 0, 3), None);
        assert_eq!(cursor_after("down", 0, 0), None);
    }

    fn rows(n: usize) -> Vec<MinutesRow> {
        (0..n)
            .map(|i| {
                serde_json::from_str(&format!(
                    r#"{{"id": "m{i}", "question": "Question {i}?", "status": "valid",
                        "created_at": "2026-11-02T10:00:00.000Z", "scope": null,
                        "marks": {{"accepted": 1, "changed": 0, "rejected": 0, "unmarked": 1}},
                        "heard": false}}"#
                ))
                .unwrap()
            })
            .collect()
    }

    #[gpui::test]
    fn the_list_takes_focus_as_one_element_and_the_arrow_keys_move_its_cursor(
        cx: &mut TestAppContext,
    ) {
        cx.update(agentsi_ui::init);
        let (nav, mut requests) = Nav::channel();
        cx.update(|cx| cx.set_global(nav));
        // The link is never used here: a socket pair with no core behind it.
        let (end, _other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| MinutesScreen::new("demo".into(), core, cx));
        screen.update(cx, |s, cx| {
            s.data = Load::Ready(rows(50));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        // One Tab reaches the list. Its 50 rows add no tab stop.
        cx.update(|window, cx| window.focus_next(cx));
        assert!(cx.update(|window, cx| screen.read(cx).focus_handle().is_focused(window)));
        cx.simulate_keystrokes("down down down");
        assert_eq!(screen.read_with(cx, |s, _| s.cursor), 3);
        cx.simulate_keystrokes("pagedown");
        assert_eq!(screen.read_with(cx, |s, _| s.cursor), 13);
        cx.simulate_keystrokes("end up");
        assert_eq!(screen.read_with(cx, |s, _| s.cursor), 48);
        cx.simulate_keystrokes("home enter");
        assert_eq!(
            requests.try_recv().unwrap(),
            ScreenRequest::minute("demo", "m0")
        );
    }

    #[gpui::test]
    fn the_cursor_of_an_empty_list_does_not_move_and_enter_opens_nothing(cx: &mut TestAppContext) {
        cx.update(agentsi_ui::init);
        let (nav, mut requests) = Nav::channel();
        cx.update(|cx| cx.set_global(nav));
        let (end, _other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| MinutesScreen::new("demo".into(), core, cx));
        screen.update(cx, |s, cx| {
            s.data = Load::Ready(Vec::new());
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.update(|window, cx| window.focus_next(cx));
        cx.simulate_keystrokes("down end enter");
        assert_eq!(screen.read_with(cx, |s, _| s.cursor), 0);
        assert!(requests.try_recv().is_err());
    }

    fn sent(other: &mut std::os::unix::net::UnixStream) -> Option<serde_json::Value> {
        other.set_nonblocking(true).unwrap();
        let frame = agentsi_proto::frame::read_frame(other).ok().flatten()?;
        serde_json::from_slice(&frame.payload).ok()
    }

    #[gpui::test]
    fn a_play_mark_on_a_row_with_a_call_opens_the_call_screen_and_not_the_minutes(
        cx: &mut TestAppContext,
    ) {
        cx.update(agentsi_ui::init);
        let (nav, mut requests) = Nav::channel();
        cx.update(|cx| cx.set_global(nav));
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| MinutesScreen::new("demo".into(), core, cx));
        assert_eq!(sent(&mut other).unwrap()["method"], "minutes.list");
        screen.update(cx, |s, cx| {
            s.data = Load::Ready(rows(3));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        // The rows in view ask for their call, once each.
        for id in ["m0", "m1", "m2"] {
            let request = sent(&mut other).expect("a call.get");
            assert_eq!(request["method"], "call.get");
            assert_eq!(
                request["params"],
                serde_json::json!({"product": "demo", "minutes_id": id})
            );
        }
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(sent(&mut other).is_none());

        // Only m1 has a call.
        let audio = CallInfo {
            audio: true,
            render: None,
        };
        screen.update(cx, |s, cx| {
            s.call_loaded("m0".into(), Ok(CallInfo::default()), cx);
            s.call_loaded("m1".into(), Ok(audio), cx);
            s.call_loaded("m2".into(), Err("not found".into()), cx);
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("button:minutes-play-0").is_none());
        assert!(cx.debug_bounds("button:minutes-play-2").is_none());
        let mark = cx
            .debug_bounds("button:minutes-play-1")
            .expect("a play mark");
        cx.simulate_click(mark.center(), gpui::Modifiers::none());
        assert_eq!(
            requests.try_recv().unwrap(),
            ScreenRequest::call("demo", "m1")
        );
        assert!(
            requests.try_recv().is_err(),
            "the row did not open the minutes"
        );

        // A click on the row itself still opens the minutes.
        let row = cx.debug_bounds("list-row:minutes-row-1").expect("a row");
        cx.simulate_click(
            row.origin + gpui::point(gpui::Pixels::from(4.), gpui::Pixels::from(4.)),
            gpui::Modifiers::none(),
        );
        assert_eq!(
            requests.try_recv().unwrap(),
            ScreenRequest::minute("demo", "m1")
        );
    }

    #[gpui::test]
    fn a_read_of_the_list_asks_for_the_calls_in_view_again(cx: &mut TestAppContext) {
        cx.update(agentsi_ui::init);
        let (end, mut other) = std::os::unix::net::UnixStream::pair().unwrap();
        let core = cx.new(|_| CoreState::new(Arc::new(Link::for_tests(end))));
        let (screen, cx) = cx.add_window_view(|_, cx| MinutesScreen::new("demo".into(), core, cx));
        screen.update(cx, |s, cx| {
            s.data = Load::Ready(rows(1));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let methods = |other: &mut std::os::unix::net::UnixStream| {
            std::iter::from_fn(|| sent(other))
                .map(|r| r["method"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(methods(&mut other), ["minutes.list", "call.get"]);
        screen.update(cx, |s, cx| s.reload(cx));
        screen.update(cx, |s, cx| {
            s.data = Load::Ready(rows(1));
            cx.notify();
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(methods(&mut other), ["minutes.list", "call.get"]);
    }
}
