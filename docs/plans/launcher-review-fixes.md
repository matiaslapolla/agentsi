# Plan: fixes from the review of the desktop entry work

Spec: `docs/specs/launcher-review-fixes.md`. One commit for each unit, in this order.

## U1 — One name for each thing in the docs
- **Files:** `CONTEXT.md`, `CONTRIBUTING.md`, `docs/specs/agentsi-desktop-launcher.md`, `README.md`, `packaging/INSTALL.md`
- **Steps:**
  1. Add "Desktop entry", "Launcher" and "App icon" to the section "The desktop app" of `CONTEXT.md`.
  2. Replace "launcher entry" and "app launcher" in the docs.
  3. Update the section "The guide" of `CONTRIBUTING.md`.
- **Check:** `grep -rn -i "launcher entry\|app launcher" README.md packaging docs/specs CONTRIBUTING.md` finds nothing.

## U2 — Smaller shell code
- **Files:** `packaging/install.sh`, `scripts/dist.sh`
- **Steps:**
  1. Add `install_tpl`, `unit_is` and the verb of `done_`. Replace the old call sites.
  2. Add the `assets` list to `dist.sh`.
- **Check:** the container test passes with no change to its output.

## U3 — Path checks before any change
- **Files:** `packaging/install.sh`, `packaging/test-inside.sh`
- **Steps:**
  1. Add `guard_home_path`, and run it on the three paths at the top.
  2. Extend the refused characters.
  3. Test an outside `XDG_DATA_HOME` and a bin dir with `%`.
- **Check:** the container test passes with the new cases.

## U4 — Quoted Exec and ExecStart
- **Files:** `packaging/applications/agentsi.desktop`, `packaging/systemd/*.service`, `packaging/test-inside.sh`
- **Steps:**
  1. Quote the paths in the templates.
  2. Update the greps of the test, and add a bin dir with a space.
- **Check:** `desktop-file-validate` and `systemd-analyze verify` pass in the container.

## U5 — Safe restart
- **Files:** `packaging/install.sh`, `packaging/test-inside.sh`
- **Steps:**
  1. Skip the core restart while the app is open.
  2. Check `is-active` after each restart.
  3. Name a core that runs outside systemd.
  4. Test each case with a fake `systemctl` and a fake `pgrep`, also the case "unit already enabled".
- **Check:** the container test passes with the restart cases.

## U6 — Test image with desktop-file-utils
- **Files:** `packaging/test-image/Dockerfile`, `packaging/test-install-container.sh`, `packaging/test-inside.sh`
- **Steps:**
  1. Add the Dockerfile, with the base image as a build argument.
  2. Build it in `test-install-container.sh`.
  3. Fail in `test-inside.sh` when `desktop-file-validate` is missing.
- **Check:** `packaging/test-install-container.sh <portable bin dir>` passes on this aarch64 machine.

## U7 — Second review round
- **Files:** `packaging/install.sh`, `packaging/test-inside.sh`, `packaging/test-image/Dockerfile`, `scripts/dist.sh`, the two specs
- **Steps:**
  1. Run `daemon-reload` also when the core restart is skipped.
  2. Check `XDG_DATA_HOME` only when the script writes there, and check `--data-root` before the copy.
  3. Give the pill its own warning after a failed restart.
  4. Copy only tracked files in `dist.sh`, and use `pacman -Syu` in the test image.
- **Check:** the container test passes with the new cases.
