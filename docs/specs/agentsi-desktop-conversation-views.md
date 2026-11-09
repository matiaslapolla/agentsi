# AgentSI desktop: the views of the conversational desk

**Status:** draft · **Source:** `docs/research/conversational-ui.md` (Design F, sections 5.4 to 5.6 and "10. Owner decisions") and the owner decisions of 2026-11-09

## Problem

After milestone 1, Claude can show only pending decisions in the desk conversation. Priorities, stories, diffs, tables, charts and lists stay as plain Markdown. The owner must open another screen to act on them.

## Goal

- Milestone 2: object views of priorities, stories and diffs.
- Milestone 3: generic views of tables, charts, checklists and pages.
- Milestone 4, after the tracker merges: an object view of tickets.

This spec needs `agentsi-desktop-conversation.md`, and it uses its terms.

## Non-goals

- A web view in the app (ADR 0004). A page opens in the browser of the owner.
- A free tree of components (Design A of the research).
- A view button that runs a story session or spends money. Such a button opens the story screen.
- A view that writes product data without a Confirm.

## Terms

- **Generic view**: a view that draws small typed data that Claude gives. The data is a copy and does not change after the call.
- **Page**: a generic view that holds one HTML page from Claude. The app never draws it.

## Design

### Catalog

Each milestone adds its variants to `View`:

```rust
Priorities { minutes_id: String },                                   // M2
Story { id: String },                                                // M2
Diff { story_id: String },                                           // M2
Table { title: String, columns: Vec<String>, rows: Vec<Vec<String>> },             // M3
Chart { chart: ChartKind, title: String, labels: Vec<String>, values: Vec<f64> },  // M3
Checklist { title: String, items: Vec<String> },                     // M3
Page { title: String, html: String },                                // M3
Tickets { ids: Vec<String> },                                        // M4
```

`ChartKind` is `bar`, `line`, `area` or `pie`. Each variant adds one `show_<kind>` tool to `agentsi mcp` and to the `--allowedTools` of the desk session. Each tool description says when Claude calls it. A stored desk session reads the new system prompt only after `desk.reset`.

### Object views

| Kind | Read method | `data.changed` kinds | Shows | Buttons |
|---|---|---|---|---|
| `priorities` | `minutes.get` | `minutes` | Title, why, owner role and mark of each priority | Accept, Accept with changes, Reject: a Confirm, then `mark.set` with `unmarked_only` |
| `story` | `story.get` | `stories` | Title, state and criteria | Open the story screen |
| `diff` | `story.review` | `stories` | The diff, in the `diff` component, with the line limit of the story screen | Open the story screen |
| `tickets` | `ticket.get` for each id | `tickets` | The ticket line of each ticket | Open the Ticket screen |

`desk.show` returns `not_found` when the minutes, the story or a ticket does not exist in the product.

### Generic views

`desk.show` returns `invalid` when the data breaks a limit:

| Kind | Limits |
|---|---|
| `table` | 1 to 12 columns, 0 to 200 rows, each row as long as `columns`, 200 characters in each cell |
| `chart` | 1 to 50 points, `labels` as long as `values`, each value finite |
| `checklist` | 1 to 30 items, 200 characters in each item |
| `page` | 120 characters in `title`, 512,000 characters in `html` |

- A table uses `table` of `gpui-component`. A chart uses `chart`. A checklist uses `checkbox`.
- The owner checks items of a checklist. The app calls `desk.check {product, seq, checked}`, which returns `{seq}`.
- `desk.check` stores the checked indexes in the `view` item. It stores a desk note such as "The owner checked 2 of 5 items of Launch steps: a, b." It writes no product data.

### Pages

- The core keeps the HTML only in the body of the `view` item. Thus a page adds no folder to the data root, and clear data needs no change.
- `desk.item` and `desk.history` give the length of `html` in place of the HTML.
- A page view shows the title and one button, "Open in browser".
- The button calls `desk.page {product, seq}`, which returns `{title, html}`.
- The app writes the HTML to `$XDG_RUNTIME_DIR/agentsi/pages/<product>-<seq>.html` on its own device, with mode 0600. Then it opens the file with `cx.open_url`. This also applies to a product on an SSH host.

### Tickets (milestone 4)

This part starts after the tracker merges. The field names of `Ticket` in `agentsi-desktop-tracker.md` are the contract of the view. The desk session gets `list_tickets` and `get_ticket` in `--allowedTools`. It never gets `add_ticket` or `update_ticket`, because the desk session is read-only.

### Shared surfaces

After a merge, write the schema file again with `AGENTSI_UPDATE_SCHEMA=1 cargo test -p agentsi-proto --test schema`.

| Surface | Change |
|---|---|
| Protocol | `desk.page`, `desk.check`, the new `View` variants |
| `schema/core.openrpc.json` | Written again from the types |
| SQLite | No migration. The items stay in `desk_items` |
| MCP server | With `AGENTSI_DESK_SESSION`: 8 more `show_*` tools, after the ticket tools. Without it: no change |
| Desk arguments | The new show tools. M4: `list_tickets` and `get_ticket` |
| Parity checks and golden tests | No change |
| App | No new screen kind, sidebar row, navigation item or key |

## Acceptance criteria

- [ ] M2: a `gpui::test` draws a priorities view, a story view and a diff view from fixture data.
- [ ] M2: Accept on a priority, with a Confirm, sets its `mark` to `accepted` in `agentsi.db`, and the view shows it with no new owner message.
- [ ] M2: `show_story` with an unknown id gives an error result, and the core stores no `view` item.
- [ ] M3: a `gpui::test` draws a table, a bar chart, a checklist and a page view.
- [ ] M3: `show_table` with a row longer than `columns` gives an error result that names the row.
- [ ] M3: a check in a checklist puts a desk note in the next owner message.
- [ ] M3: "Open in browser" opens the page in the browser, and the file has mode 0600.
- [ ] M3: `cargo tree -p agentsi-app | grep -E 'wry|webkit2gtk'` prints nothing.
- [ ] M4, after the tracker merges: a tickets view changes in 1 s or less after a ticket write.
- [ ] M4: the desk arguments name no tool that writes tickets.
- [ ] `cargo test -p agentsi-cli --test golden` passes, and no golden file changes.
