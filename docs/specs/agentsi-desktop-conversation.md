# AgentSI desktop: the conversational desk

**Status:** draft · **Source:** `docs/research/conversational-ui.md` (Design F, section "10. Owner decisions") and the owner decisions of 2026-11-09

## Problem

The front desk screen shows the state of a product, but the owner cannot ask it a question. The owner asks in the `/agentsi` pane, which draws only terminal text. Thus a decision that Claude names needs a step on another screen.

## Goal

- The front desk screen holds a desk conversation with Claude, with streamed Markdown and native views.
- Milestone 1: the owner approves or rejects a pending decision from a view in the desk conversation.
- The desk conversation and the `/agentsi` pane change the same core data, and each one shows the changes of the other.
- The core checks the Claude Code login at start and shows a clear message when the login is not usable.

Milestones 2 to 4: `agentsi-desktop-conversation-views.md`.

## Non-goals

- A web view in the app (ADR 0004).
- A Claude login of AgentSI, or a read of the Claude Code credentials.
- A tool that lets Claude write product data.
- A change of the meeting screen, `agentsi doctor`, the golden files or the plugin.

## Terms

- **Desk session**: the one `claude -p` process of the core for the front desk of one product, with its saved session id.
- **Desk conversation**: the stored items of one desk session.
- **Owner message**: one text that the owner sends to a desk session. It is not a turn of conversation mode.
- **Reply**: what Claude writes for one owner message, up to its `result` line.
- **View**: one native component in the desk conversation. The catalog, the closed `enum View`, gives its kind.
- **Object view**: a view that names stored objects by id and reads them from the core.
- **Desk note**: one line about a write of product data, which the core adds to the next owner message.
- **Auth state**: the result of the login check of `claude`.

## Design

### Desk session

`crates/agentsi-agency/src/desk/` builds the arguments and reads the stream. `crates/agentsi-core/src/desk/` keeps one process for each product, at most.

- The process plays the `pm` agent of the roster, with its model, effort and persona. `desk/system.md` adds the desk rules and the catalog text.
- The work folder is the first path of the product. The process has its own process group and stops with the core.
- `desk.send` starts a process when none runs. The core stops a process after 30 min with no owner message, or after 10 min in one reply, with an `error` item.
- The core finds `agentsi` in `AGENTSI_CLI_BIN`, else next to `agentsi-core`, else on the `PATH`.

The arguments have no `--bare` and no `--no-session-persistence`:

```
-p --input-format stream-json --output-format stream-json --verbose
--include-partial-messages --model <pm model> --system-prompt <text>
--mcp-config '{"mcpServers":{"agentsi":{"type":"stdio","command":"<agentsi>","args":["mcp"],
  "env":{"AGENTSI_DESK_SESSION":"<uuid>","AGENTSI_CORE_SOCKET":"<socket>"}}}}'
--strict-mcp-config --restricted --disable-slash-commands
--permission-mode dontAsk --permission-prompts none --tools=Read,Grep,Glob
--allowedTools Read,Grep,Glob,mcp__agentsi__search,mcp__agentsi__get_doc,
  mcp__agentsi__list_minutes,mcp__agentsi__pending_decisions,
  mcp__agentsi__role_memory,mcp__agentsi__show_decisions
--session-id <uuid>          (new session)
--resume <uuid>              (stored session)
```

The core reads each stdout line:

| Line | Result |
|---|---|
| `stream_event` with a text delta | event `desk.delta` |
| `assistant`, text block | item `reply` |
| `assistant`, `tool_use` block | item `tool`, with one line such as `Read: docs/a.md`, as the meeting screen writes it. A tool of the AgentSI server shows its words, such as `Pending decisions`. A `show_*` tool makes no item |
| `result` | item `end`, with `cost_usd`, `duration_ms` and `error` |

### Protocol

| Entry | Params | Result or payload |
|---|---|---|
| `desk.history` | `product, before?, limit?` | `{session?, state, items}` |
| `desk.send` | `product, text` | `{seq}`. `busy` while a reply runs |
| `desk.stop` | `product` | `{stopped}` |
| `desk.reset` | `product` | `{session}`. Deletes the items and makes a new uuid |
| `desk.show` | `session, view` | `{view_id}` |
| `desk.auth` | `recheck?` | `{state, message?}` |
| `decision.get` | `product, ids` | `{decisions}`, each with `status` and `archived` |
| event `desk.state` | | `{product, state, message?}`. `state`: `stopped`, `starting`, `idle`, `working`, `auth`, `failed` |
| event `desk.delta` | | `{product, text}` |
| event `desk.item` | | `{product, seq, item}`. `item.kind`: `owner`, `reply`, `tool`, `view`, `note`, `end`, `error` |
| topic `desk` | | The three events |

### Catalog and checks

```rust
// crates/agentsi-proto
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum View {
    Decisions { ids: Vec<i64> },
}
```

- `desk.show` returns `invalid` with the `serde` text for bad input, and `not_found` for an id that the product does not have.
- After the check, the core stores a `view` item and sends `desk.item`.
- When `AGENTSI_DESK_SESSION` is set, `agentsi mcp` lists one `show_<kind>` tool for each variant. Each tool calls `desk.show` and gives an error to Claude with `isError`.

