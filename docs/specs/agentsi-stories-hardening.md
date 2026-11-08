# Stories hardening and the automated browser check

**Status:** approved 2026-10-21 · **Source:** decisions 92, 93 (rejected for an automated check), 94 and 98; QA consult of 2026-10-21; minutes `2026-10-21-2158-decisions-83-89-91-92-94-and-95-are-appr`

## Problem

The 14 Stories criteria are ticked, but QA found weak evidence for criteria 7, 8, 12 and 13. The dependency install of a story runs with no sandbox, and a session can read the whole disk. AC8 and AC11 of the web UI spec wait for a manual Chrome check that the owner rejected.

## Goal

- A story install runs no package scripts.
- A runner command and a story session cannot read the owner's secrets or the data root.
- Tests prove that the deny list refuses commands at run time, and that bwrap blocks four more escapes.
- The Bash allowlist accepts only the exact test and gate commands of the config.
- Each Stories criterion names the test that proves it.
- One command runs the browser check from a clean build on demo data, and it proves AC8 and AC11 of the web UI spec and the story page render.

## Non-goals

- No sandbox for macOS or Windows.
- No change to the story proposal, accept or reject flows.
- No web stage 2 (decision 96).
- No browser check in CI. It runs on the owner's machine.

## Terms

- **Masked path:** a path that bwrap hides behind an empty tmpfs, so that a sandboxed command sees an empty folder.

## Design

### 1. Install with no scripts

`INSTALL` in `packages/core/src/story-run.ts` adds `--ignore-scripts` to each install command. ADR 0002 records the change and its cost: a package that needs a build script fails the install, and the run fails with the install output.

### 2. Masked paths

`bwrapArgs` in `packages/core/src/sandbox.ts` takes a list of masked paths and adds `--tmpfs <path>` for each one that exists. The default list is:

- `~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.config` and `~/.claude`
- the data root
- each `.env*` file of the main repo checkout, through `--ro-bind /dev/null <file>`

The story session gets the same list as `denyRead` in `storySandboxSettings`, if the Claude Code sandbox settings support `denyRead`. If they do not, the unit log records it, and ADR 0002 names the gap.

If a masked path breaks the test or gate command of `agentsi`, the unit removes that path from the list and records the reason in the log.

### 3. Exact allowlist

`storyBashAllowlist` emits the exact gate commands with no `:*` suffix. The test command keeps a prefix rule, because `{files}` changes each run. A test shows that an extra flag after the prefix, such as `--import ./evil.mjs`, does not match the rule. If Claude Code prefix rules cannot exclude it, the runner checks the command itself before the session starts, or the log records the gap.

### 4. Escape probes

`sandbox.test.ts` adds four probes. Each one must fail inside bwrap:

- write to the `.git` folder of the main repo
- write through `../` out of the worktree
- write through a symlink in the worktree that points outside
- connect to a unix socket that the test opens outside the sandbox

A fifth probe reads a masked path and gets an empty folder.

### 5. Deny list at run time

`story-run.live.test.ts` runs one real story session on haiku with a budget of $0.10. It runs only when `AGENTSI_LIVE=1`. The session prompt asks for `git status`, `curl https://example.com` and a write to `node_modules`. The test passes when the transcript shows each one denied and the worktree is unchanged.

### 6. Evidence lines

Each criterion of `docs/specs/agentsi-stories.md` gets one line under it: `Evidence: <test file>:<test name>`, or `Evidence: manual, <what and when>` for criteria 13 and 14.

### 7. Browser check

`apps/web/scripts/smoke.ts` gets a `--demo` flag. With it, the script:

1. builds the web app from an empty build folder, `.next-smoke`, on the demo data of `scripts/guide-demo.ts`
2. starts it on a free port
3. runs the routes
4. stops the server

The routes are the stage 1 rows plus the stories list and one story page, which use the demo fixture ids. Each route runs in light and dark, at 1280×800 and at 390×844. Extra checks:

- AC8: the first "Te toca" card is above the fold at 1280×800
- the minutes page opens the Decisiones tab on `#decisiones`
- the story page shows its criteria

