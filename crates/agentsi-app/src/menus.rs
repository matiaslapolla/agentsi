//! The context menus: what each object offers, and the popup that shows it.
//!
//! One module holds the menu of each object, so that a tab and an item row show the same pane
//! menu. `items` is a plain function of the target and of some facts about it, so a test can
//! read each menu without a window. The shell runs a chosen entry (`shell/menu_actions.rs`).
//!
//! Each entry has a palette entry. `palette` names it. The palette entry acts on the active
//! product, the focused pane, the story screen in view, or the group of the active product,
//! and then runs the same code as the menu entry.
//!
//! A row, a tab or a screen asks for a menu with `request`. The shell draws the popup, so no
//! view keeps menu state, and an idle window draws nothing for a closed menu.

use std::rc::Rc;

use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::*;

use agentsi_proto::ticket_api::TicketState;

use crate::model::ProductRef;
use crate::shell::PaneRef;

/// The object that a menu belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Product(ProductRef),
    /// A terminal: its item row and, for the tab bar, its tab.
    Pane(PaneRef),
    /// The meeting of a product.
    Meeting(ProductRef),
    /// The reflection of a product.
    Reflection(ProductRef),
    /// A story with a session, by story id.
    Story(ProductRef, String),
    /// A product group: its host and its name.
    Group(Option<String>, String),
    /// A tab group of the tab bar, by id.
    TabGroup(u32),
    /// A ticket, by id, with its state when the menu opened.
    Ticket(ProductRef, String, TicketState),
}

/// The kind of a target, to find the target of a palette entry.
#[cfg_attr(not(test), allow(dead_code))] // the tests check `palette` for each kind
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Product,
    Pane,
    Meeting,
    Reflection,
    Story,
    Group,
    TabGroup,
    Ticket,
}

impl Target {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn kind(&self) -> Kind {
        match self {
            Target::Product(_) => Kind::Product,
            Target::Pane(_) => Kind::Pane,
            Target::Meeting(_) => Kind::Meeting,
            Target::Reflection(_) => Kind::Reflection,
            Target::Story(..) => Kind::Story,
            Target::Group(..) => Kind::Group,
            Target::TabGroup(_) => Kind::TabGroup,
            Target::Ticket(..) => Kind::Ticket,
        }
    }
}

/// One thing that a menu entry does. The target tells on what.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmd {
    // A product.
    NewTerminal,
    OpenDesk,
    StartMeeting,
    NewStory,
    Suggest,
    NewTicket,
    OpenTickets,
    Reflect,
    CancelReflect,
    OpenGit,
    Label,
    Color,
    MoveToGroup,
    CopyPath,
    OpenFolder,
    DeleteData,
    Archive,
    // A pane.
    Rename,
    ColorPane,
    TalkToPane,
    VoiceMode,
    SplitRight,
    SplitDown,
    CopyFolder,
    ClosePane,
    // The tab of a pane.
    NewTabGroup,
    AddToTabGroup,
    LeaveTabGroup,
    CloseOthers,
    CloseToRight,
    // A meeting, a reflection and a story session.
    Open,
    CancelMeeting,
    Review,
    Accept,
    Reject,
    StoryTerminal,
    ArchiveStory,
    RestoreStory,
    RecordMerge,
    // A ticket.
    TicketDone,
    TicketWait,
    TicketDrop,
    TicketReopen,
    PromoteStory,
    PromoteDecision,
    CopyId,
    // A product group.
    RenameGroup,
    ColorGroup,
    RemoveGroup,
    // A tab group.
    TabGroupRename,
    TabGroupColor,
    Ungroup,
    CloseGroup,
}

/// What decides which entries are on: facts about the target that the shell reads from the core.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Facts {
    /// A program runs in the pane.
    pub running: bool,
    /// Voice mode of the pane is on.
    pub voice: bool,
    /// The product or the pane has a folder.
    pub folder: bool,
    /// The folder is on this device.
    pub local: bool,
    /// A meeting runs on the product.
    pub meeting: bool,
    /// A reflection of this window runs on the product.
    pub reflecting: bool,
    /// The story has a session.
    pub session: bool,
    /// The session is ready to accept.
    pub ready: bool,
    /// The pane has a tab in the dock.
    pub tabbed: bool,
    /// The tab of the pane is in a tab group.
    pub grouped: bool,
    /// The state of the ticket.
    pub ticket: Option<TicketState>,
}

