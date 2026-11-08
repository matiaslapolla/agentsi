# Story result, front desk command and disputed criterion tests

**Status:** draft · **Source:** `docs/plans/agentsi-desk-and-stories.md` units U1, U2 and U3, and the harness audit of 2026-10-24

## Problem
S009 and S010 have the result `discarded`, but the owner merged both by hand. No command can correct a recorded result. The front desk opens with three Bash calls. A wrong criterion test caused 3 of 4 failed story runs, because the owner role may not change a test.

## Goal
- The owner records a merge that happened outside `story accept` with one command.
- The front desk opens with one Bash call.
- When the owner role finds a wrong criterion test, QA repairs that test and the owner role runs again.

## Non-goals
- No change to `story accept`, `story reject` or the web.
- No result `discarded` from the new command. `story reject` records it.
- No change to the meeting sessions of QA. Only the story sessions of QA change model.

## Terms
- **Disputed test:** a criterion test that the owner role says contradicts its criterion. The owner role ends its session with a `test_disputed` line.

## Design

### U1. `agentsi story result`
`agentsi story result <story-id> merged_changed|merged_unchanged --commit <sha> [--product <id>]` corrects the result of a story.

- `recordStoryResult` in `packages/core/src/story-run.ts` does the work. The CLI subcommand is in `packages/cli/src/commands/story-run.ts`.
- The command refuses a story with a session that is not `failed`. Such a story needs `story accept` or `story reject`.
- The command resolves `<sha>` in the story repo to a full commit hash. It refuses a commit that is not an ancestor of the main branch of the product (`mainBranch`).
- The command writes the result as `story accept` does: status `implemented`, and a new result `{ result, at, commit }` after the old ones.
- The story transaction commits the stories folder with the message `story(<product>): result (<kind>) <id> <title>`.

### U2. `agentsi desk`
`agentsi desk [--product <id>]` in `packages/cli/src/commands/desk.ts` does the four steps of the front desk:

1. It resolves the product as `agentsi whoami` does. Without `--product`, a cwd of no product gives exit code 2.
2. It runs the incremental index of the product.
3. It reads the status Markdown. A pending migration counts as a status error.
4. It reads the pending decisions with `pendingItems`, and keeps the items of kind `decision`.

It prints one JSON object: the fields of `agentsi whoami`, then `index`, `status` and `pending_decisions`. A failed step gives `{ "ok": false, "error": "<text>" }` in its field, and the next steps still run. The exit code is 1 when the index or the status failed, else 0.

Section 1 of `plugin/skills/agentsi/SKILL.md` opens the front desk with one `agentsi desk` call. The rule "one plain `agentsi` command per Bash call" stays.

### U3. Disputed criterion test
The changes are in `executeStoryRun` and the prompts of `packages/core/src/story-run.ts`.

- **Owner prompt.** The owner role reads and runs the criterion tests before it changes code. When a test contradicts its criterion, it changes no file and ends its answer with `test_disputed: <test path>: <reason>`.
- **Detection.** `testDispute` reads that line from the session answer. It accepts only a criterion test path of the story.
- **Repair round.** On the first dispute of a run, the runner does these steps:
  1. It resets the worktree to `tests_commit` and removes the untracked files. The work of the owner role is lost.
  2. QA gets the reason and may change only the disputed test.
  3. When QA changed the test, the runner commits it as the new `tests_commit` and checks that the test still fails.
  4. The owner role runs again, with a note that says what QA did.
- **Attempts.** The disputed attempt does not use one of the two attempts. A run has 3 owner sessions or less.
- **Second dispute.** A second dispute in the same run fails the run with its reason.
- **Test file check.** The check "no file of `tests_commit` changed" compares the criterion test paths, so a repair commit still protects each test.
- **Bash.** `storyBashAllowlist` also gets the exact test command of the story, with its files, as an exact rule. Each session of the run gets it.
- **Model.** The runner runs each QA session of a story on `sonnet`, whatever the model of the QA role. Meetings keep the role model.

## Acceptance criteria
- [ ] `pnpm typecheck`, `pnpm lint` and `pnpm test` pass, with no test that starts a real Claude session.
- [ ] A CLI test shows that `story result` refuses a commit that is not on the main branch, and changes no story file.
- [ ] A CLI test shows that `story result S001 merged_changed --commit <short sha>` sets status `implemented`, adds the result with the full hash, keeps the old results and commits the data root.
- [ ] A core test shows that `recordStoryResult` refuses a story with a session that is not `failed`, and an unknown commit.
- [ ] A CLI test shows that `agentsi desk` exits with code 2 from a directory of no product.
- [ ] A CLI test shows that `agentsi desk` prints one JSON object with the product, the owner, `index`, `status` and `pending_decisions`, and puts the database error in `index` and `status`.
- [ ] Section 1 of `plugin/skills/agentsi/SKILL.md` runs `agentsi desk` and no `agentsi index` or `agentsi status`.
- [ ] A runner test shows: a dispute gives one QA repair round on `sonnet`, a new `tests_commit`, and a second owner session that reaches `ready` with 2 attempts.
- [ ] A runner test shows that a second dispute fails the run with its reason, and that a QA repair that changes another file fails the run.
- [ ] A test shows that `testDispute` ignores a path that is not a criterion test.
- [ ] A test shows that the allowlist has the exact test command of the story, and that the deny rules permit it.
