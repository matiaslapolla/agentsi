# AgentSI desktop: the tracker

**Status:** draft · **Source:** `docs/ideas/desktop-tracker.md`, `docs/ideas/desktop-tracker-eval.md`, owner decisions of 2026-11-09

## Problem

A Claude Code session keeps its open ends in `.claude/JOURNAL.md`, and the next session writes over them. The owner has no place in AgentSI for a note, a to-do or a question that gates nothing. The synthesis of a meeting never sees them.

## Goal

- The core keeps tickets in `agentsi.db` and serves them through the `ticket` methods.
- A Claude Code session reads and writes tickets through four tools of `agentsi mcp`.
- The app shows tickets on two screens, in menus, in the palette and on the front desk.
- Each meeting sees the open tickets in the snapshot, and a priority can cite a ticket.

## Non-goals

- A change to the TypeScript programs or to `plugin/`. The module lands in the Rust programs only (exception under Non-goals of `agentsi-desktop-parity.md`).
- An `agentsi ticket` command.
- Product docs (ADRs, specs) as tickets. They stay in a Claude pane.
- A sync with GitHub Issues. The link type `issue` keeps the place for it.
- Tickets in `search`, in the knowledge base or in a reflection.
- An archive kind `ticket`. The state `dropped` hides a ticket.
- Links that the owner adds or removes in the app.

## Terms

- **Open ticket:** a ticket in the state `open` or `waiting`.
- **Tickets screen:** the screen of kind `Tickets` that lists the tickets of one product.
- **Ticket screen:** the screen of kind `Ticket` that shows one ticket. Its id is the argument of the panel.
- **Ticket line:** the one-line text of a ticket: `<id> [<kind>[, waiting on <waiting_on>]] <title>[ (for <assignee>)]`.

## Design

### Table `tickets`

The tracker migration adds this table. Its key is `(product, id)`, and the index `tickets_product_state_idx` covers `(product, state)`.

| Column | SQLite | Rule |
|---|---|---|
| `product` | `TEXT NOT NULL` | A product of the registry |
| `id` | `TEXT NOT NULL` | `C` and 3 or more digits. The next id is the highest number of the product plus 1, from `C001` |
| `scope` | `TEXT` | A repo scope, as `documents.scope` |
| `kind` | `TEXT NOT NULL CHECK` | `note`, `todo` or `question` |
| `state` | `TEXT NOT NULL CHECK` | `open`, `waiting`, `done` or `dropped` |
| `title` | `TEXT NOT NULL` | 1 to 120 characters after trim |
| `body` | `TEXT NOT NULL DEFAULT ''` | Markdown |
| `waiting_on` | `TEXT` | Not empty in `waiting`, null in the other states |
| `assignee` | `TEXT` | `owner`, `claude`, an agent id of the roster, or null |
| `author`, `updated_by` | `TEXT NOT NULL`, `json_valid` | `{by, id}`. `by` is `owner`, `agent` or `session` |
| `links` | `TEXT NOT NULL DEFAULT '[]'`, `json_valid` | A list of `{type, ref}` |
| `objective` | `TEXT` | An objective id of the brief |
| `created_at`, `updated_at` | `TEXT NOT NULL` | ISO 8601 UTC with 3 fraction digits |
| `closed_at` | `TEXT` | Set in `done` and `dropped`, null in the other states |

`schema::TABLES` keeps the 17 tables of Postgres, and the import reads only that list. A new list `schema::LOCAL_TABLES` holds `tickets`. `read::counts`, the restore drill and the schema tests read the two lists. `clear::TABLES` adds `tickets` before `products`, so the clear items "Product" and "Everything" delete tickets.

| Caller | `author` and `by` |
|---|---|
| The app | `{by: "owner"}`. The core writes the name of the owner block as `id`, or `owner` when the block has no name |
| `agentsi mcp` | `{by: "session", id}`. The id is `AGENTSI_TERMINAL`, else `CLAUDE_CODE_SESSION_ID`, else `unknown` |
| No caller in this spec | `{by: "agent", id}` |