### The desk conversation in the app

`crates/agentsi-app/src/screens/desk_conversation.rs` is a column at the right of the front desk screen.

- `MessageScroller` holds the items. A reply streams into `TextViewState::push_str`. A tool item or a desk note is one muted line.
- The decisions view reads with `decision.get`. It reads again on `data.changed` of its product with kind `decision` or `minutes`.
- Approve and Reject open a Confirm, then call `decision.decide`.
- "New conversation" opens a Confirm, then calls `desk.reset`.

### Sync with the `/agentsi` pane

The pane writes through the CLI and reads through the MCP tools, so it uses the same core methods. After a write of `decision.decide`, `decision.add`, `mark.set` or `archive.*`, the core stores a desk note, if the product has a desk session. An example is "The owner approved decision 14." `desk.send` puts the unsent desk notes before the owner text and marks them sent.

### Auth check

At start, the core runs `claude auth status --json`, with a limit of 5 s. If the state is not `ready`, `desk.send` checks again. If it is still not `ready`, `desk.send` returns `unavailable` and starts no process. An auth error in the first reply of a `ready` state gives `not_read`.

The desk conversation shows the message in place of the input, with a "Check again" button:

| State | Source | Message |
|---|---|---|
| `ready` | `loggedIn` true | none |
| `signed_out` | `loggedIn` false | "Claude Code is not signed in. Run `claude auth login` in a terminal, then select Check again." |
| `missing` | no `claude` program | "AgentSI cannot find the `claude` program on the PATH." |
| `not_read` | auth error in a reply | "Claude Code did not use your login. A newer version can run `claude -p` in bare mode." |
| `unknown` | other output | The first line of the output |

### Storage

Migration "desk (number set at merge)":

```sql
create table desk_sessions (product text primary key, session text not null, created_at text not null);
create table desk_items (product text not null, seq integer not null, kind text not null,
  body text not null, sent integer not null default 0, at text not null, primary key (product, seq));
```

### Defaults taken

| Research question | Default |
|---|---|
| 2. Can Claude record a decision? | No. The desk session has no write tool |
| 3. Is the desk session read-only? | Yes. It has no `--permission-prompt-tool` |
| 4. Are HTML pages wanted, and is E2 enough? | "Open in browser" (E2) is enough |
| 5. Do meetings move to the views? | No. The meeting screen stays as it is |

### Shared surfaces

| Surface | Change |
|---|---|
| Protocol | The `desk` namespace, the topic `desk`, three events and `decision.get` |
| `schema/core.openrpc.json` | After a merge, write it again with `AGENTSI_UPDATE_SCHEMA=1 cargo test -p agentsi-proto --test schema`. Never merge it by hand |
| SQLite | Migration "desk (number set at merge)". `desk_sessions` and `desk_items` join `schema::LOCAL_TABLES`, the list of the tracker spec. The first module to merge adds the list. `clear::TABLES` gets the two tables before `products`, so the clear items "Product" and "Everything" delete them. The lists of `agentsi-store/tests/tables.rs` and `agentsi-core/tests/clear.rs` change |
| Core writes | `decision.decide`, `decision.add`, `mark.set` and `archive.*` in `methods/marks.rs` call `desk::note`. The ticket writes can call it |
| MCP server | `tools/list` changes only with `AGENTSI_DESK_SESSION`. Then the show tools follow the base tools, after the ticket tools of the tracker |
| Parity checks and golden tests | No change from this module. The tracker changes `mcp_golden` |
| Front desk screen | `desk.rs` gets a column at the right. The left column keeps the sections of the actions module, the tickets child of the tracker and the agent row of the immersive app |
| Keys | None in `keys.toml`. When the input has the focus, the single-letter keys of the front desk do not act |
| Navigation | One palette entry, "Ask the front desk", focuses the input. No screen kind, sidebar row or `layout.json` field |

## Acceptance criteria

- [ ] `cargo test -p agentsi-proto --test schema` passes, and the schema file lists each `desk.*` method.
- [ ] With one pending decision, the owner asks "What waits for me?". A decisions view shows its stored `text`, `why` and `action`.
- [ ] Approve and a Confirm set the decision to `approved` in `agentsi.db`. The view shows it with no new owner message.
- [ ] `show_decisions` with an unknown id gives an error result, and the core stores no `view` item.
- [ ] A second owner message goes to the same `claude` process (`cargo test -p agentsi-core --test desk_session`).
- [ ] After a core restart, the process gets `--resume <same uuid>`, and Claude remembers the last reply.
- [ ] When the owner decides in the `/agentsi` pane, the view changes in 1 s or less. The next owner message holds the desk note.
- [ ] If a fake `claude` gives `loggedIn: false`, the `signed_out` message shows, and no desk process starts.
- [ ] The desk arguments have no `--bare`, `Bash`, `Edit` or `Write`.
- [ ] `cargo test -p agentsi-cli --test golden` passes, and no golden file changes.
- [ ] `cargo tree -p agentsi-app | grep -E 'wry|webkit2gtk'` prints nothing.
- [ ] A clear of a product deletes its `desk_sessions` and `desk_items` rows.
