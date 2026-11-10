use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui::{
    Context, InteractiveElement, IntoElement, KeyDownEvent, KeyUpEvent, Keystroke, ParentElement,
    Render, Styled, TestAppContext, VisualTestContext, Window, div, px,
};

use super::{
    Button, Card, Force, ListRow, Look, Pointer, Switch, look, status_tag, switch, use_interaction,
};
use crate::tokens::Status;

#[test]
fn a_pointer_move_during_a_press_keeps_the_press() {
    let mut p = Pointer::default();
    p.hover(true);
    p.down();
    // GPUI reports the end of the hover when the pointer moves with the button down.
    p.hover(false);
    assert_eq!(p.look(), Look::Pressed);
    p.up();
    assert_eq!(p.look(), Look::Rest);
}

#[test]
fn a_release_outside_ends_the_press() {
    let mut p = Pointer::default();
    p.down();
    p.up();
    p.hover(true);
    assert_eq!(p.look(), Look::Hover);
}

struct Clicks {
    button: Rc<Cell<usize>>,
    card: Rc<Cell<usize>>,
}

impl Render for Clicks {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let (b, c) = (self.button.clone(), self.card.clone());
        div()
            .id("root")
            .tab_group()
            .size(px(400.))
            .child(
                Button::new("button")
                    .label("Approve")
                    .on_click(move |_, _, _| b.set(b.get() + 1)),
            )
            .child(Card::new("static-card").child("No input"))
            .child(
                Card::new("card")
                    .on_click(move |_, _, _| c.set(c.get() + 1))
                    .child("Open"),
            )
    }
}

fn press(cx: &mut VisualTestContext, key: &str) {
    let keystroke =
        cx.update(|_, cx| Keystroke::parse_with_os(key, cx.platform_info().os).unwrap());
    cx.simulate_event(KeyDownEvent {
        keystroke: keystroke.clone(),
        is_held: false,
        prefer_character_input: false,
    });
    cx.simulate_event(KeyUpEvent { keystroke });
}

fn clicks(cx: &mut TestAppContext) -> (&mut VisualTestContext, Rc<Cell<usize>>, Rc<Cell<usize>>) {
    cx.update(crate::init);
    let button = Rc::new(Cell::new(0));
    let card = Rc::new(Cell::new(0));
    let (_, cx) = cx.add_window_view({
        let (button, card) = (button.clone(), card.clone());
        move |_, _| Clicks { button, card }
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    (cx, button, card)
}

#[gpui::test]
fn enter_and_space_click_a_focused_button_once_each(cx: &mut TestAppContext) {
    let (cx, button, _) = clicks(cx);
    cx.update(|window, cx| window.focus_next(cx));
    press(cx, "enter");
    assert_eq!(button.get(), 1);
    press(cx, "space");
    assert_eq!(button.get(), 2);
}

#[gpui::test]
fn tab_skips_a_static_card_and_enter_clicks_a_clickable_card(cx: &mut TestAppContext) {
    let (cx, button, card) = clicks(cx);
    cx.update(|window, cx| window.focus_next(cx));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|window, cx| window.focus_next(cx));
    press(cx, "enter");
    assert_eq!((button.get(), card.get()), (0, 1));
}

struct LongTag;

impl Render for LongTag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let long = "pending review by the owner of the product ".repeat(8);
        div().w(px(360.)).child(
            ListRow::new("row", "Decisions to confirm").trailing(status_tag(long, Status::Warn)),
        )
    }
}

#[gpui::test]
fn a_long_tag_leaves_room_for_the_title(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (_, cx) = cx.add_window_view(|_, _| LongTag);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let title = cx
        .debug_bounds("list-row-title")
        .expect("the title is drawn");
    assert!(
        title.size.width >= px(120.),
        "the title is {:?} wide",
        title.size.width
    );
}

#[test]
fn each_switch_look_has_its_own_track() {
    let looks = [Look::Rest, Look::Hover, Look::Pressed];
    let mut tracks: Vec<_> = [true, false]
        .into_iter()
        .flat_map(|on| looks.map(|l| switch::track(on, l)))
        .collect();
    tracks.sort_by_key(|c| c.0);
    tracks.dedup();
    assert_eq!(tracks.len(), 6);
}

/// Reads the look of one interaction for each forced state, during a render.
struct Forced(Rc<RefCell<Vec<(Look, bool)>>>);

