# AgentSI desktop: more git visibility

**Status:** approved 2026-11-21 · **Source:** owner decisions of 2026-11-21 (git state in the sidebar and the tabs, the diff of each commit, branches, fetch, pull and push). It amends the non-goal "push" of `agentsi-desktop-git.md`.

## Problem

The git screen (`screens/git.rs`) has Changes, a commit list and Config. The owner cannot see a commit's diff, the branches or the worktrees. The sidebar and the tabs show no git state. The app cannot fetch, pull or push.

## Goal

- Each product row and each terminal tab shows the branch, the count of changed files, and ahead and behind.
- History shows the files and the diff of a selected commit.
- A Branches tab lists, creates and switches branches, and lists worktrees.
- The git screen can fetch, pull and push, with a confirm before a push.

## Non-goals

- Merge, rebase, cherry-pick and conflict resolution in the app.
- Force push. The app never sends `--force` or `--force-with-lease`.
- A commit graph. History stays a list.

## Terms

- **Git summary:** branch, upstream, ahead, behind, and the count of changed files of one folder.

## Design

### Core (`agentsi-agency/src/gitpanel.rs`, `agentsi-proto/src/git_api.rs`)

- `git.summary {product | cwd}` gives the git summary, from `git status --porcelain=v2 --branch`.
- The core sends `git.changed {product}` when the summary changes. It reads the summary when `.git/HEAD`, `.git/index` or a ref changes (`notify`), and at most once every 2 s. It watches only the folders git does not ignore (never `target/`, `node_modules/` or `.git/objects`); a product whose watches cannot all be added (the inotify limit) is read every 10 s.
- `git.show {product, commit}` gives the files of a commit and their diff, with the 512 KiB cut of `git.diff`.
- `git.branches {product}` gives local and remote branches, with upstream, ahead and behind, and the worktrees.
- `git.branch.create {product, name, from}` and `git.switch {product, name}`. A switch with changes in the tree returns `dirty`, and the app shows the files.
- `git.fetch`, `git.pull {product}` (`--ff-only`) and `git.push {product}`. They run with the product git env (`git_env.rs`) and a time limit of 120 s, and they stream progress lines as `git.progress`.
- A push of a branch with no upstream uses `--set-upstream origin <branch>`.
- A pull that cannot fast-forward returns `diverged`. The app tells the owner to use a terminal.

### App

- `sidebar.rs` product rows show `branch ·N ↑a ↓b` in meta text. A clean tree shows only the branch.
- A terminal tab tooltip shows the git summary of its folder. The tab shows a dot when the tree has changes.
- `screens/git.rs`:
  - History: a click on a commit shows its files at the left and the diff at the right, with the diff view of Changes.
  - A new tab "Branches": a list with the current branch first. Actions: Switch, New branch from here, Copy name. A section lists worktrees with their folders.
  - The header has Fetch, Pull and Push buttons with ahead and behind.
  - Push shows a confirm with the branch, the remote and the commits that go out.
- The palette gets "Git: Fetch", "Git: Pull", "Git: Push", "Git: Switch branch…" and "Git: New branch…".

## Acceptance criteria

- [ ] A file change in a product folder updates its sidebar row in less than 3 s.
- [ ] A click on a commit in History shows its files and diff.
- [ ] Switch to another branch with a clean tree works. With changes, the app shows `dirty` and the files.
- [ ] Push asks for a confirm that lists the outgoing commits, then pushes to a local bare remote in a test.
- [ ] A pull that cannot fast-forward returns `diverged`, and the tree does not change.
- [ ] `rg -- '--force' crates/agentsi-agency/src/gitpanel.rs` finds nothing.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass.
