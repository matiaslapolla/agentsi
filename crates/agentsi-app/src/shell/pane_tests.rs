//! The pane bar, the columns, the resize and the move of the panes, over the real shell with a
//! fake core.

use gpui::{
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point, TestAppContext,
    VisualTestContext,
};

use super::Persist;
use super::pane_actions::drop_selector;
use super::tests::{Fixture, at, click, fixture, folder, redraw};
use crate::alerts::{Alerts, Name};
use crate::layout;
use crate::panes::{PaneKind, Panes, Place, Side};
use agentsi_ui::tokens::control;

const P: PaneKind = PaneKind::Products;
const F: PaneKind = PaneKind::Files;
const A: PaneKind = PaneKind::Alerts;

fn panes(f: &Fixture, cx: &mut VisualTestContext) -> Panes {
    f.shell.read_with(cx, |s, _| s.panes.clone())
}

fn bounds(cx: &mut VisualTestContext, name: &str) -> gpui::Bounds<Pixels> {
    at(cx, name).unwrap_or_else(|| panic!("{name} is drawn"))
}

/// The toggle of `kind`, open or hidden, whichever is drawn.
fn toggle_state(cx: &mut VisualTestContext, kind: PaneKind) -> &'static str {
    let open = at(cx, &format!("pane-toggle:{}:open", kind.word())).is_some();
    let hidden = at(cx, &format!("pane-toggle:{}:hidden", kind.word())).is_some();
    match (open, hidden) {
        (true, false) => "open",
        (false, true) => "hidden",
        _ => panic!("one toggle of {kind:?} is drawn"),
    }
}

fn click_toggle(cx: &mut VisualTestContext, kind: PaneKind) {
    let state = toggle_state(cx, kind);
    click(cx, &format!("pane-toggle:{}:{state}", kind.word()));
}

/// Presses at `from`, moves past the drag threshold, then to `to`, and releases there unless
/// `release` is false.
fn drag_to(cx: &mut VisualTestContext, from: Point<Pixels>, to: Point<Pixels>, release: bool) {
    let held = MouseButton::Left;
    cx.simulate_mouse_down(from, held, Modifiers::none());
    cx.simulate_mouse_move(
        Point::new(from.x, from.y + Pixels::from(8.)),
        held,
        Modifiers::none(),
    );
    redraw(cx);
    cx.simulate_mouse_move(to, held, Modifiers::none());
    redraw(cx);
    if release {
        cx.simulate_mouse_up(to, held, Modifiers::none());
        redraw(cx);
    }
}

fn double_click(cx: &mut VisualTestContext, at: Point<Pixels>) {
    for count in [1, 2] {
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count: count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count: count,
        });
    }
    redraw(cx);
}

#[gpui::test]
fn the_toggles_are_bright_for_open_panes_and_muted_for_hidden_ones(cx: &mut TestAppContext) {
    let dir = folder("panes-toggles");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    redraw(cx);
    let bar = bounds(cx, "pane-bar");
    assert_eq!(bar.size.height, control::TOOLBAR_HEIGHT);
    assert_eq!(bar.size.width, control::WINDOW_WIDTH);
    assert_eq!(toggle_state(cx, P), "open");
    assert_eq!(toggle_state(cx, F), "hidden");
    assert_eq!(toggle_state(cx, A), "hidden");
    let order: Vec<_> = [P, F, A]
        .iter()
        .map(|k| {
            let name = format!("pane-toggle:{}:{}", k.word(), toggle_state(cx, *k));
            bounds(cx, &name).left()
        })
        .collect();
    assert!(
        order.windows(2).all(|w| w[0] < w[1]),
        "Products, Files, Alerts"
    );

    click_toggle(cx, F);
    assert_eq!(toggle_state(cx, F), "open");
    assert!(panes(&f, cx).is_open(F));
    assert!(at(cx, "pane:files").is_some());
    click_toggle(cx, P);
    assert_eq!(toggle_state(cx, P), "hidden");
    assert!(at(cx, "pane:products").is_none());
    assert!(at(cx, "tree-row:product-a").is_none());

    // The palette action of each pane does the same.
    for (kind, action) in [
        (
            P,
            Box::new(crate::actions::ToggleProducts) as Box<dyn gpui::Action>,
        ),
        (F, Box::new(crate::actions::ToggleFiles)),
        (A, Box::new(crate::actions::ToggleAlerts)),
    ] {
        let was = panes(&f, cx).is_open(kind);
        cx.update(|window, cx| window.dispatch_action(action.boxed_clone(), cx));
        redraw(cx);
        assert_eq!(panes(&f, cx).is_open(kind), !was, "{kind:?}");
        let want = if was { "hidden" } else { "open" };
        assert_eq!(toggle_state(cx, kind), want, "{kind:?}");
        assert_eq!(at(cx, &format!("pane:{}", kind.word())).is_some(), !was);
    }
}

