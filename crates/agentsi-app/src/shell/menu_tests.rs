//! The context menus over the real shell, the sidebar and the tab bar, with a fake core.

use agentsi_proto::desk_api::ProductInfo;
use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext};
use serde_json::Value;

use super::tests::{
    Fixture, active_tab, at, click, fixture, folder, four_tabs, make_group, order, redraw, tab_of,
    with_tabs,
};
use super::{Ask, PaneRef, Persist};
use crate::menus::{self, Cmd, Item, Target};
use crate::model::{ProductRef, two_cores::received};

fn product(id: &str) -> ProductRef {
    ProductRef {
        host: None,
        id: id.into(),
    }
}

fn pane(id: &str) -> Target {
    Target::Pane(PaneRef {
        host: None,
        id: id.into(),
    })
}

fn run(f: &Fixture, cx: &mut VisualTestContext, target: Target, cmd: Cmd) {
    f.shell
        .update_in(cx, |s, window, cx| s.run_menu(target, cmd, window, cx));
    redraw(cx);
}

/// What the core received since the last read.
fn sent(f: &mut Fixture) -> Vec<Value> {
    received(&mut f.core_end)
}

fn call(sent: &[Value], method: &str) -> Option<Value> {
    sent.iter().find(|r| r["method"] == method).cloned()
}

fn prompt_open(cx: &mut VisualTestContext) -> bool {
    at(cx, "prompt").is_some()
}

fn leave_prompt(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert!(!prompt_open(cx));
}

/// The entries of the menu of `target`, with the facts that the shell reads now.
fn entries(f: &Fixture, cx: &mut VisualTestContext, target: &Target) -> Vec<Cmd> {
    let facts = f.shell.read_with(cx, |s, cx| s.menu_facts(target, cx));
    menus::items(target, &facts)
        .into_iter()
        .filter_map(|i| match i {
            Item::Entry(e) if e.enabled => Some(e.cmd),
            _ => None,
        })
        .collect()
}

/// Product `b` has a folder, so its path entries are on.
fn give_folder(f: &Fixture, dir: &std::path::Path, cx: &mut VisualTestContext) {
    let info = |id: &str, path: Option<String>| ProductInfo {
        id: id.into(),
        name: id.into(),
        path,
        archived: false,
    };
    f.core.update(cx, |s, cx| {
        s.set_products(
            vec![info("a", None), info("b", Some(dir.display().to_string()))],
            cx,
        )
    });
}

