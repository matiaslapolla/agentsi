//! Each action of the app, its key binding and its palette entry.
//!
//! The terminal pane stops each key down event, so a shortcut of the app must be
//! an action with a key binding: GPUI matches the bindings before the key listeners.
//! A new action gets a line in `key_table!` and an entry in `ENTRIES`. A binding never takes a
//! key of the pane (`Ctrl+Shift+C`, `Ctrl+Shift+V`) or a common key of a terminal program.
//! The owner changes each key in `keys.toml`: see `keys.rs`.

use gpui_kit::component::IconName;
use gpui_kit::{Action, App, Context};

use crate::screens::call::{CallScreen, Calls};
use crate::screens::desk::DeskScreen;
use crate::screens::minute::MinuteScreen;
use crate::screens::parts::Live;

gpui_kit::actions!(
    agentsi,
    [
        /// Opens or closes the command palette.
        TogglePalette,
        /// Opens a shell in the folder of the active product.
        NewTerminal,
        /// Opens `claude` in the folder of the active product.
        NewClaude,
        /// Opens the log of the alerts of this run.
        ShowAlerts,
        /// Opens the front desk of the active product.
        OpenDesk,
        /// Opens the minutes of the active product.
        OpenMinutes,
        /// Opens the roles of the active product.
        OpenRoles,
        /// Opens the stories of the active product.
        OpenStories,
        /// Opens the tickets of the active product.
        OpenTickets,
        /// Opens the git screen of the active product.
        OpenGit,
        /// Opens the setup screen for a new product, on the host of the active product.
        NewProduct,
        /// Opens the setup screen to update the brief of the active product.
        UpdateBrief,
        /// Opens the meeting of the active product.
        OpenMeeting,
        /// Starts the meeting in the question field of the active product.
        StartMeeting,
        /// Stops the meeting that runs on the active product.
        CancelMeeting,
        /// Opens the minutes of the last meeting of the active product.
        OpenMeetingMinutes,
        /// Opens the memory of the active product.
        OpenMemory,
        /// Asks to retire the item under the cursor of the memory of the active product.
        RetireMemoryItem,
        /// Asks to revert the newest reflection that can be reverted.
        RevertReflection,
        /// Approves the newest pending proposal of the memory of the active product.
        ApproveProposal,
        /// Rejects the newest pending proposal of the memory of the active product.
        RejectProposal,
        /// Opens the call of the open minutes, or of the last meeting.
        OpenCall,
        /// Makes the call of the open minutes with the voices of the core.
        MakeCall,
        /// Plays or pauses the call on the call screen.
        PlayCall,
        /// Marks the call as listened, or removes the mark.
        ToggleCallHeard,
        /// Renders the call of the call screen with the voices of the core.
        RenderCall,
        /// Stops the render of the call on the call screen.
        CancelCallRender,
        /// Moves the call to the previous segment.
        PreviousCallSegment,
        /// Moves the call to the next segment.
        NextCallSegment,
        /// Changes the speed of the call: 1, 1.25, 1.5 or 2.
        CycleCallSpeed,
        /// Reads the open screens from the core again.
        ReloadScreens,
        /// Turns voice mode of the focused pane on or off.
        ToggleVoice,
        /// Starts conversation mode with the focused pane, or stops it. The pill listens, sends
        /// each spoken turn to the pane and speaks the reply.
        TalkToPane,
        /// Opens the settings: voice models, voices, the vocabulary and the keys.
        OpenSettings,
        /// Opens the settings at the Data section, where the owner deletes data.
        OpenDataSettings,
        /// Opens the settings at the Owner section, where the owner edits the name and the language.
        OpenOwnerSettings,
        /// Opens the settings and checks the health of the tools.
        CheckHealth,
        /// Opens the settings and backs up the data now.
        BackUpData,
        /// Opens the guide of `docs/guide/` in the browser.
        OpenGuide,
        /// Opens the section "Install and start" of the guide in the browser.
        OpenGuideStart,
        /// Shows the context of the next meeting of the active product.
        ShowMeetingContext,
        /// Opens the meeting of the active product at the field "Ask one role".
        AskMeetingRole,
        /// Updates the knowledge index of the active product from the front desk.
        UpdateIndex,
        /// Opens the form "New decision" on the front desk of the active product.
        NewDecision,
        /// Puts the focus in the input of the desk conversation of the front desk.
        AskDesk,
        /// Shows or hides the archived decisions of the front desk.
        ShowArchivedDecisions,
        /// Opens a Claude pane with the action of the decision approved last.
        RunApprovedInClaude,
        /// Approves the pending decision under the cursor of the front desk.
        ApproveDecision,
        /// Rejects the pending decision under the cursor of the front desk.
        RejectDecision,
        /// Archives the pending decision under the cursor of the front desk.
        ArchiveDecision,
        /// Marks the priority under the cursor of the open minutes accepted.
        AcceptPriority,
        /// Marks the priority under the cursor of the open minutes changed.
        ChangePriority,
        /// Marks the priority under the cursor of the open minutes rejected.
        RejectPriority,
        /// Flips the "new" flag of the priority under the cursor of the open minutes.
        ToggleNewPriority,
        /// Archives the open minutes.
        ArchiveMinutes,
        /// Brings the open minutes back from the archive.
        RestoreMinutes,
        /// Asks for a label for the active product.
        LabelProduct,
        /// Asks for a color for the active product.
        ColorProduct,
        /// Asks for the product group of the active product.
        MoveProductToGroup,
        /// Asks for a label for the focused pane.
        RenamePane,
        /// Asks for a color for the focused pane.
        ColorPane,
        /// Closes each tab of the bar of the active tab, except the active tab.
        CloseOtherTabs,
        /// Closes each tab of the bar that sits after the active tab.
        CloseTabsToTheRight,
        /// Asks for a name and puts the active tab in a new tab group.
        NewTabGroup,
        /// Asks for a tab group of the bar and puts the active tab in it.
        AddTabToGroup,
        /// Takes the active tab out of its tab group.
        RemoveTabFromGroup,
        /// Asks for a new name for the tab group of the active tab.
        RenameTabGroup,
        /// Asks for a color for the tab group of the active tab.
        ColorTabGroup,
        /// Collapses the tab group of the active tab to its chip, or opens it.
        ToggleTabGroup,
        /// Dissolves the tab group of the active tab and keeps its tabs.
        UngroupTabs,
        /// Closes each tab of the tab group of the active tab.
        CloseTabGroup,
        /// Opens the context menu of the focused row of the sidebar, or of the focused pane.
        OpenContextMenu,
        /// Asks for the text of a new story of the active product.
        NewStory,
        /// The PM suggests stories for the active product, after one Confirm.
        SuggestStories,
        /// Archives the story of the screen in view.
        ArchiveStorySession,
        /// Restores the archived story of the screen in view.
        RestoreStorySession,
        /// Opens the form that records a merge of the story.
        RecordStoryMerge,
        /// Asks for the title of a new ticket of the active product.
        NewTicket,
        /// Opens the ticket under the cursor of the tickets, or the ticket in view.
        OpenTicket,
        /// Marks the ticket in view done.
        TicketDone,
        /// Asks what the ticket in view waits on.
        TicketWait,
        /// Drops the ticket in view.
        TicketDrop,
        /// Opens the ticket in view again.
        TicketReopen,
        /// Promotes the ticket in view to a story, after one Confirm.
        PromoteTicketStory,
        /// Promotes the ticket in view to a decision.
        PromoteTicketDecision,
        /// Copies the id of the ticket in view.
        CopyTicketId,
        /// Runs the reflection of the active product.
        ReflectProduct,
        /// Shows or hides the archived products in the sidebar.
        ToggleArchivedProducts,
        /// Stops the reflection of the active product.
        CancelReflection,
        /// Copies the folder of the active product to the clipboard.
        CopyProductPath,
        /// Opens the folder of the active product in the file manager.
        OpenProductFolder,
        /// Opens the Data section of the settings, set to the active product.
        DeleteProductData,
        /// Archives the active product.
        ArchiveProduct,
        /// Opens a new terminal in the folder of the focused pane, to its right.
        SplitPaneRight,
        /// Opens a new terminal in the folder of the focused pane, below it.
        SplitPaneDown,
        /// Copies the folder of the focused pane to the clipboard.
        CopyPaneFolder,
        /// Closes the focused pane. A running program asks one Confirm.
        ClosePane,
        /// Opens the story session that the owner used last.
        OpenStorySession,
        /// Opens the story session and reads its diff again.
        ReviewStory,
        /// Asks to merge the story session that the owner used last.
        AcceptStory,
        /// Asks to reject the story session that the owner used last.
        RejectStory,
        /// Opens a terminal in the worktree of the story session that the owner used last.
        OpenStoryTerminal,
        /// Asks for a new name for the product group of the active product.
        RenameGroup,
        /// Asks for a color for the product group of the active product.
        ColorGroup,
        /// Removes the product group of the active product and keeps its products.
        RemoveGroup,
        /// Closes the window. The core and its terminals keep running.
        Quit,
        SelectProduct1,
        SelectProduct2,
        SelectProduct3,
        SelectProduct4,
        SelectProduct5,
        SelectProduct6,
        SelectProduct7,
        SelectProduct8,
        SelectProduct9,
    ]
);