| Link type | Ref | The core checks that it exists | The app opens |
|---|---|---|---|
| `story` | `S014` | Yes | The Story screen |
| `decision` | `42` | Yes | The front desk |
| `minutes` | A minutes id | Yes | The Minute screen |
| `priority` | `<minutes id>#<idx>` | Yes, a `priorities` row | The Minute screen |
| `ticket` | `C007` | Yes, in the same product | The Ticket screen |
| `url` | An http(s) URL | No | The browser |
| `commit`, `path`, `issue` | As in a minutes source | No | Nothing. A click copies the ref |

### States

| From | To | Rule |
|---|---|---|
| `open` | `waiting` | `waiting_on` is necessary |
| `waiting` | `open` | The core clears `waiting_on` |
| `open`, `waiting` | `done`, `dropped` | The core sets `closed_at` and clears `waiting_on` |
| `done`, `dropped` | `open` | The core clears `closed_at` |

A move to the same state succeeds, and only a new `waiting_on` changes the row. Each other move returns `invalid`.

A session adds any ticket. A session updates, moves or promotes only these tickets:

- A ticket with the author `{by: "session", id}` of that session. The id `unknown` matches no ticket.
- A ticket with the assignee `claude`.

For another ticket, the core returns `conflict` with the hint `Ticket <id> belongs to the owner.` The owner changes each ticket.

### Core methods

`crates/agentsi-proto/src/ticket_api.rs` declares the methods and the type `Ticket`, which has each column. `crates/agentsi-agency/src/tickets/` holds the rules and the SQL. `crates/agentsi-core/src/methods/tickets.rs` routes them. Each write runs in one transaction on the writer thread. After the commit, the core sends `data.changed {product, kind: "tickets"}`.

| Method | Params | Result | Rules |
|---|---|---|---|
| `ticket.list` | `product, states?, kind?, assignee?, author?, link?, limit?` | `{tickets}` | Order: the state (`open`, `waiting`, `done`, `dropped`), then the newest `updated_at`, then the highest id. `states` is `[open, waiting, done]` by default. `author` filters by `by`. `limit` is 1 to 200, 50 by default |
| `ticket.get` | `product, id` | `{ticket, markdown}` | The markdown has the form below |
| `ticket.add` | `product, kind, title, body?, assignee?, links?, scope?, objective?, author` | `{ticket}` | The state is `open` |
| `ticket.update` | `product, id, title?, body?, kind?, assignee?, links?, scope?, objective?, by` | `{ticket}` | Changes only the given fields. An empty `assignee`, `scope` or `objective` clears the field. `links` replaces the list |
| `ticket.move` | `product, id, state, waiting_on?, by` | `{ticket}` | The table of "States" |
| `ticket.promote` | `product, id, to, by` | `{ticket, story?, decision?}` | Only an open ticket. `to: "story"` calls the handler of `story.new` with the title and the body as `text`, and spends money. `to: "decision"` calls `decide::add` with the title as `text` and the body, or `From ticket <id>.`, as `why`. Then one transaction adds the link and moves the ticket to `done`. A failed `story.new` leaves the ticket as it was |

Each method returns `invalid` for a bad value, and `not_found` for an unknown product, ticket, link target or objective.

The markdown of `ticket.get`:

```markdown
# C012 Install the aarch64 bundle

- Kind: todo · State: waiting (on Matias) · Assignee: owner
- Author: session 3f2a · Last write: owner Matias
- Links: story S014, commit 1a2b3c4
- Created: 2026-11-09T10:00:00.000Z · Updated: 2026-11-09T11:00:00.000Z

<body>
```

### MCP tools

`crates/agentsi-cli/src/mcp/tools.rs` adds four tools after the five tools of today. Each tool also takes the optional `product`.

