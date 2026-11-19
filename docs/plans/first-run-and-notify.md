# Plan: a first install sets up its first product

Spec: `docs/specs/first-run-and-notify.md`. One commit for each unit.

## U1 — The database on first use
- **Files:** `crates/agentsi-core/src/state.rs`, `crates/agentsi-core/src/methods/{agency,call,desk,knowledge,marks,meeting,reflect,roles,story,tickets}.rs`, `crates/agentsi-core/src/desk/mod.rs`
- **Steps:**
  1. Add `Core::store_or_create()`.
  2. Call it in each method that stopped with "Import the data first".
  3. Map a missing file to `NotFound` in `product.desk` and `minutes.get`.
  4. Test that a write makes the file and that `product.list` does not.
- **Check:** `cargo test -p agentsi-core` passes.

## U2 — The first product of an empty registry
- **Files:** `crates/agentsi-agency/src/roles/registry_edit.rs`
- **Steps:**
  1. Write the registry again in the one-product-per-line format when it has no product.
  2. Test the one-line registry of `install.sh` and a registry with multi-line products.
- **Check:** `cargo test -p agentsi-agency registry_edit` passes.

## U3 — The name of the system notifications
- **Files:** `crates/agentsi-app/src/main.rs`
- **Steps:**
  1. Call `cx.set_app_identity` before the window opens.
- **Check:** a Blocked alert with the window not active posts a notification whose app name is AgentSI.

## U4 — Setup of a first install, end to end
- **Files:** none
- **Steps:**
  1. Install the new programs into an isolated data root with an empty registry.
  2. Run the setup screen from the Folder step to the Confirm.
- **Check:** the front desk of the product opens, and no file was edited by hand.
