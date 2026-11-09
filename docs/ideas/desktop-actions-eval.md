# Do every action from the app: gap table and open questions

**Date:** 2026-11-09 · **Brief:** `docs/ideas/desktop-actions.md`

This file lists each action that the owner does today by command, and the place of that action in the desktop app. It also lists the conflicts with the rules of the repo and the questions for the owner.

Sources of the inventory:

- The CLI commands: `crates/agentsi-cli/src/commands/mod.rs:80-233` and the `USAGE` constant of each command file.
- The plugin: `plugin/skills/agentsi/SKILL.md`, `plugin/skills/agentsi/commands.md`, `plugin/skills/agentsi/setup.md`, `plugin/skills/agentsi/reflect.md`.
- The MCP tools: `crates/agentsi-cli/src/mcp/tools.rs:66-167`.
- The app: `crates/agentsi-app/src/actions.rs:17-174`, `crates/agentsi-app/src/menus.rs:199-266`, the screens in `crates/agentsi-app/src/screens/`.
- The core methods: the `const NAME` of each method in `crates/agentsi-proto/src/*_api.rs`.

"Partial" means that the app has the action but not all of its options. "None" in the app column means that the owner must use the CLI or Claude.

## Gap table: CLI commands

| # | Action | Command today | App place today | Core method | Proposed place |
|---|---|---|---|---|---|
| 1 | Start the core, apply migrations | `agentsi up`, `agentsi migrate` | Not necessary. The core migrates at start (`docs/specs/agentsi-desktop-parity.md:47-49`) | `core.status` (`ops_api.rs:40`) | None |
| 2 | Check the tools | `agentsi doctor` | None | None. The CLI runs its own probes and calls only `core.status` and `models.list` (`crates/agentsi-cli/src/commands/doctor.rs:382-386`) | Settings, a "Health" section. It needs a new method, for example `core.doctor` |
| 3 | Move old data | `agentsi data import` | Not necessary. Removed (`crates/agentsi-cli/src/commands/data.rs:7-10`) | None | None |
| 4 | Make a backup now | `agentsi backup` | None. Clear data makes one first (`CONTEXT.md:299-301`) | `data.backup` (`ops_api.rs:68`) | Settings, Data section: "Back up now" |
| 5 | Delete data | `agentsi clear` | Settings, Data section (`screens/settings/data.rs:246`, `:466`) | `data.clear_plan`, `data.clear` | Done |
| 6 | Find the product of a folder | `agentsi whoami` | Not necessary. The sidebar lists the products (`model.rs:460`) | `product.whoami` (`agency_api.rs:141`) | Step 1 of the setup screen uses it |
| 7 | Open the front desk, show the status | `agentsi desk`, `agentsi status` | Front desk screen (`screens/desk.rs:202`) | `product.desk`, `product.status` | Done |
| 8 | Update the index | `agentsi index` | None. A meeting runs the index first (`crates/agentsi-agency/src/meeting/run.rs:224-225`) | `knowledge.index` (`knowledge_api.rs:60`) | Front desk: "Update the index" button and palette entry |
| 9 | Show the meeting context | `agentsi snapshot` | None | `data.snapshot` (`agency_api.rs:204`) | Meeting screen: "Show the context" (low value) |
| 10 | Check a minutes file | `agentsi check` | The minutes screen shows the check result (`screens/minute.rs:1-3`) | `minutes.check` (`meeting_api.rs:143`) | Done |
| 11 | Audit the product | `agentsi audit`, `/agentsi audit` | Roles screen, audit section (`screens/roles.rs:128`) | `data.audit` (`agency_api.rs:336`) | Done |
| 12 | Run a meeting | `agentsi meet` | Partial. Meeting screen, question field (`screens/meeting.rs:551-569`). No `--roles` subset. No replay of a frozen snapshot | `meeting.start` (`meeting_api.rs:54`), which takes `roles` and `frozen` | Meeting screen: a roster picker |
| 13 | Ask one role | `agentsi ask` | None | `meeting.ask` (`meeting_api.rs:91`) | Meeting screen: "Ask one role", with a role picker and a question field |
| 14 | Approve or reject a decision | `agentsi decide` | Front desk (`screens/desk.rs:133`). Memory screen for a proposal (`screens/memory.rs:204`) | `decision.decide` (`marks_api.rs:198`) | Done |
| 15 | Run the action of an approved decision | Claude runs it after `decide` (`SKILL.md:71`) | None. The front desk shows the text only (`screens/desk.rs:273`) | None | Open question Q6 |
| 16 | Add a decision | `agentsi decision add` | None | `decision.add` (`marks_api.rs:229`) | Front desk: "New decision" form with text, why and action |
| 17 | Mark one priority | `agentsi mark <id> <idx> <mark>`, `--set` | Minutes screen (`screens/minute.rs:192`) | `mark.set` (`marks_api.rs:131`) | Done |
| 18 | Walk all pending items | `agentsi mark`, `agentsi mark --pending` | None. The owner opens each minutes | `mark.pending` (`marks_api.rs:93`) | Front desk: "Pending items" list that steps through each one |
| 19 | Archive or restore minutes | `agentsi archive`, `agentsi restore` | Minutes screen (`screens/minute.rs:250`, `:253`) | `archive.archive`, `archive.restore` (`marks_api.rs:277`, `:294`) | Done |
| 20 | Archive a decision | `agentsi archive <n>` | Front desk (`screens/desk.rs:154`) | `archive.archive` | Done |
| 21 | Restore a decision | `agentsi restore <n>` | None | `archive.restore` | Front desk: "Show archived" with Restore |
| 22 | Archive or restore a story | `agentsi archive S001`, `agentsi restore S001` | None. The stories screen shows archived stories but has no button (`screens/stories.rs:1-3`) | `archive.archive`, `archive.restore` | Story screen: Archive and Restore |
| 23 | Archive a product | `agentsi archive` | Context menu and palette (`shell/menu_actions.rs:395-409`) | `archive.archive` | Done |
| 24 | Restore a product | `agentsi restore` | None | `archive.restore` | Sidebar: "Show archived products" with Restore |
| 25 | Show the roster | `agentsi roles`, `/agentsi roles` | Roles screen (`screens/roles.rs:110`) | `roles.list` (`roles_api.rs:90`) | Done |
| 26 | Show the effective prompt of one agent | `agentsi roles <agent>` | None. The screen asks with `agent: None` (`screens/roles.rs:112`) | `roles.list` with `agent` (`roles_api.rs:64-66`) | Roles screen: a click on a row opens the prompt |
| 27 | Validate the roster | `agentsi roles validate` | Roles screen (`screens/roles.rs:152`) | `roles.validate` (`roles_api.rs:165`) | Done |
| 28 | Apply a staged brief and roster | `agentsi roles apply <dir>` | Partial. Roles screen, stage path from the clipboard (`screens/roles.rs:193-210`) | `roles.apply` (`roles_api.rs:251`) | Setup screen (see "Setup flow") |
| 29 | Register a new product | `agentsi roles apply --register` | None. The app sends `register: None` (`screens/roles.rs:212`) | `roles.apply` with `register` (`roles_api.rs:175-195`) | Setup screen, step 1 |
| 30 | Reset an agent or the product | `agentsi reset`, `/agentsi reset` | Partial. Roles screen, always `memory: false` (`screens/roles.rs:170`) | `roles.reset` (`roles_api.rs:300`) | Roles screen: an "Also delete the memory" choice in the plan |
| 31 | Reflect | `agentsi reflect`, `/agentsi reflect` | Partial. Context menu and palette, always `memory_only: false` (`shell/menu_actions.rs:372-375`). No cancel | `reflect.run`, `reflect.cancel` (`reflect_api.rs:346`, `:399`) | Memory screen: "Memory only" choice and Cancel |
| 32 | Revert a reflection | `agentsi reflect revert` | Memory screen (`screens/memory.rs:187`) | `reflect.revert` (`reflect_api.rs:373`) | Done |
| 33 | Render the call | `agentsi render` | Minutes screen and call screen (`screens/minute.rs:361`, `screens/call.rs:463`) | `call.render` (`call_api.rs:143`) | Done |
| 34 | Voice mode | `agentsi voice`, `/agentsi:voice` | Pane voice mode (`pane.rs:562`) | `voice.mode` (`voice_api.rs:38`) | Done |
| 35 | Open the app | `agentsi web` | Not necessary | None | None |
| 36 | List and show stories | `agentsi story list`, `story show` | Stories screen and story screen (`screens/stories.rs:76`, `screens/story.rs:158`) | `story.list`, `story.get` | Done |
| 37 | New story or task | `agentsi story new [--type task]` | Partial. Palette prompt, always `kind: None` (`shell/menu_actions.rs:418-421`) | `story.new` (`story_api.rs:295`) | Stories screen: "New story" form with a story or task choice |
| 38 | Suggest stories | `agentsi story suggest` | None | `story.suggest` (`story_api.rs:306`) | Stories screen: "Suggest" button |
| 39 | Ask for changes to a story | `agentsi story changes <id> "<note>"` | Partial. Story screen, note from the clipboard (`screens/story.rs:4-5`, `:255`) | `story.changes` (`story_api.rs:348`) | Story screen: a note field |
| 40 | Approve, discard, run, review, accept, reject a story | `agentsi story approve`, ... | Story screen (`screens/story.rs:163-300`) | `story.approve`, `story.discard`, `story.run`, `story.review`, `story.accept`, `story.reject` | Done |
| 41 | Record a merge done outside accept | `agentsi story result` | None | `story.result` (`story_api.rs:453`) | Story screen: "Record a merge" with a commit field |