| Tool | Arguments | Core calls | Answer |
|---|---|---|---|
| `list_tickets` | `state?`, `kind?`, `assignee?`, `limit` (1 to 200, default 20) | `ticket.list`, with `states` = `[open, waiting]` when `state` is absent | `Tickets of <product> (<n>):`, then one ticket line each, or `No tickets.` |
| `get_ticket` | `id` | `ticket.get` | The markdown |
| `add_ticket` | `kind`, `title`, `body?`, `assignee?`, `links?` | `ticket.add` | `Added <ticket line>` |
| `update_ticket` | `id`, `title?`, `body?`, `kind?`, `state?`, `waiting_on?`, `assignee?`, `links?` | `ticket.update`, then `ticket.move` when `state` is given | The ticket line |

The descriptions carry the three rules of the eval file, section "MCP tools". `run_tool` tries a write tool again only when the connection to the core failed before the request.

### Meetings

- `data.snapshot` adds `## Open tickets` after `## Open issues` only when the product has an open ticket. It has 30 lines `- <ticket line>` at most, in the order of `ticket.list`, then `_<n> more._`. With a scope, the section lists the tickets of that scope and the tickets with no scope.
- `prompt_inputs.rs` adds `## Open tickets` to `SPLIT_AT` and `KEEP`, with a limit of 1500 characters.
- If the snapshot text has the line `## Open tickets`, the source schemas and the rule text of the role prompt add the type `ticket`. Otherwise they stay as today, so the `meet-*` golden files do not change.
- The check resolves a `ticket` source when the product has a ticket with that id.

### App

| Place | It shows | Actions |
|---|---|---|
| Tickets screen (`screens/tickets.rs`) | The groups Open, Waiting, and Done of the last 14 days. Dropped shows only with its filter. Each row has the kind, the id, the title, the author, the assignee, the link count and the age | An add field with a kind choice. Filters by kind, state, assignee and author |
| Ticket screen (`screens/ticket.rs`) | Each field, with the body as Markdown | Edit of the title, the body, the kind and the assignee (`owner`, `claude` or none). Done, Wait, Drop, Reopen, Promote to story, Promote to decision |
| Front desk (`screens/desk_tickets.rs`, one child of `desk.rs`) | Open tickets for the owner, and session tickets of the last 24 h, 8 rows at most | Open a ticket. Open the Tickets screen |
| Product menu, after "New story" | "New ticket", "Open tickets" | |
| Ticket menu (a row of the Tickets screen, the Ticket screen) | Open, Done, Wait, Drop, Reopen, Promote to story, Promote to decision, Copy id | |
| Palette | One entry for each menu entry, for example "New ticket for the active product" and "Open the tickets of the active product" | |
| Key | `tickets` = `secondary-shift-i` opens the Tickets screen | |

- "New ticket" and "Wait" open the prompt with one line of text. "New ticket" adds a `note`.
- "Promote to story" asks one Confirm, because it spends money. "Drop" asks none, because "Reopen" undoes it.
- An entry that the state of the ticket does not permit is off.
- A screen reads its data again at `data.changed` with the kind `tickets` for its product, and at `core.lagged`.
- The screens use tokens only. The idle app draws 0 frames. A product on an SSH host reads the core of that host.

### Defaults taken

The owner did not answer these questions of the eval file, so the spec takes its recommendations:

| Question | Default |
|---|---|
| Q1 | The restatement of the brief stands |
| Q3 | The kinds are `note`, `todo` and `question`. "Pending" is the state `waiting` |
| Q4 | Each ticket belongs to one product |
| Q5 | The session rule of "States" |
| Q6 | The `mcp-*` golden test compares only the 5 tools of TypeScript |
| Q7 | `search` does not find tickets |
| Q8 | No role and no synthesis writes a ticket |
| Q9 | The sidebar shows no ticket count and no item row |

### Shared surfaces

