# AgentSI desktop: git panel

**Status:** approved 2026-11-07 · **Source:** owner decisions of 2026-11-07 (log "Owner decisions after the first owner checks"), and the owner request for git config per product of the same day

## Problem

The app shows a diff only on the story screen. The owner must open a pane and type `git` commands to see the state of a product's repo, to stage files or to commit.

## Goal

- A git screen for each product shows the branch, the changed files and their diff, and the owner stages files and commits from it.
- The owner sets git config once for all products and overrides it for one product. Each git command that AgentSI or an agent runs for that product uses it.

## Non-goals

- Push, pull, fetch and any remote. The owner does them in a pane.
- Merge, rebase, conflicts, stash and history edits.
- A file watcher. The screen reads again on focus and after each action.
- macOS. U18 stands by.
- A change to the `.git/config` of a repo or to the owner's `~/.gitconfig`. AgentSI writes only in its own config folder.
- Git config on an SSH host from this device. Each core reads its own file.

## Terms

- **Git screen:** the screen kind `Git` of one product and one repo.
- **Git config file:** `<config>/agentsi/git.toml`. It holds the shared git config and the overrides for each product.

## Design

### Core

- New methods in a `git` group, all additions to the protocol:
  - `git.status {product, repo?}` gives the branch, the upstream with ahead and behind, the worktrees, and each changed file with its staged and unstaged state. It parses `git status --porcelain=v2 --branch -z`.
  - `git.diff {product, repo?, path, staged}` gives the patch of one file, cut at 512 KiB with a `truncated` flag.
  - `git.stage` and `git.unstage {product, repo?, paths}`.
  - `git.commit {product, repo?, message}` gives the new commit id, or the output of a failed hook.
  - `git.log {product, repo?, limit}` gives at most 50 commits: id, subject, author and time.
- `repo` defaults to the path of the product. Another repo must be one of the repos of the product, or the core refuses it.
- Each call runs the `git` program with a 20 s limit, as TypeScript does, with `GIT_OPTIONAL_LOCKS=0` for the reads. A commit runs the hooks of the repo, so a hook can refuse it, and the screen shows its output.
- The code uses `story/git.rs` (`git`, `git_out`) and does not copy it. A path that leaves the repo is refused.
- A write sends `data.changed {kind: "git", product}`.

### Git config per product

- The file has a `[shared]` table and one `[product.<id>]` table for each override, each a list of git config keys and values:

  ```toml
  [shared]
  "user.name" = "Ana Example"
  "commit.gpgsign" = "false"

  [product.harbor]
  "user.email" = "ana@work.example"
  ```

- The value for a product is its own table over `[shared]`. An empty string removes a shared key for that product.
- The core gives the result to git through the environment: `GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_<n>` and `GIT_CONFIG_VALUE_<n>` (git 2.31 or later; this machine has 2.56). These values win over the repo, global and system files, and the files stay as they are.
- The core sets these variables on:
  - each `git` call of the core for that product: the git screen, story branches, worktrees, commits and Accept, snapshots, the knowledge index and reflections,
  - each pane that the core starts for that product, so `claude` and the owner's own `git` commands in the pane use it,
  - each story session and meeting process of that product.
- A pane that runs before a change keeps its old values. The settings screen says so in one line.
- A key that starts a program (`core.sshCommand`, `core.fsmonitor`, `core.pager`, `*.helper` and any `alias.*`) needs a Confirm at the save, because the agents then run that program.
- New methods, additions to the protocol: `git.config.get {product?}` gives the shared table, the product table and the result. `git.config.set {product?, entries}` writes one table. The core writes the file atomically and keeps the other tables and their comments.

### App

- The product menu entry "Open the git panel", a palette entry and `Ctrl+Shift+Y` open the screen. The key follows the rules of `agentsi-desktop-keys.md` and shows in the Keys section.
- Top: the branch, ahead and behind, and a Refresh button.
- Left: two lists, "Staged" and "Changes", with the state letter of each file. Space stages or unstages the selected file. Buttons stage or unstage all.
- Right: the diff of the selected file in the `diff` view of `gpui-component`. A cut diff shows one line that says so.
- Bottom: a message field and a Commit button. The button stays off with no staged file or an empty message. A failed hook shows its output under the field and keeps the message.
- A "History" tab shows `git.log`.
- A "Config" tab shows the shared table, the override of this product and the result, with the source of each value. The owner adds, edits or removes a key in either table. The settings screen has a section "Git" with the shared table only.
- The screen reads `git.status` when it opens, when it gets the focus, after each action, and after a `data.changed` event of kind `git` for its product.

## Acceptance criteria

- [ ] Core tests on a temporary repo cover status (added, changed, deleted, renamed, untracked, a path with a space), diff, stage, unstage, commit, log, and a refused path outside the repo.
- [ ] A hook that exits 1 refuses the commit, and the result holds its output.
- [ ] A test of the real screen with a fake core stages a file, commits with a message and shows the new state.
- [ ] A diff over 512 KiB arrives cut, with the flag.
- [ ] `schema/core.openrpc.json` changes only by the `git` and `git.config` methods and the `git` kind of `data.changed`.
- [ ] The idle app draws 0 frames, and `rg -n 'rgb\(|hsla\(|px\(' crates/agentsi-app/src` finds nothing.
- [ ] A test with a shared `user.email` and an override for one product shows that a story commit, a git screen commit and `git config user.email` in a new pane of each product give the expected address.
- [ ] An empty string in an override removes the shared key for that product only.
- [ ] No test or code path writes a `.git/config` or `~/.gitconfig`.
- [ ] Saving a key from the list of program keys asks one Confirm.
- [ ] The owner stages and commits one change of a real repo from the screen.
- [ ] The owner sets an override for one product, and the next agent commit in that product uses it.