/// The product action for the sidebar row `n`, from 0.
pub fn select_product(n: usize) -> Option<Box<dyn Action>> {
    let a: Box<dyn Action> = match n {
        0 => Box::new(SelectProduct1),
        1 => Box::new(SelectProduct2),
        2 => Box::new(SelectProduct3),
        3 => Box::new(SelectProduct4),
        4 => Box::new(SelectProduct5),
        5 => Box::new(SelectProduct6),
        6 => Box::new(SelectProduct7),
        7 => Box::new(SelectProduct8),
        8 => Box::new(SelectProduct9),
        _ => return None,
    };
    Some(a)
}

/// The key id of the product action `n`, for the sidebar hint.
pub fn product_id(n: usize) -> Option<&'static str> {
    const IDS: [&str; 9] = [
        "product-1",
        "product-2",
        "product-3",
        "product-4",
        "product-5",
        "product-6",
        "product-7",
        "product-8",
        "product-9",
    ];
    IDS.get(n).copied()
}

/// Makes the action of a key binding or a palette entry.
pub type MakeAction = fn() -> Box<dyn Action>;

/// One action that has a key: the id in `keys.toml` and the label on the settings screen.
pub struct KeyAction {
    pub id: &'static str,
    pub label: &'static str,
}

/// Both tables come from one list, so the id of a row and its default key cannot drift apart.
/// The ids are the names in the `[app]` table of `keys.toml`: keep them stable.
macro_rules! key_table {
    ($(($id:expr, $label:expr, $keys:expr, $make:expr)),* $(,)?) => {
        /// Each key binding. `secondary` is `Cmd` on macOS and `Ctrl` elsewhere.
        pub const BINDINGS: &[(&str, MakeAction)] = &[$(($keys, $make)),*];

        /// The id and the label of each entry of `BINDINGS`, in the same order.
        pub const KEY_ACTIONS: &[KeyAction] = &[$(KeyAction { id: $id, label: $label }),*];
    };
}

