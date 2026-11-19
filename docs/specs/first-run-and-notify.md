# A first install sets up its first product, and the system notifications name AgentSI

**Status:** approved 2026-11-19 · **Source:** owner checks of 2026-11-19 on an isolated core and app, owner request of 2026-11-19

## Problem

A first install has a registry with no products and no `agentsi.db`. The Review of the setup stops with "the data root has no agentsi.db. Import the data first.", and with a database it stops with "Registry has no one-line products to append to.". So a new owner cannot set up a product. The system notifications show the program name `agentsi-app`.

## Goal

- The setup of a first install writes its first product with no import and no edit by hand.
- Each core method that needed an imported database makes the file on first use.
- A system notification names AgentSI.

## Non-goals

- A change to `import-postgres`. Without `--fresh` it imports into an existing file.
- A change to the system notification rules. The owner checks of 2026-11-19 showed that they work: a Blocked alert with the window not active posts a notification over D-Bus.

## Design

### The database on first use

- `Core::store_or_create()` in `crates/agentsi-core/src/state.rs` gives the store, and it opens the database file when the file does not exist. `Store::open` makes and migrates the file.
- Each method that stopped with "Import the data first" calls `store_or_create()`: agency, call, desk sessions, knowledge, marks, meeting, reflect, roles, story and tickets.
- `Core::store()` keeps `Ok(None)` while the file does not exist. The readers that show no rows then stay as they are: the product list, the minutes list, the desk history, the registry edit, the status of the ops and the backup.
- `product.desk` and `minutes.get` with no file give `NotFound`, the same as an unknown product.

### The first product

- `registry_edit::add_product` writes the whole registry again when it has no product. Each top-level key goes on one line, and each product goes on its own line inside `"products": [ ]`.
- A registry with products that are not one line each stays an error, because the edit would lose its format.

### The name of the notifications

- `main.rs` calls `cx.set_app_identity("agentsi-app", "AgentSI")` before the window opens.

## Acceptance criteria

- [ ] `add_product` on the registry of a first install, `{ "owner": …, "products": [] }` on one line, gives a registry with the product on its own line.
- [ ] `add_product` on a registry whose products are not one line each still gives "Registry has no one-line products to append to.".
- [ ] A core method that writes, such as `ticket.add` or `roles.apply`, makes `agentsi.db` when the data root has none.
- [ ] `product.list` with no `agentsi.db` gives no products and makes no file.
- [ ] No source file of the core holds "Import the data first".
- [ ] The setup of a first install, from "Read the folder" to "Confirm", ends on the front desk of the product with no file edited by hand.
- [ ] A system notification of a Blocked alert shows the app name AgentSI.
