# AgentSI desktop: sidebar tree, tabs and context menus

**Status:** approved 2026-11-07 · **Source:** owner decisions of 2026-11-07 (log "Owner decisions after the first owner checks")

## Problem

The sidebar is a flat list of products, so the owner cannot see what runs under each product. Tabs have no labels, colors or groups. No object has a right-click menu, so each action needs the palette or a key.

## Goal

- The sidebar shows each product as a tree, with its running items nested under it.
- The owner gives a label and a color to a product, a pane or a tab, and puts products into named groups.
- The owner drags tabs to reorder them, to move them to another tab bar, and into a tab group.
- A right-click on a product, a tree row, a tab or a story opens a menu with its actions.

## Non-goals

- macOS. U18 stands by.
- More than one window.
- Sync of labels and groups between devices.

## Terms

- **Item row:** a sidebar row under a product for one running or open thing: a pane, a meeting, a story session or a reflection.
- **Product group:** a named, ordered set of products in the sidebar, inside one host group.
- **Tab group:** a named, colored set of tabs in one tab bar. It collapses to one chip.
- **Look file:** `<config>/agentsi/look.toml`, which holds the labels, colors and groups.

## Design

### Sidebar tree

- Each product row gets an arrow. An open product shows its item rows:
  - each pane of the product, with its status dot, its label or title, and its host,
  - each running meeting, with its phase,
  - each story session that is not done, with its state,
  - a running reflection.
- The sources are `terminal.list`, `meeting.progress`, `story.list` and `memory.list` with their events. Stories have no progress event, so the app reads `story.list` again after `data.changed`.
- A product with a running item shows a count when it is closed. Products open and close with a click on the arrow, with Left and Right on the row, and stay so across restarts (`layout.json`).
- A click on an item row focuses its tab and opens the tab if it is closed. A pane row of a closed tab opens an attach to the same terminal.
- Product groups sit inside each host group. A product with no group stays at the top. The owner drags a product onto a group, or uses the menu entry "Move to group". A group collapses, and the owner renames, reorders or removes it from its menu. Removing a group keeps its products.

### Labels and colors

- A label is free text of 40 characters or less. A color is one of 8 tokens of `agentsi-ui` (`tokens::label`), which pass the 4.5:1 contrast rule on both surfaces.
- A product label shows next to its name. A product color is a 3 px bar at the left of its row and of each of its tabs.
- A pane label replaces the program title in the tab and in the item row. The title stays in the tooltip.
- The look file keeps product data by product id and pane data by terminal id. A pane that the core no longer knows loses its entry at the next start, as `layout.rs` prunes panels.
- The app writes the look file atomically and keeps unknown entries.

### Tabs

- The dock already moves a tab to another tab bar or to a split edge. This unit adds:
  - reorder by drag inside one tab bar,
  - a drop on a tab group chip, which adds the tab to that group,
  - "Close others" and "Close to the right".
- A tab group is a chip in the tab bar with its color and name. A click collapses it to the chip or opens it. The tabs of a group sit together, after the chip.
- `layout.json` keeps tab groups and the order of tabs. A new version of the layout file reads the old version.
- Closing a terminal tab still kills the terminal, behind the existing rule (`agentsi-desktop-ui.md`). "Close others" and "Close to the right" ask one Confirm when a running program would stop.
- If the dock of `gpui-component` 0.9.0 cannot draw a chip or reorder tabs through its public API, the worker copies `dock/` into `agentsi-ui` with its license, and records it in the log. A fork of the whole crate is not permitted.

### Context menus

Each menu uses `PopupMenu` of `gpui-component`. Each entry also has a palette entry, by the rule of `agentsi-desktop-ui.md`. A destructive entry asks one Confirm.

| Object | Entries |
|---|---|
| Product | New terminal, Open desk, Start meeting, New story, New ticket, Open tickets, Reflect, Open the git panel, Label and color, Move to group, Copy path, Open folder, Archive |
| Pane (tab or item row) | Rename, Color, Add to tab group, Talk to this pane, Voice mode, Split right, Split down, Copy folder, Close, Close others, Close to the right |
| Story session | Open, Review, Accept, Reject, Open a terminal in the worktree |
| Ticket (a row of the Tickets screen, the Ticket screen) | Open, Done, Wait, Drop, Reopen, Promote to story, Promote to decision, Copy id |
| Meeting | Open, Cancel |
| Product group | Rename, Color, Remove group |
| Tab group | Rename, Color, Ungroup, Close the group |

- "Talk to this pane" belongs to `agentsi-desktop-hearing.md`. Until it ships, the entry is not shown.
- The menu key (Shift+F10 or the Menu key) opens the menu of the focused row or tab, or of the ticket under the cursor of a Tickets or Ticket screen.
- An entry that the state of the ticket does not permit is off. "Promote to story" asks one Confirm, because it spends money.

### Rules that stay

- No color or size by hand: `rg -n 'rgb\(|hsla\(|px\(' crates/agentsi-app/src` finds nothing.
- The idle app draws 0 frames. A menu, a drag or a collapse draws only while it moves.
- No protocol change. All state of this spec lives in the app.

## Acceptance criteria

- [ ] A test of the real sidebar with a fake core shows a pane, a meeting, a story session and a reflection under their product, and a click on each focuses its tab.
- [ ] A product open or closed, a product group and a tab group survive a restart.
- [ ] A test sets a label and a color on a product and a pane, and the tab, the item row and the look file show them.
- [ ] A test drags a tab to a new place in one tab bar and into a tab group, and `layout.json` keeps the order.
- [ ] Each context menu entry of the table runs its action in a test, and each has a palette entry.
- [ ] Closing a running pane from a menu asks one Confirm.
- [ ] A look file with a gone terminal and an unknown entry starts the app and keeps the unknown entry.
- [ ] The idle app draws 0 frames, and the `rg` rule finds nothing.
- [ ] `git diff --stat` shows no change to `schema/core.openrpc.json`.
- [ ] The owner uses the tree, the labels, a tab group and three context menus, and accepts the look.