## Gap table: plugin and MCP

| # | Action | Command today | App place today | Core method | Proposed place |
|---|---|---|---|---|---|
| 42 | Talk with the PM at the front desk | `/agentsi` (`SKILL.md:12-36`) | A Claude pane in the product folder (`actions.rs:25`) | None | Keep the Claude pane. Open question Q3 |
| 43 | First setup: registry entry, brief, objectives, roster, overlays, Stories mode | `/agentsi setup` (`setup.md:32-191`) | None | `roles.apply` writes the result. No method drafts it | Setup screen |
| 44 | Update the brief | `/agentsi setup`, update mode (`setup.md:193-202`) | Partial. Roles screen apply from the clipboard | `roles.brief` reads it (`roles_api.rs:137`). `roles.apply` writes it | Setup screen, filled from `roles.brief` |
| 45 | Record the north-star decision in setup | `agentsi decision add` (`setup.md:161-167`) | None | `decision.add` | Setup screen, review step: a check box |
| 46 | Search the knowledge base, read one document | MCP `search`, `get_doc` (`tools.rs:66`, `:94`) | None | `knowledge.search`, `knowledge.get_doc` | Palette search (optional) |
| 47 | List minutes, pending decisions, role memory | MCP `list_minutes`, `pending_decisions`, `role_memory` (`tools.rs:127-167`) | Minutes screen, front desk, memory screen | `knowledge.recent_minutes`, `knowledge.pending_decisions`, `knowledge.role_memory` | Done |

