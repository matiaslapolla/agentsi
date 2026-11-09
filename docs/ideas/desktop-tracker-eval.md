# Desktop tracker: evaluation

**Brief:** `docs/ideas/desktop-tracker.md` · **Date:** 2026-11-09

This file holds the detail of the brief: the data model, the core methods, the MCP tools, the screens, the overlap with other terms, and the open questions for the owner. "Ticket" is the term that the owner chose (Q2).

## Restatement and gaps

The owner did not accept the restatement of the brief yet (Q1). The restatement assumed these points:

- "End user" in the words of the owner means the owner (`CONTEXT.md:9-11`).
- "Claude" means a Claude Code session with the AgentSI plugin. It does not mean a role in a meeting (Q8).
- Each ticket belongs to one product (Q4).
- "Pendings" means a ticket in the state `waiting`. It is not a kind, because "Pending item" is already a term (`CONTEXT.md:83-85`) (Q3).
- A ticket lives on the host of its product. ADR 0006 has no sync between devices (`docs/adr/0006-each-host-keeps-its-data-in-sqlite.md:22-24`).

## Data model

### Table `tickets`

The core keeps the tickets in a new table of `agentsi.db`. Only the core opens the file (`docs/adr/0006-each-host-keeps-its-data-in-sqlite.md:12`).

| Column | Type | Rule |
|---|---|---|
| `product` | Text | Required. A product of the registry. |
| `id` | Text | `C001`, `C002`, and so on, for each product. Stories use `S014` and `T015` (`docs/specs/agentsi-stories.md:32`). |
| `scope` | Text | Optional. A repo scope, as in `documents.scope` (`crates/agentsi-store/src/schema.rs:78`). |
| `kind` | Text | `note`, `todo` or `question`. |
| `state` | Text | `open`, `waiting`, `done` or `dropped`. |
| `title` | Text | 1 to 120 characters. |
| `body` | Text | Markdown. It can be empty. |
| `waiting_on` | Text | Who or what the ticket waits for. Required in the state `waiting`, empty in the other states. |
| `assignee` | Text | `owner`, `claude`, an agent id such as `dev.backend`, or null. |
| `author` | Json | `{by, id}`. See "Author". |
| `updated_by` | Json | The same shape as `author`, for the last write. |
| `links` | Json | A list of `{type, ref}`. See "Links". |
| `objective` | Text | Optional. An objective id of the brief, as a priority has (`crates/agentsi-store/src/schema.rs:108`). |
| `created_at`, `updated_at`, `closed_at` | Time | ISO 8601 UTC text. `closed_at` is set in `done` and `dropped`. |

The key is `(product, id)`.

### Kinds

| Kind | What it holds | Example from the seed cases |
|---|---|---|
| `note` | A fact to keep. Nobody must act on it. | "We build in the container, not on the host." |
| `todo` | Something to do, by the owner or by Claude. | "Install the aarch64 bundle." |
| `question` | Something to find out. It asks for information and gates no change. | "Does the bundle run on glibc 2.38?" |

The kind `todo` does not use the word "task", because **Task** is a story with no user (`CONTEXT.md:113-115`). The _Avoid_ line of Pending item lists "Todo" (`CONTEXT.md:85`). That line stays correct: a pending item is not a `todo` ticket. Q3 asks the owner to accept these kinds.

### States

| State | Meaning |
|---|---|
| `open` | New or active. |
| `waiting` | Blocked on `waiting_on`. This state is the "pending" of the owner. |
| `done` | Finished. For a note, the owner used it or filed it. |
| `dropped` | Closed with no result. The Tickets screen hides it behind a filter. |

The legal moves are:

- From `open` to `waiting`, with a `waiting_on` text.
- From `waiting` to `open`.
- From `open` or `waiting` to `done` or `dropped`.
- From `done` or `dropped` to `open` (reopen).

### Author

The `author` field names who wrote the ticket. It has three forms:

