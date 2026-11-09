# AgentSI desktop: parity and the switch-over

**Status:** approved 2026-11-02 · **Source:** `docs/specs/agentsi-desktop.md`, `packages/cli/src/main.ts`, `apps/web/app/`, `packages/mcp/src/server.ts`, `plugin/hooks/hooks.json`

## Problem

The plugin runs `agentsi` commands and reads their JSON. The owner uses the web app every day. The TypeScript code can leave the repo only when each command, screen and tool of today has its place in the new programs, and that place is tested.

## Goal

- A list of each CLI command, web route, MCP tool and hook of today, with its new place and its milestone.
- A golden test for each output that the plugin reads.
- The switch-over steps, with a way back.

## Non-goals

- New commands or new flags. A new feature waits until after the switch-over.
  - Exception (owner, 2026-11-09): four desktop modules come before the switch-over. They are actions, the tracker, the conversational desk and the immersive app (`docs/ideas/desktop-*.md`). They land in the Rust programs only. Their specs say which parity checks change, such as the count of MCP tools.

## Terms

- **Golden test**: a test that runs a command of the Rust CLI on a fixed database and compares its output with a file that the TypeScript CLI wrote on the same data.

## Design

### Rules for the Rust CLI

- The binary `agentsi` keeps the name, the flags, the exit codes and the `--json` output of each command.
- Each command is a client of the core. It calls the method of the same name in `agentsi-desktop-protocol.md`.
- The golden files live in `crates/agentsi-cli/tests/golden/`. A unit makes them with the TypeScript CLI before it ports a command.
- A golden test runs on synthetic data only. Both CLIs read the same `data.sql`.
- Four fields are normalized in the same way on both sides:
  - the work folder,
  - the hash and the date of a commit line,
  - the seconds of the index line of `desk`,
  - `generated_at` of the audit.
- A command that writes (`mark`, `decide`, `decision add`, `archive`, `restore`) has two checks. Its output equals the `.golden` file. The rows that it leaves in `minutes`, `priorities`, `decisions` and `archive_records` equal the `.rows` file that the TypeScript CLI wrote from Postgres on the same data. Each write case runs on a fresh copy of the data. A failed write must leave the rows as they were.
- The write commands do not refresh the `documents` rows of the minutes. TypeScript did it after a mark. `knowledge.index` does it in the Rust program, and the rows check leaves `documents` out.
- `decide` on a reflection decision settles the reflection, as in TypeScript (U25). A commit of a global role that fails prints `Commit skipped:`. A reflection with no proposal rejects the decision, prints the reason and exits with code 1. Golden cases cover both.
- `reflect` and `reflect revert` have two checks as well. Their rows are `memory`, `reflections`, `decisions` and `agent_overlays`, in `<case>.rows`, from `reflect-rows.sql`. The golden README lists the fields that a run makes.
- `crates/agentsi-cli/tests/golden/README.md` has the commands that make each file. It also has the steps to add a command.
- A command that is not ported prints `agentsi <name>: not ported to the Rust CLI yet. ...` and stops with exit code 1.

### CLI commands

