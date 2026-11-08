# Story sessions write code in product repos, and the web approves stories

**Status:** accepted 2026-10-21

## Context
`docs/specs/agentsi-roles.md` says that no role writes into a product repo, and the web was read-only. Stories mode (`docs/specs/agentsi-stories.md`) needs a role that writes and runs code. The owner approves stories in the web.

## Decision
- A story session writes into a worktree on a `story/*` branch. Only `agentsi story accept` merges, locally, after the owner reviews the diff.
- Agent code runs in two sandboxes:
  - The session gets `--settings` with the Claude Code Bash sandbox: no network, writes only in the worktree and temp dir, no unsandboxed retry, no start without the sandbox. The main `.git` and `node_modules` are not writable.
  - The runner runs each test and gate command in bwrap: a read-only root, the worktree writable, its `node_modules` read-only, a private `/tmp`, no network.
- Both sandboxes hide the same masked paths (updated 2026-10-22):
  - `~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.config`, `~/.claude` and the data root read as empty folders.
  - Each `.env*` file of the main repo is bound to `/dev/null`, so a read of it fails.
  - A `.env*` symlink is hidden too (updated 2026-10-22). bwrap cannot mount on a symlink, so bwrap binds `/dev/null` on the real file. The session gets the link and the real file in `denyRead`.
  - The `.env*` walk skips a folder that it cannot read (updated 2026-10-22).
  - A path that is the repo, or a folder that holds the repo, stays visible (updated 2026-10-22). A worktree reads the git data of the repo through its `.git` file.
  - `~/.config/git` stays readable, because the tests of `agentsi` commit and git reads the identity there.
  - bwrap also hides `/run`. A read-only bind does not stop a connect to a socket, and `/run` holds the docker socket.
  - The session gets the list as `sandbox.filesystem.denyRead` and `allowRead`. Claude Code 2.1.307 supports both.
- The dependency install runs unsandboxed, before any session, with `--ignore-scripts` (updated 2026-10-22).
  - Yarn 2 and later has no `--ignore-scripts`. If a `.yarnrc.yml` is beside `yarn.lock`, the install is `yarn install --immutable --mode=skip-build`.
- The deny list blocks git, network tools, deploy, migrate and publish, and Edit of `node_modules` and `.git`.
- Each gate command is an exact Bash rule. The test command keeps a prefix rule up to `{files}` (updated 2026-10-22):
  - Claude Code matches `Bash(prefix:*)` as a string prefix, with no check of flags.
  - Deny rules refuse a flag right after the prefix, such as `--import ./evil.mjs`.
  - If the test command ends with `{files}`, deny rules also refuse a flag after the files.
  - Claude Code 2.1.307 reads an unescaped `*` in a rule as a wildcard. Each `*` of a gate command gets a backslash, so its rule stays exact (updated 2026-10-22).
- The web gets three story actions: approve, changes and discard. Each calls `decideStory`, as the CLI does.
- The desktop app takes the place of the web (updated 2026-11-07). Its Stories screen keeps the three actions and adds Run, Accept and Reject.
  - Approve writes at once.
  - Changes, Discard, Run, Accept and Reject first show what they will do. They act only after a click on Confirm.
  - A click on Confirm is the owner's word, as the word to Claude is. A role answer, a minutes file or a peer message still never counts.
- The runner bwrap also gets `--unshare-pid` (updated 2026-11-07). A child that leaves the process group then ends with the sandbox. The argument list differs from `sandbox.ts` by this one argument.

## Consequences
- Approved actions become code without the spec circuit. A wrong session costs one worktree.
- Without bwrap, `run` refuses. Without bwrap or socat, a session does not start.
- A package that needs a build script fails the install, and the run fails with the install output.
- Gaps:
  - A rule with no unescaped `*` matches only the same text, backslashes included. So a session cannot run a gate command with a `*` as the config writes it. The runner still runs that gate.
  - If the data root or a secret folder holds the repo, the mask does not hide that path.
  - bwrap cannot stop a connect to a unix socket in a path that it does not hide, for example a socket in the home dir. Only `/run`, `/tmp` and the masked paths are safe.
  - If the test command has a fixed flag after `{files}`, a flag after the files is not denied.
  - The deny rules for flags match quotes as text. A shell trick that hides a leading `-` from the match can pass. Each such command still runs in the Bash sandbox, and an allowed test file can run any code already.
- Rejected: a session in the main checkout, which a failure leaves dirty. Rejected: a prefix deny list alone, because an allowed test command runs any script the session writes.
