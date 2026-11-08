# Stories mode

**Status:** approved 2026-10-21 · **Source:** `docs/ideas/agentsi-stories.md`, grilling of 2026-10-21

## Problem
29 approved decisions carry an `action` that no code executes. The spec circuit is too heavy for most.

## Goal
- The owner or the PM proposes a story. The owner approves it in the web or the CLI.
- A story session works in a worktree of the product repo. QA writes the criterion tests first.
- Code checks the session. After the owner reviews the diff, the PM merges locally.
- Meetings see the open stories.

## Non-goals
- No multi-role story sessions.
- No push, deploy, migration or publishing from a story session.
- No conversion of `docs/specs/` into stories.
- No budget limit.

## Terms
The other terms are in `CONTEXT.md`.

- **Story config:** the registry block that turns on Stories mode and names the test and gate commands.

## Design

### 1. Story file
`workspace/<product>/stories/<id>-<slug>.json` and a generated `.md`. One counter per product gives the next number, with the prefix `S` or `T`.

```ts
interface Story {
  id: string;                 // "S014" | "T015"
  type: "story" | "task";
  slug: string;               // kebab-case, from the title
  title: string;              // Spanish, short; the call script uses this, never the id
  as_a: string | null;        // null only for a task
  i_want: string;
  so_that: string;
  criteria: Criterion[];      // 2 to 5
  owner_role: string;         // an agent id of the roster
  objective: string | null;   // a brief objective id
  sources: Source[];          // same type as minutes sources
  status: "not_implemented" | "implemented" | "discarded";
  approval: "pending" | "approved" | "changes_requested";
  changes_note: string | null;
  proposed_by: "owner" | "pm";
  minutes: string | null;     // the minutes that proposed it, if any
  session: StorySession | null;
  results: StoryResult[];     // newest last
  created_at: string; updated_at: string;
}
interface Criterion { given: string; when: string; then: string; test: string | null; checks: string }
// test: repo-relative path of the test file. null is permitted only for a task.
// checks: one plain sentence of what the test proves, for the web page.
interface StorySession {
  state: "queued" | "tests" | "work" | "checks" | "ready" | "failed";
  branch: string;             // "story/<id>-<slug>"
  worktree: string;           // "<repo>/.worktrees/<id>-<slug>"
  base: string; tests_commit: string | null; final_commit: string | null;
  attempts: number; failure: string | null; log: string; started_at: string;
  pid?: number | null;        // the `_exec` process, set when it starts
}
interface StoryResult { result: "merged_unchanged" | "merged_changed" | "discarded"; at: string; commit: string | null }
```

`writeAllowed` adds the `stories` area. Code validates each story before each write. Each write loads, checks, saves and commits under the workspace lock. A failed commit restores the files.

### 2. Story config
A product can have a `stories` block in `agentsi.config.json`:

```json
"stories": { "repo": "/abs/path", "test": "node --import tsx --test {files}", "gate": ["pnpm typecheck", "pnpm lint"] }
```

`repo` defaults to the first path. `{files}` gets the test paths. `agentsi` and a second product have the block.

### 3. Proposal: one typed call
- `agentsi story new "<text>" [--type task]`: the PM shapes the text into a story.
- `agentsi story suggest`: the PM proposes up to 3 stories.
- `agentsi story changes <id> "<note>"`: the PM rewrites the story. Approval goes back to `pending`.
- `approve`, `discard`, `list` and `show` make no model call. `discard` refuses a story with a session.
- A source can also cite an existing story.

### 4. Story session: a workflow in code
`agentsi story run <id>` needs an approved story and bwrap. It starts a detached `_exec`, which records its pid and does these steps:

1. Reindex the code graph, if the binary exists.
2. Create the branch and the worktree from the main branch.
3. Install the dependencies when a lockfile exists, before any session.
4. QA writes the criterion tests and nothing else. The runner commits them as `tests_commit`.
5. Check: each test fails on `tests_commit`.
6. The owner role makes the tests pass.
7. Checks: no file of `tests_commit` changed, the tests pass, and each gate command passes.
8. After a failed check, the owner tries once more. A second failure sets `failed`.
9. Success sets `ready` and `final_commit`.

Each session is `claude -p --restricted`, without the user's hooks and settings, with file tools and a Bash allowlist. The deny list blocks git, network tools, deploy, migrate and publish, and Edit of `node_modules` and `.git`. Agent code runs in two sandboxes (ADR 0002): the Claude Code Bash sandbox in the session, and bwrap around each runner test and gate. The run fails when a session leaves files in `.claude/` or `.worktrees/`. The log is `workspace/<product>/stories/<id>.log`.

A session cannot run a gate command that starts with the test prefix and then a flag, for example `sh -n greet.sh` with the test `sh {files}`. The flag deny rule wins over the exact allow rule. The runner still runs that gate itself.

### 5. Review and merge
- `agentsi story review <id>` prints the diff stat and the checks.
- `agentsi story accept <id>`: the PM merges the branch with `--no-ff`. A moved branch HEAD gives `merged_changed`. The status becomes `implemented`. The runner removes the worktree and deletes the branch.
- `agentsi story reject <id> [--force]` records `discarded`, removes the worktree and the branch, and resets the session to null, in any state. While `_exec` runs, reject needs `--force`, which kills it first.

