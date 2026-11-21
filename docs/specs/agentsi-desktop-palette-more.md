# AgentSI desktop: more actions in the palette

**Status:** approved 2026-11-21 · **Source:** owner request of 2026-11-21 ("más acciones por el menú de Ctrl + K")

## Problem

The palette has 105 entries (`actions.rs:394`), but it cannot find an open tab, a settings section, a file or a recent item. Each entry is a fixed action, so the palette has no second step for a choice.

## Goal

- The palette finds open tabs, settings sections, files of the selected product, favorites and recent items, besides its actions.
- An action can ask for a second step: a list (switch branch, move to product) or a text (new branch name).
- Each feature of this batch adds its own entries. This spec adds the shared parts.

## Non-goals

- Plugins or owner-made palette commands.
- A shell command runner in the palette.

## Terms

- **Palette source:** one kind of result in the palette: actions, tabs, settings, files, favorites, recent.
- **Prefix:** a first character that limits the palette to one source.

## Design

- `shell.rs` palette code moves to `palette.rs` with a `PaletteSource` trait: `fn items(&self, query, cx) -> Vec<PaletteItem>`, `fn run(&self, item, window, cx)`.
- Sources:
  - Actions: the current `ENTRIES`, with the same ranking.
  - Tabs: the open tabs of all spaces, with the product name. Run activates the space and the tab.
  - Settings: each settings section (`agentsi-desktop-settings-nav.md`).
  - Files: the files of the selected product, from `git ls-files` plus new files, read by the core when the palette opens with the prefix.
  - Favorites and recent: the favorites (`agentsi-desktop-spaces.md`) and the last 20 opened targets, saved in `layout.json`.
- With no query, the palette shows recent items, then actions.
- Prefixes: `>` actions, `@` tabs, `#` settings, `/` files. The palette footer shows them.
- `PaletteStep` lets an action push a second step (a list or a text input). Escape goes back one step.
- New entries of this spec: "Go to tab…", "Reopen closed tab" (`Ctrl+Shift+T`), "Next space" and "Previous space" (`Ctrl+Tab` between products), "Copy product path", "Open product folder in system", "Restart core", "Reload settings files".

## Acceptance criteria

- [ ] `@build` lists the open tab "build" of each product, and Enter focuses it in its space.
- [ ] `#term` lists the Terminal settings section, and Enter opens it.
- [ ] `/main.rs` lists the files with that name in the selected product.
- [ ] With an empty query, the first items are the recent targets.
- [ ] "Git: Switch branch…" shows a second step with the branches, and Escape returns to the first step.
- [ ] "Reopen closed tab" reopens the last closed tab in its space.
- [ ] Each of the 105 old entries is still found by its old keywords (a test lists them).
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass.