#[gpui::test]
fn each_entry_of_the_product_menu_runs_its_action_and_the_costly_ones_ask_one_confirm(
    cx: &mut TestAppContext,
) {
    let dir = folder("menu-product");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    give_folder(&f, &dir, cx);
    let b = || Target::Product(product("b"));
    // Every entry that is on is tried below.
    let tried = [
        Cmd::NewTerminal,
        Cmd::OpenDesk,
        Cmd::StartMeeting,
        Cmd::NewStory,
        Cmd::NewTicket,
        Cmd::OpenTickets,
        Cmd::Reflect,
        Cmd::OpenGit,
        Cmd::Label,
        Cmd::Color,
        Cmd::MoveToGroup,
        Cmd::CopyPath,
        Cmd::OpenFolder,
        Cmd::DeleteData,
        Cmd::Archive,
    ];
    assert_eq!(entries(&f, cx, &b()), tried);
    let _ = sent(&mut f);

    run(&f, cx, b(), Cmd::NewTerminal);
    let s = sent(&mut f);
    let create = call(&s, "terminal.create").expect("a terminal is created");
    assert_eq!(create["params"]["product"], "b");
    assert_eq!(create["params"]["cwd"], dir.display().to_string());

    run(&f, cx, b(), Cmd::OpenDesk);
    assert_eq!(active_tab(&f, cx), "screen:desk");
    run(&f, cx, b(), Cmd::StartMeeting);
    assert_eq!(active_tab(&f, cx), "screen:meeting");
    run(&f, cx, b(), Cmd::OpenGit);
    assert_eq!(active_tab(&f, cx), "screen:git");

    run(&f, cx, b(), Cmd::NewStory);
    assert!(prompt_open(cx));
    cx.simulate_input("Add a login");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    let s = sent(&mut f);
    let story = call(&s, "story.new").expect("the PM shapes the story");
    assert_eq!(story["params"]["text"], "Add a login");
    assert_eq!(story["params"]["product"], "b");

    run(&f, cx, b(), Cmd::NewTicket);
    assert!(prompt_open(cx));
    cx.simulate_input("Try the tracker");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    let s = sent(&mut f);
    let ticket = call(&s, "ticket.add").expect("the ticket is added");
    assert_eq!(ticket["params"]["title"], "Try the tracker");
    assert_eq!(ticket["params"]["kind"], "note");
    assert_eq!(ticket["params"]["product"], "b");
    assert_eq!(ticket["params"]["author"]["by"], "owner");
    run(&f, cx, b(), Cmd::OpenTickets);
    assert_eq!(active_tab(&f, cx), "screen:tickets");

    for cmd in [Cmd::Label, Cmd::Color, Cmd::MoveToGroup] {
        run(&f, cx, b(), cmd);
        assert!(prompt_open(cx), "{cmd:?} asks");
        leave_prompt(cx);
    }

    // Reflect costs money: one Confirm, and nothing is sent before it.
    run(&f, cx, b(), Cmd::Reflect);
    assert!(prompt_open(cx));
    assert!(call(&sent(&mut f), "reflect.run").is_none());
    click(cx, "prompt-choice:yes");
    assert!(!prompt_open(cx), "one Confirm and no second one");
    let s = sent(&mut f);
    assert_eq!(call(&s, "reflect.run").unwrap()["params"]["product"], "b");
    assert!(f.core.read_with(cx, |c, _| c.reflecting.contains("b")));
    // While it runs, the entry is off.
    assert!(!entries(&f, cx, &b()).contains(&Cmd::Reflect));

    // The folder.
    run(&f, cx, b(), Cmd::CopyPath);
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some(dir.to_str().unwrap()));
    run(&f, cx, b(), Cmd::OpenFolder);
    let url = cx.opened_url().expect("the folder opens");
    assert!(
        url.starts_with("file://") && url.ends_with(dir.file_name().unwrap().to_str().unwrap())
    );

    // Delete product data opens the Data section of the settings.
    run(&f, cx, b(), Cmd::DeleteData);
    assert_eq!(active_tab(&f, cx), "screen:settings");

    // Archive hides the product: one Confirm, then `archive.archive`.
    run(&f, cx, b(), Cmd::Archive);
    assert!(prompt_open(cx));
    assert!(call(&sent(&mut f), "archive.archive").is_none());
    click(cx, "prompt-choice:no");
    assert!(call(&sent(&mut f), "archive.archive").is_none());
    run(&f, cx, b(), Cmd::Archive);
    click(cx, "prompt-choice:yes");
    let s = sent(&mut f);
    let archive = call(&s, "archive.archive").expect("the product is archived");
    assert_eq!(archive["params"]["kind"], "product");
    assert_eq!(archive["params"]["product"], "b");
    assert!(archive["params"]["id"].is_null());
}

