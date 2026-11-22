//! The palette over the real shell and sidebar, with a fake core: the prefixes, the recent
//! targets, the second step, and the new entries.

use gpui::{TestAppContext, VisualTestContext};
use serde_json::{Value, json};

use super::tests::{Fixture, active_tab, fixture, folder, panel_of, redraw, terminal, with_tabs};
use super::{Ask, Persist};
use crate::actions;
use crate::layout;
use crate::model::{ProductRef, two_cores::received};
use crate::palette::{self, FIND_DELAY, Kind, tests::OLD_ENTRIES};
use crate::prompt::PromptEvent;
use crate::screens::ScreenSpec;
use crate::screens::settings::{OpenedSection, Section};
use crate::tabs::SpaceId;

fn product(id: &str) -> ProductRef {
    ProductRef {
        host: None,
        id: id.into(),
    }
}

/// Opens the palette and types `query` in it.
fn type_in(f: &Fixture, cx: &mut VisualTestContext, query: &str) {
    if f.shell.read_with(cx, |s, _| s.palette.is_none()) {
        cx.dispatch_action(actions::TogglePalette);
        redraw(cx);
    }
    let state = f
        .shell
        .read_with(cx, |s, _| s.palette.as_ref().unwrap().state.clone());
    state.update_in(cx, |s, window, cx| {
        s.set_query(query.to_string(), window, cx)
    });
    redraw(cx);
}

/// The labels and the sources of the items that the palette shows now.
fn rows(f: &Fixture, cx: &mut VisualTestContext) -> Vec<(String, Kind)> {
    f.shell.read_with(cx, |s, cx| {
        let query = s.palette.as_ref().map(|p| p.query().to_string()).unwrap();
        palette::items(&query, s, cx)
            .into_iter()
            .map(|i| (i.label.to_string(), i.source))
            .collect()
    })
}

fn labels(f: &Fixture, cx: &mut VisualTestContext) -> Vec<String> {
    rows(f, cx).into_iter().map(|(l, _)| l).collect()
}

fn keys(cx: &mut VisualTestContext, keys: &str) {
    cx.simulate_keystrokes(keys);
    redraw(cx);
}

fn palette_query(f: &Fixture, cx: &mut VisualTestContext) -> Option<String> {
    f.shell
        .read_with(cx, |s, _| s.palette.as_ref().map(|p| p.query().to_string()))
}

fn asking(f: &Fixture, cx: &mut VisualTestContext) -> Option<Ask> {
    f.shell
        .read_with(cx, |s, _| s.asking.as_ref().map(|a| a.what.clone()))
}

fn methods(sent: &[Value]) -> Vec<&str> {
    sent.iter().filter_map(|r| r["method"].as_str()).collect()
}

#[gpui::test]
fn a_tab_prefix_lists_the_tab_of_each_product_and_enter_focuses_it_in_its_space(
    cx: &mut TestAppContext,
) {
    let dir = folder("palette-tabs");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    f.core.update(cx, |s, _| {
        s.put(terminal("tb-a", "a", "build"));
        s.put(terminal("tb-b", "b", "build"));
    });
    for (id, p) in [("tb-a", "a"), ("tb-b", "b")] {
        let core = f.core.clone();
        f.shell.update_in(cx, |s, window, cx| {
            s.add_terminal_panel_in(SpaceId::Product(product(p)), id.into(), core, window, cx)
        });
    }
    redraw(cx);
    f.shell.update_in(cx, |s, window, cx| {
        s.show_space(SpaceId::Product(product("a")), window, cx)
    });
    redraw(cx);

    type_in(&f, cx, "@build");
    assert_eq!(
        rows(&f, cx),
        [
            ("build · a".to_string(), Kind::Tabs),
            ("build · b".to_string(), Kind::Tabs)
        ]
    );
    keys(cx, "down enter");
    assert_eq!(palette_query(&f, cx), None, "the palette closed");
    assert_eq!(
        f.shell.read_with(cx, |s, _| s.shown_space().clone()),
        SpaceId::Product(product("b"))
    );
    assert_eq!(active_tab(&f, cx), "terminal:tb-b");
    let tb = panel_of(&f, "tb-b", cx);
    let focused = f
        .shell
        .update_in(cx, |s, window, cx| s.active_tab(window, cx));
    assert_eq!(focused, Some(tb), "the tab has the focus");
}

