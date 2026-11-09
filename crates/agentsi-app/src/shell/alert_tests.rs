//! The toast, the bell, the log and the open action over the real shell, with a fake core.

use agentsi_proto::rpc::Notification;
use gpui::{TestAppContext, VisualTestContext};
use gpui_kit::component::WindowExt as _;
use serde_json::json;

use super::Persist;
use super::tests::{active_tab, at, click, fixture, folder, redraw, with_tabs};
use crate::alerts::{AlertTarget, Alerts, Name};

fn note(method: &str, params: serde_json::Value) -> Notification {
    serde_json::from_value(json!({"jsonrpc": "2.0", "method": method, "params": params})).unwrap()
}

fn raise(cx: &mut VisualTestContext, n: Notification) {
    cx.update(|_, cx| {
        let center = Alerts::try_get(cx).expect("the fixture installs the center");
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
}

fn toasts(cx: &mut VisualTestContext) -> usize {
    cx.update(|window, cx| window.notifications(cx).len())
}

fn blocked(id: &str) -> Notification {
    note(
        "terminal.state",
        json!({"id": id, "state": "blocked", "kind": "permission", "msg": "Run it?"}),
    )
}

#[gpui::test]
fn a_blocked_pane_in_a_hidden_tab_shows_a_toast_and_the_bell_counts_it(cx: &mut TestAppContext) {
    let look = folder("alert-toast").join("look.toml");
    let (f, cx) = fixture(cx, Persist::None, &look);
    with_tabs(&f, cx);
    assert_eq!(active_tab(&f, cx), "terminal:t2-a");
    assert_eq!(toasts(cx), 0);
    raise(cx, blocked("t1-a"));
    assert_eq!(toasts(cx), 1, "the toast of the hidden pane");
    // A second alert of the same target replaces its toast.
    raise(
        cx,
        note("terminal.state", json!({"id": "t1-a", "state": "working"})),
    );
    raise(cx, blocked("t1-a"));
    assert_eq!(toasts(cx), 1);
    let unseen = cx.update(|_, cx| Alerts::try_get(cx).unwrap().read(cx).unseen());
    assert_eq!(unseen, 2);
}

#[gpui::test]
fn the_bell_opens_the_log_and_open_shows_the_pane_and_ends_its_glow(cx: &mut TestAppContext) {
    let look = folder("alert-log").join("look.toml");
    let (f, cx) = fixture(cx, Persist::None, &look);
    with_tabs(&f, cx);
    raise(
        cx,
        note("terminal.state", json!({"id": "t1-a", "state": "error"})),
    );
    assert!(at(cx, "alert-log").is_none());
    click(cx, "button:alerts-bell");
    assert!(at(cx, "alert-log").is_some(), "the popover opens");
    let unseen = cx.update(|_, cx| Alerts::try_get(cx).unwrap().read(cx).unseen());
    assert_eq!(unseen, 0, "opening the log counts the alerts as seen");
    assert!(at(cx, "alert-row-0").is_some());
    let target = AlertTarget::Pane {
        host: None,
        id: "t1-a".into(),
    };
    let glow = |cx: &mut VisualTestContext| cx.update(|_, cx| Alerts::glow(cx, &target));
    assert!(glow(cx).is_some());
    click(cx, "button:alert-open-0");
    assert_eq!(active_tab(&f, cx), "terminal:t1-a");
    assert!(glow(cx).is_none(), "opening the target ends the glow");
    assert!(at(cx, "alert-log").is_none(), "the popover closes");
}

#[gpui::test]
fn an_empty_log_says_so(cx: &mut TestAppContext) {
    let look = folder("alert-empty").join("look.toml");
    let (f, cx) = fixture(cx, Persist::None, &look);
    f.shell.update(cx, |s, cx| s.toggle_alert_log(cx));
    redraw(cx);
    assert!(at(cx, "alert-log").is_some());
    assert!(at(cx, "alert-row-0").is_none());
}
