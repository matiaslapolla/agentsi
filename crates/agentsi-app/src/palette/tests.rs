//! The prefixes, the match and the old entries of the palette. The tests over the real shell
//! are in `shell/palette_tests.rs`.

use super::{Kind, kinds, matches, split};
use crate::actions::ENTRIES;

/// Each entry of the palette before the sources, with its keywords then.
pub const OLD_ENTRIES: &[(&str, &[&str])] = &[
    (
        "New terminal in the active product",
        &["shell", "pane", "tab"],
    ),
    (
        "New Claude session in the active product",
        &["claude", "agent"],
    ),
    (
        "Open the front desk of the active product",
        &["desk", "product"],
    ),
    (
        "Open the minutes of the active product",
        &["minutes", "meetings", "list"],
    ),
    (
        "Open the roles of the active product",
        &[
            "roles",
            "roster",
            "overlay",
            "specialist",
            "reset",
            "validate",
            "apply",
        ],
    ),
    (
        "Roles: New general role",
        &[
            "roles", "general", "global", "new", "role", "products", "shared",
        ],
    ),
    (
        "Open the stories of the active product",
        &[
            "stories", "story", "task", "approve", "run", "review", "accept", "reject", "diff",
        ],
    ),
    (
        "Open the tickets of the active product",
        &[
            "tickets", "ticket", "note", "todo", "question", "pending", "waiting",
        ],
    ),
    (
        "Open the git panel of the active product",
        &[
            "git", "branch", "commit", "stage", "unstage", "diff", "status", "history", "config",
        ],
    ),
    (
        "Git: Fetch",
        &["git", "fetch", "remote", "origin", "download", "update"],
    ),
    (
        "Git: Pull",
        &[
            "git",
            "pull",
            "fast-forward",
            "upstream",
            "behind",
            "update",
        ],
    ),
    (
        "Git: Push",
        &[
            "git", "push", "upstream", "ahead", "remote", "origin", "publish",
        ],
    ),
    (
        "Git: Switch branch…",
        &[
            "git", "switch", "checkout", "branch", "branches", "worktree",
        ],
    ),
    (
        "Git: New branch…",
        &["git", "new", "branch", "create", "make"],
    ),
    (
        "Set up a new product",
        &[
            "setup", "new", "product", "register", "add", "brief", "folder",
        ],
    ),
    (
        "Update the brief of the active product",
        &[
            "setup",
            "brief",
            "roster",
            "objectives",
            "stories",
            "update",
            "apply",
        ],
    ),
    (
        "Open the meeting of the active product",
        &["meeting", "meet", "roles", "question", "live", "panes"],
    ),
    (
        "Open the memory of the active product",
        &[
            "memory",
            "items",
            "lessons",
            "reflection",
            "reflect",
            "revert",
            "proposal",
        ],
    ),
    (
        "Retire the item under the cursor (memory)",
        &["memory", "retire", "item", "lesson"],
    ),
    (
        "Revert the newest reflection (memory)",
        &["memory", "reflection", "reflect", "revert", "undo"],
    ),
    (
        "Approve the pending proposal (memory)",
        &["memory", "reflection", "proposal", "prompt", "approve"],
    ),
    (
        "Reject the pending proposal (memory)",
        &["memory", "reflection", "proposal", "prompt", "reject"],
    ),
    (
        "Start the meeting in the question field (meeting)",
        &["meeting", "meet", "start", "question"],
    ),
    (
        "Open the minutes of the last meeting",
        &["meeting", "meet", "minutes", "result"],
    ),
    (
        "Cancel the meeting of the active product",
        &["meeting", "meet", "stop", "cancel"],
    ),
    (
        "Open the call of the open minutes",
        &["call", "audio", "listen", "play", "voice"],
    ),
    (
        "Make the call of the open minutes (minutes)",
        &["call", "make", "render", "audio", "voice"],
    ),
    (
        "Play or pause the call (call)",
        &["call", "audio", "play", "pause", "listen"],
    ),
    (
        "Mark the call as listened, or remove the mark (call)",
        &["call", "listened", "heard", "mark"],
    ),
    (
        "Render the call with the voices (call)",
        &["call", "render", "audio", "voice", "kokoro"],
    ),
    (
        "Stop the render of the call (call)",
        &["call", "render", "cancel", "stop"],
    ),
    (
        "Go to the previous segment of the call (call)",
        &["call", "previous", "back", "segment"],
    ),
    (
        "Go to the next segment of the call (call)",
        &["call", "next", "skip", "segment"],
    ),
    (
        "Change the speed of the call (call)",
        &["call", "speed", "faster", "slower"],
    ),
    (
        "Reload the open screens from the core",
        &["refresh", "update", "desk"],
    ),
    (
        "Approve the pending decision under the cursor (front desk)",
        &["decide", "decision", "approve", "yes"],
    ),
    (
        "Reject the pending decision under the cursor (front desk)",
        &["decide", "decision", "reject", "no"],
    ),
    (
        "Archive the pending decision under the cursor (front desk)",
        &["decision", "archive", "hide"],
    ),
    (
        "Mark the priority under the cursor accepted (minutes)",
        &["mark", "priority", "accept"],
    ),
    (
        "Mark the priority under the cursor changed (minutes)",
        &["mark", "priority", "change"],
    ),
    (
        "Mark the priority under the cursor rejected (minutes)",
        &["mark", "priority", "reject"],
    ),
    (
        "Flag the priority under the cursor as new, or clear the flag (minutes)",
        &["mark", "priority", "new"],
    ),
    ("Archive the open minutes", &["minutes", "archive", "hide"]),
    (
        "Restore the open minutes from the archive",
        &["minutes", "restore", "archive"],
    ),
    (
        "Label the active product",
        &["label", "name", "tag", "rename", "product", "sidebar"],
    ),
    (
        "Color the active product",
        &["color", "colour", "bar", "product", "sidebar"],
    ),
    (
        "Move the active product to a group",
        &["group", "folder", "product", "sidebar", "ungroup"],
    ),
    (
        "Rename the active pane",
        &["label", "name", "tag", "pane", "tab", "terminal"],
    ),
    (
        "Color the active pane",
        &["color", "colour", "pane", "tab", "terminal"],
    ),
    (
        "Close the other tabs of the bar",
        &["close", "others", "tabs", "pane", "bar"],
    ),
    (
        "Close the tabs to the right of the active tab",
        &["close", "right", "tabs", "after", "pane"],
    ),
    (
        "New tab group with the active tab",
        &["tab", "group", "new", "chip", "name"],
    ),
    (
        "Add the active tab to a tab group",
        &["tab", "group", "add", "move", "chip"],
    ),
    (
        "Remove the active tab from its tab group",
        &["tab", "group", "remove", "leave", "out"],
    ),
    ("Close the active tab", &["close", "tab", "screen", "pane"]),
    ("Pin the active tab", &["pin", "tab", "keep", "first"]),
    ("Unpin the active tab", &["unpin", "pin", "tab"]),
    (
        "Add the active tab to the favorites",
        &[
            "favorite",
            "favourite",
            "save",
            "bookmark",
            "tab",
            "command",
        ],
    ),
    (
        "Move the active tab to a product",
        &["move", "tab", "product", "space"],
    ),
    (
        "Open a favorite",
        &["favorite", "favourite", "open", "bookmark"],
    ),
    (
        "Rename a favorite",
        &["favorite", "favourite", "rename", "label"],
    ),
    (
        "Remove a favorite",
        &["favorite", "favourite", "remove", "delete"],
    ),
    (
        "Rename the tab group of the active tab",
        &["tab", "group", "rename", "name", "chip"],
    ),
    (
        "Color the tab group of the active tab",
        &["tab", "group", "color", "colour", "chip"],
    ),
    (
        "Collapse or open the tab group of the active tab",
        &["tab", "group", "collapse", "open", "expand", "chip"],
    ),
    (
        "Ungroup the tab group of the active tab",
        &["tab", "group", "ungroup", "dissolve", "chip"],
    ),
    (
        "Close the tab group of the active tab",
        &["tab", "group", "close", "chip"],
    ),
    (
        "Suggest stories for the active product",
        &["story", "suggest", "pm", "propose"],
    ),
    ("Archive the story", &["story", "archive"]),
    ("Restore the story", &["story", "restore", "archive"]),
    (
        "Record a merge of the story",
        &["story", "merge", "commit", "result"],
    ),
    (
        "New story in the active product",
        &["story", "task", "pm", "new", "propose"],
    ),
    (
        "New ticket for the active product",
        &["ticket", "note", "todo", "question", "new", "add"],
    ),
    (
        "Open the ticket under the cursor (tickets)",
        &["ticket", "open", "cursor"],
    ),
    (
        "Mark the ticket done (tickets)",
        &["ticket", "done", "close", "finish"],
    ),
    (
        "Mark the ticket waiting (tickets)",
        &["ticket", "wait", "waiting", "pending", "blocked"],
    ),
    (
        "Drop the ticket (tickets)",
        &["ticket", "drop", "discard", "hide"],
    ),
    (
        "Reopen the ticket (tickets)",
        &["ticket", "reopen", "open", "undo"],
    ),
    (
        "Promote the ticket to a story (tickets)",
        &["ticket", "promote", "story", "pm", "cost"],
    ),
    (
        "Promote the ticket to a decision (tickets)",
        &["ticket", "promote", "decision"],
    ),
    (
        "Copy the id of the ticket (tickets)",
        &["ticket", "copy", "id", "clipboard"],
    ),
    (
        "Show the archived products",
        &["archive", "restore", "products", "sidebar"],
    ),
    (
        "Cancel the reflection of the active product",
        &["reflect", "cancel", "stop"],
    ),
    (
        "Reflect on the active product",
        &["reflect", "reflection", "memory", "learn", "cost"],
    ),
    (
        "Copy the folder of the active product",
        &["copy", "path", "folder", "product", "clipboard"],
    ),
    (
        "Open the folder of the active product",
        &["open", "folder", "files", "product", "manager"],
    ),
    (
        "Delete the data of the active product",
        &["delete", "clear", "remove", "data", "product"],
    ),
    (
        "Archive the active product",
        &["archive", "hide", "product", "sidebar"],
    ),
    (
        "Split the focused pane to the right",
        &["split", "right", "pane", "terminal", "beside"],
    ),
    (
        "Split the focused pane down",
        &["split", "down", "below", "pane", "terminal"],
    ),
    (
        "Copy the folder of the focused pane",
        &["copy", "folder", "cwd", "pane", "terminal", "clipboard"],
    ),
    (
        "Open the link under the mouse in the focused pane",
        &[
            "open", "link", "url", "browser", "editor", "pane", "terminal",
        ],
    ),
    (
        "Copy the link under the mouse in the focused pane",
        &["copy", "link", "url", "pane", "terminal", "clipboard"],
    ),
    (
        "Close the focused pane",
        &["close", "kill", "pane", "terminal", "tab"],
    ),
    ("Open the story session", &["story", "session", "open"]),
    (
        "Review the story session",
        &["story", "session", "review", "diff"],
    ),
    (
        "Accept the story session",
        &["story", "session", "accept", "merge"],
    ),
    (
        "Reject the story session",
        &["story", "session", "reject", "discard"],
    ),
    (
        "Open a terminal in the worktree of the story session",
        &["story", "session", "terminal", "worktree"],
    ),
    (
        "Rename the product group of the active product",
        &["group", "rename", "name", "product", "sidebar"],
    ),
    (
        "Color the product group of the active product",
        &["group", "color", "colour", "product", "sidebar"],
    ),
    (
        "Remove the product group of the active product",
        &["group", "remove", "ungroup", "product", "sidebar"],
    ),
    (
        "Open the context menu of the focused row or pane",
        &["menu", "context", "right", "click", "actions"],
    ),
    (
        "Show alerts",
        &[
            "alerts",
            "notifications",
            "log",
            "bell",
            "blocked",
            "failed",
        ],
    ),
    (
        "Turn voice mode of the focused pane on or off",
        &["voice", "speak"],
    ),
    (
        "Talk to this pane",
        &[
            "voice",
            "conversation",
            "talk",
            "speak",
            "hands",
            "free",
            "listen",
        ],
    ),
    (
        "Open the settings",
        &[
            "voice",
            "models",
            "download",
            "preferences",
            "hotkey",
            "keys",
        ],
    ),
    (
        "Delete data: open the Data section of the settings",
        &[
            "delete",
            "clear",
            "remove",
            "data",
            "backup",
            "product",
            "transcripts",
            "logs",
        ],
    ),
    (
        "Open the owner settings",
        &["owner", "name", "language", "settings", "prompt"],
    ),
    (
        "Show the context of the next meeting",
        &["snapshot", "context", "meeting"],
    ),
    (
        "Ask one role of the active product",
        &["ask", "role", "question", "meeting"],
    ),
    (
        "Update the index of the active product",
        &["index", "knowledge", "repos", "refresh"],
    ),
    (
        "New decision for the active product",
        &["decision", "add", "owner"],
    ),
    (
        "Ask the front desk",
        &["ask", "claude", "conversation", "chat", "desk", "question"],
    ),
    (
        "Show the archived decisions",
        &["archive", "restore", "decisions"],
    ),
    (
        "Run the action of the approved decision in Claude",
        &["claude", "approve", "action", "run"],
    ),
    (
        "Check the health of the tools",
        &["doctor", "health", "tools", "check"],
    ),
    ("Back up the data now", &["backup", "data", "save", "copy"]),
    (
        "Open the guide: how AgentSI works",
        &["help", "guide", "how", "works"],
    ),
    (
        "Open the guide: the first steps",
        &["help", "guide", "start", "docs", "install"],
    ),
    (
        "Show files",
        &["files", "explorer", "tree", "sidebar", "folder"],
    ),
    ("New file", &["files", "create", "touch", "file"]),
    (
        "New folder",
        &["files", "create", "mkdir", "folder", "directory"],
    ),
    ("Open the selected file", &["files", "open", "system"]),
    (
        "Open the selected file with $EDITOR in a terminal",
        &["files", "edit", "editor", "vim", "nvim"],
    ),
    (
        "Rename the selected file or folder",
        &["files", "rename", "move"],
    ),
    (
        "Move the selected file or folder to the trash",
        &["files", "delete", "remove", "trash"],
    ),
    (
        "Open a terminal in the selected folder",
        &["files", "terminal", "shell", "here"],
    ),
    (
        "Copy the path of the selected file or folder",
        &["files", "copy", "path", "clipboard"],
    ),
    (
        "Reveal the selected file or folder in the system",
        &["files", "reveal", "file manager", "show"],
    ),
    (
        "Add the selected folder to the favorites",
        &["files", "favorite", "folder", "save"],
    ),
    (
        "Close the window (terminals keep running)",
        &["quit", "exit"],
    ),
];