#[gpui::test]
fn each_entry_of_the_pane_menu_runs_its_action_and_a_running_pane_asks_one_confirm_on_close(
    cx: &mut TestAppContext,
) {
    let dir = folder("menu-pane");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    four_tabs(&f, cx);
    let t2 = || pane("t2-a");
    let _ = sent(&mut f);
    assert_eq!(
        entries(&f, cx, &t2()),
        [
            Cmd::Rename,
            Cmd::ColorPane,
            Cmd::NewTabGroup,
            Cmd::AddToTabGroup,
            Cmd::TalkToPane,
            Cmd::VoiceMode,
            Cmd::SplitRight,
            Cmd::SplitDown,
            Cmd::CopyFolder,
            Cmd::ClosePane,
            Cmd::CloseOthers,
            Cmd::CloseToRight,
        ]
    );
    for cmd in [Cmd::Rename, Cmd::ColorPane] {
        run(&f, cx, t2(), cmd);
        assert!(prompt_open(cx), "{cmd:?} asks");
        leave_prompt(cx);
    }
    run(&f, cx, t2(), Cmd::TalkToPane);
    run(&f, cx, t2(), Cmd::VoiceMode);
    let s = sent(&mut f);
    assert!(call(&s, "voice.converse").is_some());
    assert!(call(&s, "voice.mode").is_some());
    run(&f, cx, t2(), Cmd::CopyFolder);
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some("/"));

    // A split opens a terminal in the folder of the pane, in the product of the pane.
    for cmd in [Cmd::SplitRight, Cmd::SplitDown] {
        run(&f, cx, t2(), cmd);
        let s = sent(&mut f);
        let create = call(&s, "terminal.create").expect("a terminal is created");
        assert_eq!(create["params"]["cwd"], "/", "{cmd:?}");
        assert_eq!(create["params"]["product"], "a", "{cmd:?}");
    }

    // Tab group.
    run(&f, cx, t2(), Cmd::NewTabGroup);
    cx.simulate_input("Build");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(super::tests::groups(&f, cx).len(), 1);
    assert!(entries(&f, cx, &t2()).contains(&Cmd::LeaveTabGroup));
    run(&f, cx, t2(), Cmd::AddToTabGroup);
    assert!(prompt_open(cx), "the picker lists the group");
    leave_prompt(cx);
    run(&f, cx, t2(), Cmd::LeaveTabGroup);
    assert!(super::tests::groups(&f, cx).is_empty());

    // The tab order is t1-a, t3-b, minutes, t2-a. Close to the right of t3-b closes the minutes
    // and t2-a. A program runs in t2-a, so one Confirm.
    let t3 = || pane("t3-b");
    run(&f, cx, t3(), Cmd::CloseToRight);
    assert!(prompt_open(cx));
    click(cx, "prompt-choice:close");
    assert!(!prompt_open(cx), "one Confirm only");
    assert_eq!(order(&f, cx), ["t1-a", "t3-b"]);
    // Close others keeps t3-b only, behind one Confirm for t1-a.
    run(&f, cx, t3(), Cmd::CloseOthers);
    assert!(prompt_open(cx));
    click(cx, "prompt-choice:close");
    assert_eq!(order(&f, cx), ["t3-b"]);

    // Close: the program runs, so one Confirm. Keep leaves the tab, close kills the terminal.
    let _ = sent(&mut f);
    run(&f, cx, t3(), Cmd::ClosePane);
    assert!(prompt_open(cx));
    assert!(matches!(
        f.shell
            .read_with(cx, |s, _| s.asking.as_ref().map(|a| a.what.clone())),
        Some(Ask::CloseTabs(..))
    ));
    click(cx, "prompt-choice:keep");
    assert_eq!(order(&f, cx), ["t3-b"]);
    run(&f, cx, t3(), Cmd::ClosePane);
    click(cx, "prompt-choice:close");
    assert!(!prompt_open(cx), "one Confirm only");
    assert!(order(&f, cx).is_empty());
    assert!(call(&sent(&mut f), "terminal.kill").is_some());
    // The pane t3-b has no tab now: its menu has the tab entries off.
    assert!(!entries(&f, cx, &t3()).contains(&Cmd::CloseOthers));
}

#[gpui::test]
fn a_pane_that_exited_closes_with_no_confirm_and_a_pane_with_no_tab_is_killed(
    cx: &mut TestAppContext,
) {
    let dir = folder("menu-exited");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    with_tabs(&f, cx);
    f.core.update(cx, |s, _| {
        for t in s.terminals.values_mut() {
            t.running = false;
        }
    });
    run(&f, cx, pane("t1-a"), Cmd::ClosePane);
    assert!(!prompt_open(cx));
    assert_eq!(order(&f, cx), ["t2-a"]);
    // Talk and voice mode are off for a pane that exited.
    assert!(!entries(&f, cx, &pane("t2-a")).contains(&Cmd::TalkToPane));
    // t3-b has no tab: a running one asks, an answer kills it.
    let _ = sent(&mut f);
    f.core.update(cx, |s, _| {
        s.terminals.get_mut("t3-b").unwrap().running = true;
    });
    run(&f, cx, pane("t3-b"), Cmd::ClosePane);
    assert!(prompt_open(cx));
    click(cx, "prompt-choice:yes");
    assert!(call(&sent(&mut f), "terminal.kill").is_some());
}

#[gpui::test]
fn a_split_puts_the_new_pane_in_its_own_tab_group_at_the_side(cx: &mut TestAppContext) {
    let dir = folder("menu-split");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    with_tabs(&f, cx);
    let bars = |f: &Fixture, cx: &mut VisualTestContext| {
        f.shell
            .read_with(cx, |s, cx| crate::tabs::bars(s.dock.read(cx)).len())
    };
    assert_eq!(bars(&f, cx), 1);
    let (new, other) = (
        super::tests::panel_of(&f, "t2-a", cx),
        super::tests::panel_of(&f, "t1-a", cx),
    );
    f.shell.update_in(cx, |s, window, cx| {
        s.place_beside(
            new,
            other,
            gpui_kit::component::Placement::Right,
            window,
            cx,
        )
    });
    redraw(cx);
    assert_eq!(bars(&f, cx), 2);
}

