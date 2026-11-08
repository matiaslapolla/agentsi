---
status: done
date: 2026-10-21
---

# Hardening for a stranger's install

Unit 3 of `docs/plans/agentsi-open-source.md`. It covers should-fix 4, 6, 7 and 10 of `docs/reviews/2026-10-21-open-source-readiness.md`.

## Problem

- A story session has no cost cap. `storySessionArgs` passes no `--max-budget-usd`. One story session holds a QA session and up to two owner sessions of 30 minutes each.
- Nobody checked that a story session cannot `Edit` or `Write` a file outside its worktree.
- The web app has no auth and no Host check. A DNS-rebinding page can read every product's minutes and approve a story.
- A stranger has no command that tells which tool is missing and which feature it blocks.

## Facts from Claude Code 2.1.306

- A call that reaches `--max-budget-usd` exits 1 with a JSON result. The result has `is_error: true`, `subtype: "error_max_budget_usd"`, `terminal_reason: "budget_exhausted"` and no `result` text.
- With `--restricted`, the file tools refuse each path outside the cwd. This covers `..` paths, a sibling folder with the same name prefix, and a symlink inside the cwd that points outside. A real test with haiku proved it (see "Edit scope").

## Decision

### Story session budget

- The `stories` block of a product gets an optional key `budget_usd`, a positive number. The default is USD 5.
- The cap is for each story session call: the QA session and each owner attempt.
- Each story session passes the cap as `--max-budget-usd`.
- When a session stops at the cap, the story session ends `failed`. It records `stop: { reason: "budget", cap_usd }` and a failure text that names the session and the cap.
- After a budget stop, the runner does not start another owner attempt.
- The runner checks for an escape (a changed `.git` file, or files in `.claude/` or `.worktrees/`) before the budget stop. An escape is the reported failure.
- `agentsi story review` prints `Stopped: budget, cap USD <cap> for each session.` for such a session.
- `agentsi roles apply` accepts the key. `plugin/skills/agentsi/setup.md` names it in one line.

### Edit scope

- `--restricted` already confines `Read`, `Edit` and `Write` to the worktree. AgentSI adds no deny rule.
- A unit test keeps `--restricted` in the story session arguments.
- The proof used the exact `storySessionArgs` output plus `--model haiku --max-budget-usd 0.05`, in a throwaway git repo `wt` under `/tmp`. Six calls cost about USD 0.01 in total.

| Attempt by the session | Tool result |
| --- | --- |
| `Write` of a new file beside `wt` | "is outside …/wt, --restricted confines the file tools to the working directory" |
| `Write` into a sibling folder `wt-evil` | the same refusal |
| `Write` to `wt/../escape.txt` | the same refusal, with the path resolved |
| `Read` of a file beside `wt` (an `Edit` needs a `Read` first) | the same refusal |
| `Read` of `wt/link.txt`, a symlink to a file beside `wt` | "resolves through a symlink to …, which is outside …/wt" |
| `Write` to `wt/linkdir/y.txt`, where `linkdir` links to `wt-evil` | the same symlink refusal |
| `Write` of a new file inside `wt` | "File created successfully" |

- A test folder under `~/.claude/` gives a false result. Claude Code also refuses a `Write` inside the cwd there, because it protects `.claude` paths.

### Web Host check

- `apps/web/proxy.ts` runs on every request. It answers 421 when the hostname of the `Host` header is not `127.0.0.1`, `localhost` or `[::1]`.
- Any port passes, because `make guide` runs a second web app on 3364.

### `agentsi doctor`

- A new command checks each tool and prints one line for each check. The line has the status `ok`, `missing` or `fail`, and the version or the reason.
- A failed check names the feature that it blocks.
- Required checks: Node, pnpm, Docker, a reachable database, the Claude Code CLI with its version and the flags that AgentSI uses, and the data root (it exists, it is a git repo, it has a registry).
- The stories check: `bwrap` can start a sandbox.
- Optional checks: `gh`, `codebase-memory-mcp`, `uv`, `ffmpeg` and the TTS models folder (`TTS_MODELS_DIR`, else `~/dev/tools/models`).
- The database check gives up after 5 seconds. It hides the password of the database URL.
- The command exits 1 when a required check fails, else 0.
- The checks take their probes as arguments, so the tests run no real tool.
- `agentsi help` lists the command.

## Out of scope

- Prompts, `plugin/skills/agentsi/SKILL.md`, `plugin/agents/*`, `packages/mcp/`, Notion and the owner's name (unit 2).
- `packages/voice/` (unit 5).
- The README, SECURITY.md and the cost text (unit 6).
- A production build for `agentsi web`.

## Acceptance criteria

- [x] `storyConfig` returns `budget_usd: 5` for a stories block without the key, and the given value for a block with it. A value that is not a positive number fails with a clear message.
- [x] Each story session call passes `--max-budget-usd` with the cap of the product.
- [x] A fake session caller that reports a budget stop in the QA session makes the story session end `failed` with `stop.reason` `budget` and the cap, and the owner session does not start.
- [x] A budget stop in the first owner attempt ends the story session `failed` with `stop.reason` `budget`, and no second owner attempt starts.
- [x] The real session caller maps a `claude` result with `subtype: "error_max_budget_usd"` to a budget stop, and still throws on any other error.
- [x] `agentsi story review` prints the reason `budget` and the cap for a session that stopped at its cap.
- [x] `agentsi roles apply` accepts a `stories.json` with `budget_usd`, and rejects an unknown key as before.
- [x] `plugin/skills/agentsi/setup.md` names `budget_usd` in one line of the stories subsection.
- [x] A real `claude` run with the story session arguments cannot write a file outside its cwd. The report gives the commands and the outputs.
- [x] The Host decision accepts `127.0.0.1`, `localhost` and `[::1]` with or without a port, and rejects any other hostname and a missing Host.
- [x] A running web app on a port of 3365 or higher answers 421 to `curl -H "Host: evil.example"` and 200 to a request with `Host: 127.0.0.1:<port>`.
- [x] `agentsi doctor` prints one line for each check, names the feature that a failed check blocks, and exits 1 only when a required check fails. Tests prove it with fake probes.
- [x] `agentsi help` lists `doctor`.
- [x] Typecheck, lint and the tests pass.