- `{by: "owner", id: <the name of the owner block of the registry>}`. The app writes this form.
- `{by: "agent", id: <agent id>}`. A role writes this form. No role writes tickets in the first version (Q8). `ticket.promote` keeps the form for a later version.
- `{by: "session", id: <id>}`. `agentsi mcp` writes this form. The id is `AGENTSI_TERMINAL` for a pane of the app (`docs/specs/agentsi-desktop-protocol.md:118`), else `CLAUDE_CODE_SESSION_ID` (`docs/specs/agentsi-desktop-protocol.md:153`), else `unknown`. An open test of the brief asks if the MCP process gets that variable.

A story session writes no ticket. It runs in a sandbox with no network (`docs/specs/agentsi-desktop-protocol.md:296`), and its result is a diff, not a ticket.

### Links

A link has the shape of a minutes source (`crates/agentsi-proto/src/story_api.rs:56-61`). The types are:

| Type | Ref | Opens |
|---|---|---|
| `story` | `S014` | The Story screen |
| `decision` | `42` | The front desk |
| `minutes` | a minutes id | The Minute screen |
| `priority` | `<minutes id>#<idx>`, the key of `priorities` (`crates/agentsi-store/src/schema.rs:110`) | The Minute screen |
| `ticket` | `C007` | The Ticket screen |
| `commit`, `path`, `url`, `issue` | As in a minutes source (`crates/agentsi-agency/src/meeting/prompts.rs:24`) | The Git screen, the file or the browser |

`ticket.add` and `ticket.update` return `not_found` for a link to a story, a decision, a minutes, a priority or a ticket that does not exist. They do not resolve `commit`, `path`, `url` and `issue` links.

### Storage rules

- The schema list says "The 17 tables" (`crates/agentsi-store/src/schema.rs:1`), and ADR 0006 keeps 17 tables (`docs/adr/0006-each-host-keeps-its-data-in-sqlite.md:13`). `tickets` is the 18th table. The schema test and the comment change with it.
- `import-postgres` only inserts (`docs/adr/0006-each-host-keeps-its-data-in-sqlite.md:20`). Postgres has no tickets, so the import does not change.
- Clear data deletes "each row of the 17 tables" of a product (`docs/specs/agentsi-desktop-clear-data.md:30`). The clear plan must count the `tickets` rows too.
- The daily backup copies `agentsi.db`, so it holds the tickets with no change.
- There is no archive kind `ticket`. The state `dropped` hides a ticket. `archive.archive` keeps its four kinds (`docs/specs/agentsi-desktop-protocol.md:279`).

## Core methods

Each feature needs a method and a schema entry (`docs/adr/0007-a-headless-core-serves-the-app-over-json-rpc.md:20`). The `ticket` namespace adds six methods to `schema/core.openrpc.json`. Each write runs in one SQLite transaction on the writer thread, and then sends `data.changed {product, kind: "tickets"}`, as the writes of U20 do (`docs/specs/agentsi-desktop-protocol.md:261`).

| Method | Returns | Rules |
|---|---|---|
| `ticket.list {product, kind?, state?, assignee?, link?, limit?}` | `{tickets}` | `open` and `waiting` first, then the newest `updated_at`. `limit` is 50 by default, 200 at most. |
| `ticket.get {product, id}` | `{ticket, markdown}` | `not_found` for an unknown id. |
| `ticket.add {product, kind, title, body?, assignee?, links?, author}` | `{id}` | `invalid` for an empty or long title, or a bad kind. `not_found` for an unknown product or link. |
| `ticket.update {product, id, title?, body?, kind?, assignee?, links?, by}` | `{ticket}` | Changes only the given fields. |
| `ticket.move {product, id, state, waiting_on?, by}` | `{ticket}` | `invalid` for an illegal move, or `waiting` with no `waiting_on`. The same state again passes, so a retry works. |
| `ticket.promote {product, id, to, by}` | `{ticket, story?, decision?}` | `to` is `story` or `decision`. See below. |

`ticket.promote` makes the work of a ticket into a term that already has a circuit:

- With `to: "story"`, the core calls `story.new` with the title and the body. The PM shapes the text into a story (`docs/specs/agentsi-stories.md:78`). Thus this path makes a model call. It returns `invalid` when stories mode is off (`docs/specs/agentsi-desktop-protocol.md:290`).
- With `to: "decision"`, the core calls `decision.add` with the title as `text` and the body as `why` (`docs/specs/agentsi-desktop-protocol.md:278`).
- Then the core adds the link to the new story or decision and moves the ticket to `done`, in the same transaction as the decision row. A story write that fails leaves the ticket as it was.