Count: 17 rows have no place today (2, 4, 8, 9, 13, 15, 16, 18, 21, 22, 24, 26, 29, 38, 41, 43, 45). 7 rows are partial (12, 28, 30, 31, 37, 39, 44). Row 46 is optional.

## Creation of each element

| Element | How the owner creates it today | App place today | Core method | Proposed place |
|---|---|---|---|---|
| Product (registry entry) | `/agentsi setup` then `roles apply --register` | None | `roles.apply` with `register` | Setup screen, step 1 |
| More paths of a product | The owner edits `agentsi.config.json` by hand. `Registration` holds one path (`roles_api.rs:175-181`) | None | None | Open question Q4 |
| Owner block (name, language) | The owner edits `agentsi.config.json` by hand (`docs/specs/agentsi-owner-config.md:16-17`) | None | None. `product.whoami` reads it | Settings: "Owner". It needs a new write method |
| Brief and objectives | `/agentsi setup`, rounds 1 and 2 (`setup.md:69-77`) | None | `roles.apply` writes, `roles.brief` reads | Setup screen, forms "Product" and "Objectives" |
| Roster | `/agentsi setup`, round 3 (`setup.md:79-83`) | None | `roles.apply` | Setup screen, form "Roster" |
| Specialist and overlay | Setup stages `agents/<id>.md` with `extends` (`setup.md:127-140`) | None | `roles.apply` | Setup screen, form "Roster": one text field for each overlay |
| Custom role | Setup copies `plugin/templates/role.md` (`setup.md:142`) | None | `roles.apply` | Setup screen, form "Roster" |
| Global role (`plugin/agents/*.md`) | By hand, or an approved reflect proposal (`reflect.md:48`) | None | None | Open question Q5 |
| Stories block | Setup round 5 stages `stories.json` (`setup.md:146-159`) | None. `story.config` only reads (`story_api.rs:213`) | `roles.apply` | Setup screen, form "Stories mode" |
| Story or task | `agentsi story new`, `story suggest` | Partial (rows 37, 38) | `story.new`, `story.suggest` | Stories screen |
| Decision | `agentsi decision add` | None (row 16) | `decision.add` | Front desk |
| Meeting | `agentsi meet` | Partial (row 12) | `meeting.start` | Meeting screen |
| ADR, spec, idea brief in a product repo | The owner's own Claude Code skills (`shaping`, `domain-modeling`) in a Claude pane. No `agentsi` command | A Claude pane (`actions.rs:25`) | None | Conflict C1, open question Q1 |
| `CONTEXT.md` term in a product repo | The same skills in a Claude pane | A Claude pane | None | Conflict C1, open question Q1 |