key_table! {
    ("palette", "Command palette", "secondary-k", || Box::new(TogglePalette)),
    ("new-terminal", "New terminal", "secondary-t", || Box::new(NewTerminal)),
    ("new-claude", "New Claude session", "secondary-shift-t", || Box::new(NewClaude)),
    ("desk", "Front desk", "secondary-shift-d", || Box::new(OpenDesk)),
    ("voice", "Voice mode of the focused pane", "secondary-shift-m", || Box::new(ToggleVoice)),
    ("minutes", "Minutes", "secondary-shift-l", || Box::new(OpenMinutes)),
    ("roles", "Roles", "secondary-shift-r", || Box::new(OpenRoles)),
    ("stories", "Stories", "secondary-shift-s", || Box::new(OpenStories)),
    ("tickets", "Tickets", "secondary-shift-i", || Box::new(OpenTickets)),
    ("git", "Git", "secondary-shift-b", || Box::new(OpenGit)),
    ("meeting", "Meeting", "secondary-shift-g", || Box::new(OpenMeeting)),
    ("memory", "Memory", "secondary-shift-y", || Box::new(OpenMemory)),
    ("call", "Call", "secondary-shift-p", || Box::new(OpenCall)),
    ("settings", "Settings", "secondary-,", || Box::new(OpenSettings)),
    ("context-menu", "Context menu", "shift-f10", || Box::new(OpenContextMenu)),
    ("product-1", "Go to product 1", "secondary-1", || Box::new(SelectProduct1)),
    ("product-2", "Go to product 2", "secondary-2", || Box::new(SelectProduct2)),
    ("product-3", "Go to product 3", "secondary-3", || Box::new(SelectProduct3)),
    ("product-4", "Go to product 4", "secondary-4", || Box::new(SelectProduct4)),
    ("product-5", "Go to product 5", "secondary-5", || Box::new(SelectProduct5)),
    ("product-6", "Go to product 6", "secondary-6", || Box::new(SelectProduct6)),
    ("product-7", "Go to product 7", "secondary-7", || Box::new(SelectProduct7)),
    ("product-8", "Go to product 8", "secondary-8", || Box::new(SelectProduct8)),
    ("product-9", "Go to product 9", "secondary-9", || Box::new(SelectProduct9)),
}

/// The owner's writes act on the screen in view that the owner used last: the palette closes
/// with the focus on a pane, so the focus does not name a screen. Each handler is global, and
/// it runs when no element in the focus path takes the action.
fn on_writes(cx: &mut App) {
    use agentsi_proto::marks_api::{Mark, Verdict};
    fn desk(cx: &mut App, f: impl FnOnce(&mut DeskScreen, &mut Context<DeskScreen>)) {
        if let Some(d) = Live::desk(cx) {
            d.update(cx, |s, cx| f(s, cx));
        }
    }
    fn minute(cx: &mut App, f: impl FnOnce(&mut MinuteScreen, &mut Context<MinuteScreen>)) {
        if let Some(m) = Live::minute(cx) {
            m.update(cx, |s, cx| f(s, cx));
        }
    }
    cx.on_action(|_: &ApproveDecision, cx| {
        desk(cx, |s, cx| s.decide_selected(Verdict::Approved, cx))
    });
    cx.on_action(|_: &RejectDecision, cx| {
        desk(cx, |s, cx| s.decide_selected(Verdict::Rejected, cx))
    });
    cx.on_action(|_: &ArchiveDecision, cx| desk(cx, |s, cx| s.archive_selected(cx)));
    cx.on_action(|_: &UpdateIndex, cx| desk(cx, |s, cx| s.update_index(cx)));
    cx.on_action(|_: &NewDecision, cx| desk(cx, |s, cx| s.new_decision(cx)));
    cx.on_action(|_: &AskDesk, cx| desk(cx, |s, cx| s.ask(cx)));
    cx.on_action(|_: &ShowArchivedDecisions, cx| desk(cx, |s, cx| s.toggle_archived(cx)));
    cx.on_action(|_: &RunApprovedInClaude, cx| desk(cx, |s, cx| s.run_in_claude(cx)));
    cx.on_action(|_: &AcceptPriority, cx| minute(cx, |s, cx| s.mark_selected(Mark::Accepted, cx)));
    cx.on_action(|_: &ChangePriority, cx| minute(cx, |s, cx| s.mark_selected(Mark::Changed, cx)));
    cx.on_action(|_: &RejectPriority, cx| minute(cx, |s, cx| s.mark_selected(Mark::Rejected, cx)));
    cx.on_action(|_: &ToggleNewPriority, cx| minute(cx, |s, cx| s.toggle_new_selected(cx)));
    fn call(cx: &mut App, f: impl FnOnce(&mut CallScreen, &mut Context<CallScreen>)) {
        if let Some(c) = Calls::last(cx) {
            c.update(cx, |s, cx| f(s, cx));
        }
    }
    cx.on_action(|_: &PlayCall, cx| call(cx, |s, cx| s.toggle_play(cx)));
    cx.on_action(|_: &ToggleCallHeard, cx| call(cx, |s, cx| s.toggle_heard(cx)));
    cx.on_action(|_: &RenderCall, cx| call(cx, |s, cx| s.render_call(cx)));
    cx.on_action(|_: &CancelCallRender, cx| call(cx, |s, cx| s.cancel_render(cx)));
    cx.on_action(|_: &PreviousCallSegment, cx| call(cx, |s, cx| s.step(-1, cx)));
    cx.on_action(|_: &NextCallSegment, cx| call(cx, |s, cx| s.step(1, cx)));
    cx.on_action(|_: &CycleCallSpeed, cx| call(cx, |s, cx| s.cycle_speed(cx)));
    cx.on_action(|_: &MakeCall, cx| minute(cx, |s, cx| s.make_call(cx)));
    cx.on_action(|_: &ArchiveMinutes, cx| minute(cx, |s, cx| s.archive_minutes(true, cx)));
    cx.on_action(|_: &RestoreMinutes, cx| minute(cx, |s, cx| s.archive_minutes(false, cx)));
}

