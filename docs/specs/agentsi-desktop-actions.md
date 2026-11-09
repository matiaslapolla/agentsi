# AgentSI desktop: every action from the app

**Status:** draft · **Source:** `docs/ideas/desktop-actions.md`, `docs/ideas/desktop-actions-eval.md`, owner decisions of 2026-11-09

## Problem

17 actions of the CLI and the plugin have no place in the app, and 7 actions have only some of their options (the gap table of the eval file). For these actions, the owner must open a terminal or a Claude session. The story screen reads its note from the clipboard, because the screens have no text field.

## Goal

- Each row of the gap table that is "None" or "Partial" has a place in the app, except the rows that "Non-goals" names.
- Each place calls the core method of the CLI command. Thus the app and the CLI write the same rows.
- Each new action has a palette entry.

## Non-goals

- The setup screen, the registry writes and the rows 28, 29, 43, 44 and 45. `agentsi-desktop-setup.md` holds them.
- Product docs: ADRs, specs, idea briefs and `CONTEXT.md` terms. They stay in a Claude pane (owner, Q1, option C). The app writes nothing into a product repo.
- The rows 1, 3 and 35, which need no place, and the row 46 (see "Defaults taken").
- A meeting on a frozen snapshot from the meeting screen. The CLI has no flag for it.
- A change of the output of a ported CLI command.

## Terms

- **Form**: a group of text fields and choices inside a screen, with one button that writes and one Cancel button.
- **Health section**: the section of the settings screen that shows the result of `core.doctor`.

## Design

### Forms

- The rule of `agentsi-desktop-ui.md` that the roles and memory screens have no text field ends (conflict C6). Each screen can hold a form.
- A form uses the text field of `gpui-component`, as the question field of the meeting screen does.
- `Enter` in the last field writes. `Escape` cancels. An empty required field disables the button that writes.
- A form that starts a model call says that the call costs money and asks one Confirm, by the rule of the context menus.
- The outcome shows in the one line above the content, as for the writes of U20.

### Places of the actions

The "Row" column gives the number of the gap table.

| Row | Screen | Control | Core call |
|---|---|---|---|
| 2 | Settings, Health section | One row for each check: name, status, detail, what it blocks. A button "Check again" | `core.doctor` |
| 4 | Settings, Data section | "Back up now". The outcome line shows the `Backup:` and `Registry:` paths | `data.backup` |
| 8 | Front desk | "Update the index". The outcome line shows the repos and the time | `knowledge.index {product}` |
| 9 | Meeting | "Show the context" opens the Markdown in a panel | `data.snapshot` |
| 12 | Meeting | One check box for each roster agent, all on by default. `pm` stays on, because it writes the synthesis | `meeting.start {roles}` |
| 13 | Meeting | "Ask one role": an agent choice and a question field. The answer shows in one pane | `meeting.ask` |
| 15 | Front desk, outcome line | After an approval with an action: "Run in Claude" | `terminal.create` |
| 16 | Front desk | "New decision": a form with text, why and an optional action | `decision.add` |
| 18 | Front desk | The section "Pending items" replaces "Pending decisions" | `mark.pending`, `mark.set`, `decision.decide` |
| 21 | Front desk | "Show archived": the archived decisions, each with Restore | `decision.archived`, `archive.restore` |
| 22 | Story | Archive, or Restore for an archived story. A running session disables Archive | `archive.archive`, `archive.restore` |
| 24 | Sidebar | "Show archived products". An archived product has one menu entry: Restore | `product.list`, `archive.restore` |
| 26 | Roles | A click or `Enter` on a row opens the resolved definition of the agent in a panel | `roles.list {agent}` |
| 30 | Roles, reset plan | A check box "Also delete the memory". A change of the box plans again | `roles.reset {memory}` |
| 31 | Memory | "Reflect" with a check box "Memory only". Cancel while the reflection runs | `reflect.run`, `reflect.cancel` |
| 37 | Stories | "New story": a text field and a choice of Story, Task or "The PM picks" | `story.new {type}` |
| 38 | Stories | "Suggest" | `story.suggest` |
| 39 | Story | Changes opens a note field and a check box "The PM rewrites the story" | `story.changes {rewrite}` |
| 41 | Story | "Record a merge": a commit field and a choice of "merged unchanged" or "merged with changes" | `story.result` |

Three rows need more rules:

- **Pending items (row 18).** The front desk reads `mark.pending` and lists its items in that order. A priority item has Accept, Change, Reject and New, and the keys `A`, `C`, `R` and `N`. A decision item has Approve, Reject and Archive, and the keys `A`, `R` and `X`. After each write, the cursor moves to the next item. `Enter` opens the minutes of the item.
- **Run in Claude (row 15).** The button opens a pane in the first path of the product with `argv` `["claude", <action>]`. The click on Approve and the click on "Run in Claude" are the owner's word (ADR 0002). Nothing runs without the second click.
- **Archived products (row 24).** A toggle at the end of each host group, and a palette entry, show the archived products. An archived product shows dimmed in its host group. The toggle is not saved.

