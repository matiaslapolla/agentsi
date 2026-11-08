# AgentSI and Notion split by audience

**Status:** approved 2026-10-14 · **Source:** `docs/ideas/agentsi-notion-split.md` (shaped, skeptic TEST FIRST; the owner chose to build now) · **Plan:** `docs/plans/agentsi-notion-split.md`

## Problem

The owner has two approval paths: buttons in the web app and the conversation in Claude Code. A decision stores a verdict and nothing more. Notion knows nothing about the minutes.

## Goal

- AgentSI serves agents. The web app only reads.
- The owner approves by directive in Claude Code. `agentsi decide <id> approve` prints the action of the decision, and the agent runs it.
- `agentsi sync-notion` prepares a compact payload from the workspace. The front desk loads it into Notion through the `notion` skill. Data goes one way, from AgentSI to Notion.
- A hook asks for the owner's confirmation before a push.

## Non-goals

- No Notion code or token inside AgentSI. The CLI never calls Notion.
- No typed `action`. It is free text (iterate later).
- No change to bare `agentsi` (it prints the help).
- No write path from Notion back to AgentSI.

## Design

### 1. Registry

Remove `youpy` and `experimentos` from `agentsi.config.json`. Keep `workspace/`. Update the tests that name them.

### 2. Read-only web

Delete `DecisionButtons`, `MarkControls`, `RevertButton` and the actions `markPriority`, `decide` and `revertReflection`. Keep `finishCall` (it logs a listen, not an approval). Where a button stood, show the command in a code span: `agentsi decide <id> approve|reject`, `agentsi mark <minutes-id> <idx> accepted|changed|rejected`, `agentsi reflect revert <id>`. The owner may also tell the agent in chat.

### 3. Decision `action`

- A decision has an optional `action`: one sentence that says what to do if the owner approves.
- The meeting schema asks each decision for `action`. The prompt says: "what an agent would do if the owner approves, in one sentence".
- `sql/005_decision_action.sql` adds `decisions.action text`. A minutes JSON keeps `action` beside `text` and `why`.
- `renderMinutesMarkdown` writes `— why` and then `→ action` when it is set.
- `decide()` returns `action`. For a reflection decision it returns `null`.
- `agentsi decide <id> approve` prints `Action: <text>` after the status. A rejected decision prints no action.
- The snapshot, the MCP tool `pending_decisions` and the minutes page show `action` when it is set.
- Section 5 of the skill says: after `approve`, run the `Action` if the owner's word covers it, and ask when it does not.

### 4. Push guard

`plugin/hooks/hooks.json` adds a `PreToolUse` hook on `Bash`. The script `plugin/hooks/push-guard.mjs` reads the hook JSON from stdin. If the command runs `git push` or `gh pr create`, it answers `permissionDecision: "ask"` with a reason in Spanish. Otherwise it prints nothing. The hook applies in each session that has the plugin enabled. An explicit "pushea" from the owner passes because the owner confirms the prompt.

### 5. `agentsi sync-notion`

- `packages/core/src/notion-payload.ts` exports `buildNotionPayload(product, { root, last, now })`. It reads only files: `minutes/*.json` and `reflections.jsonl`. It makes no database or network call.
- The payload is JSON: `{ product, generated_at, tickets[], log[] }`. It covers the last 3 valid minutes (`last`). Invalid minutes give nothing.
  - A priority with mark `accepted` or `changed` gives a ticket `{ kind: "priority", minutes_id, idx, name, summary, estado: "Por hacer", source, mark, is_new, objective }`. A rejected or unmarked priority gives nothing.
  - A decision with status `pending` gives a ticket `{ kind: "decision", minutes_id, idx, name, summary, estado: "Esperando", source }`, with `action` when it is set.
  - A reflection proposal with status `pending` gives a decision ticket too (`name` is "Prompt proposal for <agent>").
  - A reflection entry gives a log item `{ kind: "reflection", id, created_at, status, signals, memory_added, memory_retired, proposal, source }` (counts, no prose).
  - `source` is a path relative to the repo root, with a fragment that makes the ticket unique: `#<idx>` for a priority, `#d<idx>` for a decision, `#<id>` for a proposal.
- `agentsi sync-notion [--product <id>] [--last <n>]` prints the payload as JSON.
- The skill `plugin/skills/agentsi/commands.md` adds `/agentsi sync-notion`: run the CLI, then run the Sync mode of the `notion` skill with the payload as an extra source, then show what Notion got.
- The `notion` skill (`sync.md`) adds the payload as a source and the mapping to Horizonte and Conocimiento (Bitácora). It keeps its rules: no emoji, no secrets, link and do not copy.

## Acceptance criteria

- [ ] `agentsi.config.json` has no `youpy` and no `experimentos`; no file under `workspace/` changes in the branch (`git diff main...HEAD --stat`); typecheck and tests pass.
- [ ] The web app has no `DecisionButtons`, `MarkControls`, `RevertButton`, `markPriority`, `decide` or `revertReflection`, and `finishCall` is its only action (unit test); `apps/web` typechecks and lints (the build runs in the end-to-end check).
- [ ] The product page, the minutes page and the roles page show the command to run in place of each removed button.
- [ ] A decision with `action` keeps it through `meet`, the minutes JSON, the database (migration 005), the minutes markdown, the snapshot and `pending_decisions`; a decision without `action` behaves as before.
- [ ] `agentsi decide <id> approve` prints `Action: <text>` when the decision has one; `reject` and reflection decisions print none (unit test).
- [ ] `push-guard.mjs` answers `ask` for `git push`, `git push origin main` and `gh pr create`, and prints nothing for `git status`, `git commit` and a command that only names `push` in a string (unit test).
- [ ] `buildNotionPayload` returns a ticket for each accepted or changed priority and each pending decision of the last valid minutes, none for a rejected or unmarked priority or for invalid minutes, unique `source` keys, and a log item for each reflection (unit test with a fixture).
- [ ] `buildNotionPayload` makes no database or network call (it runs with no `DATABASE_URL`).
- [ ] `agentsi sync-notion --product agentsi` prints valid JSON with no database and appears in `agentsi help` (unit test that runs the CLI).
- [ ] `commands.md` documents `/agentsi sync-notion`: the CLI call, the hand-off of the JSON to the Sync mode of the `notion` skill, and that data goes one way. (The matching `sync.md` lives in the `claudeconfig` repo, commit e55b54c, and is outside this gate.)
- [ ] Section 5 of `SKILL.md` says to run the `Action` after `approve`.
