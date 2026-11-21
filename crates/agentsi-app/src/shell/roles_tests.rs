//! The general roles over the real shell with a fake core: the palette entry opens the roles
//! screen of the active product at the form, and a `data.changed` of kind `roles` with no
//! product makes the screen read the general roles again.

use agentsi_proto::rpc::Notification;
use gpui::TestAppContext;
use serde_json::{Value, json};

use super::Persist;
use super::tests::{Fixture, active_tab, at, fixture, folder, redraw};
use crate::model::{take_event, two_cores::received};

fn methods(f: &mut Fixture) -> Vec<String> {
    received(&mut f.core_end)
        .iter()
        .map(|r: &Value| r["method"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[gpui::test]
fn the_palette_entry_opens_the_roles_at_the_form_of_a_new_general_role(cx: &mut TestAppContext) {
    let dir = folder("general-role-entry");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    let _ = methods(&mut f);
    cx.dispatch_action(crate::actions::NewGeneralRole);
    redraw(cx);
    redraw(cx);
    assert_eq!(active_tab(&f, cx), "screen:roles");
    assert!(at(cx, "form:roles-general").is_some(), "the form is drawn");
    assert!(methods(&mut f).contains(&"role.general.list".to_string()));

    let n: Notification = serde_json::from_value(json!({
        "jsonrpc": "2.0", "method": "data.changed",
        "params": {"product": "", "kind": "roles"}
    }))
    .unwrap();
    let core = f.core.clone();
    take_event(&core, &n, &mut cx.to_async());
    redraw(cx);
    let read = methods(&mut f);
    assert!(read.contains(&"role.general.list".to_string()), "{read:?}");
    assert!(read.contains(&"roles.list".to_string()), "{read:?}");
    assert!(at(cx, "form:roles-general").is_some(), "a reload keeps the form");
}