impl Render for Forced {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = use_interaction(&"forced".into(), true, window, cx);
        *self.0.borrow_mut() = [
            (Force::None, false),
            (Force::Hover, false),
            (Force::Pressed, false),
            (Force::Focused, false),
            (Force::Focused, true),
        ]
        .map(|(force, disabled)| look(&state, force, disabled, window, cx))
        .to_vec();
        div()
    }
}

#[gpui::test]
fn a_forced_state_replaces_the_pointer_and_the_focus(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let (_, cx) = cx.add_window_view({
        let seen = seen.clone();
        move |_, _| Forced(seen)
    });
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(
        *seen.borrow(),
        [
            (Look::Rest, false),
            (Look::Hover, false),
            (Look::Pressed, false),
            (Look::Rest, true),
            (Look::Rest, false),
        ]
    );
}

#[derive(Default)]
struct Toggle {
    on: bool,
    disabled_clicks: Rc<Cell<usize>>,
}

impl Render for Toggle {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let d = self.disabled_clicks.clone();
        div()
            .id("root")
            .tab_group()
            .size(px(400.))
            .child(Switch::new("voice").checked(self.on).on_click(cx.listener(
                |this, on: &bool, _, cx| {
                    this.on = *on;
                    cx.notify();
                },
            )))
            .child(
                Switch::new("off")
                    .disabled(true)
                    .on_click(move |_, _, _| d.set(d.get() + 1)),
            )
    }
}

#[gpui::test]
fn enter_and_space_toggle_a_focused_switch_and_tab_skips_a_disabled_one(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (view, cx) = cx.add_window_view(|_, _| Toggle::default());
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|window, cx| window.focus_next(cx));
    press(cx, "enter");
    assert!(view.read_with(cx, |t, _| t.on));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    press(cx, "space");
    assert!(!view.read_with(cx, |t, _| t.on));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.update(|window, cx| window.focus_next(cx));
    press(cx, "enter");
    assert_eq!(view.read_with(cx, |t, _| t.disabled_clicks.get()), 0);
}

/// One of each fixed-height component, in a column the width of the sidebar.
struct Fixtures;

impl Render for Fixtures {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        use crate::tokens::control;
        div()
            .w(control::SIDEBAR_WIDTH)
            .flex()
            .flex_col()
            .child(
                ListRow::new("meta", "Decisions to confirm")
                    .meta("~/dev/personal/product")
                    .trailing(status_tag("review", Status::Warn)),
            )
            .child(ListRow::new("plain", "A product").selected(true))
            .child(
                div()
                    .flex()
                    .child(Button::new("sm").small().label("Approve"))
                    .child(Button::new("md").label("Approve"))
                    .child(
                        Button::new("icon")
                            .ghost()
                            .icon(gpui_kit::component::IconName::Plus),
                    ),
            )
    }
}

fn bounds(cx: &mut VisualTestContext, selector: &str) -> gpui::Bounds<gpui::Pixels> {
    // The tests are short-lived, so a leaked name costs nothing.
    let name: &'static str = Box::leak(selector.to_string().into_boxed_str());
    cx.debug_bounds(name)
        .unwrap_or_else(|| panic!("{selector} is drawn"))
}