| Surface | Change | Merge rule |
|---|---|---|
| Protocol methods | Namespace `ticket` with 6 methods, in the catalog (`method.rs`), the router (`methods/mod.rs`) and the Methods table of the protocol spec | Keep the lines of each module |
| Events | `data.changed` gets the kind `tickets` | No schema change |
| `schema/core.openrpc.json` | Has the `ticket` methods | Do not merge it by hand. Run `AGENTSI_UPDATE_SCHEMA=1 cargo test -p agentsi-proto --test schema` after the rebase |
| Migration | `crates/agentsi-store/sql/<N>_tickets.sql`, the tracker migration, number set at merge. It is 2 in the worktree | Take the next free number on main, in the file name and in `db::MIGRATIONS` |
| Table lists | `schema::LOCAL_TABLES`, `clear::TABLES`, the table lists of `tests/tables.rs` and `agentsi-core/tests/clear.rs` | Add names. The counts follow the lists |
| Table count in docs | ADR 0006, the data spec and the clear data spec name the added tables and give no new total | Add names |
| MCP tools | `TOOLS` grows from 5 to 9. `mcp_golden` keeps only the 5 tools of TypeScript in `tools/list`. The parity spec, section "MCP tools" and its criterion, says so | A `show_*` tool of the desk module comes after the ticket tools |
| Navigation | `ScreenKind::Tickets` and `Ticket` (layout names `tickets`, `ticket`), the menus, the palette, the table of the navigation spec, the Context menu entry of `CONTEXT.md` | Keep the entries of each module. The menu and palette tests list them all |
| Keys | The id `tickets`, `secondary-shift-i` | No other module takes this key |
| Front desk | One child view in `desk.rs` | The desk module keeps it, or shows tickets through `ticket.list` |
| Sidebar | No change | |
| Golden tests | No `.golden` or `.rows` file changes | |
| Desk module views | None | A `show_*` view of tickets reads `ticket.list`. The field names of `Ticket` are its contract |

## Acceptance criteria

- [ ] `cargo test -p agentsi-store` passes. The schema test finds the 17 tables of Postgres, `tickets` and `tickets_product_state_idx`.
- [ ] A database of main before this module opens with the tracker migration, and `select count(*) from tickets` returns 0.
- [ ] `cargo test -p agentsi-core --test tickets` passes. It covers each method, each move, an illegal move, the session rule, `data.changed`, and a failed write.
- [ ] `cargo test -p agentsi-proto` passes, and `schema/core.openrpc.json` lists the six `ticket` methods.
- [ ] `tools/list` of `agentsi mcp` gives the 5 tools of today with unchanged schemas, then `list_tickets`, `get_ticket`, `add_ticket` and `update_ticket`.
- [ ] `cargo test -p agentsi-cli --test golden` passes, and `git diff --stat` shows no `.golden` or `.rows` file.
- [ ] `cargo test -p agentsi-cli --test mcp_tickets` passes. A session adds a ticket with the author id of `AGENTSI_TERMINAL`, and gets `conflict` when it drops a ticket of the owner.
- [ ] A snapshot test with two open tickets shows `## Open tickets` with two lines, and the role prompt lists the type `ticket`. With no open ticket, the snapshot and the prompt stay as today.
- [ ] `agentsi check` passes on minutes that cite a ticket that exists, and fails on an unknown ticket id.
- [ ] `cargo test -p agentsi-core --test clear` passes, and the clear plan of a product counts its `tickets` rows.
- [ ] `cargo test -p agentsi-app` passes. Each new menu entry runs its action and has a palette entry, and the key `tickets` opens the Tickets screen.
- [ ] `rg -n 'rgb\(|hsla\(|px\(' crates/agentsi-app/src` finds nothing.
- [ ] A Claude pane calls `add_ticket`. In 1 s or less, the owner sees the ticket on the Tickets screen of its product.
- [ ] The docs that "Shared surfaces" names contain its changes.
