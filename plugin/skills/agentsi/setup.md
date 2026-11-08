# /agentsi setup

Configure the roster, the brief and Stories mode of one product. You run in the current Claude Code session. Do not call `claude -p`. Talk to the owner in the owner's language, direct: the `owner.language` field that `agentsi whoami` prints.

**Never write into the product repo.** You write only to a scratch dir. `agentsi roles apply` moves the files into `workspace/<product>/` of the data root.

Run each `agentsi` command alone in its Bash call: no pipes, redirects or `;`.

## 1. Detect the product

1. Run `agentsi whoami`.
   - Exit code 0: you have the product. Go to step 2.
   - Exit code 2: the cwd matches no product. Run `agentsi whoami --list` and ask the owner: pick one of those, or register this directory as a new product.
2. Registering: propose an id (lowercase, from the repo folder name), a display name, and the absolute path of the repo (`pwd`). Wait for the owner's OK. Keep the three values for step 7. `agentsi roles apply --register` writes the registry entry.
3. Run `agentsi whoami --home`. It prints the data root, the folder that holds `agentsi.config.json` and `workspace/`.
4. Check for a brief: the brief lives in the database (the `briefs` table), not in the data root. Run `agentsi roles validate` for the product: when the product is registered and the roster has a brief, this is **update mode** (section 9). Otherwise it is a first setup.

## 2. Analyze the repo

Keep it cheap. Read the least that answers the interview questions. Stop at about 15 files.

Read, in this order:
1. The README, `CLAUDE.md`, and the `docs/` index.
2. The manifest (`package.json`, `composer.json`, `Cargo.toml`, `pyproject.toml`) for stack and scripts. Note the test runner and the typecheck and lint scripts.
3. North-star candidates: `docs/product.md`, `docs/vision*`, `ROADMAP*`, `docs/ideas/`, `docs/specs/`.
4. Activity: `git log --oneline -30` and the open issues if `gh` has access.
5. For a registered product: `agentsi status`, the last minutes, and `agentsi roles` for the current roster.

Use the `search` and `get_doc` MCP tools when the product is indexed. Use `Grep` and `Glob` before you read whole files. Do not read the lockfile, build output or tests.

Write down for yourself, in 6 lines at most:
- what the product is, and for whom, as far as the repo says;
- the stage it looks like it is in;
- the north-star docs you found (repo-relative paths), or "none";
- the areas of the code (backend, mobile, web, SEO, infra) that could deserve a specialist;
- the sources of truth you saw (issues, analytics, docs);
- the test runner and the gate scripts, and whether tests need a database or the network;
- what you could not tell.

Show the owner these lines before the first round. Say "no data" for what you could not tell. Never invent an answer to fill a gap: ask.

## 3. Interview

Ask in rounds of 3 to 5 questions. Each question has a number and a **recommended answer** you derived from the analysis. The owner can accept or correct each answer by number, for example "1 ok, 2 no: ...". Ask the next round only after the owner answers the current one. Put the frontier first: ask what the repo cannot tell you, and skip what it already answers. Confirm the rest in one line.

Round 1: the product.
- Stage: `idea`, `building`, `launched`, `growing` or `maintenance`.
- What it is, in one sentence, and the audience.
- Out of scope: what the agency must never propose.

Round 2: objectives.
- 1 to 3 objectives. Each has a text, a **signal** (how the owner will know), and a **due** date (`YYYY-MM-DD`).
- Which doc is the north star. If there is none, see section 6.
- Sources of truth: `issues`, `analytics`, `docs` or `other`, each with a ref and a note.

Round 3: the roster.
- Which of the five global roles attend (`pm`, `dev`, `qa`, `social`, `scout`) and why. `pm` is always in: it writes the synthesis.
- Specialists: a role with a focus, as `role.focus` (`dev.backend`, `dev.mobile`). Propose one only if the analysis shows two clearly different areas. Each specialist needs an overlay.
- Custom roles (`seo`, `legal`): propose one only for a judgment no global role covers.
- Decisions the owner wants to delegate to an agent, if any.

Round 4: models.
- Model and effort per agent that differ from the global role. Recommend the defaults unless the agent reads a lot of code (`opus`, effort `high`) or does simple triage (`haiku`).

Round 5: Stories mode (optional).

A story session implements an approved story in its own worktree. It runs the tests and the gate inside a sandbox with no network. Ask:
- On or off. Recommend on only if the repo has a test runner and a gate that do not need a database or the network.
- The test command. It must contain `{files}`, which gets the test paths of the story. Recommend the runner of the manifest. Example: `node --import tsx --test {files}`.
- The gate: a list of commands that must pass. Recommend the typecheck and lint scripts of the manifest. Example: `["pnpm typecheck", "pnpm lint"]`.
- The repo, only if the code is not at the first registered path.

Before you recommend on, check that `bwrap` and `codebase-memory-mcp` are on the PATH (`command -v`). Without `bwrap`, recommend off and say why. Without `codebase-memory-mcp`, stories still work, without the code graph.

Close with a one-screen summary of every answer. Ask the owner to confirm it. Do not stage anything before the owner says yes.

## 4. Brief fields

`brief.json`, valid for the schema in the spec (§3):

```json
{
  "version": 1,
  "product": { "what": "", "audience": "", "stage": "building" },
  "objectives": [ { "id": "o1", "text": "", "signal": "", "due": "2026-12-31" } ],
  "north_stars": [ "docs/product.md" ],
  "sources_of_truth": [ { "kind": "issues", "ref": "", "note": "" } ],
  "out_of_scope": [ "" ],
  "roles": { "pm": "why this agent is in the roster" },
  "updated_at": "2026-10-13"
}
```