#[test]
fn a_prefix_names_one_source_and_the_rest_is_the_query() {
    assert_eq!(split("@build"), (Some(Kind::Tabs), "build"));
    assert_eq!(split("  > git  "), (Some(Kind::Actions), "git"));
    assert_eq!(split("#term"), (Some(Kind::Settings), "term"));
    assert_eq!(split("/src/main.rs"), (Some(Kind::Files), "src/main.rs"));
    assert_eq!(split("main.rs"), (None, "main.rs"));
    assert_eq!(split("a/b"), (None, "a/b"));
    assert_eq!(split(""), (None, ""));
    assert_eq!(kinds("@"), [Kind::Tabs]);
    assert_eq!(kinds(" "), [Kind::Recent, Kind::Actions, Kind::Settings]);
    assert_eq!(
        kinds("git"),
        [
            Kind::Recent,
            Kind::Actions,
            Kind::Tabs,
            Kind::Settings,
            Kind::Favorites
        ]
    );
}

#[test]
fn the_match_reads_the_label_and_the_keywords_with_no_regard_to_case() {
    assert!(matches("Git: Fetch", &["remote"], "git: f"));
    assert!(matches("Git: Fetch", &["remote"], "REMOTE"));
    assert!(matches("Git: Fetch", &["remote"], ""));
    assert!(!matches("Git: Fetch", &["remote"], "push"));
    assert!(!matches("Git: Fetch", &[] as &[&str], "remote"));
}

#[test]
fn each_old_entry_is_still_there_with_its_old_keywords() {
    assert_eq!(OLD_ENTRIES.len(), 132);
    for (label, keywords) in OLD_ENTRIES {
        let entry = ENTRIES
            .iter()
            .find(|e| e.label == *label)
            .unwrap_or_else(|| panic!("no entry {label:?}"));
        for k in *keywords {
            assert!(
                matches(entry.label, entry.keywords, k),
                "{label:?} lost the keyword {k:?}"
            );
        }
    }
}