| Command | New place | Milestone |
|---|---|---|
| `up` | Starts the core. Kept as an alias. Prints the version, the data root and the schema version (U24) | M2 |
| `doctor` | Checks the core, the data root, Claude Code and its flags, `git`, `bwrap`, `gh`, the code graph, `espeak-ng`, the voice models and the audio devices. No `--json`: the TypeScript command has none (U24) | M2 |
| `migrate` | The core applies migrations at start. Prints the schema version that it applied (U24) | M2 |
| `data import` | Removed. `agentsi-core import-postgres` takes its place | M2 |
| `backup` | SQLite online backup (data spec), through `data.backup`. Prints `Backup:`, `Registry:` and `Removed:` lines. Done in U26b | M2 |
| `whoami` | `product.whoami` | M2 |
| `desk` | `product.desk`, front desk screen | M1 screen, M2 CLI |
| `index` | `knowledge.index` | M2 |
| `status` | `product.status`, front desk screen | M1 screen, M2 CLI |
| `snapshot` | `data.snapshot` | M2 |
| `check` | `minutes.check` | M2 |
| `audit` | `data.audit` | M2 |
| `meet` | `meeting.start`, meeting screen | M2 |
| `ask` | `meeting.ask` | M2 |
| `decide` | `decision.decide`, front desk action | M2 |
| `decision add` | `decision.add` | M2 |
| `story` (13 subcommands) | `story.*`, stories screen. Linux hosts only | M2 (U23) |
| `mark` | `mark.pending`, `mark.set`, minutes screen. The terminal wizard stays | M2 |
| `archive`, `restore` | `archive.*`, action on each list | M2 |
| `roles` | `roles.list`, `validate`, `apply`, roles screen | M2 (U21) |
| `reset` | `roles.reset` | M2 (U21) |
| `reflect` | `reflect.run`, `reflect.revert`, memory screen | M2 |
| `render` | `call.render`. The core renders the call with Kokoro, one voice for each role (U26) | M2 |
| `voice` | `voice.mode` on the pane of `AGENTSI_TERMINAL`. In a plain Claude Code session it uses `voice.session` with `CLAUDE_CODE_SESSION_ID`. With neither variable it prints an error and exits with 1. Done in U26b, and the session case on 2026-11-07 | M2 |
| `web` | Removed. Opens `agentsi-app` (`AGENTSI_APP_BIN`, else next to `agentsi`, else the `PATH`). Done in U26b | M2 |
| `mcp` | `agentsi mcp`, the tools below | M2 |

### Web routes

| Route | Screen |
|---|---|
| `/` | Sidebar and the list of products |
| `/p/[product]` | Front desk |
| `/p/[product]/m/[id]` | Minutes |
| `/p/[product]/m/[id]/call` | Call (U26). `call.get`, `call.audio`, `call.heard` |
| `/p/[product]/stories`, `/stories/[id]` | Stories, with the diff view and the review actions. Changes takes its note from the clipboard (U23) |
| `/p/[product]/memory` | Memory |
| `/p/[product]/roles` | Roles. The brief, audit and reflection sections of the web page go to the front desk, the audit view and the memory screen. |
| `/como-funciona`, `/guia` | `docs/guide/README.md`, drawn on the Guide screen, which two entries of the command palette and the Settings screen open. Done in U26b, then moved into the app (amendment of `agentsi-desktop-guide.md`, 2026-11-08) |
| `/api/audio/...` | The core reads the audio file |

### MCP tools

`agentsi mcp` keeps the 5 tools with the same names and input schemas: `search`, `get_doc`, `list_minutes`, `pending_decisions` and `role_memory`.

- It speaks MCP on stdio and answers `initialize`, `ping`, `tools/list` and `tools/call`, as the TypeScript server does. An argument that breaks the schema gets the message of `zod`.
- The golden test `mcp-*` runs the same requests against both servers, and compares each answer, the `tools/list` result included. `crates/agentsi-cli/tests/golden/README.md` lists the parts that it removes.
- `agentsi search` does not exist in the TypeScript CLI, so it is not a new command. The search is the MCP tool `search` and the core method `knowledge.search`.

### Plugin

- The agents stay unchanged. The skills change only where the owner permitted it on 2026-11-07:
  - `plugin/skills/voice/SKILL.md`: the doctor line points to `agentsi doctor`, and the text names the pane. `/agentsi:voice` exited with 1 in a plain Claude Code session, because `AGENTSI_TERMINAL` is empty there. The Rust CLI now keys voice mode to `CLAUDE_CODE_SESSION_ID` in that case, and the hook bridge speaks for it.
  - The web text of `plugin/skills/agentsi/commands.md`, `SKILL.md` and `reflect.md` describes the desktop app and what `agentsi web` does in the Rust CLI.
  - No other change of `plugin/skills/` belongs to the switch-over.
- A global role proposal keeps the lookup of U21 (`<data root>/plugin/agents/`, else `$AGENTSI_PLUGIN_DIR/agents/`, else the built-in role files). A later version moves its target. The switch-over does not.
- `push-guard.mjs` becomes `agentsi hook pre-tool-use`, so the plugin needs no Node.
  - It gives the verdict of the Node guard for each command, also for its quirks. `crates/agentsi-cli/tests/push_guard_cases.json` holds the table: the 50 cases of `push-guard.test.mjs` and 12 added cases with the Node verdicts.
  - `packaging/hooks.json` is the hooks file of the switch-over. It has no Node. U28 copies it over `plugin/hooks/hooks.json`.
