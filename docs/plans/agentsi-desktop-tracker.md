# AgentSI desktop: the tracker, build plan

**Spec:** `docs/specs/agentsi-desktop-tracker.md` · **Branch:** `devflow wt new desktop-tracker`

Each unit is one commit. Each Check also runs `python3 ~/dev/personal/productos/claudeconfig/scripts/devflow check --fast`.

## U1 — Table and migration
- **Files:** `crates/agentsi-store/sql/0002_tickets.sql`, `crates/agentsi-store/src/{db,schema,read,clear}.rs`, `crates/agentsi-store/tests/tables.rs`, `crates/agentsi-core/tests/clear.rs`
- **Steps:**
  1. Write the table and the index of the spec in `0002_tickets.sql`, and add it to `db::MIGRATIONS`.
  2. Add `schema::LOCAL_TABLES` with `tickets`, and `schema::all()` over the two lists.
  3. Use `all()` in `read::counts` and in the schema tests. The import keeps `TABLES`.
  4. Add `tickets` to `clear::TABLES`, to the delete of the item Product, and to the core clear test.
- **Check:** `cargo test -p agentsi-store` and `cargo test -p agentsi-core --test clear --test import_postgres`.

## U2 — Rules and SQL
- **Files:** `crates/agentsi-proto/src/{lib,ticket_api}.rs`, `crates/agentsi-agency/src/lib.rs`, `crates/agentsi-agency/src/tickets/{mod,write,text}.rs`
- **Steps:**
  1. Declare `Ticket`, `Author`, `Link`, the kinds and the states in `ticket_api.rs`.
  2. Write list, get, add, update and move, with the move table, the session rule and the link checks.
  3. Write the ticket line and the markdown of `ticket.get` in `text.rs`.
  4. Test each move, the session rule, the id after `C999` and each link type.
- **Check:** `cargo test -p agentsi-agency tickets`.

## U3 — Core methods
- **Files:** `crates/agentsi-proto/src/{ticket_api,method,events}.rs`, `schema/core.openrpc.json`, `crates/agentsi-core/src/methods/{mod,tickets}.rs`, `crates/agentsi-core/tests/tickets.rs`
- **Steps:**
  1. Declare `ticket.list`, `get`, `add`, `update` and `move` in the catalog, and route them.
  2. After each write, send `data.changed` with the kind `tickets`. Name it in `DataChangedParams`.
  3. Run `AGENTSI_UPDATE_SCHEMA=1 cargo test -p agentsi-proto --test schema`.
  4. Write `tests/tickets.rs` on a real core with `common::CoreProc`.
- **Check:** `cargo test -p agentsi-proto` and `cargo test -p agentsi-core --test tickets`.

## U4 — Promote
- **Files:** U3 files, `crates/agentsi-agency/src/tickets/write.rs`, `crates/agentsi-agency/tests/ticket_promote.rs`
- **Steps:**
  1. Declare and route `ticket.promote` as the spec says. Write the schema file again.
  2. Test a story promote and a failed `story.new` with the fake PM of `story_propose.rs`.
  3. Test a decision promote and stories mode off on a real core.
- **Check:** `cargo test -p agentsi-agency --test ticket_promote` and `cargo test -p agentsi-core --test tickets`.

## U5 — MCP tools
- **Files:** `crates/agentsi-cli/src/mcp.rs`, `crates/agentsi-cli/src/mcp/{tools,format}.rs`, `crates/agentsi-cli/tests/golden/support.rs`, `crates/agentsi-cli/tests/mcp_tickets.rs`
- **Steps:**
  1. Add the four tools of the spec after the five tools of `TOOLS`.
  2. Map each tool to its core calls in `ask_core`, with the session author.
  3. Retry a write tool only when the connect failed.
  4. In `mcp_golden`, keep only the 5 TypeScript tools of the `tools/list` answer.
  5. Write `tests/mcp_tickets.rs`: one MCP session on a real core, with `AGENTSI_TERMINAL` set.
- **Check:** `cargo test -p agentsi-cli --test golden`, `cargo test -p agentsi-cli --test mcp_tickets` and `cargo test -p agentsi-cli mcp`.

## U6 — Snapshot and check
- **Files:** `crates/agentsi-agency/src/{snapshot,prompt_inputs}.rs`, `crates/agentsi-agency/src/meeting/{prompts,check}.rs`, `crates/agentsi-proto/src/story_api.rs`, `schema/core.openrpc.json`
- **Steps:**
  1. Add `## Open tickets` to the snapshot, `SPLIT_AT` and `KEEP`, as the spec says.
  2. If the snapshot has the section, add `ticket` to the source schemas and the role rule.
  3. Add the ticket ids to `SourceContext`, and resolve `ticket` in `resolve_detailed`.
  4. Add `ticket` to the comment of `Source`. Write the schema file again.
  5. Test with two open tickets and with none.
- **Check:** `cargo test -p agentsi-agency snapshot`, `cargo test -p agentsi-agency meeting` and `cargo test -p agentsi-cli --test golden`.

## U7 — Screens and key
- **Files:** `crates/agentsi-app/src/screens/{mod,tickets,ticket}.rs`, `crates/agentsi-app/src/{nav,actions,shell}.rs`
- **Steps:**
  1. Add `ScreenKind::Tickets` and `Ticket`, the names `tickets` and `ticket`, and their `ScreenSpec` constructors.
  2. Write the Tickets screen and the Ticket screen of the spec.
  3. Read again at `data.changed` with the kind `tickets` and at `core.lagged`.
  4. Add the key `tickets` (`secondary-shift-i`) and its palette entry.
- **Check:** `cargo test -p agentsi-app tickets`, `cargo test -p agentsi-app keys`, and the `rg` rule of the spec finds nothing.

## U8 — Menus and front desk
- **Files:** `crates/agentsi-app/src/{menus,actions}.rs`, `crates/agentsi-app/src/shell/{menu_actions,menu_tests}.rs`, `crates/agentsi-app/src/screens/{desk,desk_tickets}.rs`
- **Steps:**
  1. Add the product entries and `Target::Ticket` of the spec, each with a palette entry.
  2. Make "Promote to story" ask one Confirm.
  3. Write `desk_tickets.rs`, and add it as one child of `desk.rs`.
  4. Add the new entries to the menu and palette tests.
- **Check:** `cargo test -p agentsi-app`.

## U9 — Docs
- **Files:** `docs/adr/0006-each-host-keeps-its-data-in-sqlite.md`, `docs/specs/agentsi-desktop-{data,clear-data,protocol,navigation,parity}.md`, `CONTEXT.md`
- **Steps:**
  1. Write each change of the "Shared surfaces" table of the spec into these docs.
  2. Copy the terms of the spec to `CONTEXT.md`, and add the ticket row to "Context menu".
- **Check:** `rg -n "17 tables" docs` finds only text about the tables of Postgres. Then run `code-review` before the merge.