#[gpui::test]
fn a_settings_prefix_lists_the_terminal_section_and_enter_opens_it(cx: &mut TestAppContext) {
    let dir = folder("palette-settings");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    type_in(&f, cx, "#term");
    assert_eq!(
        rows(&f, cx),
        [("Settings: Terminal".to_string(), Kind::Settings)]
    );
    keys(cx, "enter");
    assert_eq!(palette_query(&f, cx), None);
    assert_eq!(
        cx.update(|_, cx| cx.try_global::<OpenedSection>().map(|s| s.0)),
        Some(Section::Terminal)
    );
    assert_eq!(active_tab(&f, cx), "screen:settings");
}

#[gpui::test]
fn a_file_prefix_asks_the_core_once_after_the_typing_and_lists_the_files_it_found(
    cx: &mut TestAppContext,
) {
    let dir = folder("palette-files");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    type_in(&f, cx, "/");
    assert!(labels(&f, cx).is_empty());
    let _ = received(&mut f.core_end);
    type_in(&f, cx, "/mai");
    type_in(&f, cx, "/main.rs");
    assert!(
        !methods(&received(&mut f.core_end)).contains(&"files.find"),
        "no request before the pause"
    );
    cx.executor().advance_clock(FIND_DELAY);
    redraw(cx);
    let finds: Vec<Value> = received(&mut f.core_end)
        .into_iter()
        .filter(|r| r["method"] == "files.find")
        .collect();
    assert_eq!(finds.len(), 1, "one request for the last query: {finds:?}");
    assert_eq!(
        finds[0]["params"],
        json!({"product": "a", "query": "main.rs", "limit": 50})
    );

    let paths = vec!["src/main.rs".to_string(), "app/src/main.rs".to_string()];
    f.shell.update(cx, |s, cx| {
        // An answer to an older query changes nothing.
        s.files_found(&product("a"), "mai", Ok(found(vec!["x".into()])), cx);
        s.files_found(&product("a"), "main.rs", Ok(found(paths.clone())), cx);
    });
    redraw(cx);
    assert_eq!(
        rows(&f, cx),
        [
            ("src/main.rs".to_string(), Kind::Files),
            ("app/src/main.rs".to_string(), Kind::Files)
        ]
    );
    // A product with no folder on this device opens nothing and records nothing. The test
    // platform cannot open a file with the system, so no file opens here.
    f.shell.update_in(cx, |s, window, cx| {
        s.open_file(product("b"), "src/main.rs".into(), window, cx)
    });
    assert!(f.shell.read_with(cx, |s, _| s.recent.0.is_empty()));
}

fn found(paths: Vec<String>) -> agentsi_proto::files_api::FilesFindResult {
    agentsi_proto::files_api::FilesFindResult {
        paths,
        truncated: false,
    }
}

#[gpui::test]
fn with_no_query_the_recent_targets_come_first_and_the_layout_keeps_them(cx: &mut TestAppContext) {
    let dir = folder("palette-recent");
    let (layout_file, look_file) = (dir.join("layout.json"), dir.join("look.toml"));
    {
        let (f, cx) = fixture(cx, Persist::File(layout_file.clone()), &look_file);
        f.shell.update(cx, |s, _| s.restored = true);
        for spec in [ScreenSpec::git("a"), ScreenSpec::tickets("b")] {
            f.shell
                .update_in(cx, |s, window, cx| s.show_screen(spec, window, cx));
        }
        redraw(cx);
        type_in(&f, cx, "");
        let rows = rows(&f, cx);
        assert_eq!(
            rows[..2],
            [
                ("Tickets · b".to_string(), Kind::Recent),
                ("Git · a".to_string(), Kind::Recent)
            ]
        );
        let first_action = rows.iter().position(|r| r.1 == Kind::Actions).unwrap();
        assert!(rows[..first_action].iter().all(|r| r.1 == Kind::Recent));
        assert_eq!(rows[first_action].0, actions::ENTRIES[0].label);
        f.shell.read_with(cx, |s, cx| s.save_now(&layout_file, cx));
    }
    let saved = layout::read(&layout_file).unwrap().unwrap();
    assert_eq!(saved.recent[0].target, "tickets");
    assert_eq!(saved.recent[1].target, "git");
    let (f, cx) = fixture(cx, Persist::File(layout_file.clone()), &look_file);
    f.shell.update_in(cx, |s, window, cx| s.restore(window, cx));
    type_in(&f, cx, "");
    assert_eq!(rows(&f, cx)[1], ("Git · a".to_string(), Kind::Recent));
}

