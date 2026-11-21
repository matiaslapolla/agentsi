//! The spaces, the pinned tabs and the favorites over the real shell and sidebar, with a fake
//! core.

use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext};
use gpui_kit::component::dock::PanelId;
use serde_json::Value;

use super::tests::{
    Fixture, active_tab, at, click, fixture, folder, order, panel_of, redraw, with_tabs,
};
use super::{Ask, Persist};
use crate::favorites::{self, Favorite, Saved};
use crate::layout;
use crate::menus::{Cmd, Target};
use crate::model::{ProductRef, two_cores::received};
use crate::screens::{ScreenKind, ScreenSpec};
use crate::tabs::SpaceId;
use crate::tree::ItemKey;

fn product(id: &str) -> ProductRef {
    ProductRef {
        host: None,
        id: id.into(),
    }
}

fn space(id: &str) -> SpaceId {
    SpaceId::Product(product(id))
}

fn shown(f: &Fixture, cx: &mut VisualTestContext) -> SpaceId {
    f.shell.read_with(cx, |s, _| s.shown_space().clone())
}

/// The tabs of a space in the order of its bars, as `order` names them.
fn tabs_of(f: &Fixture, cx: &mut VisualTestContext, space: &SpaceId) -> Vec<String> {
    let rows = f.shell.read_with(cx, |s, cx| s.tab_rows(cx));
    rows.get(space)
        .map(|rows| {
            rows.iter()
                .map(|r| match &r.key {
                    ItemKey::Pane(id) => id.clone(),
                    ItemKey::Screen(kind, _) => format!("screen:{}", kind.name()),
                    other => other.slug(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn select(f: &Fixture, cx: &mut VisualTestContext, id: &str) {
    f.shell.update_in(cx, |s, window, cx| {
        s.select_product(&product(id), window, cx)
    });
    redraw(cx);
}

fn run(f: &Fixture, cx: &mut VisualTestContext, target: Target, cmd: Cmd) {
    f.shell
        .update_in(cx, |s, window, cx| s.run_menu(target, cmd, window, cx));
    redraw(cx);
}

fn asking(f: &Fixture, cx: &mut VisualTestContext) -> Option<Ask> {
    f.shell
        .read_with(cx, |s, _| s.asking.as_ref().map(|a| a.what.clone()))
}

fn sent(f: &mut Fixture) -> Vec<Value> {
    received(&mut f.core_end)
}

fn pane(id: &str) -> Target {
    Target::Pane(super::PaneRef {
        host: None,
        id: id.into(),
    })
}

#[gpui::test]
fn a_terminal_of_a_stays_out_of_the_bar_of_b_keeps_running_and_its_row_brings_it_back(
    cx: &mut TestAppContext,
) {
    let dir = folder("spaces-ab");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    select(&f, cx, "a");
    with_tabs(&f, cx);
    assert_eq!(shown(&f, cx), space("a"));
    assert_eq!(order(&f, cx), ["screen:desk", "t1-a", "t2-a"]);
    let t1 = panel_of(&f, "t1-a", cx);
    let _ = sent(&mut f);

    select(&f, cx, "b");
    assert_eq!(shown(&f, cx), space("b"));
    assert_eq!(
        order(&f, cx),
        ["screen:desk"],
        "the bar shows the tabs of b only"
    );
    assert!(at(cx, &format!("tab:{}", t1.as_u64())).is_none());
    // The terminal of a keeps running: its tab lives on in the space of a, nothing is killed.
    assert_eq!(
        tabs_of(&f, cx, &space("a")),
        ["screen:desk", "t1-a", "t2-a"]
    );
    let s = sent(&mut f);
    assert!(s.iter().all(|r| r["method"] != "terminal.kill"), "{s:?}");
    assert!(
        f.core
            .read_with(cx, |c, _| c.terminals.contains_key("t1-a"))
    );

    // The sidebar row of a lists its desk tab and its terminals. A click on the terminal row
    // selects a and focuses that terminal.
    assert!(at(cx, "tree-row:product-a-screen-desk").is_some());
    assert!(at(cx, "tree-row:product-b-screen-desk").is_some());
    click(cx, "tree-row:product-a-pane-t1-a");
    assert_eq!(shown(&f, cx), space("a"));
    assert_eq!(active_tab(&f, cx), "terminal:t1-a");
    let a = f.core.read_with(cx, |c, _| c.active.clone());
    assert_eq!(a, Some(product("a")));
    let focused = f.shell.update_in(cx, |s, window, cx| {
        s.focused_terminal(window, cx)
            .map(|p| p.read(cx).id.clone())
    });
    assert_eq!(focused.as_deref(), Some("t1-a"));
    // The desk row of b shows b at its desk.
    click(cx, "tree-row:product-b-screen-desk");
    assert_eq!(shown(&f, cx), space("b"));
    assert_eq!(active_tab(&f, cx), "screen:desk");
}

#[gpui::test]
fn a_middle_click_on_a_row_closes_its_tab_but_not_a_pinned_one(cx: &mut TestAppContext) {
    let dir = folder("spaces-middle");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    select(&f, cx, "a");
    f.shell.update_in(cx, |s, window, cx| {
        s.show_screen(ScreenSpec::minutes("a"), window, cx);
        s.show_screen(ScreenSpec::git("a"), window, cx);
    });
    redraw(cx);
    let middle = |cx: &mut VisualTestContext, name: &str| {
        let b = at(cx, name).unwrap_or_else(|| panic!("{name} is drawn"));
        cx.simulate_mouse_down(b.center(), MouseButton::Middle, Modifiers::none());
        cx.simulate_mouse_up(b.center(), MouseButton::Middle, Modifiers::none());
        redraw(cx);
    };
    middle(cx, "tree-row:product-a-screen-minutes");
    assert_eq!(order(&f, cx), ["screen:desk", "screen:git"]);
    let git = f.shell.read_with(cx, |s, cx| {
        s.tab_rows(cx)[&space("a")]
            .iter()
            .find(|r| r.key == ItemKey::Screen(ScreenKind::Git, None))
            .map(|r| r.panel)
            .unwrap()
    });
    run(&f, cx, Target::Tab(git), Cmd::PinTab);
    middle(cx, "tree-row:product-a-screen-git");
    assert_eq!(order(&f, cx), ["screen:git", "screen:desk"]);
    assert_eq!(
        asking(&f, cx),
        None,
        "a pinned row does not close, and asks nothing"
    );
}

#[gpui::test]
fn a_pinned_tab_sits_first_with_icon_width_survives_close_others_and_asks_before_it_closes(
    cx: &mut TestAppContext,
) {
    let dir = folder("spaces-pin");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    select(&f, cx, "a");
    f.shell.update_in(cx, |s, window, cx| {
        s.show_screen(ScreenSpec::minutes("a"), window, cx);
        s.show_screen(ScreenSpec::git("a"), window, cx);
    });
    redraw(cx);
    let tab = |f: &Fixture, cx: &mut VisualTestContext, kind: ScreenKind| -> PanelId {
        f.shell.read_with(cx, |s, cx| {
            s.tab_rows(cx)[&space("a")]
                .iter()
                .find(|r| r.key == ItemKey::Screen(kind, None))
                .map(|r| r.panel)
                .unwrap()
        })
    };
    let git = tab(&f, cx, ScreenKind::Git);
    let desk = tab(&f, cx, ScreenKind::Desk);
    let wide = at(cx, &format!("tab:{}", desk.as_u64()))
        .unwrap()
        .size
        .width;
    run(&f, cx, Target::Tab(git), Cmd::PinTab);
    assert_eq!(
        order(&f, cx),
        ["screen:git", "screen:desk", "screen:minutes"]
    );
    let pinned = at(cx, &format!("tab:{}", git.as_u64())).expect("the pinned tab is drawn");
    assert!(at(cx, &format!("tab-pinned:{}", git.as_u64())).is_some());
    assert!(
        pinned.size.width < wide,
        "icon width: {:?} against {:?}",
        pinned.size.width,
        wide
    );
    assert!(
        at(cx, &format!("tab-close:{}", git.as_u64())).is_none(),
        "a pinned tab has no close button"
    );
    // A pinned tab joins no tab group.
    run(&f, cx, Target::Tab(git), Cmd::NewTabGroup);
    assert_eq!(asking(&f, cx), None);
    // Close others keeps the pinned tab and asks nothing: no program runs in a screen.
    run(&f, cx, Target::Tab(desk), Cmd::CloseOthers);
    assert_eq!(order(&f, cx), ["screen:git", "screen:desk"]);
    // Close on the pinned tab asks one Confirm. "Keep them" keeps it.
    run(&f, cx, Target::Tab(git), Cmd::CloseTab);
    assert!(matches!(asking(&f, cx), Some(Ask::CloseTabs(_, 0, 1))));
    click(cx, "prompt-choice:keep");
    assert_eq!(order(&f, cx), ["screen:git", "screen:desk"]);
    run(&f, cx, Target::Tab(git), Cmd::CloseTab);
    click(cx, "prompt-choice:close");
    assert_eq!(order(&f, cx), ["screen:desk"]);

    // A pinned terminal tab: the pane menu pins it, and Close others skips it.
    with_tabs(&f, cx);
    let _ = sent(&mut f);
    run(&f, cx, pane("t2-a"), Cmd::PinTab);
    assert_eq!(order(&f, cx), ["t2-a", "screen:desk", "t1-a"]);
    f.shell.update_in(cx, |s, window, cx| {
        let p = s.dock.read(cx).panel(desk).is_some();
        assert!(p);
        s.close_others(desk, window, cx)
    });
    redraw(cx);
    // t1-a runs a program: one Confirm for it, and the pinned t2-a is not in the count.
    assert!(matches!(asking(&f, cx), Some(Ask::CloseTabs(ref t, 1, 0)) if t.len() == 1));
    click(cx, "prompt-choice:close");
    assert_eq!(order(&f, cx), ["t2-a", "screen:desk"]);
}

#[gpui::test]
fn a_version_2_layout_puts_each_tab_in_the_space_of_its_product(cx: &mut TestAppContext) {
    let dir = folder("spaces-v2");
    let (layout_file, look_file) = (dir.join("layout.json"), dir.join("look.toml"));
    let screen = |kind: &str, product: &str| {
        serde_json::json!({"panel_name": "screen", "children": [],
            "info": {"panel": {"kind": kind, "product": product}}})
    };
    let term = |id: &str| {
        serde_json::json!({"panel_name": "terminal", "children": [],
            "info": {"panel": {"terminal": id}}})
    };
    let t1 = serde_json::to_string(&layout::terminal_info("t1-a", None)).unwrap();
    let key = format!(
        "terminal:{}",
        serde_json::from_str::<Value>(&t1).unwrap()["panel"]
    );
    let v2 = serde_json::json!({
        "version": 2,
        "active_product": "b",
        "tab_groups": [{"name": "Build", "tabs": [key]}],
        "dock": {"center": {"panel_name": "TabPanel", "info": {"tabs": {"active_index": 0}},
            "children": [screen("desk", "a"), term("t1-a"), term("t3-b"),
                         screen("minutes", "b"), screen("settings", "")]}},
    });
    std::fs::write(&layout_file, v2.to_string()).unwrap();
    let (f, cx) = fixture(cx, Persist::File(layout_file.clone()), &look_file);
    f.shell.update_in(cx, |s, window, cx| s.restore(window, cx));
    redraw(cx);
    assert_eq!(tabs_of(&f, cx, &space("a")), ["screen:desk", "t1-a"]);
    assert_eq!(tabs_of(&f, cx, &space("b")), ["t3-b", "screen:minutes"]);
    assert_eq!(tabs_of(&f, cx, &SpaceId::Home), ["screen:settings"]);
    // The active product's space shows, and the group came with its tab.
    assert_eq!(shown(&f, cx), space("b"));
    select(&f, cx, "a");
    let groups = f
        .shell
        .read_with(cx, |s, cx| s.tabs.read(cx).groups().to_vec());
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].members, [panel_of(&f, "t1-a", cx)]);
    // The next save writes version 3.
    f.shell.update(cx, |s, cx| s.save_now(&layout_file, cx));
    let saved = layout::read(&layout_file).unwrap().unwrap();
    assert_eq!(saved.version, 3);
    assert!(saved.dock.is_none());
    let keys: Vec<&str> = saved.spaces.keys().map(String::as_str).collect();
    assert_eq!(keys, [layout::HOME, "a", "b"]);
}

#[gpui::test]
fn the_spaces_the_pins_and_the_favorites_are_the_same_after_a_restart(cx: &mut TestAppContext) {
    let dir = folder("spaces-restart");
    let (layout_file, look_file, fav_file) = (
        dir.join("layout.json"),
        dir.join("look.toml"),
        dir.join(favorites::FILE_NAME),
    );
    {
        cx.update(|cx| favorites::install(cx, Some(fav_file.clone())));
        let (f, cx) = fixture(cx, Persist::File(layout_file.clone()), &look_file);
        f.shell.update(cx, |s, _| s.restored = true);
        select(&f, cx, "a");
        with_tabs(&f, cx);
        run(&f, cx, pane("t2-a"), Cmd::PinTab);
        run(&f, cx, pane("t2-a"), Cmd::AddToFavorites);
        select(&f, cx, "b");
        f.shell.update_in(cx, |s, window, cx| {
            s.focus_terminal(None, "t3-b", window, cx)
        });
        redraw(cx);
        assert_eq!(shown(&f, cx), space("b"));
        assert_eq!(order(&f, cx), ["screen:desk", "t3-b"]);
        f.shell.read_with(cx, |s, cx| s.save_now(&layout_file, cx));
    }
    let text = std::fs::read_to_string(&layout_file).unwrap();
    assert_eq!(text.matches("\"pinned\": true").count(), 1, "{text}");
    let (back, note) = favorites::read(&fav_file);
    assert_eq!(note, None);
    assert_eq!(back.len(), 1);

    cx.update(|cx| favorites::install(cx, Some(fav_file.clone())));
    let (f, cx) = fixture(cx, Persist::File(layout_file.clone()), &look_file);
    f.shell.update_in(cx, |s, window, cx| s.restore(window, cx));
    redraw(cx);
    assert_eq!(shown(&f, cx), space("b"));
    assert_eq!(order(&f, cx), ["screen:desk", "t3-b"]);
    assert_eq!(
        tabs_of(&f, cx, &space("a")),
        ["t2-a", "screen:desk", "t1-a"]
    );
    let t2 = panel_of(&f, "t2-a", cx);
    assert!(f.shell.read_with(cx, |s, cx| s.is_pinned(t2, cx)));
    // The load of the hidden space of a tells its tabs nothing that counts: none is seen.
    let seen: Vec<String> = f
        .open
        .borrow()
        .terminals
        .iter()
        .filter_map(gpui::WeakEntity::upgrade)
        .filter(|p| p.read_with(cx, |p, _| p.shown))
        .map(|p| p.read_with(cx, |p, _| p.id.clone()))
        .collect();
    assert_eq!(seen, ["t3-b"]);
    // The favorite shows in the sidebar.
    assert!(at(cx, "tree-row:favorite-0").is_some());
}

#[gpui::test]
fn a_favorite_of_a_terminal_command_opens_that_command_in_that_folder(cx: &mut TestAppContext) {
    let dir = folder("spaces-fav");
    let fav_file = dir.join(favorites::FILE_NAME);
    let list = cx.update(|cx| favorites::install(cx, Some(fav_file.clone())));
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    // The Favorites header is folded while the list is empty.
    assert!(at(cx, "tree-row:favorites").is_some());
    assert!(at(cx, "tree-row:favorite-0").is_none());
    list.update(cx, |l, cx| {
        l.add(
            Favorite {
                label: "Build".into(),
                product: Some(product("b")),
                saved: Saved::Command {
                    argv: vec!["cargo".into(), "build".into()],
                    folder: "/work/b".into(),
                },
            },
            cx,
        );
        l.add(
            Favorite {
                label: String::new(),
                product: Some(product("b")),
                saved: Saved::Screen(ScreenKind::Tickets, None),
            },
            cx,
        );
    });
    redraw(cx);
    let _ = sent(&mut f);
    click(cx, "tree-row:favorite-0");
    let s = sent(&mut f);
    let create = s
        .iter()
        .find(|r| r["method"] == "terminal.create")
        .expect("a terminal is created");
    assert_eq!(
        create["params"]["argv"],
        serde_json::json!(["cargo", "build"])
    );
    assert_eq!(create["params"]["cwd"], "/work/b");
    assert_eq!(create["params"]["product"], "b");
    // A screen favorite opens in the space of its product.
    click(cx, "tree-row:favorite-1");
    assert_eq!(shown(&f, cx), space("b"));
    assert_eq!(active_tab(&f, cx), "screen:tickets");
    // Rename and Remove from the menu. Remove asks one Confirm.
    run(&f, cx, Target::Favorite(1), Cmd::RenameFavorite);
    assert_eq!(asking(&f, cx), Some(Ask::RenameFavorite(1)));
    cx.simulate_keystrokes("ctrl-a");
    cx.simulate_input("Tickets of b");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(
        list.read_with(cx, |l, _| l.list[1].label.clone()),
        "Tickets of b"
    );
    run(&f, cx, Target::Favorite(0), Cmd::RemoveFavorite);
    click(cx, "prompt-choice:yes");
    let labels = list.read_with(cx, |l, _| {
        l.list.iter().map(|f| f.label.clone()).collect::<Vec<_>>()
    });
    assert_eq!(labels, ["Tickets of b"]);
    assert_eq!(favorites::read(&fav_file).0.len(), 1);
}

#[gpui::test]
fn a_favorite_row_dragged_onto_another_takes_its_place(cx: &mut TestAppContext) {
    let dir = folder("spaces-fav-drag");
    let list = cx.update(|cx| favorites::install(cx, Some(dir.join(favorites::FILE_NAME))));
    let (_f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    for name in ["one", "two", "three"] {
        list.update(cx, |l, cx| {
            l.add(
                Favorite {
                    label: name.into(),
                    product: None,
                    saved: Saved::Folder(format!("/{name}")),
                },
                cx,
            )
        });
    }
    redraw(cx);
    let (a, b) = (
        at(cx, "tree-row:favorite-2").unwrap().center(),
        at(cx, "tree-row:favorite-0").unwrap().center(),
    );
    let held = MouseButton::Left;
    cx.simulate_mouse_down(a, held, Modifiers::none());
    cx.simulate_mouse_move(
        gpui::Point::new(a.x + gpui::Pixels::from(8.), a.y),
        held,
        Modifiers::none(),
    );
    redraw(cx);
    cx.simulate_mouse_move(b, held, Modifiers::none());
    redraw(cx);
    cx.simulate_mouse_up(b, held, Modifiers::none());
    redraw(cx);
    let labels = list.read_with(cx, |l, _| {
        l.list.iter().map(|f| f.label.clone()).collect::<Vec<_>>()
    });
    assert_eq!(labels, ["three", "one", "two"]);
}

#[gpui::test]
fn move_to_product_takes_a_terminal_tab_to_another_space_and_keeps_the_terminal(
    cx: &mut TestAppContext,
) {
    let dir = folder("spaces-move");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    select(&f, cx, "a");
    with_tabs(&f, cx);
    let _ = sent(&mut f);
    run(&f, cx, pane("t2-a"), Cmd::PinTab);
    run(&f, cx, pane("t2-a"), Cmd::MoveToProduct);
    assert!(matches!(asking(&f, cx), Some(Ask::MoveTab(_))));
    click(cx, "prompt-choice:b");
    assert_eq!(shown(&f, cx), space("b"));
    assert_eq!(tabs_of(&f, cx, &space("a")), ["screen:desk", "t1-a"]);
    assert_eq!(tabs_of(&f, cx, &space("b")), ["t2-a"]);
    let t2 = panel_of(&f, "t2-a", cx);
    assert!(
        f.shell.read_with(cx, |s, cx| s.is_pinned(t2, cx)),
        "the pin moves with it"
    );
    let s = sent(&mut f);
    assert!(s.iter().all(|r| r["method"] != "terminal.kill"), "{s:?}");
    assert!(
        f.core
            .read_with(cx, |c, _| c.terminals.contains_key("t2-a"))
    );
    // The row of t2-a moved under b.
    assert!(at(cx, "tree-row:product-b-pane-t2-a").is_some());
    assert!(at(cx, "tree-row:product-a-pane-t2-a").is_none());
}