#[gpui::test]
fn the_story_meeting_reflection_and_group_menus_run_their_entries(cx: &mut TestAppContext) {
    let dir = folder("menu-others");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    let a = || product("a");
    // The reflection and the meeting of product a run in the fixture.
    run(&f, cx, Target::Reflection(a()), Cmd::Open);
    assert_eq!(active_tab(&f, cx), "screen:memory");
    run(&f, cx, Target::Meeting(a()), Cmd::Open);
    assert_eq!(active_tab(&f, cx), "screen:meeting");
    let _ = sent(&mut f);
    // Cancel stops a meeting that runs: one Confirm.
    run(&f, cx, Target::Meeting(a()), Cmd::CancelMeeting);
    assert!(prompt_open(cx));
    assert!(call(&sent(&mut f), "meeting.cancel").is_none());
    click(cx, "prompt-choice:yes");
    assert!(call(&sent(&mut f), "meeting.cancel").is_some());

    // The story session S1 is at work: Accept is off, the others run.
    let story = || Target::Story(a(), "S1".into());
    assert_eq!(
        entries(&f, cx, &story()),
        [Cmd::Open, Cmd::Review, Cmd::Reject, Cmd::StoryTerminal]
    );
    run(&f, cx, story(), Cmd::Open);
    assert_eq!(active_tab(&f, cx), "screen:story:S1");
    run(&f, cx, Target::Reflection(a()), Cmd::Open);
    run(&f, cx, story(), Cmd::Review);
    assert_eq!(active_tab(&f, cx), "screen:story:S1");
    assert!(
        sent(&mut f).iter().any(|r| r["method"] == "story.review"),
        "Review reads the diff again"
    );
    // Reject keeps the Confirm of the story screen: the screen holds the pending action.
    run(&f, cx, story(), Cmd::Reject);
    let pending = f.open.borrow().screens.iter().any(|s| {
        s.upgrade()
            .is_some_and(|s| s.read_with(cx, |s, cx| s.story_pending(cx)))
    });
    assert!(pending, "the story screen asks its own Confirm");
    run(&f, cx, story(), Cmd::StoryTerminal);
    assert!(call(&sent(&mut f), "story.get").is_some());

    // The group menu.
    f.look
        .update(cx, |l, cx| l.move_product(None, "a", "Work", cx));
    redraw(cx);
    let g = || Target::Group(None, "Work".into());
    run(&f, cx, g(), Cmd::RenameGroup);
    cx.simulate_input(" 2");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(
        f.look.read_with(cx, |l, _| l
            .data
            .group_names(None)
            .iter()
            .map(|n| n.to_string())
            .collect::<Vec<_>>()),
        ["Work 2"]
    );
    let g = || Target::Group(None, "Work 2".into());
    run(&f, cx, g(), Cmd::ColorGroup);
    click(cx, "prompt-choice:teal");
    assert!(f.look.read_with(cx, |l, _| l.data.groups[0].hue).is_some());
    run(&f, cx, g(), Cmd::RemoveGroup);
    assert!(f.look.read_with(cx, |l, _| l.data.groups.is_empty()));
    assert!(
        f.look
            .read_with(cx, |l, _| l.data.group_of(None, "a").is_none())
    );
}

#[gpui::test]
fn the_tab_group_menu_runs_each_entry(cx: &mut TestAppContext) {
    let dir = folder("menu-tabgroup");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    four_tabs(&f, cx);
    f.core.update(cx, |s, _| {
        for t in s.terminals.values_mut() {
            t.running = false;
        }
    });
    make_group(&f, "t2-a", "Build", cx);
    let id = super::tests::groups(&f, cx)[0].id;
    let g = || Target::TabGroup(id);
    assert_eq!(
        entries(&f, cx, &g()),
        [
            Cmd::TabGroupRename,
            Cmd::TabGroupColor,
            Cmd::Ungroup,
            Cmd::CloseGroup
        ]
    );
    run(&f, cx, g(), Cmd::TabGroupRename);
    cx.simulate_input("!");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(super::tests::groups(&f, cx)[0].name, "Build!");
    run(&f, cx, g(), Cmd::TabGroupColor);
    click(cx, "prompt-choice:sky");
    assert!(super::tests::groups(&f, cx)[0].hue.is_some());
    run(&f, cx, g(), Cmd::Ungroup);
    assert!(super::tests::groups(&f, cx).is_empty());
    make_group(&f, "t2-a", "Again", cx);
    let id = super::tests::groups(&f, cx)[0].id;
    run(&f, cx, Target::TabGroup(id), Cmd::CloseGroup);
    assert_eq!(order(&f, cx), ["t1-a", "t3-b", "screen:minutes"]);
}