#[gpui::test]
fn switch_branch_shows_the_branches_as_a_second_step_and_escape_goes_back(cx: &mut TestAppContext) {
    let dir = folder("palette-branch");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    type_in(&f, cx, "switch branch");
    assert_eq!(labels(&f, cx), ["Git: Switch branch…"]);
    let _ = received(&mut f.core_end);
    keys(cx, "enter");
    assert_eq!(palette_query(&f, cx), None);
    assert_eq!(asking(&f, cx), Some(Ask::SwitchBranch(product("a"))));
    let sent = received(&mut f.core_end);
    assert!(methods(&sent).contains(&"git.branches"), "{sent:?}");

    let prompt = f
        .shell
        .read_with(cx, |s, _| s.asking.as_ref().unwrap().prompt.clone());
    let branches: agentsi_proto::git_api::GitBranchesResult = serde_json::from_value(json!({
        "local": [
            {"name": "main", "head": "abc", "subject": "s", "current": true},
            {"name": "feat", "head": "def", "subject": "t", "current": false},
        ],
        "remote": [{"name": "origin/main", "head": "abc", "subject": "s", "current": false}],
        "worktrees": [],
    }))
    .unwrap();
    prompt.update(cx, |p, cx| {
        p.set_choices(
            super::palette_actions::branch_choices(&branches),
            0,
            None,
            cx,
        )
    });
    redraw(cx);
    assert_eq!(
        prompt.read_with(cx, |p, _| p.labels()),
        ["main (current)", "feat", "origin/main"]
    );

    keys(cx, "escape");
    assert_eq!(asking(&f, cx), None);
    assert_eq!(
        palette_query(&f, cx).as_deref(),
        Some("switch branch"),
        "Escape goes back to the first step with its query"
    );

    // The answer of the second step switches on the git screen.
    keys(cx, "enter");
    // The owner goes to another product while the question is open.
    cx.dispatch_action(actions::NextSpace);
    redraw(cx);
    let _ = received(&mut f.core_end);
    f.shell.update_in(cx, |s, window, cx| {
        s.asked(PromptEvent::Answer("feat".into()), window, cx)
    });
    redraw(cx);
    let sent = received(&mut f.core_end);
    let switch = sent.iter().find(|r| r["method"] == "git.switch");
    assert_eq!(
        switch.map(|r| (r["params"]["product"].clone(), r["params"]["name"].clone())),
        Some((json!("a"), json!("feat"))),
        "the switch acts on the product that the question named"
    );
    assert_eq!(active_tab(&f, cx), "screen:git");
}

#[gpui::test]
fn move_to_product_from_the_palette_is_a_second_step_too(cx: &mut TestAppContext) {
    let dir = folder("palette-move");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    with_tabs(&f, cx);
    let t2 = panel_of(&f, "t2-a", cx);
    f.shell
        .update_in(cx, |s, window, cx| s.select_tab(t2, window, cx));
    type_in(&f, cx, "tab to a product");
    assert_eq!(labels(&f, cx), ["Move the active tab to a product"]);
    keys(cx, "enter");
    assert!(matches!(asking(&f, cx), Some(Ask::MoveTab(_))));
    keys(cx, "escape");
    assert_eq!(palette_query(&f, cx).as_deref(), Some("tab to a product"));
}

