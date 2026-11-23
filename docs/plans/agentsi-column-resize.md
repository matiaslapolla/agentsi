# Column resize plan

Spec: `docs/specs/agentsi-column-resize.md`. One unit, one branch `column-resize`, local session.

## U1 — Screen columns
- **Files:** `crates/agentsi-app/src/columns.rs`, `main.rs`, `layout.rs`, `shell.rs`, `screens/settings.rs`, `screens/git.rs`, `screens/git/history.rs`, `screens/desk.rs`
- **Steps:**
  1. Write `columns.rs` with `ScreenColumn`, the `ScreenColumns` global and unit tests of the clamp, the reset and the load.
  2. Add `screen_column`, `ColumnDrag` and `on_column_drag`, with the gpui-kit `resize_handle`.
  3. Add `columns` to `Layout`. The shell loads it at start and saves on a change of the global.
  4. Use `screen_column` in the four screens, one commit for each screen.
  5. Add gpui tests that drag and double-click each edge.
  6. Run `code-review`, then `make install`, then the owner check of the spec.
- **Check:** `devflow check` passes, and the owner drags the four columns in the installed app.
