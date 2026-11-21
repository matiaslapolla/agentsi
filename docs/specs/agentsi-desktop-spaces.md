# AgentSI desktop: a space for each product, pinned tabs and favorites

**Status:** approved 2026-11-21 · **Source:** owner request and decisions of 2026-11-21 ("agrupar las pestañas del top bar y accederlas desde el sidebar por proyecto, tipo Chrome"; "favoritos y pinned tabs")

## Problem

All products share one dock (`shell.rs:140`), so the top bar mixes the tabs of every product. Screen tabs (desk, tickets, git) never show in the sidebar, which lists only terminals, meetings, story sessions and reflections. A tab cannot be pinned, and the owner cannot save an item to reopen it later.

## Goal

- Each product has its own space. Selecting a product shows only its tabs in the top bar.
- The sidebar lists each open tab of a product under that product. A click selects the product and the tab.
- A tab can be pinned. A pinned tab is narrow, sits at the left of its space and cannot close by accident.
- The sidebar has a section "Favorites". A favorite reopens a screen, a folder or a terminal command.

## Non-goals

- A tab that shows in two spaces at once.
- Drag of a tab from one space to another. The tab menu has "Move to product…".
- Sync of favorites between hosts.

## Terms

- **Space:** the dock of one product: its tabs, tab groups, splits and focus. One more space, "Home", holds tabs that have no product.
- **Pinned tab:** a tab with a pin mark. It shows only its icon and its state mark.
- **Favorite:** a saved target (screen spec, folder, or terminal command with its folder) with a label.

## Design

### Spaces

- `shell.rs` keeps one `DockArea` for each space in a `BTreeMap<SpaceId, Entity<DockArea>>`. `SpaceId` is a product id or `Home`.
- `select_product` swaps the shown dock. It does not close or pause the panes of the other spaces. Their terminals keep running.
- A new tab opens in the space of the selected product. A terminal opened from a product row opens in that product's space, as today.
- `layout.json` gets version 3: `spaces: {id: {dock, tab_groups}}`. A version 2 file loads into `Home` and each panel moves to the space of its product, by `panel_key`.
- `tabs.rs` `TabGroups` becomes one per space. Its tests keep their meaning.
- Alerts of a hidden space glow on its product row, as alerts of a hidden tab glow today.

### Sidebar rows for tabs

- `tree.rs` `ItemKey` gets `Screen(ScreenKind, Option<String>)`. The rows of a product list its open tabs in the order of its top bar, pinned tabs first.
- A row shows the tab title, its color and its state mark. A click selects the space and activates the tab. The middle click closes it, except a pinned tab.
- The context menu of a tab row is the menu of the tab (`menus.rs` `Target::Tab`).

### Pinned tabs

- The tab menu and the palette have "Pin tab" and "Unpin tab".
- `tab_bar.rs` draws pinned tabs first, with icon width only. They do not join a tab group.
- Close on a pinned tab asks for a confirm. "Close others" and "Close to the right" skip pinned tabs.
- `layout.json` saves `pinned: true` in the panel entry.

### Favorites

- `<config>/agentsi/favorites.toml`: a list of `{label, product, kind, target}`. `kind` is `screen`, `folder` or `command`. Atomic writes, as `look.toml`.
- The sidebar shows "Favorites" above the products, folded when empty. A click opens the target in the space of its product.
- "Add to favorites" is in the menu of a tab, of a terminal (it saves the command and the folder) and of a folder in the file explorer (`agentsi-desktop-files.md`).
- A favorite has "Rename" and "Remove" in its menu, and can be reordered by drag.

## Acceptance criteria

- [ ] With products A and B, a terminal opened in A does not show in the top bar when B is selected, and it keeps running.
- [ ] The sidebar row of A lists its desk tab and its terminal. A click on the terminal row selects A and focuses that terminal.
- [ ] A version 2 `layout.json` loads, and each tab lands in the space of its product.
- [ ] A pinned tab sits first, has icon width, and survives "Close others".
- [ ] Close on a pinned tab asks for a confirm.
- [ ] A favorite of a terminal command reopens a terminal with that command in that folder.
- [ ] After a restart, the spaces, pinned tabs and favorites are the same.
- [ ] The idle bench draws 0 frames with 3 spaces open.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass.