impl Facts {
    /// Every entry on, for the tests.
    #[cfg(test)]
    pub fn all() -> Facts {
        Facts {
            running: true,
            voice: false,
            folder: true,
            local: true,
            meeting: true,
            reflecting: false,
            session: true,
            ready: true,
            tabbed: true,
            grouped: true,
            ticket: Some(TicketState::Open),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub cmd: Cmd,
    pub label: &'static str,
    pub enabled: bool,
    pub checked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Entry(Entry),
    Separator,
}

fn on(cmd: Cmd, label: &'static str) -> Item {
    Item::Entry(Entry {
        cmd,
        label,
        enabled: true,
        checked: false,
    })
}

fn when(cmd: Cmd, label: &'static str, enabled: bool) -> Item {
    Item::Entry(Entry {
        cmd,
        label,
        enabled,
        checked: false,
    })
}

/// The entries of the menu of `target`.
pub fn items(target: &Target, f: &Facts) -> Vec<Item> {
    use Cmd::*;
    match target {
        Target::Product(_) => vec![
            on(NewTerminal, "New terminal"),
            on(OpenDesk, "Open desk"),
            when(StartMeeting, "Start meeting", !f.meeting),
            on(NewStory, "New story"),
            on(Suggest, "Suggest stories"),
            on(NewTicket, "New ticket"),
            on(OpenTickets, "Open tickets"),
            when(Reflect, "Reflect", !f.reflecting),
            when(CancelReflect, "Cancel the reflection", f.reflecting),
            on(OpenGit, "Open the git panel"),
            Item::Separator,
            on(Label, "Label"),
            on(Color, "Color"),
            on(MoveToGroup, "Move to group"),
            Item::Separator,
            when(CopyPath, "Copy path", f.folder),
            when(OpenFolder, "Open folder", f.folder && f.local),
            Item::Separator,
            when(DeleteData, "Delete product data", f.local),
            on(Archive, "Archive"),
        ],
        Target::Pane(_) => {
            let mut v = vec![
                on(Rename, "Rename"),
                on(ColorPane, "Color"),
                Item::Separator,
                when(NewTabGroup, "New tab group", f.tabbed),
                when(AddToTabGroup, "Add to tab group", f.tabbed),
            ];
            if f.grouped {
                v.push(when(LeaveTabGroup, "Remove from tab group", f.tabbed));
            }
            v.extend([
                Item::Separator,
                when(TalkToPane, "Talk to this pane", f.running),
                Item::Entry(Entry {
                    cmd: VoiceMode,
                    label: "Voice mode",
                    enabled: f.running,
                    checked: f.voice,
                }),
                Item::Separator,
                on(SplitRight, "Split right"),
                on(SplitDown, "Split down"),
                when(CopyFolder, "Copy folder", f.folder),
                Item::Separator,
                on(ClosePane, "Close"),
                when(CloseOthers, "Close others", f.tabbed),
                when(CloseToRight, "Close to the right", f.tabbed),
            ]);
            v
        }
        Target::Story(..) => vec![
            on(Open, "Open"),
            when(Review, "Review", f.session),
            when(Accept, "Accept", f.ready),
            when(Reject, "Reject", f.session),
            Item::Separator,
            when(StoryTerminal, "Open a terminal in the worktree", f.session),
            Item::Separator,
            on(ArchiveStory, "Archive"),
            on(RestoreStory, "Restore"),
            on(RecordMerge, "Record a merge"),
        ],
        Target::Meeting(_) => vec![on(Open, "Open"), when(CancelMeeting, "Cancel", f.meeting)],
        Target::Reflection(_) => vec![on(Open, "Open")],
        Target::Group(..) => vec![
            on(RenameGroup, "Rename"),
            on(ColorGroup, "Color"),
            on(RemoveGroup, "Remove group"),
        ],
        Target::Ticket(..) => {
            let open = |act| {
                f.ticket
                    .is_some_and(|s| crate::screens::ticket::allowed(s, act))
            };
            use crate::screens::ticket::Act;
            vec![
                on(Open, "Open"),
                Item::Separator,
                when(TicketDone, "Done", open(Act::Done)),
                when(TicketWait, "Wait", open(Act::Wait)),
                when(TicketDrop, "Drop", open(Act::Drop)),
                when(TicketReopen, "Reopen", open(Act::Reopen)),
                Item::Separator,
                when(PromoteStory, "Promote to story", open(Act::PromoteStory)),
                when(
                    PromoteDecision,
                    "Promote to decision",
                    open(Act::PromoteDecision),
                ),
                Item::Separator,
                on(CopyId, "Copy id"),
            ]
        }
        Target::TabGroup(_) => vec![
            on(TabGroupRename, "Rename"),
            on(TabGroupColor, "Color"),
            on(Ungroup, "Ungroup"),
            on(CloseGroup, "Close the group"),
        ],
    }
}

/// The label of the palette entry that does what `cmd` does on a target of `kind`. The tests
/// check that each menu entry has one.
#[cfg_attr(not(test), allow(dead_code))]
pub fn palette(kind: Kind, cmd: Cmd) -> &'static str {
    use Cmd::*;
    match (kind, cmd) {
        (_, NewTerminal) => "New terminal in the active product",
        (_, OpenDesk) => "Open the front desk of the active product",
        (_, StartMeeting) => "Start the meeting in the question field (meeting)",
        (_, NewStory) => "New story in the active product",
        (_, Suggest) => "Suggest stories for the active product",
        (_, ArchiveStory) => "Archive the story",
        (_, RestoreStory) => "Restore the story",
        (_, RecordMerge) => "Record a merge of the story",
        (_, NewTicket) => "New ticket for the active product",
        (_, OpenTickets) => "Open the tickets of the active product",
        (_, Reflect) => "Reflect on the active product",
        (_, CancelReflect) => "Cancel the reflection of the active product",
        (_, OpenGit) => "Open the git panel of the active product",
        (_, Label) => "Label the active product",
        (_, Color) => "Color the active product",
        (_, MoveToGroup) => "Move the active product to a group",
        (_, CopyPath) => "Copy the folder of the active product",
        (_, OpenFolder) => "Open the folder of the active product",
        (_, DeleteData) => "Delete the data of the active product",
        (_, Archive) => "Archive the active product",
        (_, Rename) => "Rename the active pane",
        (_, ColorPane) => "Color the active pane",
        (_, TalkToPane) => "Talk to this pane",
        (_, VoiceMode) => "Turn voice mode of the focused pane on or off",
        (_, SplitRight) => "Split the focused pane to the right",
        (_, SplitDown) => "Split the focused pane down",
        (_, CopyFolder) => "Copy the folder of the focused pane",
        (_, ClosePane) => "Close the focused pane",
        (_, NewTabGroup) => "New tab group with the active tab",
        (_, AddToTabGroup) => "Add the active tab to a tab group",
        (_, LeaveTabGroup) => "Remove the active tab from its tab group",
        (_, CloseOthers) => "Close the other tabs of the bar",
        (_, CloseToRight) => "Close the tabs to the right of the active tab",
        (_, TabGroupRename) => "Rename the tab group of the active tab",
        (_, TabGroupColor) => "Color the tab group of the active tab",
        (_, Ungroup) => "Ungroup the tab group of the active tab",
        (_, CloseGroup) => "Close the tab group of the active tab",
        (Kind::Ticket, Open) => "Open the ticket under the cursor (tickets)",
        (_, TicketDone) => "Mark the ticket done (tickets)",
        (_, TicketWait) => "Mark the ticket waiting (tickets)",
        (_, TicketDrop) => "Drop the ticket (tickets)",
        (_, TicketReopen) => "Reopen the ticket (tickets)",
        (_, PromoteStory) => "Promote the ticket to a story (tickets)",
        (_, PromoteDecision) => "Promote the ticket to a decision (tickets)",
        (_, CopyId) => "Copy the id of the ticket (tickets)",
        (Kind::Meeting, Open) => "Open the meeting of the active product",
        (Kind::Reflection, Open) => "Open the memory of the active product",
        (_, Open) => "Open the story session",
        (_, CancelMeeting) => "Cancel the meeting of the active product",
        (_, Review) => "Review the story session",
        (_, Accept) => "Accept the story session",
        (_, Reject) => "Reject the story session",
        (_, StoryTerminal) => "Open a terminal in the worktree of the story session",
        (_, RenameGroup) => "Rename the product group of the active product",
        (_, ColorGroup) => "Color the product group of the active product",
        (_, RemoveGroup) => "Remove the product group of the active product",
    }
}

/// A command that asks one Confirm before it runs: it stops a program, spends money, or hides
/// a product. Accept and Reject of a story keep the Confirm of the story screen.
pub fn confirms(cmd: Cmd, f: &Facts) -> bool {
    match cmd {
        Cmd::Archive | Cmd::Reflect | Cmd::Suggest | Cmd::CancelMeeting | Cmd::PromoteStory => true,
        // A pane with a tab closes through the tab, which has its own Confirm.
        Cmd::ClosePane => f.running && !f.tabbed,
        _ => false,
    }
}

/// The title and the verb of the Confirm of `cmd`.
pub fn confirm_text(cmd: Cmd, name: &str) -> (String, &'static str) {
    match cmd {
        Cmd::Archive => (
            format!("Archive {name}? It leaves the lists and the counts. Nothing is deleted."),
            "Archive",
        ),
        Cmd::Reflect => (
            format!("Reflect on {name}? Each agent calls the model, which costs money."),
            "Reflect",
        ),
        Cmd::Suggest => (
            format!("Suggest stories for {name}? The PM calls the model, which costs money."),
            "Suggest",
        ),
        Cmd::CancelMeeting => (
            format!("Cancel the meeting of {name}? The roles stop."),
            "Cancel the meeting",
        ),
        Cmd::PromoteStory => (
            format!("Promote {name} to a story? The PM calls the model, which costs money."),
            "Promote to story",
        ),
        _ => (
            format!("Close {name}? The program in it stops."),
            "Close the pane",
        ),
    }
}

/// A menu that a view asks for.
#[derive(Debug, Clone)]
pub struct MenuRequest {
    pub target: Target,
    /// Where the menu opens, in window coordinates.
    pub position: Point<Pixels>,
}

/// The entity that carries the requests to the shell.
pub struct MenuHub;

impl EventEmitter<MenuRequest> for MenuHub {}

struct MenuHubGlobal(Entity<MenuHub>);

impl Global for MenuHubGlobal {}

/// Makes the hub that the views ask. The shell calls it once.
pub fn install(cx: &mut App) -> Entity<MenuHub> {
    let hub = cx.new(|_| MenuHub);
    cx.set_global(MenuHubGlobal(hub.clone()));
    hub
}

/// Asks the shell for the menu of `target` at `position`. Without a shell nothing happens.
pub fn request(cx: &mut App, target: Target, position: Point<Pixels>) {
    if let Some(hub) = cx.try_global::<MenuHubGlobal>().map(|g| g.0.clone()) {
        hub.update(cx, |_, cx| cx.emit(MenuRequest { target, position }));
    }
}

/// The popup of `items`. A chosen entry calls `run`; `back` gets the focus when the popup closes.
pub fn popup(
    items: &[Item],
    run: impl Fn(Cmd, &mut Window, &mut App) + 'static,
    back: FocusHandle,
    window: &mut Window,
    cx: &mut App,
) -> Entity<PopupMenu> {
    let run = Rc::new(run);
    let items = items.to_vec();
    PopupMenu::build(window, cx, move |mut menu, _, _| {
        for item in items {
            menu = match item {
                Item::Separator => menu.separator(),
                Item::Entry(e) => {
                    let run = run.clone();
                    menu.item(
                        PopupMenuItem::new(e.label)
                            .disabled(!e.enabled)
                            .checked(e.checked)
                            .on_click(move |_, window, cx| run(e.cmd, window, cx)),
                    )
                }
            };
        }
        menu.action_context(back)
    })
}

#[cfg(test)]
mod tests {
    // No glob import: `super` brings `gpui::test` in as `test`, and `#[test]` then loops.
    use super::{Cmd, Entry, Facts, Item, Target, confirms, items, palette};
    use crate::actions::ENTRIES;
    use crate::model::ProductRef;
    use crate::shell::PaneRef;
    use agentsi_proto::ticket_api::TicketState;