#[gpui::test]
fn a_right_click_opens_the_menu_of_a_row_a_tab_and_a_chip_and_a_pick_runs_the_entry(
    cx: &mut TestAppContext,
) {
    let dir = folder("menu-click");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    four_tabs(&f, cx);
    let right_click = |cx: &mut VisualTestContext, name: &str| {
        let at = at(cx, name)
            .unwrap_or_else(|| panic!("{name} is drawn"))
            .center();
        cx.simulate_mouse_down(at, MouseButton::Right, Modifiers::none());
        cx.simulate_mouse_up(at, MouseButton::Right, Modifiers::none());
        redraw(cx);
    };
    let open =
        |f: &Fixture, cx: &mut VisualTestContext| f.shell.read_with(cx, |s, _| s.menu.is_some());
    assert!(!open(&f, cx));
    for name in ["tree-row:product-a", "tree-row:product-a-pane-t1-a"] {
        right_click(cx, name);
        assert!(open(&f, cx), "{name}");
        cx.simulate_keystrokes("escape");
        redraw(cx);
        assert!(!open(&f, cx), "escape closes the menu of {name}");
    }
    // A tab of a terminal opens the pane menu.
    let tab = tab_of(&f, "t1-a", cx);
    right_click(cx, &tab);
    assert!(open(&f, cx), "the tab of a pane has a menu");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    // The chip of a tab group opens the group menu.
    make_group(&f, "t2-a", "Build", cx);
    right_click(cx, "tab-chip:group-0");
    assert!(open(&f, cx), "the chip has a menu");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    // The story screen and the meeting screen open the menu of their object.
    click(cx, "tree-row:product-a-story-S1");
    right_click(cx, "story-screen");
    assert!(open(&f, cx), "the story screen has a menu");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    click(cx, "tree-row:product-a-meeting");
    right_click(cx, "meeting-screen");
    assert!(open(&f, cx), "the meeting screen has a menu");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    // A pick runs: the single entry of the reflection row opens the memory screen.
    right_click(cx, "tree-row:product-a-reflection");
    assert!(open(&f, cx));
    cx.simulate_keystrokes("down enter");
    redraw(cx);
    assert!(!open(&f, cx), "a pick closes the menu");
    assert_eq!(active_tab(&f, cx), "screen:memory");
}

