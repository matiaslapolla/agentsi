# A first install sets up its first product, and the system notifications name AgentSI

**Status:** approved 2026-11-19 · **Source:** owner checks of 2026-11-19 on an isolated core and app, owner request of 2026-11-19

## Problem

A first install has a registry with no products and no database file. The Review step of the setup screen stops with "Import the data first", and with a database file it stops with "Registry has no one-line products to append to.". The system notifications show the program name `agentsi-app`.

## Goal

- The setup of a first install writes its first product with no import and no edit by hand.
- Each core method that needed an imported database makes the file on first use.
- A system notification names AgentSI.

## Non-goals

- A change to the system notification rules. The owner checks of 2026-11-19 showed that they work.
- `import-postgres --fresh` after the first use. The core then has a database file, and the import without `--fresh` imports into it.
- A backup soon after the core makes the database file. The next nightly backup takes it.

## Terms

- **Database file**: `agentsi.db` in the data root. It holds the rows of the agency, such as the products, the briefs and the tickets.

## Design

### The database on first use

- `Core::store_or_create()` in `crates/agentsi-core/src/state.rs` opens the database file, and `Store::open` makes and migrates it when it does not exist.
- `store_or_create()` makes no file in a folder without `agentsi.config.json`. A wrong `AGENTSI_HOME` gets an error.
- Each method that stopped with "Import the data first" calls `store_or_create()`, reads and writes alike: agency, call, the desk session runs, knowledge, marks, meeting, reflect, roles, story and tickets.
- `Core::store()` keeps `Ok(None)` while the file does not exist. These readers show no rows then: the product list, the minutes list, `desk.history`, the registry edit, the status of the ops and the backup.
- `product.desk` and `minutes.get` with no file give `NotFound`, the same as an unknown product.

### The first product

- `registry_edit::add_product` writes the whole registry again when it has no product. Each top-level key goes on one line. Each product goes on its own line inside `"products": [ ]`.
- A registry with products that are not one line each stays an error, because the edit would lose its format.

### The name of the notifications

- `main.rs` calls `cx.set_app_identity("agentsi-app", "AgentSI")` before the window opens.

## Acceptance criteria

- [ ] `add_product` on the registry of a first install, `{ "owner": …, "products": [] }` on one line, gives a registry with the product on its own line.
- [ ] `add_product` on a registry whose products are not one line each still gives "Registry has no one-line products to append to.".
- [ ] A core method of the list, such as `roles.apply` or `mark.set`, makes `agentsi.db` when the data root has none.
- [ ] `mark.set` on a folder with no `agentsi.config.json` gives an error and makes no file.
- [ ] `product.list` with no `agentsi.db` gives no products and makes no file.
- [ ] No source file of the core holds "Import the data first".
- [ ] The setup screen of a first install, from the Folder step to the Confirm, ends on the front desk of the product with no file edited by hand.
- [ ] A system notification of a Blocked alert shows the app name AgentSI.