## Setup flow

Today the owner's Claude Code session runs setup. Claude reads at most 15 files, asks 5 rounds of 3 to 5 questions with a recommended answer each, stages the files, runs a dry run and applies after the owner's yes (`setup.md:41-181`). The setup screen keeps the same rounds and the same apply. It moves the questions into form fields.

1. **Folder.** The owner selects a folder.
   - Code proposes the id from the folder name, a display name and the absolute path (`setup.md:37`).
   - `product.whoami` finds out if the registry holds the folder. If it does, the screen opens in update mode and fills the forms from `roles.brief` and `roles.list`.
2. **Facts that code finds (level 0).**
   - The test runner and the typecheck and lint scripts of the manifest.
   - The north-star candidates that exist: `docs/product.md`, `docs/vision*`, `ROADMAP*`.
   - `bwrap` and `codebase-memory-mcp` on the `PATH` (`setup.md:96`).
3. **Draft (Claude).** The screen shows the cost cap and asks one Confirm, because the call spends money (`CONTEXT.md:312`).
   - The core builds the context from the files of `setup.md:45-49`.
   - The core runs one Claude call with `--json-schema` and `--max-budget-usd` (`crates/agentsi-agency/src/meeting/claude.rs:195`, `:245-247`).
   - The result holds the brief fields, the roster, the overlays and, for each field, "no data" when the repo does not tell.
4. **Forms (owner).** Five forms: product, objectives, roster, models, Stories mode (`setup.md:69-96`).
   - The draft fills each field as the recommended answer. A "no data" field stays empty and marked.
   - The owner edits each field. In update mode, the screen marks the fields that differ from the stored brief.
5. **Review (owner).** The core stages the files and runs `roles.apply` with `yes: false`.
   - The screen shows the plan: roster before and after, the diff of each file, the stories line. The roles screen already draws this plan (`screens/roles.rs:58-59`).
   - If the plan has failures, the core sends them to Claude one time. If the second plan fails, the form marks the failed fields.
   - If the product has no north-star doc, a check box proposes the decision of `setup.md:166`. A checked box calls `decision.add` at apply.
6. **Apply (owner).** Confirm calls `roles.apply` with `yes: true` and, for a new product, `register`. The click on Confirm is the owner's word, as ADR 0002 says for the story screen (`docs/adr/0002-story-sessions-write-product-repos.md:34`).

The owner approves in three places: the cost of the draft (step 3), each field (step 4) and the plan (step 6). No write occurs before step 6.

## Conflicts

**C1. A product doc written from the app writes into the product repo.**
- `docs/specs/agentsi-roles.md:24` says "No writes into product repos. Ever."
- `plugin/skills/agentsi/SKILL.md:96` and `plugin/skills/agentsi/setup.md:28` say that setup and reflect never write into the product repo.
- ADR 0002 lets only a story session write, in a worktree on a `story/*` branch, and only `story accept` merges (`docs/adr/0002-story-sessions-write-product-repos.md:9`).
- But the git screen already lets the owner stage and commit in the product repo (`CONTEXT.md:243-245`). Thus the rule binds the roles and Claude. It does not bind the owner's own hand.
- Options:
  - A: A doc is a task story. A story session writes it on a `story/*` branch, and the owner accepts it. This keeps ADR 0002.
  - B: A new ADR. Claude drafts the doc in the app, the owner edits it, and the owner saves it. The save is the owner's write, as a commit on the git screen is.
  - C: Docs stay out of the module. The app opens a Claude pane in the product folder, as today.

