# AgentSI desktop: panes and the pane bar

**Status:** approved 2026-11-22 · **Source:** owner request of 2026-11-22 ("resize y draggear los panes, y colapsar todo lo que se pueda en iconos, o esconder, lo que sea mejor"; "botones en la top bar que cambian su estado si la herramienta está abierta o no, como el FL Studio")

## Problem

The sidebar has a fixed width of 248 px (`tokens.rs` `control::SIDEBAR_WIDTH`). The owner cannot hide it, resize it or move it. Products and Files share the sidebar through a mode switch, so the owner cannot see both. The alert log is an overlay. No control shows which tools are open.

## Goal

- A pane bar at the top of the window. It has one toggle button for each pane. A bright button means the pane is open. A muted button means the pane is hidden.
- The owner resizes each pane column and each pane in a column.
- The owner drags a pane to another place in its column, or to the other column.
- The app keeps the panes, their places and their sizes in `layout.json`.

## Non-goals

- An icon rail. The toggle buttons of the pane bar are the icons of the hidden panes.
- Custom window decorations on Linux (`agentsi-desktop-ui.md`).
- Floating panes, or panes in their own window.
- New panes for screens of the dock (git, desk, tickets).

## Terms

- **Pane**: one tool outside the dock: Products, Files or Alerts.
- **Column**: the left or the right side of the window. A column holds the open panes of that side, top to bottom.
- **Pane bar**: the row at the top of the window, above the columns and the dock.

## Decisions

- D1. A hidden pane disappears. It does not collapse to an icon. The pane bar always shows its toggle, so one mechanism does the work of an icon rail.
- D2. Products and Files become two panes. The mode switch of the sidebar goes away.
- D3. The pane bar replaces the sidebar header. It shows the app icon and name, the toggles, and the palette hint. The Alerts toggle keeps the unseen count of the bell.
- D4. The panes are global. Each space shows the same columns. The dock of each space stays as it is.
- D5. A column with no open pane takes no width.

## Design

### Pane bar

- Height `control::TOOLBAR_HEIGHT`, full width, `surface::base()`, bottom border `line::border()`.
- Left: the app icon and "AgentSI". Then the toggles in this order: Products, Files, Alerts.
- Right: the palette hint (`keys::hint(cx, keys::PALETTE)`).
- Each toggle is an icon button with a tooltip. The tooltip gives the pane name and its key.
- An open pane: the icon and label in `text::primary()`, a fill from the accent tokens, and a 2 px accent line at the bottom of the button.
- A hidden pane: the icon in `text::tertiary()`, ghost style, no line.
- Hover and press use the transitions of `agentsi-ui`. All colors come from tokens.

### Columns and resize

- The left column and the right column each have a drag handle on their inner edge.
- A column width stays between 180 px and 50 % of the window width. A double click on the handle sets the width back to 248 px.
- Two or more panes in one column split its height. A drag handle between two panes resizes them. A pane height is at least 120 px.
- Use `gpui-component` `resizable` when it fits. Otherwise use own code on plain GPUI.

### Drag

- Each pane has a header with its name. A drag on the header moves the pane.
- During a drag, the app shows a drop line at each place where the pane can go: between panes of a column, and at the top of an empty column at the window edge.
- A drop puts the pane at that place. Escape ends the drag and changes nothing.

### Actions and keys

- One action for each pane: "Toggle Products", "Toggle Files", "Toggle Alerts". Each is in the palette.
- One action "Toggle all panes": it hides each open pane, and the next time it shows the same panes again.
- `actions::ShowFiles` opens the Files pane. `actions::ShowAlerts` toggles the Alerts pane.
- Pick free keys. Check them against `key_table!` in `actions.rs`, the pane keys and `keys.toml` rebinding. Do not change existing keys.

### Saved state

- `layout.json` gets a top-level field `panes`: for each pane, its column, its order, its height share and whether it is open. It also holds the two column widths and the panes that "Toggle all panes" hid.
- The field has a serde default, so no version bump. A file with `sidebar_mode: "files"` and no `panes` opens Files and hides Products. A file with no `panes` and no `sidebar_mode` opens Products on the left, and hides Files and Alerts.
- The app writes the file at the end of a drag or a resize, not at each move.

### Performance

- An idle app draws 0 frames (`agentsi-desktop-ui.md`). A drag or a resize calls `cx.notify` only while it moves.
- Keep the `.cached` style of the sidebar for each pane, so a change in one pane does not draw the others.

## Acceptance criteria

- [ ] The pane bar shows the toggles Products, Files and Alerts. A toggle of an open pane has the bright style, and a toggle of a hidden pane has the muted style. A `#[gpui::test]` checks the style changes when a pane opens and closes.
- [ ] A click on a toggle opens or hides its pane. The palette action of each pane does the same.
- [ ] Products and Files can be open at the same time, one above the other.
- [ ] With all panes hidden, the dock takes the full width of the window.
- [ ] A drag on a column handle changes the column width. The width stays between 180 px and 50 % of the window. A double click sets it to 248 px.
- [ ] A drag on the handle between two panes changes their heights. No pane gets less than 120 px.
- [ ] A drag of a pane header to the other column moves the pane there. A drag inside a column changes the order. Escape during a drag changes nothing.
- [ ] "Toggle all panes" hides the open panes, and a second use shows the same panes again.
- [ ] After a restart, each pane has the same column, order, size and open state. A test writes and reads `layout.json` for this.
- [ ] A `layout.json` of before this change opens with the old sidebar state: Files open for `sidebar_mode: "files"`, else Products open.
- [ ] The Alerts toggle shows the unseen count, and it shows it also when the pane is hidden.
- [ ] Each existing test of the sidebar, the files tree and the alert log passes, changed only where the mode switch or the bell went away.
- [ ] The idle bench draws 0 frames with two open panes.
- [ ] Owner check: the toggles look bright and muted as expected on the M1, and a drag of a pane feels smooth.