    fn product() -> ProductRef {
        ProductRef {
            host: None,
            id: "a".into(),
        }
    }

    fn targets() -> Vec<Target> {
        vec![
            Target::Product(product()),
            Target::Pane(PaneRef {
                host: None,
                id: "t1-a".into(),
            }),
            Target::Meeting(product()),
            Target::Reflection(product()),
            Target::Story(product(), "S1".into()),
            Target::Group(None, "Work".into()),
            Target::TabGroup(1),
            Target::Ticket(product(), "C001".into(), TicketState::Open),
        ]
    }

    fn labels(target: &Target) -> Vec<&'static str> {
        items(target, &Facts::all())
            .into_iter()
            .filter_map(|i| match i {
                Item::Entry(e) => Some(e.label),
                Item::Separator => None,
            })
            .collect()
    }

    #[test]
    fn each_menu_lists_the_entries_of_the_table_of_the_spec() {
        let t = targets();
        assert_eq!(
            labels(&t[0]),
            [
                "New terminal",
                "Open desk",
                "Start meeting",
                "New story",
                "Suggest stories",
                "New ticket",
                "Open tickets",
                "Reflect",
                "Cancel the reflection",
                "Open the git panel",
                "Label",
                "Color",
                "Move to group",
                "Copy path",
                "Open folder",
                "Delete product data",
                "Archive",
            ]
        );
        assert_eq!(
            labels(&t[1]),
            [
                "Rename",
                "Color",
                "New tab group",
                "Add to tab group",
                "Remove from tab group",
                "Talk to this pane",
                "Voice mode",
                "Split right",
                "Split down",
                "Copy folder",
                "Close",
                "Close others",
                "Close to the right",
            ]
        );
        assert_eq!(
            labels(&t[4]),
            [
                "Open",
                "Review",
                "Accept",
                "Reject",
                "Open a terminal in the worktree",
                "Archive",
                "Restore",
                "Record a merge",
            ]
        );
        assert_eq!(labels(&t[2]), ["Open", "Cancel"]);
        assert_eq!(labels(&t[5]), ["Rename", "Color", "Remove group"]);
        assert_eq!(
            labels(&t[6]),
            ["Rename", "Color", "Ungroup", "Close the group"]
        );
        assert_eq!(
            labels(&t[7]),
            [
                "Open",
                "Done",
                "Wait",
                "Drop",
                "Reopen",
                "Promote to story",
                "Promote to decision",
                "Copy id",
            ]
        );
    }