## MCP tools

The Rust `agentsi mcp` (`crates/agentsi-cli/src/mcp.rs`, `crates/agentsi-cli/src/mcp/tools.rs`) gets four tools. Each tool takes the optional `product` argument of the other tools (`packages/mcp/src/server.ts:30`).

| Tool | Arguments | Core method | Writes |
|---|---|---|---|
| `list_tickets` | `state?`, `kind?`, `assignee?`, `limit` (default 20) | `ticket.list` | No |
| `get_ticket` | `id` | `ticket.get` | No |
| `add_ticket` | `kind`, `title`, `body?`, `assignee?`, `links?` | `ticket.add` | Yes |
| `update_ticket` | `id`, `title?`, `body?`, `state?`, `waiting_on?`, `assignee?`, `links?` | `ticket.update`, then `ticket.move` when `state` is given | Yes |

The descriptions of the tools carry three rules:

- A ticket gates nothing. If Claude needs the approval of the owner, it records a decision with `agentsi decision add`. It does not write a ticket.
- A ticket does not replace a story. If the work changes code and stories mode is on, Claude proposes a story.
- Claude does not drop or close a ticket of the owner (Q5).

The four tools are the first MCP tools that write. Today the five tools only read, and `pending_decisions` says "Never act on a pending decision" (`packages/mcp/src/server.ts:168`).

### How a Claude Code session writes a ticket

1. Claude Code starts `agentsi mcp` from the plugin, in the folder of the session.
2. The server finds the product of that folder with `product.whoami` (`crates/agentsi-cli/src/mcp.rs:48-80`).
3. Claude calls `add_ticket`. The server calls `ticket.add` with the author `{by: "session", id}`.
4. The core writes the row and sends `data.changed`.
5. An open Tickets screen reads `ticket.list` again and shows the ticket.

## Meetings

The roles see the tickets through the snapshot, the Markdown context of a meeting (`docs/specs/agentsi-desktop-protocol.md:232`).

- `data.snapshot` adds a `## Open tickets` section after `## Open issues` (`crates/agentsi-agency/src/snapshot.rs:409-430`). It lists the `open` and `waiting` tickets, 30 at most, one line each: `- C012 [todo, waiting on Matias] Install the aarch64 bundle (for owner)`.
- If the product has no open ticket, the section stays out. Thus the golden test of `snapshot` on synthetic data stays equal (`docs/specs/agentsi-desktop-protocol.md:222`). This is an open test of the brief.
- Each role reads the full snapshot (`crates/agentsi-agency/src/meeting/prompts.rs:201`). The synthesis reads the compact snapshot. `## Open tickets` goes into `SPLIT_AT` and `KEEP`, with a limit of 1500 characters (`crates/agentsi-agency/src/prompt_inputs.rs:102-130`).
- A priority can cite a ticket. The source type `ticket` goes into the source schema (`crates/agentsi-agency/src/meeting/prompts.rs:24`) and the rule text (`crates/agentsi-agency/src/meeting/prompts.rs:181`). The check resolves it by id among the tickets of the product (`crates/agentsi-agency/src/meeting/check.rs:149-165`).
- The `snapshots` table keeps the snapshot of each minutes (`crates/agentsi-store/src/schema.rs:191-195`). Thus a frozen snapshot holds the tickets of its day, and a replay sees the same tickets.
- The role agents of the plugin keep their tool lists until the switch-over (`docs/specs/agentsi-desktop-parity.md:98`). Thus a role that the front desk consults sees tickets only through the snapshot. The main session of the front desk has the four tools.

## Screens

The app adds two screen kinds, as Stories and Story do (`crates/agentsi-app/src/screens/mod.rs:59-62`, `crates/agentsi-app/src/nav.rs:48-64`).

