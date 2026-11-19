# Fixes from the review of the desktop entry work

**Status:** approved 2026-11-19 · **Source:** code review of `8988446..657ba81` (spec `docs/specs/agentsi-desktop-launcher.md`), owner request of 2026-11-19

## Problem

The review of the desktop entry work found five bugs in `packaging/install.sh`. A bad `XDG_DATA_HOME` stops the script after the copy, so the services keep the old programs. A second install restarts the core under an open app, and a path with a space breaks the `Exec` line.

The docs use three names for the desktop entry, and `CONTEXT.md` defines none of them. The rules of the guide in `CONTRIBUTING.md` are out of date.

## Goal

- `install.sh` checks each path before it changes a file.
- `install.sh` never restarts the core under an open app, and it tells the owner each core that keeps the old program.
- The `Exec` line of the desktop entry and the `ExecStart` lines of the units work for a path with a space.
- The docs use one name for each thing, from `CONTEXT.md`.
- The tests of `packaging/` cover the restart and `desktop-file-validate`.

## Non-goals

- A reconnect of the app to a new core.
- A stop of a core that runs outside systemd. The script only names it.
- macOS.

## Terms

- **Desktop entry**: the file `agentsi.desktop` that makes the launcher list the app.
- **Launcher**: the program of the desktop that lists the apps and starts them, for example Walker on Omarchy.
- **App icon**: the file `agentsi.svg`. The launcher and the sidebar header show it.

## Design

### Path checks (bug B1)

- One function `guard_home_path <name> <path>` checks that a path is absolute, inside `$HOME` and free of `..`.
- The script runs it on the bin dir, `XDG_CONFIG_HOME` and `XDG_DATA_HOME` before the copy. A relative `XDG_*` value now stops the script.
- The script computes `config_home` and `data_home` one time, at the top.
- The script refuses `"`, a backtick, `$`, `%` and a backslash in the bin dir and in `$HOME`, in addition to `|` and `&`.

### Quoting (bug B3)

- The desktop entry has `Exec="@BINDIR@/agentsi-app"`.
- The units have `ExecStart="@BINDIR@/..."` and `Environment="PATH=@BINDIR@:..."`.
- The refused characters are the ones that need an escape inside these quotes, so no escape is necessary.

### Restart (bugs B2, B4 and B5)

- An app is open when `pgrep -u <uid> -x agentsi-app` finds a process. Without `pgrep`, the script treats the app as closed.
- When the core service is active and the app is open, the script does not restart the core. It prints the restart command for after the app closes. The pill restarts as before.
- After each restart, the script waits 1 s and checks `is-active`. It prints "restarted <unit>" only when the unit is active. Otherwise it prints that another core holds the socket.
- When the core service is not active and a process `agentsi-core` runs, the script prints that this core keeps the old program, with its pid and `kill <pid>`.
- Without `--enable`, the script prints the enable command only for a unit that is not enabled.

### Smaller shell code

- `install_tpl <template> <target>` renders a template to a temp name and renames it. It replaces the four copies of that step.
- `done_ <path> [verb]` also prints "restarted". `unit_is <active|enabled> <unit>` replaces `active()` and `enabled()`.
- `scripts/dist.sh` copies each folder of one list `assets="systemd applications icons"`.

### Tests

- `packaging/test-image/Dockerfile` adds systemd and `desktop-file-utils` to a base image. The default base is `archlinux` on x86_64 and `ubuntu:24.04` on aarch64.
- `test-install-container.sh` builds that image, and `test-inside.sh` fails when `desktop-file-validate` is missing.
- `test-inside.sh` runs the restart cases with a fake `systemctl` and a fake `pgrep` first on `PATH`.

### Docs

- The spec of the desktop entry, the README and `INSTALL.md` say "desktop entry", "launcher" and "app icon".
- `CONTEXT.md` gets the three terms.
- The section "The guide" of `CONTRIBUTING.md` says that `IMAGES` lists the embedded images, that `img/` also holds the shots of the root README, and that `img-unused/` holds shots that no page shows.

## Acceptance criteria

- [ ] With `XDG_DATA_HOME` outside `$HOME`, `install.sh` exits 1 and the bin dir and the unit folder are unchanged.
- [ ] `install.sh --bin-dir "$HOME/my bin"` writes an entry that `desktop-file-validate` accepts, with `Exec="$HOME/my bin/agentsi-app"`.
- [ ] A bin dir with `%` or `$` is refused before any change.
- [ ] With the core service active and the app open, `install.sh` does not call `systemctl --user restart agentsi-core.service` and prints the command.
- [ ] With the core service active and the app closed, `install.sh` restarts it and prints "restarted agentsi-core.service".
- [ ] When the unit is not active after the restart, `install.sh` does not print "restarted" for it.
- [ ] With the core service inactive and a process `agentsi-core` running, `install.sh` prints its pid.
- [ ] An install without `--enable` prints the enable command when the unit is not enabled, and not when it is enabled.
- [ ] `packaging/test-install-container.sh` passes on aarch64 with the portable programs, and its log shows `desktop-file-validate`.
- [ ] `CONTEXT.md` defines "Desktop entry", "Launcher" and "App icon", and the docs use no other name for them.
- [ ] `CONTRIBUTING.md` names `IMAGES`, the README shots in `img/` and `img-unused/`.