    #[test]
    fn each_entry_has_a_palette_entry() {
        for target in targets() {
            for item in items(&target, &Facts::all()) {
                let Item::Entry(e) = item else { continue };
                let label = palette(target.kind(), e.cmd);
                assert!(
                    ENTRIES.iter().any(|p| p.label == label),
                    "{:?} of {:?} has no palette entry {label:?}",
                    e.cmd,
                    target.kind()
                );
            }
        }
    }

    #[test]
    fn an_entry_that_cannot_run_is_off_and_the_voice_mode_shows_its_state() {
        let off = Facts::default();
        let enabled = |t: &Target, f: &Facts| -> Vec<(Cmd, bool)> {
            items(t, f)
                .into_iter()
                .filter_map(|i| match i {
                    Item::Entry(e) => Some((e.cmd, e.enabled)),
                    Item::Separator => None,
                })
                .collect()
        };
        let t = targets();
        let product_off = enabled(&t[0], &off);
        assert!(product_off.contains(&(Cmd::CopyPath, false)));
        assert!(product_off.contains(&(Cmd::OpenFolder, false)));
        assert!(product_off.contains(&(Cmd::StartMeeting, true)));
        // A meeting that runs cannot start again, and a reflection too.
        let busy = Facts {
            meeting: true,
            reflecting: true,
            ..Facts::default()
        };
        let product_busy = enabled(&t[0], &busy);
        assert!(product_busy.contains(&(Cmd::StartMeeting, false)));
        assert!(product_busy.contains(&(Cmd::Reflect, false)));
        // The folder of a host is not on this device: it copies, it does not open.
        let remote = Facts {
            folder: true,
            local: false,
            ..Facts::default()
        };
        let product_remote = enabled(&t[0], &remote);
        assert!(product_remote.contains(&(Cmd::CopyPath, true)));
        assert!(product_remote.contains(&(Cmd::OpenFolder, false)));
        let story_off = enabled(&t[4], &off);
        assert!(story_off.contains(&(Cmd::Open, true)));
        assert!(story_off.contains(&(Cmd::Accept, false)));
        assert!(story_off.contains(&(Cmd::Reject, false)));
        let voice = Facts {
            running: true,
            voice: true,
            ..Facts::default()
        };
        let checked = items(&t[1], &voice).into_iter().any(|i| {
            matches!(
                i,
                Item::Entry(Entry {
                    cmd: Cmd::VoiceMode,
                    checked: true,
                    enabled: true,
                    ..
                })
            )
        });
        assert!(checked);
    }

    #[test]
    fn only_a_destructive_command_asks_a_confirm_and_a_pane_asks_only_while_it_runs() {
        let idle = Facts::default();
        let running = Facts {
            running: true,
            ..Facts::default()
        };
        for cmd in [
            Cmd::Archive,
            Cmd::Reflect,
            Cmd::CancelMeeting,
            Cmd::PromoteStory,
        ] {
            assert!(confirms(cmd, &idle), "{cmd:?}");
        }
        assert!(confirms(Cmd::ClosePane, &running));
        assert!(!confirms(Cmd::ClosePane, &idle));
        // A pane with a tab closes through the Confirm of the tabs.
        let tabbed = Facts {
            tabbed: true,
            ..running
        };
        assert!(!confirms(Cmd::ClosePane, &tabbed));
        for cmd in [Cmd::Label, Cmd::Accept, Cmd::Reject, Cmd::RemoveGroup] {
            assert!(!confirms(cmd, &running), "{cmd:?}");
        }
    }
}