### 6. Web
- `/p/[product]/stories`: the stories by status.
- `/p/[product]/stories/[id]`: the story, its criteria with `checks` and test paths, the session and the results.
- Three server actions call `decideStory`, as the CLI does: approve, changes and discard. "Changes" records the note, then the owner runs `agentsi story changes` for the PM rewrite. The read-only test permits them and `finishCall`.

### 7. Meetings
- The snapshot gets a `## Stories` section with the open stories and their state.
- A priority can cite `{ "type": "story", "ref": "S014" }`. The check rejects an unknown id.
- The call script names a story by its title, never by its id.

### 8. Code graph
`codebase-memory-mcp` serves meetings and story sessions as an MCP server. AgentSI reindexes the repo at the start of each one. Without the binary, both continue. `--safe-mode` drops the `--mcp-config` servers, so a call with the code graph omits it.

### 9. Front desk
The `agentsi` skill maps "nueva story", "sugerí stories", "corré la story" and "revisemos la story" to the commands.

## Acceptance criteria
- [x] `pnpm typecheck`, `pnpm lint` and `pnpm test` pass.
  Evidence:
  - .github/workflows/ci.yml:jobs.node (pnpm typecheck, pnpm lint, pnpm test)
  - docs/plans/agentsi-stories-hardening-log.md:U1 Tests (0 fail, 0 skipped)
- [x] A test shows that numbering gives `S001`, `T002`, then `S003`.
  Evidence: packages/core/src/story.test.ts:numbering gives S001, then T002, then S003 in one product
- [x] A test shows that the check rejects 1 or 6 criteria, `test: null` in a story, and an unknown `owner_role`.
  Evidence: packages/core/src/story.test.ts:the check rejects 1 criterion, 6 criteria, a null test in a story and an unknown owner_role
- [x] A test shows that `writeAllowed` accepts `workspace/<p>/stories/<file>` and rejects `workspace/<p>/stories/../x`.
  Evidence: packages/core/src/workspace-write.test.ts:stories accept one level of story files and refuse traversal, subfolders and other names
- [x] `agentsi story run` refuses a story with approval `pending`.
  Evidence: packages/cli/src/story-run.test.ts:story run refuses a story with approval pending and starts nothing
- [x] A runner test shows: the run fails when a test passes on `tests_commit` or the owner changes a test, and a moved branch HEAD gives `merged_changed`.
  Evidence:
  - packages/core/src/story-run.test.ts:the run fails when a criterion test passes on tests_commit
  - packages/core/src/story-run.test.ts:the run fails when the owner changes a test file on both attempts
  - packages/core/src/story-run.test.ts:accept gives merged_changed when the branch HEAD moved after final_commit
- [x] The session arguments contain the deny list, the sandbox `--settings` and no `--dangerously-skip-permissions`.
  Evidence:
  - packages/core/src/story-run.test.ts:the session arguments carry the deny list and no permission bypass
  - packages/core/src/story-run.test.ts:the session settings deny reads of the hidden paths and keep the kept ones readable
- [x] A bwrap test shows that a runner command cannot write outside the worktree, change `node_modules` or open a socket.
  Evidence:
  - packages/core/src/sandbox.test.ts:a sandboxed command writes in the worktree only, cannot change node_modules and has no network
  - packages/core/src/sandbox.test.ts:a sandboxed command cannot write to the main .git, through ../ or through a symlink out of the worktree
  - packages/core/src/sandbox.test.ts:a sandboxed command cannot connect to a unix socket in the runtime dir of the owner
  - packages/core/src/sandbox.test.ts:a sandboxed command sees ~/.ssh and the data root as empty folders, cannot read a .env file and still reads ~/.config/git
  - packages/core/src/story-run.live.test.ts:a real story session is refused git, curl, a node_modules write and extra test flags (AGENTSI_LIVE=1, run in U2, see docs/plans/agentsi-stories-hardening-log.md)
- [x] A test shows that reject refuses a running session, and that `--force` kills its process.
  Evidence: packages/core/src/story-run.test.ts:reject refuses a running session whose process lives, and --force kills that process first
- [x] Only the snapshot of a product with stories has a `## Stories` section.
  Evidence: packages/core/src/snapshot.test.ts:the Stories section lists the open stories and counts the closed ones, and is absent without stories
- [x] The minutes check rejects a `story` source with an unknown id.
  Evidence: packages/core/src/check.test.ts:a story source must name a story of the product, and the script must not say its id
- [x] The web story page renders the criteria, and its three actions change the story file.
  Evidence:
  - docs/reviews/web-smoke-demo.md:rows of the story page /p/miapp/stories/S001 (2 criteria, 4 of 4 PASS)
  - apps/web/lib/stories-write.test.ts:approve changes the story file and commits it
  - apps/web/lib/stories-write.test.ts:changes records the note and commits it
  - apps/web/lib/stories-write.test.ts:discard changes the status and commits it
- [x] `agentsi.config.json` has a `stories` block for `agentsi` and a second product.
  Evidence: manual, 2026-10-22: the agentsi.config.json of the data repo agentsi-data has a stories block for agentsi and for a second product. Commit 27edef8 of agentsi-data adds budget_usd 10 to the agentsi block.
- [x] ADR 0002 records the product repo writes, the sandbox and the web story actions.
  Evidence: manual, 2026-10-22: docs/adr/0002-story-sessions-write-product-repos.md, section Decision (worktree writes, two sandboxes, masked paths, web story actions) and section Consequences (install cost and gaps).
