# Plan: the setup screen

**Spec:** `docs/specs/agentsi-desktop-setup.md` · Each unit is one commit. Each Check ends with `python3 ~/dev/personal/productos/claudeconfig/scripts/devflow check --fast`. Build it after U4 of `docs/plans/agentsi-desktop-actions.md`, which adds the form part.

## U1 — Inline stage and the `products` row
- **Files:** `crates/agentsi-proto/src/roles_api.rs`, `crates/agentsi-core/src/methods/roles.rs`, `crates/agentsi-agency/src/roles/apply.rs`, `crates/agentsi-cli/src/commands/roles.rs`, `crates/agentsi-core/tests/setup.rs`, `schema/core.openrpc.json`
- **Steps:**
  1. Add `stage` to `RolesApplyParams`, and make `stage_dir` an `Option`.
  2. Refuse both, neither, and each key outside the four names of the spec.
  3. Write an inline stage to a temporary folder with mode 0700. Plan from it, then delete it.
  4. In `write_plan`, write the `products` row in the same transaction.
  5. Send `data.changed` with kind `product` after the write.
  6. Make the CLI send `Some(stage_dir)`.
  7. Write the schema again.
- **Check:** `cargo test -p agentsi-core --test setup inline_stage`, `cargo test -p agentsi-core --test setup register` and `cargo test -p agentsi-cli --test golden` pass.

## U2 — `registry.get` and `registry.update`
- **Files:** `crates/agentsi-proto/src/registry_api.rs`, `crates/agentsi-proto/src/lib.rs`, `crates/agentsi-proto/src/method.rs`, `crates/agentsi-agency/src/roles/registry_edit.rs`, `crates/agentsi-core/src/methods/registry.rs`, `crates/agentsi-core/src/methods/mod.rs`, `crates/agentsi-core/tests/registry.rs`, `schema/core.openrpc.json`
- **Steps:**
  1. Add the two methods to the catalog and the router.
  2. Edit `agentsi.config.json` in place with `registry_edit.rs`, and keep unknown keys.
  3. Write the `products` row and send `data.changed` for a product change.
  4. Make no commit.
- **Check:** `cargo test -p agentsi-core --test registry` passes.

## U3 — `setup.scan`
- **Files:** `crates/agentsi-proto/src/setup_api.rs`, `crates/agentsi-proto/src/lib.rs`, `crates/agentsi-proto/src/method.rs`, `crates/agentsi-agency/src/setup.rs`, `crates/agentsi-core/src/methods/setup.rs`, `crates/agentsi-core/src/methods/mod.rs`, `crates/agentsi-core/tests/setup.rs`
- **Steps:**
  1. Read the folder name, the four manifests, the north-star paths and the `PATH`.
  2. Find `registered` with the lookup of `product.whoami`.
  3. Add a fixture folder with a `package.json` and `docs/product.md`.
- **Check:** `cargo test -p agentsi-core --test setup scan` passes.

## U4 — Setup screen with no draft
- **Files:** `crates/agentsi-app/src/screens/setup.rs`, `crates/agentsi-app/src/screens/mod.rs`, `crates/agentsi-app/src/nav.rs`
- **Steps:**
  1. Add `ScreenKind::Setup` and `ScreenSpec::setup`.
  2. Add the steps Folder, Forms, Review and Apply.
  3. Build the stage files by `setup.md` sections 4 and 5.
  4. Add a test: the five steps with a fake core, and no write call before Confirm.
- **Check:** `cargo test -p agentsi-app screens::setup` passes.

## U5 — Places that open the setup screen
- **Files:** `crates/agentsi-app/src/sidebar.rs`, `crates/agentsi-app/src/menus.rs`, `crates/agentsi-app/src/shell/menu_actions.rs`, `crates/agentsi-app/src/actions.rs`, `crates/agentsi-app/src/screens/roles.rs`
- **Steps:**
  1. Add the button "New product", the menu entry and the palette entries.
  2. Replace "Apply stage from the clipboard" with "Update the brief".
- **Check:** `cargo test -p agentsi-app shell::menu_tests` passes. `rg -n 'read_from_clipboard' crates/agentsi-app/src/screens/roles.rs` finds nothing.

## U6 — Settings: Owner
- **Files:** `crates/agentsi-app/src/screens/settings.rs`, `crates/agentsi-app/src/screens/settings/owner.rs`, `crates/agentsi-app/src/actions.rs`
- **Steps:**
  1. Read the owner with `registry.get`, and save with `registry.update`.
  2. Add the palette entry "Open the owner settings".
- **Check:** `cargo test -p agentsi-app screens::settings` passes.

## U7 — `setup.draft` and `setup.outcome`
- **Files:** `crates/agentsi-proto/src/setup_api.rs`, `crates/agentsi-agency/src/story/claude.rs`, `crates/agentsi-agency/src/setup.rs`, `crates/agentsi-core/src/methods/setup.rs`, `crates/agentsi-store/src/clear.rs`, `crates/agentsi-core/tests/setup.rs`, `schema/core.openrpc.json`
- **Steps:**
  1. Add `budget_usd: Option<f64>` to `Call`. `None` keeps `default_budget_usd`.
  2. Read the context and run the call of the spec.
  3. Append the lines of `logs/setup.jsonl`.
  4. Make the "Logs" part of clear data delete `logs/setup.jsonl`.
- **Check:** `cargo test -p agentsi-core --test setup draft` passes with a fake `claude` on the `PATH`.

## U8 — Draft in the screen and docs
- **Files:** `crates/agentsi-app/src/screens/setup.rs`, `docs/specs/agentsi-desktop-protocol.md`, `docs/specs/agentsi-desktop-ui.md`, `docs/guide/README.md`, `CONTEXT.md`
- **Steps:**
  1. Add "Draft with Claude" with one Confirm, the "no data" marks and one repair.
  2. Call `setup.outcome` after an apply.
  3. Add the methods, the screen and the terms to the docs.
- **Check:** `cargo test -p agentsi-app screens::setup` passes. The owner registers one real product, and `agentsi roles validate` passes.