/// Registers the handlers of the actions and the default keys, with no key file.
#[cfg(test)]
pub fn bind(cx: &mut App) {
    bind_from(cx, None);
}

/// Registers the handlers and the keys in effect: the defaults and the key file at `path`.
pub fn bind_from(cx: &mut App, path: Option<std::path::PathBuf>) {
    on_writes(cx);
    crate::keys::install(cx, path);
}

/// One palette entry: a label, an icon, extra search words and the action.
pub struct Entry {
    pub label: &'static str,
    pub icon: IconName,
    pub keywords: &'static [&'static str],
    pub action: MakeAction,
}

pub const ENTRIES: &[Entry] = &[
    Entry {
        label: "New terminal in the active product",
        icon: IconName::SquareTerminal,
        keywords: &["shell", "pane", "tab"],
        action: || Box::new(NewTerminal),
    },
    Entry {
        label: "New Claude session in the active product",
        icon: IconName::Bot,
        keywords: &["claude", "agent"],
        action: || Box::new(NewClaude),
    },
    Entry {
        label: "Open the front desk of the active product",
        icon: IconName::LayoutDashboard,
        keywords: &["desk", "product"],
        action: || Box::new(OpenDesk),
    },
    Entry {
        label: "Open the minutes of the active product",
        icon: IconName::FileText,
        keywords: &["minutes", "meetings", "list"],
        action: || Box::new(OpenMinutes),
    },
    Entry {
        label: "Open the roles of the active product",
        icon: IconName::User,
        keywords: &[
            "roles",
            "roster",
            "overlay",
            "specialist",
            "reset",
            "validate",
            "apply",
        ],
        action: || Box::new(OpenRoles),
    },
    Entry {
        label: "Open the stories of the active product",
        icon: IconName::Check,
        keywords: &[
            "stories", "story", "task", "approve", "run", "review", "accept", "reject", "diff",
        ],
        action: || Box::new(OpenStories),
    },
    Entry {
        label: "Open the tickets of the active product",
        icon: IconName::Check,
        keywords: &[
            "tickets", "ticket", "note", "todo", "question", "pending", "waiting",
        ],
        action: || Box::new(OpenTickets),
    },
    Entry {
        label: "Open the git panel of the active product",
        icon: IconName::Github,
        keywords: &[
            "git", "branch", "commit", "stage", "unstage", "diff", "status", "history", "config",
        ],
        action: || Box::new(OpenGit),
    },
    Entry {
        label: "Set up a new product",
        icon: IconName::Plus,
        keywords: &["setup", "new", "product", "register", "add", "brief", "folder"],
        action: || Box::new(NewProduct),
    },
    Entry {
        label: "Update the brief of the active product",
        icon: IconName::FileText,
        keywords: &["setup", "brief", "roster", "objectives", "stories", "update", "apply"],
        action: || Box::new(UpdateBrief),
    },
    Entry {
        label: "Open the meeting of the active product",
        icon: IconName::User,
        keywords: &["meeting", "meet", "roles", "question", "live", "panes"],
        action: || Box::new(OpenMeeting),
    },
    Entry {
        label: "Open the memory of the active product",
        icon: IconName::BookOpen,
        keywords: &[
            "memory",
            "items",
            "lessons",
            "reflection",
            "reflect",
            "revert",
            "proposal",
        ],
        action: || Box::new(OpenMemory),
    },
    Entry {
        label: "Retire the item under the cursor (memory)",
        icon: IconName::CircleX,
        keywords: &["memory", "retire", "item", "lesson"],
        action: || Box::new(RetireMemoryItem),
    },
    Entry {
        label: "Revert the newest reflection (memory)",
        icon: IconName::RotateCw,
        keywords: &["memory", "reflection", "reflect", "revert", "undo"],
        action: || Box::new(RevertReflection),
    },
    Entry {
        label: "Approve the pending proposal (memory)",
        icon: IconName::Check,
        keywords: &["memory", "reflection", "proposal", "prompt", "approve"],
        action: || Box::new(ApproveProposal),
    },
    Entry {
        label: "Reject the pending proposal (memory)",
        icon: IconName::CircleX,
        keywords: &["memory", "reflection", "proposal", "prompt", "reject"],
        action: || Box::new(RejectProposal),
    },
    Entry {
        label: "Start the meeting in the question field (meeting)",
        icon: IconName::Check,
        keywords: &["meeting", "meet", "start", "question"],
        action: || Box::new(StartMeeting),
    },
    Entry {
        label: "Open the minutes of the last meeting",
        icon: IconName::FileText,
        keywords: &["meeting", "meet", "minutes", "result"],
        action: || Box::new(OpenMeetingMinutes),
    },
    Entry {
        label: "Cancel the meeting of the active product",
        icon: IconName::CircleX,
        keywords: &["meeting", "meet", "stop", "cancel"],
        action: || Box::new(CancelMeeting),
    },
    Entry {
        label: "Open the call of the open minutes",
        icon: IconName::Mic,
        keywords: &["call", "audio", "listen", "play", "voice"],
        action: || Box::new(OpenCall),
    },
    Entry {
        label: "Make the call of the open minutes (minutes)",
        icon: IconName::Mic,
        keywords: &["call", "make", "render", "audio", "voice"],
        action: || Box::new(MakeCall),
    },
    Entry {
        label: "Play or pause the call (call)",
        icon: IconName::Mic,
        keywords: &["call", "audio", "play", "pause", "listen"],
        action: || Box::new(PlayCall),
    },
    Entry {
        label: "Mark the call as listened, or remove the mark (call)",
        icon: IconName::Check,
        keywords: &["call", "listened", "heard", "mark"],
        action: || Box::new(ToggleCallHeard),
    },
    Entry {
        label: "Render the call with the voices (call)",
        icon: IconName::Mic,
        keywords: &["call", "render", "audio", "voice", "kokoro"],
        action: || Box::new(RenderCall),
    },
    Entry {
        label: "Stop the render of the call (call)",
        icon: IconName::CircleX,
        keywords: &["call", "render", "cancel", "stop"],
        action: || Box::new(CancelCallRender),
    },
    Entry {
        label: "Go to the previous segment of the call (call)",
        icon: IconName::Mic,
        keywords: &["call", "previous", "back", "segment"],
        action: || Box::new(PreviousCallSegment),
    },
    Entry {
        label: "Go to the next segment of the call (call)",
        icon: IconName::Mic,
        keywords: &["call", "next", "skip", "segment"],
        action: || Box::new(NextCallSegment),
    },
    Entry {
        label: "Change the speed of the call (call)",
        icon: IconName::Mic,
        keywords: &["call", "speed", "faster", "slower"],
        action: || Box::new(CycleCallSpeed),
    },
    Entry {
        label: "Reload the open screens from the core",
        icon: IconName::RefreshCw,
        keywords: &["refresh", "update", "desk"],
        action: || Box::new(ReloadScreens),
    },
    Entry {
        label: "Approve the pending decision under the cursor (front desk)",
        icon: IconName::Check,
        keywords: &["decide", "decision", "approve", "yes"],
        action: || Box::new(ApproveDecision),
    },
    Entry {
        label: "Reject the pending decision under the cursor (front desk)",
        icon: IconName::Close,
        keywords: &["decide", "decision", "reject", "no"],
        action: || Box::new(RejectDecision),
    },
    Entry {
        label: "Archive the pending decision under the cursor (front desk)",
        icon: IconName::FileText,
        keywords: &["decision", "archive", "hide"],
        action: || Box::new(ArchiveDecision),
    },
    Entry {
        label: "Mark the priority under the cursor accepted (minutes)",
        icon: IconName::Check,
        keywords: &["mark", "priority", "accept"],
        action: || Box::new(AcceptPriority),
    },
    Entry {
        label: "Mark the priority under the cursor changed (minutes)",
        icon: IconName::FileText,
        keywords: &["mark", "priority", "change"],
        action: || Box::new(ChangePriority),
    },
    Entry {
        label: "Mark the priority under the cursor rejected (minutes)",
        icon: IconName::Close,
        keywords: &["mark", "priority", "reject"],
        action: || Box::new(RejectPriority),
    },
    Entry {
        label: "Flag the priority under the cursor as new, or clear the flag (minutes)",
        icon: IconName::FileText,
        keywords: &["mark", "priority", "new"],
        action: || Box::new(ToggleNewPriority),
    },
    Entry {
        label: "Archive the open minutes",
        icon: IconName::FileText,
        keywords: &["minutes", "archive", "hide"],
        action: || Box::new(ArchiveMinutes),
    },
    Entry {
        label: "Restore the open minutes from the archive",
        icon: IconName::FileText,
        keywords: &["minutes", "restore", "archive"],
        action: || Box::new(RestoreMinutes),
    },
    Entry {
        label: "Label the active product",
        icon: IconName::Replace,
        keywords: &["label", "name", "tag", "rename", "product", "sidebar"],
        action: || Box::new(LabelProduct),
    },
    Entry {
        label: "Color the active product",
        icon: IconName::Palette,
        keywords: &["color", "colour", "bar", "product", "sidebar"],
        action: || Box::new(ColorProduct),
    },
    Entry {
        label: "Move the active product to a group",
        icon: IconName::Folder,
        keywords: &["group", "folder", "product", "sidebar", "ungroup"],
        action: || Box::new(MoveProductToGroup),
    },
    Entry {
        label: "Rename the active pane",
        icon: IconName::Replace,
        keywords: &["label", "name", "tag", "pane", "tab", "terminal"],
        action: || Box::new(RenamePane),
    },
    Entry {
        label: "Color the active pane",
        icon: IconName::Palette,
        keywords: &["color", "colour", "pane", "tab", "terminal"],
        action: || Box::new(ColorPane),
    },
    Entry {
        label: "Close the other tabs of the bar",
        icon: IconName::Close,
        keywords: &["close", "others", "tabs", "pane", "bar"],
        action: || Box::new(CloseOtherTabs),
    },
    Entry {
        label: "Close the tabs to the right of the active tab",
        icon: IconName::Close,
        keywords: &["close", "right", "tabs", "after", "pane"],
        action: || Box::new(CloseTabsToTheRight),
    },
    Entry {
        label: "New tab group with the active tab",
        icon: IconName::Folder,
        keywords: &["tab", "group", "new", "chip", "name"],
        action: || Box::new(NewTabGroup),
    },
    Entry {
        label: "Add the active tab to a tab group",
        icon: IconName::Folder,
        keywords: &["tab", "group", "add", "move", "chip"],
        action: || Box::new(AddTabToGroup),
    },
    Entry {
        label: "Remove the active tab from its tab group",
        icon: IconName::Folder,
        keywords: &["tab", "group", "remove", "leave", "out"],
        action: || Box::new(RemoveTabFromGroup),
    },
    Entry {
        label: "Rename the tab group of the active tab",
        icon: IconName::Replace,
        keywords: &["tab", "group", "rename", "name", "chip"],
        action: || Box::new(RenameTabGroup),
    },
    Entry {
        label: "Color the tab group of the active tab",
        icon: IconName::Palette,
        keywords: &["tab", "group", "color", "colour", "chip"],
        action: || Box::new(ColorTabGroup),
    },
    Entry {
        label: "Collapse or open the tab group of the active tab",
        icon: IconName::ChevronsUpDown,
        keywords: &["tab", "group", "collapse", "open", "expand", "chip"],
        action: || Box::new(ToggleTabGroup),
    },
    Entry {
        label: "Ungroup the tab group of the active tab",
        icon: IconName::Folder,
        keywords: &["tab", "group", "ungroup", "dissolve", "chip"],
        action: || Box::new(UngroupTabs),
    },
    Entry {
        label: "Close the tab group of the active tab",
        icon: IconName::Close,
        keywords: &["tab", "group", "close", "chip"],
        action: || Box::new(CloseTabGroup),
    },
    Entry {
        label: "Suggest stories for the active product",
        icon: IconName::Bot,
        keywords: &["story", "suggest", "pm", "propose"],
        action: || Box::new(SuggestStories),
    },
    Entry {
        label: "Archive the story",
        icon: IconName::Folder,
        keywords: &["story", "archive"],
        action: || Box::new(ArchiveStorySession),
    },
    Entry {
        label: "Restore the story",
        icon: IconName::Folder,
        keywords: &["story", "restore", "archive"],
        action: || Box::new(RestoreStorySession),
    },
    Entry {
        label: "Record a merge of the story",
        icon: IconName::Check,
        keywords: &["story", "merge", "commit", "result"],
        action: || Box::new(RecordStoryMerge),
    },
    Entry {
        label: "New story in the active product",
        icon: IconName::Check,
        keywords: &["story", "task", "pm", "new", "propose"],
        action: || Box::new(NewStory),
    },
    Entry {
        label: "New ticket for the active product",
        icon: IconName::Plus,
        keywords: &["ticket", "note", "todo", "question", "new", "add"],
        action: || Box::new(NewTicket),
    },
    Entry {
        label: "Open the ticket under the cursor (tickets)",
        icon: IconName::Check,
        keywords: &["ticket", "open", "cursor"],
        action: || Box::new(OpenTicket),
    },
    Entry {
        label: "Mark the ticket done (tickets)",
        icon: IconName::Check,
        keywords: &["ticket", "done", "close", "finish"],
        action: || Box::new(TicketDone),
    },
    Entry {
        label: "Mark the ticket waiting (tickets)",
        icon: IconName::Check,
        keywords: &["ticket", "wait", "waiting", "pending", "blocked"],
        action: || Box::new(TicketWait),
    },
    Entry {
        label: "Drop the ticket (tickets)",
        icon: IconName::Close,
        keywords: &["ticket", "drop", "discard", "hide"],
        action: || Box::new(TicketDrop),
    },
    Entry {
        label: "Reopen the ticket (tickets)",
        icon: IconName::Check,
        keywords: &["ticket", "reopen", "open", "undo"],
        action: || Box::new(TicketReopen),
    },
    Entry {
        label: "Promote the ticket to a story (tickets)",
        icon: IconName::Check,
        keywords: &["ticket", "promote", "story", "pm", "cost"],
        action: || Box::new(PromoteTicketStory),
    },
    Entry {
        label: "Promote the ticket to a decision (tickets)",
        icon: IconName::Check,
        keywords: &["ticket", "promote", "decision"],
        action: || Box::new(PromoteTicketDecision),
    },
    Entry {
        label: "Copy the id of the ticket (tickets)",
        icon: IconName::Copy,
        keywords: &["ticket", "copy", "id", "clipboard"],
        action: || Box::new(CopyTicketId),
    },
    Entry {
        label: "Show the archived products",
        icon: IconName::Folder,
        keywords: &["archive", "restore", "products", "sidebar"],
        action: || Box::new(ToggleArchivedProducts),
    },
    Entry {
        label: "Cancel the reflection of the active product",
        icon: IconName::Close,
        keywords: &["reflect", "cancel", "stop"],
        action: || Box::new(CancelReflection),
    },
    Entry {
        label: "Reflect on the active product",
        icon: IconName::BookOpen,
        keywords: &["reflect", "reflection", "memory", "learn", "cost"],
        action: || Box::new(ReflectProduct),
    },
    Entry {
        label: "Copy the folder of the active product",
        icon: IconName::Folder,
        keywords: &["copy", "path", "folder", "product", "clipboard"],
        action: || Box::new(CopyProductPath),
    },
    Entry {
        label: "Open the folder of the active product",
        icon: IconName::Folder,
        keywords: &["open", "folder", "files", "product", "manager"],
        action: || Box::new(OpenProductFolder),
    },
    Entry {
        label: "Delete the data of the active product",
        icon: IconName::Delete,
        keywords: &["delete", "clear", "remove", "data", "product"],
        action: || Box::new(DeleteProductData),
    },
    Entry {
        label: "Archive the active product",
        icon: IconName::FileText,
        keywords: &["archive", "hide", "product", "sidebar"],
        action: || Box::new(ArchiveProduct),
    },
    Entry {
        label: "Split the focused pane to the right",
        icon: IconName::SquareTerminal,
        keywords: &["split", "right", "pane", "terminal", "beside"],
        action: || Box::new(SplitPaneRight),
    },
    Entry {
        label: "Split the focused pane down",
        icon: IconName::SquareTerminal,
        keywords: &["split", "down", "below", "pane", "terminal"],
        action: || Box::new(SplitPaneDown),
    },
    Entry {
        label: "Copy the folder of the focused pane",
        icon: IconName::Folder,
        keywords: &["copy", "folder", "cwd", "pane", "terminal", "clipboard"],
        action: || Box::new(CopyPaneFolder),
    },
    Entry {
        label: "Close the focused pane",
        icon: IconName::Close,
        keywords: &["close", "kill", "pane", "terminal", "tab"],
        action: || Box::new(ClosePane),
    },
    Entry {
        label: "Open the story session",
        icon: IconName::Check,
        keywords: &["story", "session", "open"],
        action: || Box::new(OpenStorySession),
    },
    Entry {
        label: "Review the story session",
        icon: IconName::Check,
        keywords: &["story", "session", "review", "diff"],
        action: || Box::new(ReviewStory),
    },
    Entry {
        label: "Accept the story session",
        icon: IconName::Check,
        keywords: &["story", "session", "accept", "merge"],
        action: || Box::new(AcceptStory),
    },
    Entry {
        label: "Reject the story session",
        icon: IconName::Close,
        keywords: &["story", "session", "reject", "discard"],
        action: || Box::new(RejectStory),
    },
    Entry {
        label: "Open a terminal in the worktree of the story session",
        icon: IconName::SquareTerminal,
        keywords: &["story", "session", "terminal", "worktree"],
        action: || Box::new(OpenStoryTerminal),
    },
    Entry {
        label: "Rename the product group of the active product",
        icon: IconName::Replace,
        keywords: &["group", "rename", "name", "product", "sidebar"],
        action: || Box::new(RenameGroup),
    },
    Entry {
        label: "Color the product group of the active product",
        icon: IconName::Palette,
        keywords: &["group", "color", "colour", "product", "sidebar"],
        action: || Box::new(ColorGroup),
    },
    Entry {
        label: "Remove the product group of the active product",
        icon: IconName::Folder,
        keywords: &["group", "remove", "ungroup", "product", "sidebar"],
        action: || Box::new(RemoveGroup),
    },
    Entry {
        label: "Open the context menu of the focused row or pane",
        icon: IconName::Settings,
        keywords: &["menu", "context", "right", "click", "actions"],
        action: || Box::new(OpenContextMenu),
    },
    Entry {
        label: "Show alerts",
        icon: IconName::Bell,
        keywords: &[
            "alerts",
            "notifications",
            "log",
            "bell",
            "blocked",
            "failed",
        ],
        action: || Box::new(ShowAlerts),
    },
    Entry {
        label: "Turn voice mode of the focused pane on or off",
        icon: IconName::Mic,
        keywords: &["voice", "speak"],
        action: || Box::new(ToggleVoice),
    },
    Entry {
        label: "Talk to this pane",
        icon: IconName::Mic,
        keywords: &[
            "voice",
            "conversation",
            "talk",
            "speak",
            "hands",
            "free",
            "listen",
        ],
        action: || Box::new(TalkToPane),
    },
    Entry {
        label: "Open the settings",
        icon: IconName::Settings,
        keywords: &[
            "voice",
            "models",
            "download",
            "preferences",
            "hotkey",
            "keys",
        ],
        action: || Box::new(OpenSettings),
    },
    Entry {
        label: "Delete data: open the Data section of the settings",
        icon: IconName::Delete,
        keywords: &[
            "delete",
            "clear",
            "remove",
            "data",
            "backup",
            "product",
            "transcripts",
            "logs",
        ],
        action: || Box::new(OpenDataSettings),
    },
    Entry {
        label: "Open the owner settings",
        icon: IconName::Settings,
        keywords: &["owner", "name", "language", "settings", "prompt"],
        action: || Box::new(OpenOwnerSettings),
    },
    Entry {
        label: "Show the context of the next meeting",
        icon: IconName::FileText,
        keywords: &["snapshot", "context", "meeting"],
        action: || Box::new(ShowMeetingContext),
    },
    Entry {
        label: "Ask one role of the active product",
        icon: IconName::Bot,
        keywords: &["ask", "role", "question", "meeting"],
        action: || Box::new(AskMeetingRole),
    },
    Entry {
        label: "Update the index of the active product",
        icon: IconName::Folder,
        keywords: &["index", "knowledge", "repos", "refresh"],
        action: || Box::new(UpdateIndex),
    },
    Entry {
        label: "New decision for the active product",
        icon: IconName::Plus,
        keywords: &["decision", "add", "owner"],
        action: || Box::new(NewDecision),
    },
    Entry {
        label: "Ask the front desk",
        icon: IconName::Bot,
        keywords: &["ask", "claude", "conversation", "chat", "desk", "question"],
        action: || Box::new(AskDesk),
    },
    Entry {
        label: "Show the archived decisions",
        icon: IconName::Folder,
        keywords: &["archive", "restore", "decisions"],
        action: || Box::new(ShowArchivedDecisions),
    },
    Entry {
        label: "Run the action of the approved decision in Claude",
        icon: IconName::Bot,
        keywords: &["claude", "approve", "action", "run"],
        action: || Box::new(RunApprovedInClaude),
    },
    Entry {
        label: "Check the health of the tools",
        icon: IconName::Check,
        keywords: &["doctor", "health", "tools", "check"],
        action: || Box::new(CheckHealth),
    },
    Entry {
        label: "Back up the data now",
        icon: IconName::Folder,
        keywords: &["backup", "data", "save", "copy"],
        action: || Box::new(BackUpData),
    },
    Entry {
        label: "Open the guide: how AgentSI works",
        icon: IconName::FileText,
        keywords: &["help", "guide", "how", "works"],
        action: || Box::new(OpenGuide),
    },
    Entry {
        label: "Open the guide: the first steps",
        icon: IconName::FileText,
        keywords: &["help", "guide", "start", "docs", "install"],
        action: || Box::new(OpenGuideStart),
    },
    Entry {
        label: "Close the window (terminals keep running)",
        icon: IconName::Close,
        keywords: &["quit", "exit"],
        action: || Box::new(Quit),
    },
];

