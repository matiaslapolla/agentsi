# AgentSI desktop: the setup screen

**Status:** draft · **Source:** `docs/ideas/desktop-actions.md`, `docs/ideas/desktop-actions-eval.md` (setup flow, C3, C4, C7), owner decisions of 2026-11-09

## Problem

Only a Claude conversation (`/agentsi setup`) registers a product and writes its brief, roster, overlays and Stories mode. The roles screen applies a stage folder whose path it reads from the clipboard. The owner block and the extra paths of a product need a hand edit of `agentsi.config.json`.

## Goal

- The owner registers a product, or updates its brief, from one screen of the app.
- The screen writes only after the owner's Confirm, through `roles.apply`, `decision.add` and `registry.update`.
- One optional Claude call drafts the fields. The forms work without it.
- The owner edits the owner block and the paths of a product in the app.

## Non-goals

- Product docs in a product repo (owner, Q1, option C). The north-star decision only records a decision.
- Edits of the global roles (`plugin/agents/*.md`).
- A change of `/agentsi setup` or of the output of `agentsi roles apply`.

## Terms

- **Setup screen**: the screen of kind `Setup` that registers a product or updates its brief, in five steps.
- **Setup draft**: the field values that one Claude call proposes from the files of a product folder.
- **Inline stage**: the stage files of `roles.apply`, sent as text in the request in place of a folder path.

## Design

### Steps of the screen

1. **Folder.** The owner types an absolute path. On this device, a button also opens the system folder dialog. `setup.scan` reads the folder. If the registry holds the folder, the screen opens in update mode and fills the forms from `roles.brief`, `roles.list` and `story.config`.
2. **Draft (optional).** "Draft with Claude" shows the cost cap and asks one Confirm. Then the screen calls `setup.draft`. A field that the draft marks "no data" stays empty and marked.
3. **Forms.** The owner edits the five forms of the table below. The field rules are those of `plugin/skills/agentsi/setup.md`, sections 3 to 5.
4. **Review.** The app makes the inline stage and calls `roles.apply` without `yes`. The plan view of `crates/agentsi-app/src/screens/roles_report.rs` shows the plan. Each failure shows at its field. If a draft filled the forms, the screen sends the failures to `setup.draft` one time. If the owner chose no north star, a check box proposes the decision of `setup.md` section 6.
5. **Apply.** Confirm calls `roles.apply` with `yes`. For a new product, the call also has `register` with the first path. Then the app calls `registry.update` for more paths, and `decision.add` when the box is checked. Then the app opens the front desk of the product.

| Form | Fields |
|---|---|
| Product | id, name, paths, what, audience, stage, out of scope |
| Objectives | 1 to 3 objectives (text, signal, due date), north stars from the found paths, sources of truth |
| Roster | global roles (`pm` always on), specialists (id, role, overlay text), custom roles (id, full definition), one "why" line for each agent |
| Models | model and effort for each agent, by default those of the global role |
| Stories mode | on or off, test command, gate commands, repo, budget. The scan gives the manifest commands. Without `bwrap` the form recommends off and says why |

No write occurs before step 5. In update mode, a field that differs from the stored brief has a mark. The places that open the screen are in "Shared surfaces". On the roles screen, "Update the brief" replaces "Apply stage from the clipboard".

### Changes of `roles.apply`

- It takes `stage`, a map from the name of a stage file to its text. `stage_dir` becomes optional. A request with both, or with neither, returns `invalid`.
- The keys of `stage` are `brief.json`, `roster.json`, `stories.json` and `agents/<id>.md`. Each other key returns `invalid`.
- The core writes an inline stage into a temporary folder with mode 0700, plans from it, and then deletes it. Thus the rules of a stage folder apply without change, also on an SSH host (conflict C4).
- With `yes`, the same transaction writes the `products` row of the product: `id`, `name`, `paths` and `status`. `product.list` reads this table, and today only the import writes it.
- After the write, the core sends `data.changed {product, kind: "product"}`. Thus the sidebar reads the product list again.

### New methods