Rules:
- 1 to 3 objectives, ids `o1`, `o2`, `o3`.
- `north_stars` are paths relative to the product root, and the files must exist. List none rather than a path that does not exist.
- `roles` has one key per roster agent, no more and no fewer. The value is one line on why the agent is there.
- `updated_at` is today.

## 5. Roster and overlays

`roster.json` is an array of agent ids, for example `["pm","dev.backend","dev.mobile","qa","seo"]`. Ids match `^[a-z][a-z-]*(\.[a-z][a-z-]*)?$`.

Write `agents/<agent-id>.md` for each agent that differs from its global role.

**Overlay** (a specialist, or a global role with a product-specific change). Frontmatter has `extends: <global-role>` plus only the keys that change (`model`, `effort`, `tools`). The body is appended to the global prompt under `## This product`. Keep it to what is true of this product: the area the agent judges, the files it reads, the trade-offs it weighs.

```markdown
---
extends: dev
model: sonnet
effort: medium
---

You cover the backend: `app/`, the API routes and the migrations.
You weigh: data integrity before speed, and breaking changes to the API.
```

**Custom role** (no `extends`). A complete definition. Copy `plugin/templates/role.md`, replace every `<...>` placeholder, and set `name` to the agent id. Keep it to persona lines (job, what it looks at, what it weighs): the shared rules (language, sources, the "[no data]" marker, memory notes) are added by AgentSI at call time.

Each specialist needs an overlay. A global role with no change needs no file.

### Stories block

`stories.json` in the stage dir sets the `stories` block of the product in `agentsi.config.json`:

```json
{ "test": "node --import tsx --test {files}", "gate": ["pnpm typecheck", "pnpm lint"] }
```

- An object turns Stories mode on, or changes it. `repo` is optional, an existing absolute path.
- `budget_usd` is optional: the cap in USD for each story session (the QA session and each owner attempt). The default is 5.
- `null` turns Stories mode off.
- No file leaves the current block as it is.

Stage the file only when round 5 changes the block.

## 6. North star missing

If the repo has no north-star doc (no vision, product or roadmap doc), do not write one. Do not put a made-up path in `north_stars`. Do this instead:

1. Leave `north_stars` as `[]`. The objectives then carry the weight in the check.
2. Tell the owner: the repo has no north star. Propose the decision text in the chat, in one sentence, with its reason. Example: "Write `docs/product.md` with the vision, the audience and what stays out." Reason: "The repo has no north star, so the priorities cannot cite one."
3. Wait for the owner's explicit yes. Then record it: `agentsi decision add --product <id> --text "<text>" --why "<why>" [--action "<what to run once approved>"]`. It prints the id of the pending decision. If the owner says no, record nothing.

## 7. Stage, dry run, diff, apply

1. Make a scratch dir with `mktemp -d` (use `$CLAUDE_JOB_DIR/tmp` when it is set). Put `brief.json`, `roster.json`, `agents/*.md` and, if round 5 changed it, `stories.json` in it with the Write tool.
2. Dry run. Without `--yes`, `agentsi roles apply` writes nothing:
   - registered product: `agentsi roles apply <dir> --product <id>`
   - new product: `agentsi roles apply <dir> --register --product <id> --name "<name>" --path <abs-path>`
3. If it fails, it prints the failures and exits 1. Fix the staged files, not the validation, and run the dry run again. Show the owner what you fixed.
4. If it passes, it prints the roster change and a diff of every file. When the stories block changes, it also prints a `Stories:` line. Show that output in the chat, shortened if it is long. Say in two lines what changes for the meetings.
5. Ask the owner to apply it. Wait for the owner's explicit yes. A silence, or a "go on" about something else, is not a yes.
6. Run the same command with `--yes`. It moves the files into `workspace/<product>/`. It updates the roster and the stories block in the registry. It makes one commit.
7. Run `agentsi roles validate`. Report its result.

If the owner says no, change the stage, or stop and leave the scratch dir. Nothing was written.

## 8. Close

Tell the owner in at most 6 lines:
- the roster and the objectives set;
- the commit that applied it;
- the north-star decision, if section 6 applied;
- that the next meeting uses the new roster;
- with Stories mode on, that `/agentsi story` works, and that "new story <text>" starts one;
- that `/agentsi audit` shows how the agents do after a few meetings, and `/agentsi setup` again updates the brief.

## 9. Update mode

A brief exists.

1. Read the brief and the roster (`agentsi roles`). Analyze the repo as in section 2, but only look for changes: new docs, a new area of code, a stage change, objectives past their date (`agentsi audit` lists them), north-star files that moved.
2. Run `agentsi audit` and read the config and marks sections. An agent that is often rejected may need a different focus.
3. Read the `stories` block of the product in `agentsi.config.json` of the data root. Check that each gate command still exists as a script. Check that the test command still matches the runner. Without a block, ask round 5 once.
4. Show a short list of what changed against the brief, the roster and the stories block. Ask only about the changes, in numbered questions with a recommended answer.
5. Stage a **complete** new set (the brief and roster as they will be, not a patch), plus the overlays that remain. The apply diff shows what is new, changed and removed. To drop an agent, leave it out of `roster.json` and out of `agents/`.
6. Continue from section 7. Stage `stories.json` only if the block changes.

## Limits

- No write into the product repo, no push, no publish.
- Only `agentsi roles apply --yes` changes the workspace and the registry, and only after the owner's yes. Never edit `agentsi.config.json` by hand.
- If a command fails, show the error. Do not write workspace files by hand.