#[cfg(test)]
mod tests {
    use super::{BINDINGS, ENTRIES, KEY_ACTIONS};

    #[test]
    fn each_screen_action_has_a_palette_entry() {
        for label in [
            "Open the front desk of the active product",
            "Open the minutes of the active product",
            "Open the roles of the active product",
            "Open the stories of the active product",
            "Open the tickets of the active product",
            "Open the git panel of the active product",
            "Open the meeting of the active product",
            "Start the meeting in the question field (meeting)",
            "Open the minutes of the last meeting",
            "Cancel the meeting of the active product",
            "Open the call of the open minutes",
            "Make the call of the open minutes (minutes)",
            "Play or pause the call (call)",
            "Mark the call as listened, or remove the mark (call)",
            "Render the call with the voices (call)",
            "Stop the render of the call (call)",
            "Go to the previous segment of the call (call)",
            "Go to the next segment of the call (call)",
            "Change the speed of the call (call)",
            "Reload the open screens from the core",
            "Open the settings",
            "Talk to this pane",
            "Delete data: open the Data section of the settings",
            "Show the context of the next meeting",
            "Ask one role of the active product",
            "Update the index of the active product",
            "New decision for the active product",
            "Ask the front desk",
            "Show the archived decisions",
            "Run the action of the approved decision in Claude",
            "Check the health of the tools",
            "Back up the data now",
            "Open the guide: how AgentSI works",
            "Open the guide: the first steps",
            "Approve the pending decision under the cursor (front desk)",
            "Reject the pending decision under the cursor (front desk)",
            "Archive the pending decision under the cursor (front desk)",
            "Mark the priority under the cursor accepted (minutes)",
            "Mark the priority under the cursor changed (minutes)",
            "Mark the priority under the cursor rejected (minutes)",
            "Flag the priority under the cursor as new, or clear the flag (minutes)",
            "Archive the open minutes",
            "Restore the open minutes from the archive",
            "Label the active product",
            "Color the active product",
            "Move the active product to a group",
            "Rename the active pane",
            "Color the active pane",
            "Close the other tabs of the bar",
            "Close the tabs to the right of the active tab",
            "New tab group with the active tab",
            "Add the active tab to a tab group",
            "Remove the active tab from its tab group",
            "Rename the tab group of the active tab",
            "Color the tab group of the active tab",
            "Collapse or open the tab group of the active tab",
            "Ungroup the tab group of the active tab",
            "Close the tab group of the active tab",
            "New story in the active product",
            "Suggest stories for the active product",
            "Archive the story",
            "Restore the story",
            "Record a merge of the story",
            "New ticket for the active product",
            "Open the ticket under the cursor (tickets)",
            "Mark the ticket done (tickets)",
            "Mark the ticket waiting (tickets)",
            "Drop the ticket (tickets)",
            "Reopen the ticket (tickets)",
            "Promote the ticket to a story (tickets)",
            "Promote the ticket to a decision (tickets)",
            "Copy the id of the ticket (tickets)",
            "Reflect on the active product",
            "Cancel the reflection of the active product",
            "Show the archived products",
            "Copy the folder of the active product",
            "Open the folder of the active product",
            "Delete the data of the active product",
            "Archive the active product",
            "Split the focused pane to the right",
            "Split the focused pane down",
            "Copy the folder of the focused pane",
            "Close the focused pane",
            "Open the story session",
            "Review the story session",
            "Accept the story session",
            "Reject the story session",
            "Open a terminal in the worktree of the story session",
            "Rename the product group of the active product",
            "Color the product group of the active product",
            "Remove the product group of the active product",
            "Open the context menu of the focused row or pane",
        ] {
            assert!(
                ENTRIES.iter().any(|e| e.label == label),
                "no entry: {label}"
            );
        }
    }

