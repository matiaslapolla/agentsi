# Column resize

**Status:** approved 2026-11-22 · **Source:** owner request in the session of 2026-11-22

## Problem
The owner can drag the width of the two pane columns of the shell. The side columns inside the screens have a fixed width. A long branch name, a long file path or a wide table in the desk conversation does not fit, and the owner cannot make the column wider.

## Goal
- The owner can drag the width of each side column of a screen:
  1. the section list of Settings (200 px now),
  2. the list of the Git screen (248 px now),
  3. the commit list of the Git history view (248 px now),
  4. the desk conversation of the front desk (380 px now).
- A drag works like the edge of a pane column: the same handle, the same limits rule and a double click that gives the first width again.
- `layout.json` keeps each width, so the width stays after a restart.

## Non-goals
- The pane columns of the shell and the splits of a pane column. They already resize.
- The splits of the dock. The dock of gpui-kit already resizes them.
- A width for each product. One width for each screen column is enough.
- A drag of a height.

## Terms
**Screen column**: A side column inside a screen whose width the owner can drag. The four columns of the goal are the screen columns.

## Design

### The model, `crates/agentsi-app/src/columns.rs`
- `enum ScreenColumn { SettingsNav, GitList, GitHistory, DeskConversation }`. Each one gives its key in `layout.json`, its first width and its edge: trailing for a left column, leading for the desk conversation.
- `struct ScreenColumns` is a gpui `Global`. It keeps one width for each screen column that the owner changed.
- `width(column)` gives the width, at least `PANE_COLUMN_MIN`.
- `set(column, width, row) -> bool` clamps the width between `PANE_COLUMN_MIN` and `PANE_COLUMN_MAX` of the row width, with `panes::clamp_width`. `reset(column) -> bool` removes the width. Each one returns `false` if nothing changed, so a move that changes nothing does not notify.
- `save()` and `load()` read and write `BTreeMap<String, f32>`. A key that does not read, or a width that is not finite and positive, is dropped.

### The element
- `screen_column(column, content, cx)` wraps the content in a `flex_none` div of the width, with a `resize_handle` of gpui-kit on its edge. `max_w(relative(PANE_COLUMN_MAX))` keeps the column to the half of its row, so the screen does not measure the row.
- The handle starts a drag of `ColumnDrag { column, owner }`, where `owner` is the entity of the screen. A double click calls `reset`.
- `on_column_drag(row, column, cx)` adds the `on_drag_move::<ColumnDrag>` listener to the flex row that holds the column. It ignores a drag of another column or another screen, because two screens can show at the same time. The bounds of the row give the new width: `x - row.left` for a trailing edge, `row.right - x` for a leading edge.
- Each screen observes the global, so a drag in one screen also draws the other screens again.

### The screens
- `screens/settings.rs`, `screens/git.rs`, `screens/git/history.rs` and `screens/desk.rs` use `screen_column` in place of the fixed `.w(...)`.
- Each one adds `on_column_drag` to its row.

### Save
- `Layout` gets `columns: BTreeMap<String, f32>`, with `skip_serializing_if` empty. An old `layout.json` reads with no change.
- The shell loads the map into `ScreenColumns` at start.
- The shell observes the global and calls `schedule_save`, so a drag saves after its debounce.

## Acceptance criteria
- [ ] `cargo test -p agentsi-app columns` passes: `width` clamps to the min and the max, `reset` gives the first width, and `load` drops a bad key and a bad width.
- [ ] `cargo test -p agentsi-app layout` passes: a `layout.json` without `columns` reads, and a saved width reads back.
- [ ] A gpui test drags the edge of each of the four screen columns, and the column width changes by the drag distance.
- [ ] A gpui test double-clicks a dragged edge, and the column gets its first width again.
- [ ] In the installed app, the owner drags each of the four columns, restarts the app and sees the same widths.
- [ ] `devflow check` passes.