**C2. The doc formats belong to the owner, not to AgentSI.** The formats of ADRs, specs and idea briefs come from the owner's own skills in `~/.claude/skills/`. The owner-config spec removed each tie to skills that the repo does not ship (`docs/specs/agentsi-owner-config.md:12`, `:26`). If the app ships these formats, a stranger gets one owner's conventions. A template setting for each product removes the tie.

**C3. Setup runs in the owner's session today.** `setup.md:26` says "Do not call `claude -p`". The setup screen moves the draft into the core, which runs `claude -p` with the owner's account and spends money with no conversation. The design needs a cost cap and a Confirm before the call.

**C4. `roles.apply` reads a folder path.** The core reads `stage_dir` from its own disk (`roles_api.rs:185-186`). On a remote host the folder must be on the host (`screens/roles.rs:217-218`). A form needs the stage files inline in the request. That changes the protocol (ADR 0007, `docs/specs/agentsi-desktop-protocol.md`).

**C5. New methods and options during the switch-over.** `docs/specs/agentsi-desktop-parity.md:17` says "New commands or new flags. A new feature waits until after the switch-over." The owner lifted it on 2026-11-09 for this module. But the parity tests compare the Rust CLI with the TypeScript CLI. New core methods must not change the output of a ported command.

**C6. The screens have no text field by design.** `docs/specs/agentsi-desktop-ui.md:108` and `:119` say that the roles and memory screens have no text field. The setup screen and rows 13, 16, 37, 39 and 41 need text fields. The meeting screen and the git screen already use one (`screens/meeting.rs:22`, `screens/git.rs:18`).

**C7. `roles.apply` commits the registry.** `roles_api.rs:252-256` says that `roles.apply` commits `agentsi.config.json`. `CONTEXT.md:144` says that the data root is not a git repo and that AgentSI makes no commits in it (ADR 0003). The setup screen inherits this rule. The owner must say which rule holds.

## Eval seed cases

Count: 0. The registry of this machine (`~/.local/share/agentsi/data/agentsi.config.json`) holds no products, and no database file exists beside it. The draft needs real product repos with a brief that the owner accepted. Open question Q8.

## Open questions for the owner

1. **Q1, product docs.** Do you want the app to create ADRs, specs, idea briefs and `CONTEXT.md` terms in a product repo? If yes, which option of C1: A (a task story), B (a new ADR for the owner's own write) or C (a Claude pane, as today)?
2. **Q2, setup draft.** Can the core run `claude -p` for the setup draft? What is the cost cap of one draft? The brief proposes 0.50 USD.
3. **Q3, front desk.** Does the free conversation with the PM stay in a Claude pane? Or do you want a chat in the app?
4. **Q4, registry.** Do you want to edit the owner block and the paths of a product in the app? Both need a new core method that writes the registry.
5. **Q5, global roles.** Do you want to edit `plugin/agents/*.md` in the app? Today only you and an approved reflect proposal change them.
6. **Q6, decision actions.** After you approve a decision in the app, who runs its action? Options: nobody (as today), a task story, or a Claude pane with the action text.
7. **Q7, remote hosts.** Must the setup screen work for a product on an SSH host? If yes, `roles.apply` must take the stage inline (C4).
8. **Q8, seed cases.** Which 3 to 5 product repos, with a brief that you accept, can the prototype use?
9. **Q9, split.** Do you accept two specs: one for the level 0 gaps (rows 2 to 41) and one for the setup screen?
10. **Q10, registry commit.** C7: must `roles.apply` stop the commit of `agentsi.config.json`?

## Owner decisions (2026-11-09)

- Q1, product docs: option C. ADRs, specs and idea briefs of a product stay in a Claude pane, as today. The app does not write them into a product repo.
- The tracker term is **Ticket** (`CONTEXT.md`).