    #[test]
    fn the_two_guide_entries_open_the_top_and_the_first_steps() {
        use super::{OpenGuide, OpenGuideStart};
        let action = |label: &str| {
            let entry = ENTRIES.iter().find(|e| e.label == label).unwrap();
            (entry.action)()
        };
        let top = action("Open the guide: how AgentSI works");
        assert!(top.partial_eq(&OpenGuide));
        let start = action("Open the guide: the first steps");
        assert!(start.partial_eq(&OpenGuideStart));
    }

    #[test]
    fn each_binding_has_a_unique_id_in_the_same_order() {
        assert_eq!(KEY_ACTIONS.len(), BINDINGS.len());
        let mut seen = std::collections::BTreeSet::new();
        for (a, (_, make)) in KEY_ACTIONS.iter().zip(BINDINGS) {
            assert!(seen.insert(a.id), "two actions have the id {}", a.id);
            assert!(
                a.id.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "{} is no kebab-case id",
                a.id
            );
            assert!(!a.label.is_empty());
            make();
        }
        // The ids of the product keys follow the order of the product actions.
        for n in 0..9 {
            let id = super::product_id(n).unwrap();
            let at = KEY_ACTIONS.iter().position(|a| a.id == id).unwrap();
            assert_eq!(BINDINGS[at].0, format!("secondary-{}", n + 1));
        }
        assert_eq!(super::product_id(9), None);
    }

    #[test]
    fn no_binding_takes_a_key_of_the_pane_or_of_a_common_program_key() {
        // Copy and paste of the pane, and keys that shells and editors use.
        const TAKEN: [&str; 8] = [
            "secondary-shift-c",
            "secondary-shift-v",
            "secondary-w",
            "secondary-r",
            "secondary-q",
            "secondary-c",
            "secondary-d",
            "secondary-z",
        ];
        for (keys, _) in BINDINGS {
            assert!(
                !TAKEN.contains(keys),
                "{keys} is taken by the pane or a program"
            );
        }
    }
}