### New core methods

- `core.doctor {}` returns `{checks}`. A check has `name`, `level` (`required`, `stories` or `optional`), `status` (`ok`, `missing` or `fail`), `detail` and `blocks`.
  - The rows and the probes are those of `agentsi doctor`, in the same order. They run on the host of the core.
  - The `core` row reads the state of the core itself.
  - The probe code moves from `crates/agentsi-cli/src/commands/doctor.rs` to `crates/agentsi-agency/src/doctor.rs`. `agentsi doctor` prints the same lines.
- `decision.archived {product}` returns `{decisions}`. It only reads.
  - It gives each decision of the product that `archive_records` holds, with `id`, `kind`, `minutes_id`, `text`, `why`, `action` and `status`, newest id first.
  - An unknown product returns `not_found`. A missing `agentsi.db` returns `unavailable`.

### Palette

Each entry acts on the active product, or on the screen in view that the owner used last (rule of U20). The entry "New story" opens the form of the stories screen in place of the prompt.

| Entry | Row |
|---|---|
| Check the health of the tools | 2 |
| Back up the data now | 4 |
| Update the index of the active product | 8 |
| Show the context of the next meeting | 9 |
| Ask one role of the active product | 13 |
| Run the action of the approved decision in Claude | 15 |
| New decision for the active product | 16 |
| Show the archived decisions | 21 |
| Archive the story, Restore the story | 22 |
| Show the archived products | 24 |
| Cancel the reflection of the active product | 31 |
| Suggest stories for the active product | 38 |
| Record a merge of the story | 41 |

### Defaults taken

The owner did not answer these questions. Each line takes the recommendation of the eval file, or the option that keeps the rule of today when the file gives none. The owner can overturn each line.

- Q3: The free conversation with the PM stays in a Claude pane. The conversational desk module owns a chat.
- Q5: The app does not edit the global roles (`plugin/agents/*.md`). Only the owner and an approved reflect proposal change them.
- Q6: After an approval, "Run in Claude" opens a Claude pane with the action text.
- Q9: The module has two specs, this one and `agentsi-desktop-setup.md`.
- Row 46: The app has no search of the knowledge base. The MCP tools serve Claude.

### Shared surfaces

| Surface | Change |
|---|---|
| Protocol methods and events | Two new methods: `core.doctor` and `decision.archived`. No other method changes its params or its result. No new event |
| OpenRPC schema file | `schema/core.openrpc.json` gets the two methods. Do not merge it by hand. After a rebase, `AGENTSI_UPDATE_SCHEMA=1 cargo test -p agentsi-proto --test schema` writes it again |
| Front desk | `crates/agentsi-app/src/screens/desk.rs` gets the section "Pending items" and the buttons of rows 8, 16 and 21. It keeps the views of the tracker and the conversational desk |
| SQLite tables and migrations | None |
| MCP tool list | None |
| Sidebar and navigation | The toggle "Show archived products" and the menu entry Restore of an archived product. No new screen kind |
| Keys | None. `BINDINGS` and the ids of `keys.toml` stay. The keys of row 18 act only in the focused front desk |
| Command palette | The entries of "Palette", each with one action in `crates/agentsi-app/src/actions.rs` |
| Golden tests | No file changes |
| Other specs | `agentsi-desktop-ui.md` loses the "no text field" rule. `agentsi-desktop-protocol.md` lists the two methods |

## Acceptance criteria

- [ ] `cargo test -p agentsi-proto --test schema` passes, and `schema/core.openrpc.json` lists `core.doctor` and `decision.archived`.
- [ ] `cargo test -p agentsi-cli --test golden` passes, and `git diff --stat main -- crates/agentsi-cli/tests/golden` prints nothing.
- [ ] `cargo test -p agentsi-cli doctor` passes with the moved probe code.
- [ ] `cargo test -p agentsi-core --test doctor` passes: `core.doctor` returns the 10 rows of `agentsi doctor` in the same order.
- [ ] `cargo test -p agentsi-core --test marks decision_archived` passes: an archived decision is in the list, and a restore removes it.
- [ ] For each row of "Places of the actions", a test of `agentsi-app` with a fake core uses the control and finds the core call with the expected params.
- [ ] A test of `agentsi-app` finds that `meeting.ask`, `story.new`, `story.suggest` and `reflect.run` each ask one Confirm before the call.
- [ ] A test of `agentsi-app` finds a palette entry for each new action, and `KEY_ACTIONS` keeps the same ids.
- [ ] `rg -n 'read_from_clipboard' crates/agentsi-app/src/screens/story.rs` finds nothing.
- [ ] The owner does the rows 4, 13, 16, 18, 37 and 41 in the app with no terminal, and accepts the screens.
