# AgentSI desktop: the launcher entry and the icon

**Status:** approved 2026-11-11 · **Source:** owner request of 2026-11-11, audit of `make install`

## Problem

`make install` copies the programs to `~/.local/bin`, but the app has no desktop entry. The launcher of the desktop (Walker on Omarchy) does not list it. The owner must type `agentsi-app` in a terminal.

A second install does not restart the core and the pill. They keep the old programs until the next login.

The app has no icon. The sidebar header shows only the word "AgentSI".

## Goal

- `install.sh` installs a desktop entry and an icon when it installs `agentsi-app`.
- `install.sh` restarts the core and the pill services when they are active.
- The sidebar header shows the icon on the left of the word "AgentSI".

## Non-goals

- A PNG icon or other sizes. Each current launcher reads the scalable SVG.
- A desktop entry for the pill. The pill starts with the graphical session.
- macOS. The `.app` bundle comes later.

## Design

- `packaging/icons/agentsi.svg` is the icon, the owner's `layer-cube.svg`. The app embeds the same file with `include_bytes!`.
- `packaging/applications/agentsi.desktop` is a template. `Exec=@BINDIR@/agentsi-app`, `Icon=agentsi`, `StartupWMClass=agentsi-app` (the `app_id` of the window).
- On Linux, `install.sh` writes the entry to `${XDG_DATA_HOME:-~/.local/share}/applications/agentsi.desktop` and the icon to `.../icons/hicolor/scalable/apps/agentsi.svg`. It does this only when `agentsi-app` is installed. It runs `update-desktop-database` when the program exists.
- After the copy, `install.sh` restarts each of `agentsi-core` and `agentsi-pill` that is active. `--enable` keeps `enable --now`. `--dry-run` prints the restart.
- `scripts/dist.sh` copies `applications/` and `icons/` into the tarball next to `systemd/`.
- The sidebar header renders the icon with `img` at 18 px. `img` renders an SVG in its own colors.

## Acceptance criteria

- [ ] `make install` writes `~/.local/share/applications/agentsi.desktop` with an absolute `Exec` path and no `@` placeholder.
- [ ] `make install` writes `~/.local/share/icons/hicolor/scalable/apps/agentsi.svg`.
- [ ] An install without `agentsi-app` writes no desktop entry and no icon.
- [ ] A dry run writes no desktop entry and no icon, and it names both.
- [ ] A second `make install` restarts the active core and pill services.
- [ ] The desktop entry passes `desktop-file-validate` when the program exists.
- [ ] The tarball of `make dist` holds `applications/agentsi.desktop` and `icons/agentsi.svg`.
- [ ] The sidebar header shows the icon on the left of "AgentSI".
- [ ] `packaging/test-inside.sh` checks the entry, the icon and the case without the app.
