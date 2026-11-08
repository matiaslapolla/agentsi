# Data in the database

**Status:** approved 2026-11-02, done 2026-11-02 · **Source:** owner decisions on 2026-11-02 (one repo; data shaped to the database; local Postgres with a daily dump; audio in an ignored folder). Replaces the separate data repo of `docs/specs/agentsi-data-root.md`.

## Problem

AgentSI keeps its data in a second git repo, `agentsi-data`, and commits there after each meeting, mark, decision and story. The owner wants one repo. Most of the data is in files, and only products, minutes rows, priorities, decisions and usage are in Postgres.

## Goal

- The code repo is the only git repo of AgentSI.
- Postgres holds all the data of the agency. Only the registry, audio and backups stay as files.
- These files live in `data/` inside the code checkout, and git ignores `data/`.
- AgentSI makes no git commits of data.
- `agentsi backup` writes a dump of the database and a copy of the registry. A daily timer runs it.
- `agentsi data import <dir>` loads an old data root into the database. A second run changes nothing.

## Non-goals

- No hosted database. Postgres stays in the local Docker container.
- The registry stays a file that the owner can edit by hand: `data/agentsi.config.json`.
- Audio stays as files. AgentSI can render audio again from the minutes.
- No change to the product repos or to story sessions, which write only in the worktree of the product.

## Terms

**Data folder**:
The ignored `data/` folder of the code checkout. It holds the registry, the audio and the backups.

**Store**:
The module that reads and writes one kind of data in Postgres, such as the story store or the memory store.

## Design

### 1. Data folder

`resolveDataRoot` keeps its order: `AGENTSI_HOME`, then the home file, then a default. The default changes from the code checkout to `<code>/data`. The root `.gitignore` ignores `/data/`. The `workspace/` layout stays only for audio: `data/workspace/<product>/audio/<minutes id>/`.

### 2. Test database

Tests use PGlite, an in-process Postgres, through one helper, `testDb()`. It applies every migration in `packages/core/sql/` and returns a pool-like object. `setPool()` points `getPool()` at it. With this, a story session can test database code with no network.

### 3. Stores

One migration and one store for each kind of data. Each store replaces the file reads and writes of the CLI, the web app and the MCP server for its kind.

| Kind | Today | Table |
|---|---|---|
| Stories, story logs | `stories/*.json`, `*.md`, `*.log` | `stories`, `story_logs` |
| Role memory | `memory/<agent>.json` | `memory` |
| Roster overlays | `agents/<agent>.md` | `agent_overlays` |
| Brief | `brief.json` | `briefs` |
| Minutes body | `minutes/*.json`, `*.md`, `*.usage.json` | `minutes.body` (jsonb), `usage` |
| Meeting log, listens, reflections | `*.jsonl` | `meeting_log`, `listens`, `reflections` |
| Snapshots | `snapshots/*` | `snapshots` |

The Markdown views of stories and minutes are rendered on read, not stored.

### 4. No data commits

`commitPaths` and `commitWorkspace` go away, with their call sites. A store write is one transaction. A story session still commits in the product worktree, as before.

### 5. Backup

`agentsi backup` runs `pg_dump` in the Postgres container and writes `data/backups/<date>.sql.gz` with a copy of the registry. It keeps the last 14 dumps. A systemd user timer runs it once a day. `agentsi doctor` warns when the newest dump is older than 2 days.

### 6. Import

`agentsi data import <dir>` reads an old data root, which is `agentsi-data` for the owner. It writes each item with an upsert on its id, so a second run changes nothing. It copies audio and the registry into `data/`. It never deletes the old folder.

## Acceptance criteria

- [ ] `git -C <code> status` shows nothing under `data/` after a meeting, a mark, a decision and a story run.
- [x] No source file in `packages/` or `apps/` reads or writes `stories/`, `memory/`, `agents/`, `brief.json`, `minutes/` bodies, the `.jsonl` logs or `snapshots/` (a grep finds none).
- [x] `node --import tsx --test` passes for each store with `testDb()`, with no Postgres server running.
- [x] `agentsi data import ../agentsi-data` loads the owner's data. A second run reports 0 changes. `agentsi story list`, `agentsi desk` and the web app show the same items as before the import.
- [x] `agentsi backup` writes a gzipped dump that `psql` restores into an empty database. A restore gives the same counts per table.
- [x] `systemctl --user list-timers` shows the backup timer, and `agentsi doctor` reports the age of the newest dump.
- [x] Nothing in the code calls `git commit` on the data root.
