# AgentSI roles per product: setup, audit, roles, reflect, reset

**Status:** approved 2026-10-13 · **Source:** grilling session 2026-10-12/13 (Q1–Q24), research `docs/research/self-improving-agents.md` · **Plan:** `docs/plans/agentsi-roles.md`

## Problem

Every product gets the same five global roles (`plugin/agents/*.md`). A product can only pick which of them attend. The roles do not know the product's goals, the memory only grows (append-only, tail-cut at 6,000 chars, already stale), and nothing learns from failed checks, repairs or the owner's marks. The first-round check failures are lost (stderr only).

## Goal

Each product gets its own roster, tailored by an interview, and the agency improves from grounded signals under the owner's control:

- `/agentsi setup` analyzes the repo and interviews the owner, then writes the product's brief and roster.
- `agentsi audit` reports config health and meeting quality, deterministically.
- `agentsi roles` shows and validates the effective roster.
- `/agentsi reflect` curates memory automatically and proposes prompt changes gated by a replay eval and the owner's approval.
- `agentsi reset` returns an agent to the global role.
- The web shows the roster, the audit, the reflection history with diffs, and approves or reverts reflections.

## Non-goals

- No automatic reflection after each meeting (reflect runs only when invoked; audit says when evidence is ready).
- No LLM judgement inside `audit`.
- No writes into product repos. Ever.
- No ensemble voting, debate, fine-tuning, or DSPy/GEPA as libraries (research §1.8).
- `reflect` never edits `packages/`, `apps/`, `plugin/skills/`, `scripts/` or any `check` code.

## Terms (added to `CONTEXT.md`)

- **Agent**: one seat in a product's roster that answers in a meeting. It belongs to one role. Its id is the role (`dev`) or `role.focus` for a specialist (`dev.backend`).
- **Specialist**: an agent with a focus inside a role, with its own voice, memory and credit.
- **Roster**: the agents of a product (registry `roles`).
- **Overlay**: a product file that extends a global role for one agent.
- **Brief**: the product's definition, objectives, north stars and scope, written by setup.
- **Objective**: one goal of the brief, with a signal and a date. Priorities point to one.
- **Meeting log**: `meetings.jsonl`, one deterministic line per meeting.
- **Reflection**: one execution of reflect and the changes it applied or proposed.
- **Lesson**: a memory item learned from a failure or a mark, not from the repo.
- **Replay**: re-running one agent plus synthesis on frozen meetings to compare prompts.

The glossary keeps "run" under *Avoid*. Role keeps its meaning (perspective); "agent" stops being an avoided word and gets the meaning above.

## Design

### 1. Files

```
agentsi.config.json                  products[].roles = agent ids, e.g. ["pm","dev.backend","dev.mobile","qa","seo"]
workspace/<p>/brief.json             the brief (§3)
workspace/<p>/agents/<agent>.md      overlays and custom roles (§2)
workspace/<p>/memory/<agent>.json    memory items (§5)
workspace/<p>/meetings.jsonl         meeting log (§4)
workspace/<p>/snapshots/<minutes-id>.md   frozen snapshot per meeting (replay input)
workspace/<p>/reflections.jsonl      reflection history (§7)
```

All under `workspace/`, versioned and committed by the existing `commitWorkspace`. Agent ids match `^[a-z][a-z-]*(\.[a-z][a-z-]*)?$`.

### 2. Roster, overlays, specialists

`loadRole(agentId, productId)` resolves the effective definition:

1. If `workspace/<p>/agents/<agent>.md` exists:
   - with `extends: <global-role>` in its frontmatter: start from `plugin/agents/<global-role>.md`; frontmatter keys present in the overlay (`model`, `effort`, `tools`) replace the global ones; the overlay body is appended under `## This product`.
   - without `extends`: it is a complete custom role (setup writes it from `plugin/agents/_template.md`).