#[gpui::test]
fn each_component_has_the_height_of_the_tokens_in_each_of_the_27_combinations(
    cx: &mut TestAppContext,
) {
    use crate::appearance::{Appearance, INK_EM};
    use crate::tokens::{control, size};
    cx.update(crate::init);
    let (_, cx) = cx.add_window_view(|_, _| Fixtures);
    let mut rows = std::collections::BTreeSet::new();
    for a in Appearance::all() {
        cx.update(|window, cx| {
            a.apply(cx);
            window.draw(cx).clear(cx);
        });
        let ctx = format!("{a:?}");
        let (meta, plain) = (bounds(cx, "list-row:meta"), bounds(cx, "list-row:plain"));
        assert_eq!(meta.size.height, control::row(), "{ctx}");
        rows.insert(f32::from(meta.size.height).to_bits());
        assert_eq!(plain.size.height, control::row(), "{ctx}");
        // No two rows overlap.
        assert!(meta.bottom() <= plain.top(), "{ctx}");
        // The two lines of text stay inside the row, and the text box is not cut at the sides.
        let text = bounds(cx, "list-row:meta:text");
        let ink = (f32::from(size::sm()) + f32::from(size::xs())) * INK_EM;
        let (top, bottom) = (
            f32::from(text.top() + text.size.height / 2.) - ink / 2.,
            f32::from(text.top() + text.size.height / 2.) + ink / 2.,
        );
        assert!(
            top >= f32::from(meta.top()),
            "{ctx}: text starts above the row"
        );
        assert!(
            bottom <= f32::from(meta.bottom()),
            "{ctx}: text ends below the row"
        );
        assert!(
            text.right() <= meta.right() && text.left() >= meta.left(),
            "{ctx}"
        );
        let title = bounds(cx, "list-row-title");
        assert!(
            title.size.width >= px(64.),
            "{ctx}: the title is {:?} wide",
            title.size.width
        );
        for (id, h) in [
            ("sm", control::height_sm()),
            ("md", control::height_md()),
            ("icon", control::height_md()),
        ] {
            let b = bounds(cx, &format!("button:{id}"));
            assert_eq!(b.size.height, h, "{ctx}: button {id}");
        }
        assert_eq!(
            bounds(cx, "button:icon").size.width,
            control::height_md(),
            "{ctx}: an icon button is square"
        );
        // The buttons sit side by side inside the column.
        let (sm, md, icon) = (
            bounds(cx, "button:sm"),
            bounds(cx, "button:md"),
            bounds(cx, "button:icon"),
        );
        assert!(
            sm.right() <= md.left() && md.right() <= icon.left(),
            "{ctx}"
        );
        assert!(
            icon.right() <= meta.right(),
            "{ctx}: the buttons fit the column"
        );
    }
    // The drawn rows moved with the density: the test would pass with a stale look otherwise.
    assert_eq!(rows.len(), 3);
}

#[gpui::test]
fn a_tab_chip_shows_its_count_only_when_it_is_collapsed_and_a_click_reaches_its_handler(
    cx: &mut TestAppContext,
) {
    use super::TabChip;
    use crate::tokens::label::Hue;

    struct Chips {
        collapsed: Rc<Cell<bool>>,
    }
    impl Render for Chips {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let (collapsed, flag) = (self.collapsed.get(), self.collapsed.clone());
            let this = cx.entity_id();
            div().size(px(400.)).child(
                TabChip::new("chip", "Build")
                    .hue(Some(Hue::Teal))
                    .collapsed(collapsed)
                    .count(3)
                    .on_click(move |_, _, cx| {
                        flag.set(!flag.get());
                        cx.notify(this);
                    }),
            )
        }
    }
    cx.update(crate::init);
    let collapsed = Rc::new(Cell::new(false));
    let c = collapsed.clone();
    let (_, cx) = cx.add_window_view(move |_, _| Chips { collapsed: c });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(cx.debug_bounds("tab-chip:chip").is_some());
    assert!(cx.debug_bounds("tab-chip:chip:count").is_none());
    let b = cx.debug_bounds("tab-chip:chip").unwrap();
    cx.simulate_click(b.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(collapsed.get());
    assert!(cx.debug_bounds("tab-chip:chip:count").is_some());
}

#[test]
fn only_a_speaking_card_with_a_level_grows_a_ring() {
    use super::ring_spread;
    use crate::tokens::WorkState;
    assert_eq!(ring_spread(WorkState::Speaking, None), 0.);
    assert_eq!(ring_spread(WorkState::Speaking, Some(0.5)), 2.);
    assert_eq!(ring_spread(WorkState::Speaking, Some(9.)), 4.);
    assert_eq!(ring_spread(WorkState::Working, Some(1.)), 0.);
}

struct Agent;

impl Render for Agent {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child(
            super::AgentCard::new("Dev", crate::tokens::WorkState::Working).activity("Writing"),
        )
    }
}

#[gpui::test]
fn the_activity_line_of_an_agent_card_has_its_full_height(cx: &mut TestAppContext) {
    cx.update(crate::init);
    let (_, cx) = cx.add_window_view(|_, _| Agent);
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let line = cx
        .debug_bounds("agent-card-activity")
        .expect("the activity is drawn");
    assert_eq!(line.size.height, crate::tokens::size::xs_line());
    assert!(
        line.size.height >= crate::tokens::size::xs() * 1.4,
        "the line is {:?} high, so it clips the text",
        line.size.height
    );
}
