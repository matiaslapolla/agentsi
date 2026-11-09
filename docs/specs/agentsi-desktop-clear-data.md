# AgentSI desktop: clear data

**Status:** approved 2026-11-07 · **Source:** owner decisions of 2026-11-07 (log "Owner decisions after the first owner checks"). The owner wants to select what to remove.

## Problem

The app and the CLI can only archive. Archive hides records and deletes nothing. The owner cannot delete a product, old meetings, voice transcripts, call audio or backups without SQL or a file manager.

## Goal

A "Data" section in the settings screen lets the owner select what to delete, see what the delete removes, and delete it. A backup runs before each delete.

## Non-goals

- A delete of a repo, a branch or a file in a repo. AgentSI never deletes the owner's code.
- A delete of models. The model rows of the settings screen already do it.
- An undo after the backup. The backup is the way back, with `agentsi restore`.
- A delete on an SSH host from this device.

## Terms

- **Clear plan:** the list of rows and files that a delete removes, with counts and bytes, made before the delete.

## Design

### What the owner can select

| Item | Scope | What goes |
|---|---|---|
| Product | one product | each row of the 17 tables and of the local tables (`tickets`) with that product, its `workspace/<product>/` folder, its registry entry |
| Meetings | one product, all or older than a date | minutes, their priorities, decisions, usage, snapshots, listens, meeting log lines, minutes documents, archive records, call folders |
| Stories | one product, done or rejected only | stories, story logs, their archive records. Worktrees and branches stay |
| Role learning | one product | memory, agent overlays, briefs, reflections. This is the existing `roles.reset` with memory |
| Call audio | one product or all | the `audio/` folders and their listens. Minutes stay |
| Voice transcripts | all, or older than a date | `voice/transcripts/*.md` |
| Logs | all | `logs/hook.log` |
| Backups | all but the newest | `backups/agentsi-*.db` and their registry copies |
| Everything | all | each item above, the database rows of each product, and the registry. The data root folder and the newest backup stay |

- Priorities and usage are keyed by `minutes_id` only, so they go with their minutes.

### Core

- New methods, additions to the protocol:
  - `data.clear_plan {items}` gives the clear plan: for each item, the row counts per table, the files and the bytes. It writes nothing.
  - `data.clear {items, plan_id}` deletes. The `plan_id` must match a plan of the last 5 minutes with the same counts, or the core refuses with `plan_changed`.
- `data.clear` first runs `data.backup`. If the backup fails, nothing is deleted.
- The rows go in one SQLite transaction. The files go after the commit. A file that cannot go is listed in the answer, and the rows stay deleted.
- A product with a running pane, meeting, story session or reflection is refused with `busy`, and the answer names the running work.
- After a delete, the core sends `data.changed` for each product touched, and runs `PRAGMA wal_checkpoint(TRUNCATE)` and `VACUUM` off the request thread.
- The CLI gets `agentsi clear <item> [--product <id>] [--before <date>] [--yes]`. With no `--yes` it prints the plan and deletes nothing.
- Each delete uses the core's paths only. A path that leaves the data root is refused.

### App

- The settings screen gets a section "Data" with the data root path, its size, and one row for each item of the table, with a check box and, where the table permits, a product choice and a date.
- "Review" shows the clear plan. "Delete" stays off until the plan shows, and it asks one Confirm that names the counts.
- After the delete, the section shows the backup path and the files that could not go.
- The product context menu gets "Delete product data", which opens this section with the product selected.

## Acceptance criteria

- [ ] Core tests on a store with two products delete each item of the table, and the other product keeps each of its rows and files.
- [ ] A plan matches the delete: the counts of the plan equal the deleted rows.
- [ ] A changed store between plan and delete gives `plan_changed` and deletes nothing.
- [ ] A failed backup deletes nothing.
- [ ] A product with a running pane gives `busy`.
- [ ] No test or code path deletes a file outside the data root, a repo, a branch or a worktree.
- [ ] `agentsi clear meetings --product x` with no `--yes` deletes nothing.
- [ ] A test of the real settings section selects an item, reviews the plan, confirms, and shows the backup path.
- [ ] `schema/core.openrpc.json` changes only by `data.clear_plan`, `data.clear` and their errors.
- [ ] The idle app draws 0 frames, and `rg -n 'rgb\(|hsla\(|px\(' crates/agentsi-app/src` finds nothing.
- [ ] The owner deletes old call audio from a copy of the data and checks the result.