#[gpui::test]
fn products_and_files_open_one_above_the_other_and_files_shows_the_active_product(
    cx: &mut TestAppContext,
) {
    let dir = folder("panes-both");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    f.core.update(cx, |s, cx| {
        let mut a = s.product("a").cloned().unwrap();
        a.path = Some("/w/a".into());
        let b = s.product("b").cloned().unwrap();
        s.set_products(vec![a, b], cx);
    });
    click_toggle(cx, F);
    let (products, files) = (bounds(cx, "pane:products"), bounds(cx, "pane:files"));
    assert_eq!(products.left(), files.left());
    assert!(products.bottom() <= files.top(), "Products is above Files");
    assert!(at(cx, "tree-row:product-a").is_some());
    let tree = f
        .shell
        .read_with(cx, |s, cx| s.sidebar.read(cx).files.clone());
    let names = tree.read_with(cx, |t, cx| t.shown_names(cx));
    assert_eq!(
        names,
        ["Reading…"],
        "the open tree reads the active product"
    );
    // Each pane has the same share at first.
    assert_eq!(products.size.height, files.size.height);
}

#[gpui::test]
fn with_all_panes_hidden_the_dock_takes_the_full_width_and_toggle_all_brings_them_back(
    cx: &mut TestAppContext,
) {
    let dir = folder("panes-all");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    click_toggle(cx, A);
    let dock = bounds(cx, "dock-area");
    assert_eq!(dock.left(), control::SIDEBAR_WIDTH);
    assert_eq!(dock.right(), control::WINDOW_WIDTH - control::SIDEBAR_WIDTH);
    cx.dispatch_action(crate::actions::ToggleAllPanes);
    redraw(cx);
    assert!(panes(&f, cx).open().is_empty());
    assert_eq!(toggle_state(cx, P), "hidden");
    assert_eq!(toggle_state(cx, A), "hidden");
    assert!(at(cx, "pane-column:left").is_none());
    let dock = bounds(cx, "dock-area");
    assert_eq!(dock.left(), Pixels::ZERO);
    assert_eq!(dock.size.width, control::WINDOW_WIDTH);
    assert_eq!(dock.top(), control::TOOLBAR_HEIGHT);
    cx.dispatch_action(crate::actions::ToggleAllPanes);
    redraw(cx);
    assert_eq!(panes(&f, cx).open(), [P, A], "the same panes come back");
}

