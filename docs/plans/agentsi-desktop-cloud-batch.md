# Plan: the desktop batch of 2026-11-21 on cloud agents

Each unit is one branch and one PR, made by one cloud routine (`RemoteTrigger`, environment "Default", no connectors). The coordinator is a local session that does not compile. It reviews each PR, waits for green CI, merges it, and starts the next unit. Units of one wave run in parallel because they touch different files. Each agent rebases on `main` before it opens its PR.

## Before wave 1

- The owner allows `tip.files.ghostty.org`, `cdn.pyke.io` and `ziglang.org` in the network settings of the cloud environment.
- A second smoke run builds and tests the whole workspace, and pushes one throwaway branch.

## U1 — PATH (wave 1)
- **Files:** `agentsi-proto/src/path.rs`, `agentsi-core` start and `terminal/mod.rs`, `agentsi-agency/src/setup.rs`, `story/claude.rs`, `agentsi-cli` doctor and web, `agentsi-voice/src/tts/phonemes.rs`, `packaging/`
- **Spec:** `docs/specs/agentsi-path.md`
- **Check:** its acceptance criteria. The owner check runs after the merge.

## U2 — Settings sections, fonts, terminal (wave 1)
- **Files:** `screens/settings*.rs`, `agentsi-ui/src/{tokens,appearance}.rs`, new `terminal_settings.rs`, `agentsi-term-view/src/pane.rs`
- **Spec:** `docs/specs/agentsi-desktop-settings-nav.md`

## U3 — General roles (wave 1)
- **Files:** `agentsi-agency/src/roles/`, `agentsi-proto`, `screens/roles.rs`
- **Spec:** `docs/specs/agentsi-desktop-global-roles.md`

## U4 — Spaces, pinned tabs, favorites (wave 2)
- **Files:** `shell.rs`, `tabs.rs`, `tab_bar.rs`, `layout.rs`, `sidebar.rs`, `tree.rs`, `menus.rs`
- **Spec:** `docs/specs/agentsi-desktop-spaces.md`

## U5 — Terminal links (wave 2)
- **Files:** `agentsi-vt/src/grid.rs`, `agentsi-term-view`, `agentsi-app/src/pane.rs`
- **Spec:** `docs/specs/agentsi-desktop-terminal-links.md`
- **Depends on:** U2 (terminal settings, `pane.rs`).

## U6 — Git core and git screen (wave 2)
- **Files:** `agentsi-agency/src/gitpanel.rs`, `agentsi-proto/src/git_api.rs`, `screens/git.rs`
- **Spec:** `docs/specs/agentsi-desktop-git-more.md`, all parts except the sidebar and tab marks.

## U7 — File explorer (wave 3)
- **Files:** `agentsi-agency/src/files`, `agentsi-proto/src/files_api.rs`, `sidebar.rs`
- **Spec:** `docs/specs/agentsi-desktop-files.md`
- **Depends on:** U4 (sidebar, favorites).

## U8 — Git marks in the sidebar and tabs (wave 3)
- **Files:** `sidebar.rs`, `tab_bar.rs`
- **Spec:** `docs/specs/agentsi-desktop-git-more.md`, part "App", first two items.
- **Depends on:** U4, U6.

## U9 — Palette sources (wave 4)
- **Files:** new `palette.rs`, `shell.rs`, `actions.rs`
- **Spec:** `docs/specs/agentsi-desktop-palette-more.md`
- **Depends on:** U2, U4, U6, U7.

## Each unit

1. The routine prompt gives the spec path, the files, the GPUI rules (ADR 0004, `agentsi-desktop-ui.md`, the `tokens` rule, 0 idle frames), the setup script and "rebase on main, open a PR, do not merge".
2. The agent runs `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` before it opens the PR.
3. The coordinator runs `code-review` on the PR diff (read only), and waits for green CI.
4. The coordinator merges with a merge commit and adds an entry to `docs/plans/desktop-modules-log.md`.
5. The owner checks the visual criteria on the M1 after a `git pull` and a build.