2. Else, `plugin/agents/<agent>.md` (today's behaviour).
3. A specialist id `dev.backend` with no overlay is an error (`roles validate` reports it).

The agent's **role** is `extends`, else the part before the dot, else the id. Colors and labels in the web use the role.

Meetings run every agent of the roster in parallel, as today. The synthesis still runs as the product's `pm` agent (overlay applied). `owner_role` in the synthesis holds an agent id. `check()` "every active role answered" becomes: for each role in the roster, at least one of its agents answered.

### 3. Brief

`workspace/<p>/brief.json`, validated by a schema in `packages/core/src/brief.ts`:

```json
{
  "version": 1,
  "product": { "what": "", "audience": "", "stage": "idea|building|launched|growing|maintenance" },
  "objectives": [ { "id": "o1", "text": "", "signal": "", "due": "YYYY-MM-DD" } ],
  "north_stars": [ "docs/product.md" ],
  "sources_of_truth": [ { "kind": "issues|notion|analytics|docs|other", "ref": "", "note": "" } ],
  "out_of_scope": [ "" ],
  "roles": { "<agent-id>": "why this agent is in the roster" },
  "updated_at": ""
}
```

- 1 to 3 objectives. `north_stars` are paths relative to the product root that must exist.
- Role and synthesis prompts get the brief rendered as XML tags (`<brief><product>…</product><objectives><objective id="o1" due="…">…</objective></objectives>…</brief>`), placed before the snapshot.
- With a brief, the synthesis schema requires `objective` (an objective id) on every priority, and `Priority` gains `objective: string | null`.
- `check()` with a brief:
  - every priority's `objective` is an id of the brief;
  - the north-star rule accepts only paths listed in `north_stars` (if the list is empty, the rule is skipped and the objective rule carries the weight);
  - without a brief, today's rules apply unchanged.

### 4. Meeting log and credit per agent

After each meeting, `writeMinutes` appends one line to `meetings.jsonl` and writes the frozen snapshot:

```json
{
  "minutes_id": "", "created_at": "", "question": "", "scope": null,
  "brief_hash": "sha256|null",
  "agents": {
    "<agent>": {
      "prompt_hash": "", "memory_hash": "", "model": "", "cost_usd": 0, "duration_ms": 0,
      "error": null, "priorities": 0, "sources": 0,
      "unresolved_sources": [ { "type": "path", "ref": "" } ],
      "sin_datos": 0
    }
  },
  "synthesis": {
    "first_check": { "passed": false, "failures": [""] },
    "repaired": true,
    "final_check": { "passed": true, "failures": [] },
    "invented_sources": [ { "type": "path", "ref": "" } ],
    "cost_usd": 0
  },
  "total_cost_usd": 0
}
```

- `unresolved_sources`: each agent's priority sources run through the same resolvers as `check()` before synthesis (export them from `check.ts`). This charges a bad source to the agent that cited it.
- `invented_sources`: sources in the synthesis that no agent gave and that are not in the snapshot.
- `sin_datos`: count of the case-insensitive phrase "sin datos" in the agent's answer and concerns.
- `first_check` is kept even when the repair passes. `Minutes.check` gains `first_failures: string[]`.
- `prompt_hash` is the sha256 of the effective system prompt; `memory_hash` of the memory text given to the agent.

### 5. Memory as items

`workspace/<p>/memory/<agent>.json`:

```json
{ "agent": "", "product": "", "items": [
  { "id": "m-3f2a", "text": "", "kind": "fact|lesson", "helpful": 0, "harmful": 0,
    "source": "minutes-id|reflection-id|migration", "created_at": "", "retired_at": null, "retired_why": null } ] }
```

- `writeMinutes` appends each `memory_notes` entry as a `fact` item (deterministic; exact-duplicate texts are skipped). No LLM.
- The prompt gets the active items as a bulleted list with ids, newest first, within `MEMORY_MAX_CHARS`; items that do not fit are named in a count line, never cut mid-item.
- A one-time migration (`agentsi memory migrate`) converts every `memory/*.md` into items (counters 0, `source: "migration"`) and deletes the `.md` files. It retires the known stale note in `workspace/agentsi/memory/qa.md` ("Pendientes del plan a 2026-10-11 …") with `retired_why: "stale: plan marks U1–U9 done"`.
- The web memory page and the MCP `role_memory` tool read the items (active first, retired collapsed).

### 6. Commands

CLI (`packages/cli`), deterministic, product detected from cwd or `--product`:

- `agentsi roles [agent] [--json]`: the roster with each agent's role, source (global / overlay / custom), model; with an agent id, its effective system prompt.
- `agentsi roles validate`: registry ids parse; every agent resolves; overlays parse and `extends` exists; specialists have overlays; brief validates; north stars exist; `brief.roles` keys equal the roster. Exit 1 with a list of failures.
- `agentsi roles apply <staging-dir> [--register --name <n> --path <abs>]`: validates a staged `brief.json` + `agents/*.md` + `roster.json` (agent ids), prints the diff against the current config, moves them into `workspace/<p>/`, updates the registry `roles` (and registers the product when `--register`), commits. Refuses on validation failure.
- `agentsi audit [--json]`: see §8.
- `agentsi reset <agent> [--memory] [--yes]` and `agentsi reset --all [--yes]`: removes the overlay (and with `--memory` the memory file); `--all` removes brief, overlays and roster changes back to the five global roles (memory kept unless `--memory`). Asks for confirmation unless `--yes`. Commits.
- `agentsi reflect [--memory-only] [--json]`, `agentsi reflect revert <reflection-id>`: see §7.
- `agentsi memory migrate`: §5.

Skill (`plugin/skills/agentsi/SKILL.md`, subcommand sections; long flows in `plugin/skills/agentsi/setup.md` and `reflect.md`):

- `/agentsi setup`: runs in the current Claude Code session (no `claude -p`).
  1. Detect the product (`agentsi whoami`); if unregistered, the flow registers it.
  2. Analyze the repo: stack, docs, existing north-star docs, recent activity, issues, existing memory and minutes.
  3. Interview the owner in rounds (grilling format: numbered questions with a recommended answer): stage, audience, objectives with signal and date, decisions to delegate, sources of truth, out of scope, roster (which roles, which specialists, which custom roles and why), model/effort per agent.
  4. Write the staged files to a scratch dir, run `agentsi roles apply`, show the diff, ask for a yes, apply.
  5. If the repo lacks a north-star doc, record a pending decision proposing one (never write it).
  - Update mode when a brief exists: re-analyze, show what changed against the brief and roster, ask only about the changes.
- `/agentsi audit`: runs `agentsi audit` and explains it; offers `/agentsi reflect` when evidence is ready.
- `/agentsi roles`, `/agentsi reset <agent>`: wrap the CLI.
- `/agentsi reflect`: runs `agentsi reflect`, then walks the owner through the result and any pending proposal.

### 7. Reflect

`agentsi reflect` (core `packages/core/src/reflect.ts`):

1. **Signals since the last reflection** (by id: each `reflections.jsonl` entry lists in `consumed` the minutes ids and memory item sources it processed, and everything else is new; entries without `consumed` count by their time): meeting log lines; marks and rejected decisions in minutes, credited to `owner_role` (an agent id); reverted reflections. Only grounded signals count; no self-critique pass.
2. **Memory curation**, per agent with signals or new items: one structured call (model `haiku`, effort low) gets the agent's active items plus its signals and returns deltas only:
   `{ "add": [{ "text": "", "kind": "fact|lesson", "evidence": ["minutes-id"] }], "retire": [{ "id": "", "why": "" }], "helpful": [""], "harmful": [""] }`.
   Code validates (ids exist, every add cites evidence from the signals, at most 5 adds and 10 retires per agent) and applies. Lessons come from failures and from marks, both accepted and rejected. The call never sees or returns the whole file.
3. **Prompt proposal**, only for an agent whose evidence crosses a threshold: the same failure class (unresolved source, invented source, missing objective, timeout, `sin datos` ≥ 2) in 3 meetings, or 3 rejected priorities owned by it. One call (model `sonnet`, effort high) reads that agent's traces (answers, check failures, marks) and proposes one change to its overlay (a new overlay with `extends` if none exists). When the same lesson text (normalized) holds in 2 or more products, it proposes a change to `plugin/agents/<role>.md` instead.
4. **Replay** (§9) runs the proposal. If it passes the guards, reflect records a decision (`kind: "reflection"`, `reflection_id`) with the diff and the replay table. If it fails, the entry is kept as `discarded` with its numbers.
5. One `reflections.jsonl` entry per execution, then commit.

```json
{
  "id": "r-2026-10-13-1530", "product": "", "created_at": "", "trigger": "manual",
  "signals": [ { "agent": "", "kind": "", "ref": "" } ],
  "memory": [ { "agent": "", "added": [""], "retired": [""], "helpful": [""], "harmful": [""], "diff": "unified diff of the rendered items" } ],
  "proposal": { "agent": "", "target": "workspace/p/agents/dev.md|plugin/agents/dev.md", "diff": "", "replay": {}, "decision_id": 0, "status": "pending|approved|rejected|discarded" } ,
  "status": "applied|reverted",
  "commit": ""
}
```

- **Writes are allowlisted** in one module (`packages/core/src/workspace-write.ts`): `workspace/<p>/memory/`, `workspace/<p>/agents/`, `workspace/<p>/reflections.jsonl`, and `plugin/agents/*.md` only through an approved reflection decision. Any other path throws. A unit test proves `packages/core/src/check.ts` cannot be written through it.
- **Approving** a reflection decision (`agentsi decide <id> approve`, MCP, or web) applies the stored diff with a hash guard: if the target file changed since the proposal, it refuses and marks the proposal stale. Rejecting records it; reflect does not propose the same diff again.
- **Revert** (`agentsi reflect revert <id>`): applies the inverse of the memory changes, bumps `harmful` on the items that entry added, appends an entry `{ "reverts": "<id>" }`, commits.
- The `decisions` table gains `kind text default 'minutes'` and `reflection_id text null` (migration `NNN_reflection_decisions.sql`). `minutes_id` stays null for reflection decisions.

### 8. Audit

`audit(product)` in core returns a typed report; the CLI prints it, `--json` dumps it, the web renders it.

- **Config**: the `roles validate` failures; objectives past their date.
- **Meetings** (from `meetings.jsonl`, last 20): count; first-pass check rate; repair rate; per agent: answers, errors and timeouts, unresolved sources per meeting, `sin_datos` per meeting, mean cost; synthesis share of cost; invented sources.
- **Repairs** (from the minutes files, every meeting): repaired meetings and repair rounds per meeting for the first 2 minutes (the o1 baseline) and for every later one. See `docs/specs/agentsi-audit-repair-baseline.md`.
- **Marks**: per owning agent: accepted, changed, rejected, unmarked.
- **Memory**: per agent: active, retired, lessons, items never marked helpful after 10 meetings, characters vs `MEMORY_MAX_CHARS`.
- **Reflections**: last one, pending proposals, reverted count.
- **Ready to reflect**: agents past the §7.3 threshold, or "no hay evidencia suficiente".

### 9. Replay

`packages/core/src/replay.ts`:

- **Cases**: the product's meetings that have a frozen snapshot and in which the agent answered. A minimum of 4; below that, reflect reports "evidencia insuficiente para proponer" and stops at memory curation. Cases split by id hash into a tuning half (shown to the proposal call) and a held-out half (used only by replay). A case is held out when the first byte of sha256(minutes id) is even, and replay needs at least 2 held-out cases. Minutes without a frozen snapshot, such as 2026-10-11-0402, are not cases. They still count as meetings toward the o1 signal and its repair baseline (decision 39).
- **Run**: for each held-out case, re-run the changed agent with the proposed prompt on the frozen snapshot, keep the other agents' answers from the minutes, re-run synthesis and `check()`. The baseline is the stored answer and check result, re-run with the current prompt only when its `prompt_hash` differs from today's.
- **Metrics**: first-pass check rate, unresolved sources of the agent per case, invented sources, missing objectives, cost of the agent.
- **Guards**: the proposal passes only if no metric gets worse on the held-out half and at least one improves. A budget cap of 15 USD per replay (`maxBudgetUsd`) aborts it as `discarded`.
- The replay table (per case and totals, before vs after) goes into the reflection entry and the decision text.

### 10. Web

`/p/[product]/roles` (link next to Memoria on the product page):

- **Roster**: each agent with role color, source (global / overlay / custom), model and effort, the `why` from the brief; expanding shows the overlay body and the effective prompt.
- **Brief**: product, objectives with signal and date, north stars, out of scope.
- **Audit**: the §8 report.
- **Reflections**: newest first; each shows signals, memory changes as a rendered diff, and the proposal with its replay table.
  - Pending proposal: Approve and Reject buttons (existing `decide` action).
  - Applied memory change: Revert button with confirmation (new action calling `agentsi reflect revert`).
- Specialists use their role's color and the label `Dev · backend`.

## Acceptance criteria

- [ ] `CONTEXT.md` defines Agent, Specialist, Roster, Overlay, Brief, Objective, Meeting log, Reflection, Lesson and Replay; "run" stays avoided.
- [ ] `loadRole(agent, product)` returns overlay-merged definitions; a unit test covers extends + frontmatter override + appended body, a custom role, a specialist without overlay (error), and a product with no overlays (today's output, byte for byte).
- [ ] A meeting on a product with roster `["pm","dev.backend","dev.mobile","qa"]` runs four agents in parallel, and `check()` passes the "every role answered" rule when only one of the two dev specialists answered.
- [ ] Every meeting appends one valid line to `workspace/<p>/meetings.jsonl` and writes `workspace/<p>/snapshots/<minutes-id>.md`; `first_check.failures` is kept when the repair passes; an agent that cites a missing path has it in its `unresolved_sources`.
- [ ] With a `brief.json`, every priority has an `objective` that is a brief id, and `check()` rejects a north-star citation that is not in `north_stars`; without a brief, `check.test.ts` passes unchanged.
- [ ] `memoryDir` holds only `<agent>.json` files after `agentsi memory migrate`; the stale `qa.md` plan note is retired with a reason; a meeting appends notes as `fact` items and skips exact duplicates; prompts never cut an item mid-text.
- [ ] `agentsi roles`, `agentsi roles <agent>`, `agentsi roles validate`, `agentsi roles apply`, `agentsi audit [--json]`, `agentsi reset` and `agentsi reflect [revert]` exist and appear in `agentsi help`.
- [ ] `agentsi roles apply` refuses an invalid stage (exit 1, failures listed) and on a valid one updates the registry roster, moves the files and makes one commit.
- [ ] `/agentsi setup` is documented in the skill: analysis, interview rounds, staged apply with diff and confirmation, update mode, registering an unregistered product, and a pending decision instead of writing a north-star doc.
- [ ] `agentsi audit --json` on `agentsi` and a second product returns the §8 sections, and it makes no LLM call (unit test with the claude wrapper stubbed to throw).
- [ ] `agentsi reflect` writes one `reflections.jsonl` entry and one commit; memory deltas that cite no signal are dropped; the curation call never receives a whole memory file as text to rewrite.
- [ ] Writing any path outside the §7 allowlist through `workspace-write.ts` throws; a test proves `packages/core/src/check.ts` and `plugin/skills/` are refused.
- [ ] Below 4 replay cases, reflect makes no prompt proposal and says so; with a stubbed claude wrapper, a proposal that worsens any held-out metric is `discarded` and one that improves without worsening becomes a pending decision with `kind: "reflection"`.
- [ ] Approving a reflection decision applies its diff and commits; if the target changed since the proposal, approval refuses and marks it stale; `reflect revert` restores the previous items and bumps `harmful` on the reverted additions.
- [ ] `/p/<id>/roles` shows roster, brief, audit and reflections; a pending proposal can be approved or rejected and an applied memory change reverted from the page.
- [ ] The guide (`scripts/guide.ts`) has a step for `/agentsi setup` and lists the new commands.
- [ ] Root `pnpm typecheck`, `pnpm lint` and `pnpm test` pass on `main` after the merge.