#[gpui::test]
fn a_column_edge_resizes_within_its_limits_saves_at_the_end_and_a_double_click_resets(
    cx: &mut TestAppContext,
) {
    let dir = folder("panes-width");
    let file = dir.join("layout.json");
    let _ = std::fs::remove_file(&file);
    let (f, cx) = fixture(cx, Persist::File(file.clone()), &dir.join("look.toml"));
    f.shell.update(cx, |s, _| s.restored = true);
    redraw(cx);
    let width = |cx: &mut VisualTestContext| bounds(cx, "pane-column:left").size.width;
    assert_eq!(width(cx), control::SIDEBAR_WIDTH);
    let y = Pixels::from(400.);
    let edge = |cx: &mut VisualTestContext| Point::new(bounds(cx, "pane-edge:left").center().x, y);
    let from = edge(cx);
    drag_to(cx, from, Point::new(Pixels::from(400.), y), false);
    assert_eq!(width(cx), Pixels::from(400.));
    assert!(
        f.shell.read_with(cx, |s, _| s.save.is_none()),
        "no save while the edge moves"
    );
    cx.simulate_mouse_up(
        Point::new(Pixels::from(400.), y),
        MouseButton::Left,
        Modifiers::none(),
    );
    redraw(cx);
    assert!(
        f.shell.read_with(cx, |s, _| s.save.is_some()),
        "the end saves"
    );
    for (to, want) in [(40., 180.), (1000., 640.)] {
        let from = edge(cx);
        drag_to(cx, from, Point::new(Pixels::from(to), y), true);
        assert_eq!(width(cx), Pixels::from(want), "a drag to {to}");
    }
    let at = edge(cx);
    double_click(cx, at);
    assert_eq!(width(cx), control::SIDEBAR_WIDTH);
}

#[gpui::test]
fn the_edge_between_two_panes_resizes_them_and_keeps_each_at_its_minimum(cx: &mut TestAppContext) {
    let dir = folder("panes-split");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    click_toggle(cx, F);
    let column = bounds(cx, "pane-column:left");
    let split = bounds(cx, "pane-split:products").center();
    let x = split.x;
    drag_to(
        cx,
        split,
        Point::new(x, column.top() + Pixels::from(200.)),
        true,
    );
    let (p, fl) = (bounds(cx, "pane:products"), bounds(cx, "pane:files"));
    assert!((f32::from(p.size.height) - 200.).abs() < 1., "{p:?}");
    assert!((f32::from(p.size.height + fl.size.height - column.size.height)).abs() < 1.);
    for to in [
        column.top() + Pixels::from(5.),
        column.bottom() - Pixels::from(5.),
    ] {
        let from = bounds(cx, "pane-split:products").center();
        drag_to(cx, from, Point::new(x, to), true);
        let (p, fl) = (bounds(cx, "pane:products"), bounds(cx, "pane:files"));
        let low = p.size.height.min(fl.size.height);
        assert!(
            (f32::from(low - control::PANE_MIN_HEIGHT)).abs() < 1.,
            "{p:?} {fl:?}"
        );
    }
    let shares = panes(&f, cx);
    assert!(shares.share(P) > shares.share(F));
}

#[gpui::test]
fn a_header_drag_moves_a_pane_to_the_other_column_and_inside_its_column_and_escape_cancels(
    cx: &mut TestAppContext,
) {
    let dir = folder("panes-move");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    click_toggle(cx, F);
    assert_eq!(panes(&f, cx).column(Side::Left), [P, F]);
    // Files onto the top of Products.
    let from = bounds(cx, "pane-header:files").center();
    let top = bounds(cx, "pane:products");
    let to = Point::new(top.center().x, top.top() + Pixels::from(10.));
    drag_to(cx, from, to, false);
    let above_products = Place {
        side: Side::Left,
        before: Some(P),
    };
    assert!(at(cx, &drop_selector(above_products)).is_some());
    let right_end = Place {
        side: Side::Right,
        before: None,
    };
    assert!(
        at(cx, &drop_selector(right_end)).is_some(),
        "the empty right column has its line at the window edge"
    );
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    redraw(cx);
    assert_eq!(panes(&f, cx).column(Side::Left), [F, P]);
    assert!(at(cx, &drop_selector(right_end)).is_none(), "the lines go");
    let (files, products) = (bounds(cx, "pane:files"), bounds(cx, "pane:products"));
    assert!(files.bottom() <= products.top());

    // Products to the right edge of the window.
    let from = bounds(cx, "pane-header:products").center();
    let edge = Point::new(control::WINDOW_WIDTH - Pixels::from(4.), Pixels::from(300.));
    drag_to(cx, from, edge, true);
    assert_eq!(panes(&f, cx).column(Side::Left), [F]);
    assert_eq!(panes(&f, cx).column(Side::Right), [P]);
    assert_eq!(
        bounds(cx, "pane-column:right").right(),
        control::WINDOW_WIDTH
    );

    // Escape during a drag changes nothing.
    let before = panes(&f, cx);
    let from = bounds(cx, "pane-header:files").center();
    drag_to(cx, from, edge, false);
    assert!(at(cx, &drop_selector(right_end)).is_some());
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert!(at(cx, &drop_selector(right_end)).is_none());
    cx.simulate_mouse_up(edge, MouseButton::Left, Modifiers::none());
    redraw(cx);
    assert_eq!(panes(&f, cx), before);
}

