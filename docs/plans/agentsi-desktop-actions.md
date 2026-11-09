# Plan: every action from the app

**Spec:** `docs/specs/agentsi-desktop-actions.md` · Each unit is one commit. Each Check ends with `python3 ~/dev/personal/productos/claudeconfig/scripts/devflow check --fast`. The setup screen has its own plan: `docs/plans/agentsi-desktop-setup.md`.

## U1 — Move the probe code of `agentsi doctor`
- **Files:** `crates/agentsi-agency/src/doctor.rs`, `crates/agentsi-agency/src/lib.rs`, `crates/agentsi-cli/src/commands/doctor.rs`
- **Steps:**
  1. Move `Level`, `Status`, `CheckResult`, `Probes`, `CLAUDE_FLAGS`, `CHECKS` and `checks` to `agentsi-agency`.
  2. Keep `lines`, `report`, `Machine` and the tests in the CLI.
- **Check:** `cargo test -p agentsi-cli doctor` and `cargo test -p agentsi-cli --test golden` pass.

## U2 — `core.doctor`
- **Files:** `crates/agentsi-proto/src/ops_api.rs`, `crates/agentsi-core/src/methods/ops.rs`, `crates/agentsi-core/tests/doctor.rs`, `schema/core.openrpc.json`
- **Steps:**
  1. Add `CoreDoctor` with the result of the spec.
  2. Implement `Probes` in the core. `core()` reads the state of the core, and `models()` reads its catalog.
  3. Add the method to `ops::register`.
  4. Run `AGENTSI_UPDATE_SCHEMA=1 cargo test -p agentsi-proto --test schema`.
- **Check:** `cargo test -p agentsi-core --test doctor` passes with the 10 rows in order.

## U3 — `decision.archived`
- **Files:** `crates/agentsi-proto/src/marks_api.rs`, `crates/agentsi-store/src/read.rs`, `crates/agentsi-core/src/methods/marks.rs`, `crates/agentsi-core/tests/marks.rs`, `schema/core.openrpc.json`
- **Steps:**
  1. Add a read of the decisions that `archive_records` holds with kind `decision`, newest id first.
  2. Add the method and its handler.
  3. Write the schema again.
- **Check:** `cargo test -p agentsi-core --test marks decision_archived` passes.

## U4 — Forms and the settings screen
- **Files:** `crates/agentsi-app/src/screens/parts.rs`, `crates/agentsi-app/src/screens/settings.rs`, `crates/agentsi-app/src/screens/settings/data.rs`, `crates/agentsi-app/src/actions.rs`
- **Steps:**
  1. Add a form part: text fields, choices, one write button, Cancel, `Enter` and `Escape`.
  2. Add the Health section with "Check again".
  3. Add "Back up now" to the Data section.
  4. Add the palette entries of rows 2 and 4.
- **Check:** `cargo test -p agentsi-app screens::settings` passes.

## U5 — Front desk
- **Files:** `crates/agentsi-app/src/screens/desk.rs`, `crates/agentsi-app/src/actions.rs`
- **Steps:**
  1. Replace "Pending decisions" with "Pending items" from `mark.pending`, with the keys of the spec.
  2. Add "Update the index", the form "New decision" and "Show archived" with Restore.
  3. Show "Run in Claude" in the outcome line of an approval with an action.
  4. Add the palette entries of rows 8, 15, 16 and 21.
- **Check:** `cargo test -p agentsi-app screens::desk` passes.

## U6 — Meeting screen
- **Files:** `crates/agentsi-app/src/screens/meeting.rs`, `crates/agentsi-app/src/actions.rs`
- **Steps:**
  1. Add one check box for each roster agent from `roles.list`. Send the checked ids as `roles`.
  2. Add "Ask one role" with one Confirm, and show the answer in one pane.
  3. Add "Show the context" with a Markdown panel.
- **Check:** `cargo test -p agentsi-app screens::meeting` passes.

## U7 — Stories and story screens
- **Files:** `crates/agentsi-app/src/screens/stories.rs`, `crates/agentsi-app/src/screens/story.rs`, `crates/agentsi-app/src/shell/menu_actions.rs`, `crates/agentsi-app/src/actions.rs`
- **Steps:**
  1. Add the form "New story" and "Suggest", each with one Confirm.
  2. Make the action `NewStory` open the form.
  3. Add Archive and Restore to the story screen.
  4. Replace the clipboard note of Changes with a note field and the rewrite box.
  5. Add the form "Record a merge".
- **Check:** `cargo test -p agentsi-app screens::stories` and `cargo test -p agentsi-app screens::story` pass. `rg -n 'read_from_clipboard' crates/agentsi-app/src/screens/story.rs` finds nothing.

## U8 — Roles and memory screens
- **Files:** `crates/agentsi-app/src/screens/roles.rs`, `crates/agentsi-app/src/screens/memory.rs`, `crates/agentsi-app/src/actions.rs`
- **Steps:**
  1. Open the definition of the agent of a row in a panel.
  2. Add "Also delete the memory" to the reset plan.
  3. Add "Reflect" with "Memory only" and one Confirm, and Cancel.
- **Check:** `cargo test -p agentsi-app screens::roles` and `cargo test -p agentsi-app screens::memory` pass.

## U9 — Archived products
- **Files:** `crates/agentsi-app/src/model.rs`, `crates/agentsi-app/src/sidebar.rs`, `crates/agentsi-app/src/menus.rs`, `crates/agentsi-app/src/shell/menu_actions.rs`, `crates/agentsi-app/src/actions.rs`
- **Steps:**
  1. Keep the archived products of `product.list` in `CoreState`.
  2. Add the toggle and the dimmed rows.
  3. Give an archived product one menu entry, Restore.
  4. Add a test: each new action has a palette entry, and `KEY_ACTIONS` keeps its ids.
- **Check:** `cargo test -p agentsi-app sidebar` and `cargo test -p agentsi-app shell::menu_tests` pass.

## U10 — Docs
- **Files:** `docs/specs/agentsi-desktop-ui.md`, `docs/specs/agentsi-desktop-protocol.md`, `docs/guide/README.md`, `CONTEXT.md`
- **Steps:**
  1. Remove the "no text field" rule, and add the two methods to the protocol spec.
  2. Add the new places to the guide.
  3. Copy the terms "Form" and "Health section" to `CONTEXT.md`.
- **Check:** `rg -n 'no text field' docs/specs/agentsi-desktop-ui.md` finds nothing. The owner accepts the guide text.