`pnpm --filter @agentsi/web smoke:demo` runs it. The output goes to `docs/reviews/web-smoke-demo.md`.

## Acceptance criteria

- [x] `pnpm typecheck`, `pnpm lint` and `pnpm test` pass.
  Evidence: docs/plans/agentsi-stories-hardening-log.md:U5 Tests. On 2026-10-22, after the U5 fixes, typecheck and lint pass. `pnpm test`: core 261 pass and 1 skipped (the live test), mcp 4, web 42, cli 28. 0 fail.
- [x] Each install command in `INSTALL` has `--ignore-scripts`, and a test asserts it.
  Evidence:
  - packages/core/src/story-run.ts:INSTALL (5 commands, each with --ignore-scripts)
  - packages/core/src/story-run.test.ts:each install command runs no package scripts
  - packages/core/src/story-run.test.ts:the install command follows the lockfile, and yarn Berry skips the build scripts (yarn 2 and later has no `--ignore-scripts`, so it gets `--mode=skip-build`)
- [x] A bwrap test shows that a runner command sees `~/.ssh` and the data root as empty folders.
  Evidence: packages/core/src/sandbox.test.ts:a sandboxed command sees ~/.ssh and the data root as empty folders, cannot read a .env file and still reads ~/.config/git
- [x] `sandbox.test.ts` has the four escape probes of §4, and they pass with 0 skipped on this machine.
  Evidence:
  - packages/core/src/sandbox.test.ts:a sandboxed command cannot write to the main .git, through ../ or through a symlink out of the worktree
  - packages/core/src/sandbox.test.ts:a sandboxed command cannot connect to a unix socket in the runtime dir of the owner
  - docs/plans/agentsi-stories-hardening-log.md:U5 Tests (the command `node --import tsx --test src/sandbox.test.ts src/story-run.test.ts` in packages/core and its output, `pass 44` and `skipped 0`)
- [x] A test shows that the allowlist has no `:*` on the gate commands, and that an extra flag after the test prefix is refused.
  Evidence: packages/core/src/story-run.test.ts:the gate rules are exact, and a flag after the test prefix is refused
- [x] With `AGENTSI_LIVE=1`, the live test shows `git`, `curl` and a `node_modules` write denied, for $0.10 or less.
  Evidence:
  - packages/core/src/story-run.live.test.ts:a real story session is refused git, curl, a node_modules write and extra test flags
  - docs/plans/agentsi-stories-hardening-log.md:U2 (run 2 passed for $0.0018, total $0.0047). U4 did not run it again.
  - docs/plans/agentsi-stories-hardening-log.md:U5 re-run by the coordinator at cfe115b: `AGENTSI_LIVE=1 node --import tsx --test src/story-run.live.test.ts` gave pass 1, fail 0, cost $0.0018.
- [x] Each of the 14 criteria in `docs/specs/agentsi-stories.md` has an `Evidence:` line.
  Evidence: docs/specs/agentsi-stories.md:Acceptance criteria (14 `Evidence:` lines, commit 91d5316)
- [x] `pnpm --filter @agentsi/web smoke:demo` passes from an empty build folder (`.next-smoke`, so the live `.next` stays untouched), and its report is in `docs/reviews/`.
  Evidence: docs/reviews/web-smoke-demo.md (PASS, 36 of 36, re-run in U5 at 9174847). The build goes to an empty `.next-smoke`, so the owner's `.next` stays as it is (docs/plans/agentsi-stories-hardening-log.md:U3).
- [x] AC8 and AC11 of `docs/specs/agentsi-web-ui.md` are ticked, with evidence from the report.
  Evidence: docs/specs/agentsi-web-ui.md:Acceptance criteria (AC8 and AC11 ticked, commit c6fdb3e)
- [x] ADR 0002 records the install change, the masked paths and any gap that §2 or §3 found.
  Evidence: docs/adr/0002-story-sessions-write-product-repos.md:Decision (masked paths, `--ignore-scripts`, exact gate rules) and Consequences (install cost, three gaps). §2 found no gap for `denyRead` (docs/plans/agentsi-stories-hardening-log.md:U1).
