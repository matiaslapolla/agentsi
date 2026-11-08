---
status: done
date: 2026-10-21
---

# The public tree

Unit 6 of `docs/plans/agentsi-open-source.md`. It covers release plan steps 12 to 16 and the "Can wait" items of `docs/reviews/2026-10-21-open-source-readiness.md`.

## Problem

- The repo has no README, no LICENSE, no SECURITY.md, no CONTRIBUTING.md, no CI and no CHANGELOG. A stranger cannot install AgentSI or judge its risk.
- The manifests say `UNLICENSED` or nothing. The versions differ: the plugin says `0.2.0`, the packages and the MCP server say `0.1.0`.
- Hygiene items of should-fix 12 are open: `.claude/` is not in `.gitignore`, `agentsi memory migrate` still exists, and the web app copies the default database URL twice. The marketplace has a local-only name.
- Tests and some public docs name the owner and the owner's products.
- No script builds the public tree, and nobody ran the README on a clean machine.

## Non-goals

- No push, no remote and no GitHub repo. The owner publishes later.
- No behavior change in the CLI, the core, the web app or the voice package beyond the hygiene items.
- No translation of the Spanish UI.

## Design

### Files in the repo

- `LICENSE`: MIT, copyright "AgentSI contributors". Every `package.json` and `plugin/.claude-plugin/plugin.json` say `"license": "MIT"`. `packages/voice` keeps GPL-3.0-or-later.
- One version, `0.2.0`, in the plugin, every package, the voice package and the MCP server.
- `README.md` in English. It says that the web UI and some CLI output are Spanish-first. It covers prerequisites, install, the data root, `make dev`, the plugin install, a first meeting, `agentsi doctor`, costs and account terms, privacy and voices.
- `SECURITY.md`: the threat model of story sessions, the web app, the MCP server, the push guard, meeting roles and Postgres. It reflects the budget cap, the Host check and the edit scope of `docs/specs/agentsi-hardening.md`. Reports go to a GitHub private advisory.
- `CONTRIBUTING.md`: setup, the checks, the commit gate, STE for docs, a spec before code.
- `CHANGELOG.md`: one entry, `0.2.0`.
- `.github/workflows/ci.yml`: `pnpm install`, `pnpm typecheck`, `pnpm lint`, `pnpm test`, and `uv run pytest` in `packages/voice`. The tests need no Postgres and no `claude` CLI, so the job has no service.
- The tests make git commits, so the CI job sets a git identity. It installs bubblewrap for the story session tests. Without a working `bwrap`, those tests skip.

### Hygiene

- `.gitignore` gets `.claude/`.
- `packages/core/src/memory-migrate.ts`, its tests and the `agentsi memory` command go away.
- `apps/web/lib/env.ts` and `apps/web/scripts/demo.ts` import `DEFAULT_DATABASE_URL` from `@agentsi/core`.
- The marketplace is `agentsi`. The plugin installs as `agentsi@agentsi`.
- Tests use neutral product ids.
- Public docs (`CONTEXT.md`, `docs/adr/`, `docs/specs/`, `docs/research/`) say "the owner" and name example products. The read-time migration of the legacy `proposed_by` value stays, because old story files still hold that value.

### Export

- `scripts/export-public.sh <target>` extracts `git archive HEAD` into an empty target. It removes `docs/plans/`, `docs/ideas/`, `docs/reviews/` and `docs/handoffs/`.
- The script holds the list of private names, so it removes itself from the target too.
- The script then runs the name scan on the target and fails on a hit outside the allowed list. The allowed list holds the read-time migration in `packages/core/src/story.ts` and its test.
- A secret scanner (gitleaks in Docker, pinned version) scans the target.

### Clean run

- A fresh `node` container with the repo's Node major follows the README: `pnpm install`, `pnpm typecheck`, `pnpm lint`, `pnpm test`, `agentsi help`, `agentsi doctor`.
- A meeting needs a logged-in Claude Code. The clean run skips it.

### Local export

- The script exports into a sibling directory of the checkout, `../agentsi-public` from the main checkout. That directory gets `git init` and one commit by `AgentSI contributors` with the owner's GitHub noreply email. It has no remote.

## Acceptance criteria

- [x] `LICENSE` holds the MIT text. Every `package.json` and `plugin/.claude-plugin/plugin.json` say `"license": "MIT"`. `packages/voice/pyproject.toml` says `GPL-3.0-or-later`.
- [x] The plugin, every package, `packages/voice/pyproject.toml` and the MCP server say version `0.2.0`.
- [x] `README.md`, `SECURITY.md`, `CONTRIBUTING.md` and `CHANGELOG.md` exist with the sections of the design.
- [x] `.github/workflows/ci.yml` is valid YAML and runs install, typecheck, lint, the tests and the voice tests.
- [x] `.gitignore` has `.claude/`. `memory-migrate.ts` and `agentsi memory` are gone. `git grep -n "postgres://agentsi"` shows the URL in `packages/core/src/db.ts` and not in `apps/web`.
- [x] `.claude-plugin/marketplace.json` names the marketplace `agentsi`, and no public doc names the old local-only name.
- [x] No test or fixture uses the id of a product of the owner.
- [x] `scripts/export-public.sh` builds a tree without `docs/plans/`, `docs/ideas/`, `docs/reviews/` and itself.
- [x] The name scan of the exported tree has no hit outside the allowed list, and gitleaks finds no leak.
- [x] In a fresh Node container, the exported tree passes `pnpm install`, `pnpm typecheck`, `pnpm lint` and `pnpm test`, and `agentsi help` and `agentsi doctor` run.
- [x] `../agentsi-public` holds one commit by `AgentSI contributors` and no remote.
- [x] `pnpm typecheck`, `pnpm lint` and `pnpm test` pass, and `uv run pytest` passes in `packages/voice`.