#[gpui::test]
fn the_panes_survive_a_restart_and_an_old_layout_opens_its_sidebar_mode(cx: &mut TestAppContext) {
    let dir = folder("panes-restart");
    let (file, look) = (dir.join("layout.json"), dir.join("look.toml"));
    let _ = std::fs::remove_file(&file);
    let saved = {
        let (f, cx) = fixture(cx, Persist::File(file.clone()), &look);
        f.shell.update(cx, |s, _| s.restored = true);
        click_toggle(cx, F);
        click_toggle(cx, A);
        let from = bounds(cx, "pane-header:alerts").center();
        let top = bounds(cx, "pane:products");
        drag_to(
            cx,
            from,
            Point::new(top.center().x, top.top() + Pixels::from(5.)),
            true,
        );
        let split = bounds(cx, "pane-split:alerts").center();
        drag_to(cx, split, Point::new(split.x, Pixels::from(300.)), true);
        let edge = bounds(cx, "pane-edge:left").center();
        drag_to(cx, edge, Point::new(Pixels::from(320.), edge.y), true);
        f.shell.read_with(cx, |s, cx| s.save_now(&file, cx));
        panes(&f, cx)
    };
    assert_eq!(saved.column(Side::Left), [A, P, F]);
    assert_eq!(saved.width(Side::Left), 320.);
    let (f, cx) = fixture(cx, Persist::File(file.clone()), &look);
    f.shell.update_in(cx, |s, window, cx| s.restore(window, cx));
    redraw(cx);
    assert_eq!(panes(&f, cx), saved);
    assert_eq!(
        bounds(cx, "pane-column:left").size.width,
        Pixels::from(320.)
    );

    // A file of before the panes.
    let old = layout::Layout {
        version: layout::VERSION,
        sidebar_mode: Some("files".into()),
        ..layout::Layout::default()
    };
    layout::write(&file, &old).unwrap();
    let (f, cx) = fixture(cx, Persist::File(file.clone()), &look);
    f.shell.update_in(cx, |s, window, cx| s.restore(window, cx));
    redraw(cx);
    assert_eq!(panes(&f, cx).open(), [F]);
    assert_eq!(toggle_state(cx, F), "open");
    assert_eq!(toggle_state(cx, P), "hidden");
}

#[gpui::test]
fn the_alerts_toggle_counts_the_unseen_alerts_also_when_the_pane_is_hidden(
    cx: &mut TestAppContext,
) {
    let dir = folder("panes-count");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    let raise = |cx: &mut VisualTestContext| {
        let n: agentsi_proto::rpc::Notification = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0", "method": "terminal.state",
            "params": {"id": "t1-a", "state": "error"}
        }))
        .unwrap();
        cx.update(|_, cx| {
            let center = Alerts::try_get(cx).unwrap();
            center.update(cx, |c, cx| {
                c.take(
                    None,
                    &n,
                    &|_| Name {
                        title: "claude".into(),
                        product: Some("a".into()),
                    },
                    cx,
                )
            });
        });
        redraw(cx);
    };
    assert!(at(cx, "pane-toggle-count:alerts:1").is_none());
    raise(cx);
    assert_eq!(toggle_state(cx, A), "hidden");
    assert!(at(cx, "pane-toggle-count:alerts:1").is_some());
    click_toggle(cx, A);
    assert!(
        at(cx, "pane-toggle-count:alerts:1").is_none(),
        "the open log counts its alerts as seen"
    );
    assert!(at(cx, "alert-row-0").is_some());
    click_toggle(cx, A);
    assert!(!panes(&f, cx).is_open(A));
}
