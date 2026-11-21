# AgentSI desktop: a file explorer in the sidebar

**Status:** approved 2026-11-21 · **Source:** owner request and decision of 2026-11-21 ("crear carpetas y archivos desde el sidebar y tener un explorador de archivos"; a sidebar mode)

## Problem

The app shows no files. To create a file or a folder in a product, the owner opens a terminal or another editor.

## Goal

- The top of the sidebar has a switch with two modes: Products and Files.
- Files shows the folder tree of the selected product.
- The owner can create, rename, delete and reveal files and folders from the tree.

## Non-goals

- A code editor. The owner decided against it on 2026-11-21. "Open" uses the system or `$EDITOR`.
- A file search over the content. The tree has a filter by name only.
- Remote hosts in this spec. Files of a product on a remote host come later.

## Terms

- **Sidebar mode:** what the sidebar shows: Products (the current tree) or Files.
- **File tree:** the folders and files of the selected product, read by the core.

## Design

### Core

- `agentsi-agency` gets a module `files` and `agentsi-proto` gets `files_api.rs`, with these methods:
  - `files.list {product, path}` gives the entries of one folder: name, kind, size, modified time, git state.
  - `files.create {product, path, kind}`, `files.rename {product, from, to}`, `files.trash {product, path}`.
- Each path is relative to the product folder. The core refuses a path that leaves the folder after it resolves links.
- `files.trash` moves to the system trash (the `trash` crate, or `gio trash`). No method deletes for good.
- The list skips `.git/` and marks entries that `.gitignore` ignores. A setting shows or hides ignored entries.
- The core watches open folders with `notify` and sends `files.changed {product, path}`.

### App

- `sidebar.rs` gets a `SidebarMode` and a segmented control at the top. The mode is saved in `layout.json`.
- Files mode uses `gpui_component::tree` (`TreeState`, `TreeItem`) with lazy loading: a folder loads when it opens.
- A row shows the name, an icon by kind, and the git state color (changed, new, ignored).
- The context menu of a folder: New file, New folder, Rename, Move to trash, Open terminal here, Copy path, Reveal in system, Add to favorites.
- The context menu of a file: Open, Open with `$EDITOR` in a terminal, Rename, Move to trash, Copy path, Reveal in system.
- New file and New folder show an inline name field in the tree. Enter creates. Escape cancels.
- The filter field at the top of the tree filters the loaded names.
- `Ctrl+Shift+E` switches to Files mode. The palette has "New file", "New folder" and "Show files".

## Acceptance criteria

- [ ] The sidebar switch shows Files, with the tree of the selected product folder.
- [ ] New folder `docs/tmp` from the tree creates the folder, and it shows at once.
- [ ] A file that a terminal creates shows in the open tree in less than 1 s.
- [ ] `files.create` with `../x` returns `invalid`, and no file is made outside the product.
- [ ] Move to trash puts the file in the system trash.
- [ ] Open terminal here opens a terminal in that folder, in the space of the product.
- [ ] An ignored file shows with the ignored color, and the setting hides it.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass.
