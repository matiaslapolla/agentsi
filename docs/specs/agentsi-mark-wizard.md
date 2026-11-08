# Mark wizard

**Status:** draft · **Source:** The owner's request on 2026-10-17 and a grilling round the same day

## Problem

The owner marks each priority with one `agentsi mark <minutes-id> <idx> <mark>` call. They also run one `agentsi decide` call for each decision. Four priorities and five decisions mean nine calls, and they must look up each id by hand.

## Goal

- The owner sees every pending item in one list and answers each item in one pass.
- The wizard reads data that exists. It calls no model and creates no text.
- The same data feeds two front ends: a terminal wizard and the front desk in Claude Code.
- One pass of marks makes one commit.

## Non-goals

- No model call. No generated summary or suggestion.
- No change to `agentsi mark <minutes-id> <idx> <mark>` and `agentsi decide <id> approve|reject`. They keep working.
- No write path in the web app. It stays read-only.
- The wizard never runs the `Action` of an approved decision. It prints the action, as `decide` does.

## Terms

**Pending item**:
A priority with no mark, or a decision with status `pending`, in a product.

## Design

### 1. Pending items (core)

`packages/core/src/pending.ts` exports `pendingItems(product, { root, db })`.

- Priorities come from `readMinutesFiles(product)`. A priority is pending when `mark` is null. Minutes with status other than valid give nothing.
- Decisions come from the table `decisions` with `status = 'pending'`. They carry their database id, so `decide()` can take them.
- Order: oldest minutes first. In each minutes, priorities by `idx`, then its decisions. Product decisions (no minutes) come last.
- Item shape for a priority: `{ kind: "priority", minutes_id, idx, title, why, owner_role, objective, sources }`.
- Item shape for a decision: `{ kind: "decision", id, minutes_id, text, why, action }`.
- If the database is down, the function returns the priorities and sets `decisions_error`. The wizard shows that line and goes on with priorities.

### 2. Batch mark (core)

`applyMarks(entries, product)` in `minutes.ts` takes `{ minutesId, idx, mark, isNew }[]`. It writes each minutes file once, updates the table `priorities`, indexes the workspace once and makes one commit. The message is `mark(<product>): <n> priorities` and lists each `<minutes-id> #<idx> <mark>` in the body. `applyMark` calls `applyMarks` with one entry, and its commit message stays as it is.

An unknown minutes id or index stops the whole batch before any write.

### 3. CLI

`agentsi mark` gets three new forms. The form with three positionals does not change.

- `agentsi mark --pending [--json] [--product <id>]` prints the pending items. The default output is a numbered text list. `--json` prints the item array and nothing else.
- `agentsi mark --set <minutes-id>:<idx>:<mark>[:new] ...` applies several marks in one commit. `--set` repeats.
- `agentsi mark` with no positionals and a TTY on stdin runs the wizard. Without a TTY it exits with the usage line.

Wizard flow:

1. Print the count of pending items by kind. If the count is zero, print `Nothing pending.` and exit 0.
2. For a priority, print title, owner role, objective, why and sources. Ask `[a]ccept [c]hange [r]eject [s]kip [q]uit`. After a mark, ask `New? [y/N]`.
3. For a decision, print text, why and action. Ask `[a]pprove [r]eject [s]kip [q]uit`.
4. `q` or Ctrl+C stops asking. The wizard still applies the answers already given.
5. At the end, apply all marks with `applyMarks`, then each decision with `decide()`. Print the `Action:` line of each approved decision. A stale decision prints the same error as `agentsi decide` and does not stop the others.
6. Print a summary: `marked <n>, approved <n>, rejected <n>, skipped <n>`.

`decide()` keeps one commit for each decision, because it already commits by itself.

### 4. Front desk

`plugin/skills/agentsi/SKILL.md` section 3, step 4, and section 5 point to this flow:

1. Run `agentsi mark --pending --json`.
2. Ask the owner about the items with `AskUserQuestion`, up to 4 items in each round.
3. Run one `agentsi mark --set ...` for all the marks. Run `agentsi decide` for each decision they answered.

The skill says that a skipped item stays pending.

### 5. Guide and web

The web page of the minutes shows `agentsi mark` in a code span beside the title of the priorities, when one priority has no mark. The guide gets one line for the wizard.

## Acceptance criteria

- [ ] `agentsi mark --pending --json` prints the unmarked priorities and the pending decisions of a seeded product, oldest first.
- [ ] `agentsi mark --pending` prints `Nothing pending.` when a product has none.
- [ ] `agentsi mark --set a:0:accepted b:1:rejected:new` writes both marks and makes one commit.
- [ ] A batch with one unknown minutes id writes nothing and exits 1.
- [ ] The wizard run on a TTY with scripted answers marks, approves, rejects and skips as answered, and prints the `Action:` line for an approved decision.
- [ ] The wizard run with `q` after one answer still applies that answer.
- [ ] `agentsi mark` with no positionals and no TTY prints the usage line and exits 1.
- [ ] `agentsi mark <minutes-id> <idx> accepted` still works as before.
- [ ] With the database down, `agentsi mark --pending` lists the priorities and one `decisions_error` line.
- [ ] Typecheck, lint and the tests of core, cli, mcp and web pass.