#[gpui::test]
fn shift_f10_and_the_menu_key_open_the_menu_of_the_row_or_the_pane(cx: &mut TestAppContext) {
    let dir = folder("menu-key");
    let (f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    with_tabs(&f, cx);
    let open =
        |f: &Fixture, cx: &mut VisualTestContext| f.shell.read_with(cx, |s, _| s.menu.is_some());
    // With the focus on a pane the key opens the menu of that pane.
    super::tests::use_pane(&f, "t2-a", cx);
    cx.simulate_keystrokes("shift-f10");
    redraw(cx);
    assert!(open(&f, cx));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    cx.simulate_keystrokes("menu");
    redraw(cx);
    assert!(open(&f, cx), "the Menu key does the same");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    // With the cursor of the sidebar on a row the key opens the menu of that row.
    f.shell.update_in(cx, |s, window, cx| {
        let sidebar = s.sidebar.clone();
        sidebar.update(cx, |sb, cx| sb.take_focus(window, cx));
    });
    cx.simulate_keystrokes("down");
    redraw(cx);
    cx.simulate_keystrokes("shift-f10");
    redraw(cx);
    assert!(open(&f, cx), "the menu of the row under the cursor");
    cx.simulate_keystrokes("down enter");
    redraw(cx);
    assert_eq!(active_tab(&f, cx), "terminal:t2-a");
}

#[gpui::test]
fn the_ticket_menu_runs_its_entries_and_the_state_decides_which_are_on(cx: &mut TestAppContext) {
    use agentsi_proto::ticket_api::TicketState;
    let dir = folder("menu-ticket");
    let (mut f, cx) = fixture(cx, Persist::None, &dir.join("look.toml"));
    let ticket = |state| Target::Ticket(product("a"), "C001".into(), state);
    let _ = sent(&mut f);
    assert_eq!(
        entries(&f, cx, &ticket(TicketState::Open)),
        [
            Cmd::Open,
            Cmd::TicketDone,
            Cmd::TicketWait,
            Cmd::TicketDrop,
            Cmd::PromoteStory,
            Cmd::PromoteDecision,
            Cmd::CopyId,
        ]
    );
    assert_eq!(
        entries(&f, cx, &ticket(TicketState::Waiting)),
        [
            Cmd::Open,
            Cmd::TicketDone,
            Cmd::TicketDrop,
            Cmd::TicketReopen,
            Cmd::PromoteStory,
            Cmd::PromoteDecision,
            Cmd::CopyId,
        ]
    );
    assert_eq!(
        entries(&f, cx, &ticket(TicketState::Dropped)),
        [Cmd::Open, Cmd::TicketReopen, Cmd::CopyId]
    );

    run(&f, cx, ticket(TicketState::Open), Cmd::Open);
    assert_eq!(active_tab(&f, cx), "screen:ticket:C001");
    run(&f, cx, ticket(TicketState::Open), Cmd::CopyId);
    let copied = cx.read_from_clipboard().and_then(|c| c.text());
    assert_eq!(copied.as_deref(), Some("C001"));
    let _ = sent(&mut f);

    run(&f, cx, ticket(TicketState::Open), Cmd::TicketDone);
    let s = sent(&mut f);
    let moved = call(&s, "ticket.move").expect("the ticket moves");
    assert_eq!(moved["params"]["state"], "done");
    assert_eq!(moved["params"]["by"]["by"], "owner");
    run(&f, cx, ticket(TicketState::Dropped), Cmd::TicketReopen);
    assert_eq!(
        call(&sent(&mut f), "ticket.move").unwrap()["params"]["state"],
        "open"
    );
    run(&f, cx, ticket(TicketState::Open), Cmd::PromoteDecision);
    let s = sent(&mut f);
    assert_eq!(
        call(&s, "ticket.promote").unwrap()["params"]["to"],
        "decision"
    );

    // Wait asks for its target first.
    run(&f, cx, ticket(TicketState::Open), Cmd::TicketWait);
    assert!(prompt_open(cx));
    assert!(call(&sent(&mut f), "ticket.move").is_none());
    cx.simulate_input("Matias");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    let s = sent(&mut f);
    let wait = call(&s, "ticket.move").expect("the ticket waits");
    assert_eq!(
        (
            wait["params"]["state"].as_str(),
            wait["params"]["waiting_on"].as_str()
        ),
        (Some("waiting"), Some("Matias"))
    );

    // Promote to story spends money: one Confirm, and nothing before it.
    run(&f, cx, ticket(TicketState::Open), Cmd::PromoteStory);
    assert!(prompt_open(cx));
    assert!(call(&sent(&mut f), "ticket.promote").is_none());
    click(cx, "prompt-choice:no");
    assert!(call(&sent(&mut f), "ticket.promote").is_none());
    run(&f, cx, ticket(TicketState::Open), Cmd::PromoteStory);
    click(cx, "prompt-choice:yes");
    let s = sent(&mut f);
    assert_eq!(call(&s, "ticket.promote").unwrap()["params"]["to"], "story");

    // The palette entries run the same code on the active product and on the ticket in view.
    cx.dispatch_action(crate::actions::NewTicket);
    redraw(cx);
    assert!(prompt_open(cx));
    leave_prompt(cx);
    cx.simulate_keystrokes("ctrl-shift-i");
    redraw(cx);
    assert_eq!(active_tab(&f, cx), "screen:tickets");
    // The new Tickets screen has no ticket yet, so a ticket entry says so and sends nothing.
    let _ = sent(&mut f);
    cx.dispatch_action(crate::actions::TicketDone);
    redraw(cx);
    assert!(call(&sent(&mut f), "ticket.move").is_none());
    assert!(f.core.read_with(cx, |c, _| {
        c.notices.iter().any(|n| n.contains("No ticket is in view"))
    }));
}
