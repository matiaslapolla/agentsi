//! The tracker over the real shell with a fake core: the key opens the Tickets screen, and a
//! `data.changed` of kind `tickets` makes the screen read again.

use agentsi_proto::rpc::Notification;
use gpui::{TestAppContext, VisualTestContext};
use serde_json::{Value, json};

use super::Persist;
use super::tests::{Fixture, active_tab, fixture, folder, redraw};
use crate::model::{take_event, two_cores::received};

fn sent(f: &mut Fixture) -> Vec<Value> {
    received(&mut f.core_end)
}

fn methods(sent: &[Value]) -> Vec<String> {
    sent.iter()
        .map(|r| r["method"].as_str().unwrap_or_default().to_string())
        .collect()
}

fn changed(cx: &mut VisualTestContext, f: &Fixture, kind: &str) {
    let n: Notification = serde_json::from_value(json!({
        "jsonrpc": "2.0", "method": "data.changed",
        "params": {"product": "a", "kind": kind}
    }))
    .unwrap();
    let core = f.core.clone();
    take_event(&core, &n, &mut cx.to_async());
    redraw(cx);
}

#[gpui::test]
fn the_key_opens_the_tickets_of_the_active_product_and_reads_them(cx: &mut TestAppContext) {
    let dir = folder("ticket-key");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    let _ = sent(&mut f);
    cx.simulate_keystrokes("ctrl-shift-i");
    redraw(cx);
    assert_eq!(active_tab(&f, cx), "screen:tickets");
    let s = sent(&mut f);
    let list = s
        .iter()
        .find(|r| r["method"] == "ticket.list")
        .expect("the screen reads the tickets");
    assert_eq!(list["params"]["product"], "a");
}

#[gpui::test]
fn a_ticket_change_makes_the_open_screen_read_again(cx: &mut TestAppContext) {
    let dir = folder("ticket-reload");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    cx.simulate_keystrokes("ctrl-shift-i");
    redraw(cx);
    let _ = sent(&mut f);
    changed(cx, &f, "tickets");
    assert!(methods(&sent(&mut f)).contains(&"ticket.list".to_string()));
}
