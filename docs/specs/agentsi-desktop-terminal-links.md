# AgentSI desktop: links in terminal panes

**Status:** approved 2026-11-21 · **Source:** owner request and decision of 2026-11-21 ("un navegador que se abra al clickear links de la terminal"; the system browser)

## Problem

A terminal pane shows URLs and OSC 8 hyperlinks as plain text. The owner copies each link by hand. The FFI call `ghostty_grid_ref_hyperlink_uri` exists (`agentsi-vt/src/sys.rs:976`), but nothing calls it.

## Goal

- Ctrl+click on a link in a pane opens it in the system browser.
- A link is an OSC 8 hyperlink or a URL that the pane finds in the text.
- A file path with a line number opens in `$EDITOR` in a new terminal.

## Non-goals

- A browser inside the app. ADR 0004 stands.
- Links in the scrollback search results.

## Terms

- **Link:** a range of cells with a target, from OSC 8 or from the URL finder.
- **URL finder:** the code that finds `http://`, `https://`, `file://` and `mailto:` URLs, and paths like `src/main.rs:12:5`, in the text of a row.

## Design

### agentsi-vt

- `grid.rs` `Cell` gets `link: Option<LinkId>`. The grid keeps a table `LinkId -> uri`, filled from `ghostty_grid_ref_hyperlink_uri` when the grid is read.

### agentsi-term-view

- The URL finder runs on the rows on screen only, when the rows change. It joins wrapped rows. Trailing `.`, `,`, `)` and quotes are cut, except a `)` that closes a `(` in the URL.
- With Ctrl held, the link under the mouse gets an underline and the pointer cursor. A tooltip shows the target.
- `Event` gets `OpenLink(String)`. Ctrl+click sends it. A plain click keeps its current job (selection, or mouse reports to the program).
- When the program has mouse reporting on, Ctrl+click still opens the link.

### agentsi-app

- `pane.rs` handles `OpenLink`:
  - A URL goes to `cx.open_url`.
  - A path is resolved against the pane folder. It opens `$EDITOR +line path` in a new terminal in the same space. With no `$EDITOR`, it goes to `cx.open_url` as a `file://` URL.
- The pane menu gets "Open link" and "Copy link" when the mouse is on a link.
- The terminal settings (`agentsi-desktop-settings-nav.md`) get `link_modifier`: `ctrl` (default) or `none`.

## Acceptance criteria

- [ ] `printf '\e]8;;https://example.com\e\\link\e]8;;\e\\\n'` in a pane, then Ctrl+click on "link", opens https://example.com in the system browser.
- [ ] Ctrl+click on a plain `https://github.com/x/y).` opens `https://github.com/x/y`.
- [ ] Ctrl+click on `src/main.rs:12` opens `$EDITOR +12 src/main.rs` in a new terminal.
- [ ] Without Ctrl, a click on a link starts a selection, as before.
- [ ] Inside `htop` (mouse reporting on), Ctrl+click on a link opens it.
- [ ] The idle bench draws 0 frames with 5 links on screen.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass.