| Method | Result | Rules |
|---|---|---|
| `setup.scan {path}` | `{path, id, name, registered, manifest, north_stars, tools}` | Code only, no write. `id` is the folder name in lowercase, with `-` for each character that an id does not permit. `registered` is the product id of `product.whoami`, or null. `manifest` holds `test` and `gate` of `package.json`, `Cargo.toml`, `pyproject.toml` or `composer.json`, or null. `north_stars` holds the paths of `docs/product.md`, `docs/vision*` and `ROADMAP*` that exist. `tools` holds `bwrap` and `codebase_memory_mcp`, true on the `PATH`. A path that is not an existing absolute folder returns `invalid` |
| `setup.draft {path, previous?, failures?}` | `{draft, fields, no_data, cost_usd, seconds}` | Reads at most 15 files and 60 KiB in the order of `setup.md` section 2, and `git log --oneline -30`. One `claude -p` with `--json-schema`, `--max-budget-usd 0.50`, model `sonnet`, no tools. `previous` and `failures` ask for a repair. `fields` has the shape of the five forms. `no_data` lists the fields that the files do not answer. A failed call returns `internal` |
| `setup.outcome {draft, product, fields}` | `{}` | Records `kept`, `edited` or `cleared` for each drafted field. The app calls it after an apply |
| `registry.get {}` | `{owner, products}` | `owner` holds `name` and `language`. A product holds `id`, `name`, `paths` and `status` |
| `registry.update {owner?, product?}` | `{owner, products}` | Writes `agentsi.config.json` in place and keeps each unknown key. `product` holds `id` and a new `name` or new `paths`. Each path is an existing absolute folder, and one path at least stays. A product change also writes its `products` row and sends `data.changed {product, kind: "product"}`. No commit (ADR 0003) |

### Settings: Owner

The settings screen gets a section "Owner". It shows the name and the language of the owner from `registry.get`. Save calls `registry.update {owner}`.

### Learn from use

`setup.draft` and `setup.outcome` each append one JSON line to `<data root>/logs/setup.jsonl`: the cost and the seconds of a draft, or the field results. A failed draft also appends one line, with the error and the seconds. The stop conditions of the brief read this file.

### Defaults taken

The owner did not answer these questions. Each line takes the recommendation of the eval file and the brief. The owner can overturn each line.

- Q2: The core runs the draft with `claude -p`, with a cap of 0.50 USD and one Confirm (conflict C3).
- Q4: The owner edits the owner block in the settings, and the name and the paths of a product in the setup screen.
- Q7: The screen works for a product on an SSH host, through the inline stage. The folder dialog is for this device only.
- Q8: No seed cases exist. The tests use a fake `claude` on the `PATH`.
- Q10: `roles.apply` keeps its commit of `agentsi.config.json`, because the golden files hold it. The screen shows a commit error as a note. The new methods make no commit.

### Shared surfaces

| Surface | Change |
|---|---|
| Protocol methods and events | `roles.apply` gets `stage`, and `stage_dir` becomes optional. New methods `setup.scan`, `setup.draft`, `setup.outcome`, `registry.get` and `registry.update`. `core.hello` lists the namespaces `setup` and `registry` in `features`. `data.changed` with kind `product` comes from two more writes |
| OpenRPC schema file | `schema/core.openrpc.json` gets the changes above. Do not merge it by hand. After a rebase, `AGENTSI_UPDATE_SCHEMA=1 cargo test -p agentsi-proto --test schema` writes it again |
| SQLite tables and migrations | None. `roles.apply` and `registry.update` write the existing `products` table |
| Clear data | The "Logs" part deletes `logs/setup.jsonl` too |
| MCP tool list | None |
| Sidebar and navigation | The screen kind `Setup` in `layout.json`, with the product id, or "" for a new product. A button "New product" at the end of each host group. The product menu entry "Update the brief". The button "Update the brief" of the roles screen |
| Keys | None |
| Command palette | "Set up a new product", "Update the brief of the active product", "Open the owner settings" |
| Golden tests | No file changes. The cases `roles-apply-*` pass unchanged |
| Other specs | `agentsi-desktop-protocol.md` lists the methods. `agentsi-desktop-ui.md` drops "Apply takes the path of the stage folder from the clipboard" |

## Acceptance criteria

- [ ] `cargo test -p agentsi-core --test setup inline_stage` passes: an inline stage gives the same plan as the same files in a folder. A request with `stage` and `stage_dir`, or with the key `../brief.json`, returns `invalid`.
- [ ] `cargo test -p agentsi-core --test setup register` passes: after `roles.apply` with `register` and `yes`, `product.list` holds the product, and a subscriber gets `data.changed` with kind `product`.
- [ ] `cargo test -p agentsi-core --test registry` passes: `registry.update` keeps an unknown key, refuses a missing path, and makes no commit in a data root that is a git repo.
- [ ] `cargo test -p agentsi-core --test setup scan` passes: on a fixture folder, the id, the manifest commands and the north stars are correct.
- [ ] `cargo test -p agentsi-core --test setup draft` passes with a fake `claude`: the call has `--max-budget-usd 0.50`, and `setup.jsonl` gets one line.
- [ ] `cargo test -p agentsi-cli --test golden` passes, and `git diff --stat main -- crates/agentsi-cli/tests/golden` prints nothing.
- [ ] `cargo test -p agentsi-proto --test schema` passes.
- [x] A test of `agentsi-app` with a fake core does the five steps for a new product and finds no write call before Confirm.
- [x] `rg -n 'read_from_clipboard' crates/agentsi-app/src/screens/roles.rs` finds nothing.
- [ ] The owner registers one real product in the app, and `agentsi roles validate` passes on it.