| Place | What it shows | Actions |
|---|---|---|
| Tickets screen (`ScreenKind::Tickets`) | The tickets of one product in three groups: Open, Waiting, and Done of the last 14 days. Each row has the kind, the id, the title, the author, the assignee, the count of links and the age. | A one-line add field with a kind choice. Filters by kind, state, assignee and author. Dropped tickets show behind a filter. |
| Ticket screen (`ScreenKind::Ticket`) | The title, the Markdown body, the kind, the state, `waiting_on`, the assignee, the links, the author and the times. Each link opens its screen. | Edit in place. Done, Wait, Drop, Reopen, Promote to story, Promote to decision. |
| Front desk | A block "Tickets": the open tickets for the owner, and the tickets that sessions added in the last 24 h. | Open a ticket. Open the Tickets screen. |
| Product context menu | Two more entries: "New ticket" and "Open tickets" (`docs/specs/agentsi-desktop-navigation.md:68`). | |
| Ticket row context menu | Open, Done, Wait, Drop, Promote to story, Promote to decision, Copy id. | A Drop asks no Confirm. Reopen undoes it. |
| Palette | "New ticket for the active product" opens the prompt with one line of text and the kind `note`. "Open tickets" opens the Tickets screen. | Each menu entry has a palette entry (`docs/specs/agentsi-desktop-navigation.md:64`). |

These rules stay:

- No ticket gets an item row in the sidebar. Item rows are for running things (`docs/specs/agentsi-desktop-navigation.md:24`). Q9 asks about a count.
- No color or size by hand, and the idle app draws 0 frames (`docs/specs/agentsi-desktop-navigation.md:80-81`).
- The Tickets screen of a product on an SSH host reads the core of that host, as the other remote screens do (`docs/specs/agentsi-desktop-protocol.md:413-425`).

## Overlap

A ticket holds what the other terms do not hold: a fact, a to-do or a question that gates nothing and runs nothing.

| Term | What it is | Who writes it | What it does | The line with a ticket |
|---|---|---|---|---|
| Story (`CONTEXT.md:109-111`) | A unit of work with 2 to 5 criteria | The owner or the PM | A story session changes code, and the owner gives a result | A ticket has no criteria, no session and no diff. If a `todo` ticket needs code, "Promote to story" makes the story and closes the ticket. |
| Task (`CONTEXT.md:113-115`) | A story with no user | The owner or the PM | The same as a story | A `todo` ticket is not a task. A promoted `todo` ticket can become a task. |
| Decision (`CONTEXT.md:75-77`) | A point that waits for the approval of the owner | The synthesis, or `decision.add` | It blocks a change until the owner approves it, and it can carry an action | A `question` ticket asks for information and blocks nothing. If Claude needs approval, it writes a decision, not a ticket. |
| Priority (`CONTEXT.md:71-73`) | A ranked item of the minutes, with a source | Only the synthesis of a meeting | The owner marks it | A ticket has no rank and no mark. A priority can cite a ticket as a source, and a ticket can link to a priority. |
| Pending item (`CONTEXT.md:83-85`) | A priority with no mark, or a pending decision | Nobody. `mark.pending` finds it | `agentsi mark` walks through it | A `waiting` ticket is not a pending item. `mark.pending` does not list tickets. |
| Memory (`CONTEXT.md:135-137`) | What one agent learned about a product | A reflection or a meeting | It goes into the prompt of the agent | A `note` ticket belongs to the owner or a session, not to an agent. A reflection does not read tickets. |
| Journal (`.claude/JOURNAL.md`) | The working state of the last session | The Stop hook and the session | The next session reads it | The journal holds the state of one session and the next session rewrites it. A ticket stays until the owner closes it. |
| GitHub issue | An issue of the repo | People on GitHub | `knowledge.index` reads the open ones (`crates/agentsi-agency/src/knowledge/collect.rs:186-207`) | An issue lives in GitHub. A ticket lives in `agentsi.db`. A link of type `issue` joins them. |

## GitHub Issues later

`ROADMAP.md:13` plans "Issue trackers, so the roles read and update the issues of a product." This brief does not design the sync. The model already has the place for it:

- Today, `knowledge.index` reads the open issues with `gh issue list`, and only reads (`crates/agentsi-agency/src/knowledge/collect.rs:186-207`). The snapshot lists them (`crates/agentsi-agency/src/snapshot.rs:412-430`), and a priority cites them with the source type `issue` (`crates/agentsi-agency/src/meeting/check.rs:153`).
- A ticket joins an issue through a link of type `issue`, with the same ref as the knowledge base (`#12`, or the scope form of `crates/agentsi-agency/src/knowledge/collect.rs:232`). Thus the sync needs no field in the ticket now.
- The core runs `gh`, as it does for the index. The app never calls GitHub.
- A write to GitHub acts outside AgentSI. Thus it needs an action of the owner or an approved decision (`CONTEXT.md:3`).
- The sync spec decides the direction, the owner of each field, and the map of states to open and closed.