#[gpui::test]
fn reopen_closed_tab_opens_the_last_closed_screen_and_terminal_in_their_space(
    cx: &mut TestAppContext,
) {
    let dir = folder("palette-reopen");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    with_tabs(&f, cx);
    f.shell.update_in(cx, |s, window, cx| {
        s.show_screen(ScreenSpec::git("b"), window, cx)
    });
    redraw(cx);
    let git = f
        .shell
        .update_in(cx, |s, window, cx| s.active_tab(window, cx))
        .unwrap();
    f.shell
        .update_in(cx, |s, window, cx| s.close_tabs(vec![git], window, cx));
    redraw(cx);
    let t2 = panel_of(&f, "t2-a", cx);
    f.shell
        .update_in(cx, |s, window, cx| s.close_tabs(vec![t2], window, cx));
    // t2-a runs a program: the Confirm of the tabs comes first.
    f.shell.update_in(cx, |s, window, cx| {
        s.asked(PromptEvent::Answer("close".into()), window, cx)
    });
    redraw(cx);
    let _ = received(&mut f.core_end);

    cx.dispatch_action(actions::ReopenClosedTab);
    redraw(cx);
    let create = received(&mut f.core_end)
        .into_iter()
        .find(|r| r["method"] == "terminal.create")
        .expect("a new terminal");
    assert_eq!(create["params"]["argv"], json!(["sh"]));
    assert_eq!(create["params"]["cwd"], json!("/"));
    assert_eq!(create["params"]["product"], json!("a"));

    f.shell.update_in(cx, |s, window, cx| {
        s.show_space(SpaceId::Product(product("a")), window, cx)
    });
    cx.dispatch_action(actions::ReopenClosedTab);
    redraw(cx);
    assert_eq!(
        f.shell.read_with(cx, |s, _| s.shown_space().clone()),
        SpaceId::Product(product("b")),
        "the screen opens in its own space"
    );
    assert_eq!(active_tab(&f, cx), "screen:git");
}

#[gpui::test]
fn next_and_previous_space_go_round_the_products(cx: &mut TestAppContext) {
    let dir = folder("palette-spaces");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    let shown = |cx: &mut VisualTestContext| f.shell.read_with(cx, |s, _| s.shown_space().clone());
    f.shell.update_in(cx, |s, window, cx| {
        s.select_product(&product("a"), window, cx)
    });
    cx.dispatch_action(actions::NextSpace);
    redraw(cx);
    assert_eq!(shown(cx), SpaceId::Product(product("b")));
    cx.dispatch_action(actions::NextSpace);
    redraw(cx);
    assert_eq!(shown(cx), SpaceId::Product(product("a")));
    cx.dispatch_action(actions::PreviousSpace);
    redraw(cx);
    assert_eq!(shown(cx), SpaceId::Product(product("b")));
}

#[gpui::test]
fn go_to_tab_opens_the_palette_at_the_tab_prefix_and_restart_asks_first(cx: &mut TestAppContext) {
    let dir = folder("palette-entries");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    type_in(&f, cx, "go to tab");
    assert_eq!(labels(&f, cx), ["Go to tab…"]);
    keys(cx, "enter");
    assert_eq!(palette_query(&f, cx).as_deref(), Some("@"));

    let _ = received(&mut f.core_end);
    cx.dispatch_action(actions::RestartCore);
    redraw(cx);
    assert_eq!(asking(&f, cx), Some(Ask::RestartCore));
    f.shell.update_in(cx, |s, window, cx| {
        s.asked(PromptEvent::Answer(String::new()), window, cx)
    });
    redraw(cx);
    assert!(
        !methods(&received(&mut f.core_end)).contains(&"core.shutdown"),
        "Keep stops nothing"
    );
}

#[gpui::test]
fn each_old_entry_is_found_by_its_old_keywords_in_the_real_palette(cx: &mut TestAppContext) {
    let dir = folder("palette-old");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    type_in(&f, cx, "");
    f.shell.read_with(cx, |s, cx| {
        for (label, keywords) in OLD_ENTRIES {
            for k in keywords.iter().copied().chain([*label]) {
                for query in [k.to_string(), format!(">{k}")] {
                    let found = palette::items(&query, s, cx);
                    assert!(
                        found.iter().any(|i| i.label == *label),
                        "{query:?} does not find {label:?}"
                    );
                }
            }
        }
    });
}
