#!/bin/sh
# Runs inside the container of test-install-container.sh, as an ordinary user.
# Never run this on a real machine: it calls install.sh.
set -eu
pkg=${1:?usage: test-inside.sh <packaging dir>}
fail() { echo "FAIL: $*" >&2; exit 1; }

echo "== dry run changes nothing"
sh "$pkg/install.sh" --from /in --dry-run
[ ! -e "$HOME/.local/bin" ] || fail "dry run made ~/.local/bin"
[ ! -e "$HOME/.config/systemd" ] || fail "dry run made the unit folder"
apps="$HOME/.local/share/applications"
icon="$HOME/.local/share/icons/hicolor/scalable/apps/agentsi.svg"
[ ! -e "$apps" ] || fail "dry run made the applications folder"
[ ! -e "$icon" ] || fail "dry run wrote the icon"

echo "== a bin dir outside the home is refused"
if sh "$pkg/install.sh" --from /in --bin-dir /usr/local/bin 2>/tmp/err; then fail "accepted /usr/local/bin"; fi
cat /tmp/err
grep -q "outside" /tmp/err || fail "no reason given"
if sh "$pkg/install.sh" --from /in --bin-dir "$HOME/../root/bin" 2>/tmp/err; then fail "accepted .."; fi

echo "== missing required program is refused"
mkdir /tmp/empty
if sh "$pkg/install.sh" --from /tmp/empty 2>/tmp/err; then fail "accepted an empty folder"; fi
cat /tmp/err

echo "== install (the app and the pill are missing: a warning, not a failure)"
mkdir /tmp/two
cp /in/agentsi /in/agentsi-core /tmp/two/
sh "$pkg/install.sh" --from /tmp/two
for p in agentsi agentsi-core; do [ -x "$HOME/.local/bin/$p" ] || fail "$p not installed"; done
"$HOME/.local/bin/agentsi-core" --version
"$HOME/.local/bin/agentsi" help | head -3
unit="$HOME/.config/systemd/user/agentsi-core.service"
[ -f "$unit" ] || fail "no unit"
grep -q "^ExecStart=$HOME/.local/bin/agentsi-core serve$" "$unit" || fail "ExecStart not rendered"
if grep -q "@" "$unit"; then fail "a placeholder is left in the unit"; fi
[ ! -e "$HOME/.config/systemd/user/default.target.wants/agentsi-core.service" ] || fail "enabled without --enable"
[ ! -e "$HOME/.config/systemd/user/agentsi-pill.service" ] || fail "a pill unit without the pill program"
[ ! -e "$apps/agentsi.desktop" ] || fail "a desktop entry without the app program"
[ ! -e "$icon" ] || fail "an icon without the app program"

echo "== all four programs, custom bin dir, second run replaces the first"
mkdir /tmp/four
cp /in/agentsi /in/agentsi-core /tmp/four/
cp /in/agentsi /tmp/four/agentsi-app
cp /in/agentsi /tmp/four/agentsi-pill
sh "$pkg/install.sh" --from /tmp/four --bin-dir "$HOME/bin2"
ls "$HOME/bin2"
[ "$(ls "$HOME/bin2" | wc -l)" = 4 ] || fail "expected 4 programs and no leftovers"
sh "$pkg/install.sh" --from /tmp/four --bin-dir "$HOME/bin2"
pill="$HOME/.config/systemd/user/agentsi-pill.service"
[ -f "$pill" ] || fail "no pill unit"
grep -q "^ExecStart=$HOME/bin2/agentsi-pill$" "$pill" || fail "pill ExecStart not rendered"
if grep -q "@" "$pill"; then fail "a placeholder is left in the pill unit"; fi
grep -q "^WantedBy=graphical-session.target$" "$pill" || fail "the pill does not start with the graphical session"
[ ! -e "$HOME/.config/systemd/user/graphical-session.target.wants/agentsi-pill.service" ] || fail "pill enabled without --enable"
entry="$apps/agentsi.desktop"
[ -f "$entry" ] || fail "no desktop entry"
grep -q "^Exec=$HOME/bin2/agentsi-app$" "$entry" || fail "Exec not rendered"
grep -q "^Icon=agentsi$" "$entry" || fail "no Icon line"
grep -q "^StartupWMClass=agentsi-app$" "$entry" || fail "StartupWMClass is not the app id"
if grep -q "@" "$entry"; then fail "a placeholder is left in the desktop entry"; fi
if command -v desktop-file-validate >/dev/null 2>&1; then desktop-file-validate "$entry" || fail "the desktop entry is not valid"; fi
cmp -s "$icon" "$pkg/icons/agentsi.svg" || fail "the icon is not the packaged icon"
echo "== a dry run with the pill writes no unit"
rm "$pill"
rm "$entry" "$icon"
sh "$pkg/install.sh" --from /tmp/four --bin-dir "$HOME/bin2" --dry-run >/tmp/out
grep -q "agentsi-pill.service" /tmp/out || fail "the dry run does not name the pill unit"
grep -q "agentsi.desktop" /tmp/out || fail "the dry run does not name the desktop entry"
grep -q "agentsi.svg" /tmp/out || fail "the dry run does not name the icon"
[ ! -e "$pill" ] || fail "dry run wrote the pill unit"
[ ! -e "$entry" ] || fail "dry run wrote the desktop entry"
[ ! -e "$icon" ] || fail "dry run wrote the icon"
sh "$pkg/install.sh" --from /tmp/four --bin-dir "$HOME/bin2"

echo "== --enable without a user manager fails loudly, not silently"
if sh "$pkg/install.sh" --from /in --enable >/tmp/out 2>&1; then
  echo "(systemctl --user worked)"
else
  tail -3 /tmp/out
  echo "(expected: a container has no user manager)"
fi
echo "ALL INSIDE CHECKS PASSED"