## Conflicts with approved specs

These approved texts block the build before the switch-over. The owner decision of 2026-11-09 puts the tracker first, so each text needs an amendment (Q6):

- The parity spec says "A new feature waits until after the switch-over" (`docs/specs/agentsi-desktop-parity.md:17`).
- The parity spec checks that `agentsi mcp` lists the 5 tools (`docs/specs/agentsi-desktop-parity.md:148`), and the `mcp-*` golden test compares `tools/list` with the TypeScript server (`docs/specs/agentsi-desktop-parity.md:93`). Four more tools break both checks.
- The schema comment and ADR 0006 count 17 tables, and so does the clear data spec (see "Storage rules").
- A `search` kind `ticket` changes the input schema of `search` (`packages/mcp/src/server.ts:9`). Thus the brief keeps tickets out of the knowledge base (Q7).

## Seed cases

These inputs are real. They come from the journal of AgentSI on 2026-11-09 (`.claude/JOURNAL.md:3-8`). The count is 6, fewer than 20. The brief lists "collect real inputs" as an open test.

| # | Real input | Expected result |
|---|---|---|
| 1 | "next: install the aarch64 bundle (`packaging/install.sh --from ... --enable`)" | A `todo` ticket, `open`, author session, assignee owner. |
| 2 | "then ask Matias before uploading it to release v0.1.0" | Not a ticket. The upload acts outside AgentSI, so it is a decision with an action. |
| 3 | "blocked: — (Matias runs sudo; docker group needs `newgrp docker` in this session)" | A `todo` ticket, `waiting`, `waiting_on` "the owner runs sudo". |
| 4 | "decided: build in the container (portable), not on the host" | A `note` ticket, `open`, author session. |
| 5 | "state: bundle built with `make dist-portable` ... not installed, not uploaded, nothing pushed." | Not a ticket. It is the state of one session, and the journal holds it. |
| 6 | "goal: ship an aarch64 tarball beside the x86_64 one" | A `todo` ticket. If stories mode is on and the work changes code, Claude proposes a task in its place. |

## Open questions for the owner

**Q1. Restatement.** Do you accept the Idea of the brief and the gaps above? Where do you keep notes and to-dos today?
Recommendation: accept, and name the tool that you use today, so the stop conditions have a baseline.

**Q2. Name.** Answered 2026-11-09: Ticket. Story no longer lists "Ticket" in its _Avoid_ line, and `CONTEXT.md` defines the term.

**Q3. Kinds and states.** Are `note`, `todo` and `question` the right kinds? Is your "pending" the state `waiting`?
Recommendation: yes to both. A kind `pending` blurs with Pending item, and a kind `task` blurs with Task.

**Q4. Product.** Must each ticket belong to a product?
Recommendation: yes in the first version. A ticket with no product has no host, no snapshot and no Tickets screen.

**Q5. What Claude can change.** Can a session close or drop a ticket that the owner wrote?
Recommendation: no. A session adds any ticket. It edits, closes and drops only its own tickets and the tickets with the assignee `claude`.

**Q6. Parity amendment.** Do you amend the parity spec so that four MCP tools and six core methods land before the switch-over?
Recommendation: yes. Change the non-goal to name this exception. The `mcp-*` golden test then compares only the 5 old tools.

**Q7. Search.** Must `search` find tickets?
Recommendation: not in the first version. A kind `ticket` changes the `search` schema. Add it after the switch-over.

**Q8. Roles that write.** Can the synthesis of a meeting write a ticket, for example for a disagreement?
Recommendation: no in the first version. The roles read tickets in the snapshot. A role output that needs action is already a priority or a decision.

**Q9. Sidebar count.** Must a product row show the count of open tickets?
Recommendation: no. The chip of a closed product counts running items (`CONTEXT.md:271-273`). A second count blurs it. The front desk shows the tickets.
