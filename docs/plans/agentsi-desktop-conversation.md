# AgentSI desktop: the conversational desk, build plan

**Specs:** `docs/specs/agentsi-desktop-conversation.md` (U1 to U7), `docs/specs/agentsi-desktop-conversation-views.md` (U8 to U10) · **Branch:** `devflow wt new desktop-conversation`

Each unit is one commit. Each Check also runs `python3 ~/dev/personal/productos/claudeconfig/scripts/devflow check --fast`. Tests use a fake `claude` on the `PATH` (see `crates/agentsi-agency/tests/claude_process.rs`) and no Claude login.

## U1 — Protocol types
- **Files:** `crates/agentsi-proto/src/{lib,method,core_api,marks_api,desk_session_api}.rs`, `schema/core.openrpc.json`
- **Steps:**
  1. Declare `View`, the items, the auth states, `Topic::Desk`, the events and the methods.
  2. Run `AGENTSI_UPDATE_SCHEMA=1 cargo test -p agentsi-proto --test schema`.
- **Check:** `cargo test -p agentsi-proto`.

## U2 — Desk tables
- **Files:** `crates/agentsi-store/sql/0002_desk.sql`, `crates/agentsi-store/src/{db,schema,clear,desk}.rs`, `crates/agentsi-store/tests/tables.rs`, `crates/agentsi-core/tests/clear.rs`
- **Steps:**
  1. Write the migration. The merge sets its number.
  2. Add the tables to `schema::LOCAL_TABLES`. Create the list if the tracker has not merged.
  3. Add the tables to `clear::TABLES` before `products`.
  4. Write the store functions.
- **Check:** `cargo test -p agentsi-store` and `cargo test -p agentsi-core --test clear`.

## U3 — Desk process and auth check
- **Files:** `crates/agentsi-agency/src/desk/{mod,args,stream,auth}.rs`, `crates/agentsi-agency/src/desk/system.md`, `crates/agentsi-agency/tests/desk_process.rs`
- **Steps:**
  1. Build the arguments of the spec.
  2. Map each stream line to a delta or an item.
  3. Map `claude auth status --json` to an auth state.
  4. Test with a fake `claude` for `auth status` and stdin lines.
- **Check:** `cargo test -p agentsi-agency --test desk_process`.

## U4 — Core methods and desk notes
- **Files:** `crates/agentsi-core/src/desk/`, `crates/agentsi-core/src/methods/{mod,desk_session,marks}.rs`, `crates/agentsi-core/src/server.rs`, `crates/agentsi-core/tests/desk_session.rs`
- **Steps:**
  1. Keep one process for each product, with the time limits.
  2. Store the items, send the events and check `desk.show`.
  3. Call `desk::note` after each write of `methods/marks.rs`.
  4. Stop the desk processes with the core.
  5. Test two messages, a restart, a desk note, `busy` and `signed_out`.
- **Check:** `cargo test -p agentsi-core --test desk_session`.

## U5 — Show tools
- **Files:** `crates/agentsi-cli/src/mcp.rs`, `crates/agentsi-cli/src/mcp/desk.rs`, `crates/agentsi-cli/tests/mcp_desk.rs`
- **Steps:**
  1. With `AGENTSI_DESK_SESSION`, list a `show_<kind>` tool for each variant, after the base tools.
  2. Send `desk.show`, and return errors with `isError`.
- **Check:** `cargo test -p agentsi-cli --test mcp_desk` and `cargo test -p agentsi-cli --test golden`.

## U6 — Desk conversation in the app
- **Files:** `crates/agentsi-app/src/screens/{desk,desk_conversation}.rs`, `crates/agentsi-app/src/{actions,model}.rs`
- **Steps:**
  1. Subscribe to `desk`. Draw the items with `MessageScroller` and a streamed `TextView`.
  2. Draw the decisions view. Read again on `data.changed`.
  3. Add the Confirms, the auth messages and the palette entry.
- **Check:** `cargo test -p agentsi-app desk_conversation`. The owner checks the spec criteria.

## U7 — Docs
- **Files:** `docs/specs/agentsi-desktop-{protocol,data,clear-data}.md`, `docs/adr/0006-each-host-keeps-its-data-in-sqlite.md`, `CONTEXT.md`
- **Steps:**
  1. Add the methods and tables to the protocol spec, ADR 0006 and the data specs.
  2. Copy the terms to `CONTEXT.md`.
- **Check:** `rg -l desk_items docs/adr docs/specs` lists the changed docs. `code-review` passes.

## U8 — Milestone 2: object views
- **Files:** `crates/agentsi-proto/src/desk_session_api.rs`, `schema/core.openrpc.json`, `crates/agentsi-core/src/desk/`, `crates/agentsi-agency/src/desk/args.rs`, `crates/agentsi-cli/src/mcp/desk.rs`, `crates/agentsi-app/src/screens/desk_views.rs`
- **Steps:**
  1. Add the three variants, their checks and their allowed tools.
  2. Draw each view. Read again on its `data.changed` kinds.
- **Check:** `cargo test -p agentsi-proto`, `cargo test -p agentsi-core --test desk_session` and `cargo test -p agentsi-app desk_views`.

## U9 — Milestone 3: generic views
- **Files:** the files of U8
- **Steps:**
  1. Add `Table`, `Chart`, `Checklist` and `Page`, with their limits.
  2. Add `desk.check` and `desk.page`.
  3. Open the page from a 0600 file.
- **Check:** the commands of U8. `cargo tree -p agentsi-app | grep -E 'wry|webkit2gtk'` prints nothing.

## U10 — Milestone 4: tickets, after the tracker merges
- **Files:** the files of U8
- **Steps:**
  1. Rebase on `main`.
  2. Add `Tickets`, its `ticket.get` check and the ticket read tools.
  3. Draw the view. Read again on kind `tickets`.
- **Check:** the commands of U8. A test finds no ticket write tool in the desk arguments.