- `voice-mode.mjs` and `voice-pill.qml` leave the plugin. The hook bridge and the pill take their place, also for a Claude Code session outside the app. Nothing reads the TypeScript voice state file `<data root>/voice/<session>.json` after step 7: the Rust state lives in `voice/sessions/`.

### Packaging (U24)

- `packaging/systemd/agentsi-core.service` runs `agentsi-core serve` as a user service. `packaging/launchd/dev.agentsi.core.plist` does the same on macOS.
- `packaging/systemd/agentsi-pill.service` runs `agentsi-pill` as a user service that starts with the graphical session (`WantedBy=graphical-session.target`), after the core. Nothing else starts the pill, and without it no reply is spoken and the dictation key does nothing. There is no launchd agent for the pill yet (macOS waits for U18).
- `packaging/install.sh` copies `agentsi`, `agentsi-core`, `agentsi-app` and `agentsi-pill` to `~/.local/bin` and writes the unit or the agent. On Linux it also writes the pill unit when `agentsi-pill` is installed. It enables the services only with `--enable`. It refuses a target outside `$HOME`.
- `packaging/test-install-container.sh` runs the script in a throwaway container and verifies both units with `systemd-analyze verify`. `agentsi voice on` prints one warning line on stderr while no pill is connected (`core.status` field `pill`).

### The audio format of calls (decided in U26)

The Python package wrote `call.webm` and `timings.json` with `ffmpeg`. The core writes `call.opus` (Ogg Opus, 24 kHz, mono, 32 kbit/s) and the same `timings.json`. The call screen plays both files, so no old call is lost. The test with its numbers, and the rules, are in `docs/specs/agentsi-desktop-calls.md`.

The web app plays only `call.webm`. It stays until the switch-over, so a call that the core renders plays on the desktop, and not on the web page.

### Switch-over

The owner runs these steps, with a worker session as help:

1. Make sure that each row of this spec and each acceptance criterion of the other desktop specs passes. The macOS criteria are out: U18 stands by, and the switch-over runs on Linux first (`docs/plans/agentsi-desktop-log.md`, "Owner decision: Linux first" and "Owner decisions for the switch-over").
2. Run `agentsi backup` of the TypeScript CLI for a last `pg_dump`.
3. Stop the web app and each Claude session that uses the old CLI.
4. Run `agentsi-core import-postgres` and read the line of each table.
5. Install the Rust binaries on the `PATH`.
6. Make the tag `last-typescript` on the last commit with the TypeScript code.
7. In one commit, delete these parts and change the license:
   - `packages/` and `apps/web/`,
   - the TypeScript files of `scripts/`,
   - `docker-compose.yml` and the Postgres targets of the `Makefile`,
   - the Node hooks of `plugin/hooks/`,
   - the backup timer of `scripts/systemd/`, because the core makes the daily backup.

   Before the delete, in the same commit, move `packages/voice/voices.toml` and `lexicon.toml` into `crates/agentsi-voice/`, and change the three `include_str!` paths in `agentsi-voice` and `agentsi-core`. Otherwise the build fails.

`docs/guide/README.md` stays as a file. The program embeds it.

**Caution:** Keep the Postgres volume for 30 days after step 7. The way back is the tag `last-typescript` and that volume.

## Acceptance criteria

- [ ] Each golden test in `crates/agentsi-cli/tests/golden/` passes: `cargo test -p agentsi-cli --test golden`.
- [ ] The Rust core runs a meeting on each of the 23 stored snapshots, and each minutes passes `agentsi check`.
- [ ] `agentsi mcp` lists the 5 tools, with input schemas equal to those of today.
- [ ] The plugin skill `/agentsi` opens the front desk with the Rust CLI on the `PATH`. `plugin/skills/` has only the edits that "Plugin" names: the voice skill and the web text.
- [ ] `/agentsi:voice on` in a plain Claude Code session turns voice mode on, and the next reply is spoken and saved.
- [ ] Each web route of the table opens its screen in the app, as the owner sees.
- [ ] After step 7, `rg -l --type ts .` finds no file outside `node_modules`, and `cargo test --workspace` passes.
- [ ] After step 7, `git show last-typescript:packages/cli/src/main.ts` prints the old CLI.
