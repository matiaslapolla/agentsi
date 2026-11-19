#!/bin/sh
# Installs AgentSI for the current user. It changes nothing outside $HOME.
#
#   packaging/install.sh [--from <dir>] [--bin-dir <dir>] [--data-root <dir>] [--enable] [--dry-run]
#
#   --from <dir>     Folder with the built programs. Default: bin/ next to this script (a tarball of
#                    make dist), else target/release of this checkout.
#   --bin-dir <dir>  Where the programs go. Default: ~/.local/bin. It must be inside $HOME.
#   --enable         Also enable and start the core service (systemd user unit or launchd agent),
#                    and on Linux the pill service when agentsi-pill is installed. The pill service
#                    starts with the graphical session.
#                    Without it the script only installs the files and prints the command.
#   --data-root <dir> The data root for a first install. Default:
#                    ${XDG_DATA_HOME:-~/.local/share}/agentsi/data. The script writes it into
#                    ~/.config/agentsi/home and makes an empty registry there. It never changes an
#                    existing data root, home file or registry.
#   --dry-run        Print what would happen. Change nothing.
#
# Installs: agentsi and agentsi-core (required), agentsi-app and agentsi-pill (a headless
# machine has no use for them: the script warns and goes on when they are missing).
# Linux: the units go to ${XDG_CONFIG_HOME:-~/.config}/systemd/user/ (agentsi-core.service, and
# agentsi-pill.service when agentsi-pill is installed). With agentsi-app, the desktop entry goes to
# ${XDG_DATA_HOME:-~/.local/share}/applications/ and the app icon to .../icons/hicolor/scalable/apps/.
# The script restarts the core and the pill services that are active, so they run the new programs.
# It does not restart the core while agentsi-app is open, and it names a core that runs outside
# systemd. Both keep the old program, and the script prints the command for each.
# macOS: the agent goes to ~/Library/LaunchAgents/dev.agentsi.core.plist.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
from="$here/../target/release"
[ -d "$here/bin" ] && from="$here/bin"
bin_dir="${HOME:?HOME is not set}/.local/bin"
data_root=""
enable=0
dry=0

while [ $# -gt 0 ]; do
  case "$1" in
    --from) from=${2:?--from needs a folder}; shift 2 ;;
    --bin-dir) bin_dir=${2:?--bin-dir needs a folder}; shift 2 ;;
    --data-root) data_root=${2:?--data-root needs a folder}; shift 2 ;;
    --enable) enable=1; shift ;;
    --dry-run) dry=1; shift ;;
    -h|--help) awk 'NR == 1 { next } /^set -eu$/ { exit } { sub(/^# ?/, ""); print }' "$0"; exit 0 ;;
    *) echo "install.sh: unknown option $1" >&2; exit 2 ;;
  esac
done

die() { echo "install.sh: $*" >&2; exit 1; }
say() { echo "$*"; }
# Runs a command, or only prints it with --dry-run.
do_() { if [ "$dry" = 1 ]; then echo "would run: $*"; else "$@"; fi; }
# Reports a finished step: done_ <what> [install|restart]. With --dry-run the step did not happen.
done_() { v=${2:-install}; if [ "$dry" = 1 ]; then say "would $v $1"; else say "${v}ed $1"; fi; }

[ -d "$from" ] || die "no folder $from. Build first (cargo build --release) or pass --from."
from=$(cd "$from" && pwd)
# With a trailing /, the pattern "$HOME"/* of guard_home_path matches no path.
[ "$HOME" = / ] || HOME=${HOME%/}
config_home="${XDG_CONFIG_HOME:-$HOME/.config}"
data_home="${XDG_DATA_HOME:-$HOME/.local/share}"
# Stops unless the path is absolute, inside $HOME and free of "..": guard_home_path <name> <path>.
guard_home_path() {
  case "$2" in /*) ;; *) die "$1 must be an absolute path" ;; esac
  case "$2/" in "$HOME"/*) ;; *) die "$1 $2 is outside $HOME. This script changes nothing outside your home." ;; esac
  case "$2" in *..*) die "$1 must not hold .." ;; esac
}
# All checks come before the first change, so a bad path changes nothing.
guard_home_path --bin-dir "$bin_dir"
guard_home_path XDG_CONFIG_HOME "$config_home"
case "$data_root" in ""|/*) ;; *) die "--data-root must be an absolute path" ;; esac
# render() puts these paths into sed commands, and the templates put them inside double quotes
# (Exec of the desktop entry, ExecStart and Environment of the units).
case "$bin_dir$HOME" in
  *'|'*|*'&'*|*'\'*|*'"'*|*'`'*|*'$'*|*'%'*) die "a path holds |, &, a backslash, \", \`, \$ or %" ;;
esac

for p in agentsi agentsi-core; do
  [ -x "$from/$p" ] || die "$from/$p is missing or not executable."
done
# A program built on a newer system stops at once with a glibc error. Find it before the copy.
"$from/agentsi-core" --version >/dev/null 2>&1 || die "$from/agentsi-core does not run here. $("$from/agentsi-core" --version 2>&1 | head -1)"
programs="agentsi agentsi-core"
have_pill=0
for p in agentsi-app agentsi-pill; do
  if [ -x "$from/$p" ]; then programs="$programs $p"; else say "warning: $from/$p not found. Skipped."; fi
done
if [ -x "$from/agentsi-pill" ]; then have_pill=1; fi
have_app=0
if [ -x "$from/agentsi-app" ]; then have_app=1; fi
home_file="$config_home/agentsi/home"
# The script writes under XDG_DATA_HOME only for the desktop entry and for the default data root.
first_root=0
if [ -z "${AGENTSI_HOME:-}" ] && [ ! -s "$home_file" ]; then first_root=1; fi
if { [ "$have_app" = 1 ] && [ "$(uname -s)" = Linux ]; } || { [ "$first_root" = 1 ] && [ -z "$data_root" ]; }; then
  guard_home_path XDG_DATA_HOME "$data_home"
fi

do_ mkdir -p "$bin_dir"
for p in $programs; do
  # Copy to a new name, then rename: a running program keeps working.
  do_ cp "$from/$p" "$bin_dir/.$p.new"
  do_ chmod 755 "$bin_dir/.$p.new"
  do_ mv -f "$bin_dir/.$p.new" "$bin_dir/$p"
  done_ "$bin_dir/$p"
done

# The data root of a first install: the home file names it, and it holds an empty registry.
if [ -n "${AGENTSI_HOME:-}" ]; then
  say "AGENTSI_HOME is set ($AGENTSI_HOME). The data root stays as it is."
elif [ "$first_root" = 0 ]; then
  say "The data root stays as $home_file names it: $(head -1 "$home_file")"
else
  [ -n "$data_root" ] || data_root="$data_home/agentsi/data"
  do_ mkdir -p "$data_root" "$config_home/agentsi"
  [ "$dry" = 1 ] || printf '%s\n' "$data_root" > "$home_file"
  done_ "$home_file (data root $data_root)"
  if [ ! -e "$data_root/agentsi.config.json" ]; then
    [ "$dry" = 1 ] || printf '%s\n' '{ "owner": { "name": "the owner", "language": "English" }, "products": [] }' > "$data_root/agentsi.config.json"
    done_ "$data_root/agentsi.config.json (empty registry)"
  fi
fi

# Replaces @BINDIR@ and @HOME@ of a template. sed uses | as the separator: paths hold no |.
render() {
  sed -e "s|@BINDIR@|$bin_dir|g" -e "s|@HOME@|$HOME|g" "$1"
}
# Renders a template to a hidden name next to the target, then renames it onto the target.
install_tpl() {
  if [ "$dry" = 0 ]; then
    render "$1" > "${2%/*}/.${2##*/}.new"
    mv -f "${2%/*}/.${2##*/}.new" "$2"
  fi
  done_ "$2"
}

case "$(uname -s)" in
  Linux)
    unit_dir="$config_home/systemd/user"
    do_ mkdir -p "$unit_dir"
    install_tpl "$here/systemd/agentsi-core.service" "$unit_dir/agentsi-core.service"
    if [ "$have_pill" = 1 ]; then
      install_tpl "$here/systemd/agentsi-pill.service" "$unit_dir/agentsi-pill.service"
    fi
    if [ "$have_app" = 1 ]; then
      apps_dir="$data_home/applications"
      icon_dir="$data_home/icons/hicolor/scalable/apps"
      do_ mkdir -p "$apps_dir" "$icon_dir"
      install_tpl "$here/applications/agentsi.desktop" "$apps_dir/agentsi.desktop"
      do_ cp "$here/icons/agentsi.svg" "$icon_dir/agentsi.svg"
      done_ "$icon_dir/agentsi.svg"
      if command -v update-desktop-database >/dev/null 2>&1; then
        do_ update-desktop-database -q "$apps_dir" || true
      fi
    fi
    # A running service keeps the old program after the copy. The script restarts each active one.
    # unit_is <active|enabled> <unit>
    unit_is() { command -v systemctl >/dev/null 2>&1 && systemctl --user "is-$1" --quiet "$2" 2>/dev/null; }
    # pids_of <name>: the pids of this user's processes with that name. Empty without pgrep.
    pids_of() { if command -v pgrep >/dev/null 2>&1; then pgrep -u "$(id -u)" -x "$1" | tr '\n' ' ' | sed 's/ $//'; fi; }
    restart=""
    # systemd keeps the unit it loaded. A reload makes the restart, or the printed command, use the new file.
    reload=$enable
    if unit_is active agentsi-core.service; then
      reload=1
      # The app has no reconnect: a core restart ends its link, its meetings and its terminals.
      if [ -n "$(pids_of agentsi-app)" ]; then
        say "agentsi-app is open, so the core keeps the old program. Close the app, then run: systemctl --user restart agentsi-core"
      else
        restart="agentsi-core.service"
      fi
    else
      core_pids=$(pids_of agentsi-core)
      if [ -n "$core_pids" ]; then
        say "warning: agentsi-core runs outside systemd (pid $core_pids) and keeps the old program. Close the app if it is open, then run: kill $core_pids"
      fi
    fi
    if [ "$have_pill" = 1 ] && unit_is active agentsi-pill.service; then
      restart="$restart agentsi-pill.service"
      reload=1
    fi
    if [ "$reload" = 1 ]; then do_ systemctl --user daemon-reload; fi
    for unit in $restart; do
      do_ systemctl --user restart "$unit"
      if [ "$dry" = 1 ]; then done_ "$unit" restart; continue; fi
      # A client can start its own core between the stop and the start. Then the unit exits with 3.
      sleep 1
      if unit_is active "$unit"; then
        done_ "$unit" restart
        continue
      fi
      say "warning: $unit is not active after the restart. See: systemctl --user status $unit"
      if [ "$unit" = agentsi-core.service ]; then
        other=$(pids_of agentsi-core)
        [ -z "$other" ] || say "Another agentsi-core holds the socket (pid $other). It keeps the old program. Close the app if it is open, then run: kill $other"
      fi
    done
    if [ "$enable" = 1 ]; then
      do_ systemctl --user enable --now agentsi-core.service
      if [ "$have_pill" = 1 ]; then do_ systemctl --user enable --now agentsi-pill.service; fi
    else
      unit_is enabled agentsi-core.service || say "The core service is not enabled. To enable it: systemctl --user enable --now agentsi-core"
      if [ "$have_pill" = 1 ] && ! unit_is enabled agentsi-pill.service; then say "The pill service is not enabled. It starts with the graphical session. To enable it: systemctl --user enable --now agentsi-pill"; fi
    fi
    ;;
  Darwin)
    agent_dir="$HOME/Library/LaunchAgents"
    do_ mkdir -p "$agent_dir" "$HOME/Library/Logs"
    install_tpl "$here/launchd/dev.agentsi.core.plist" "$agent_dir/dev.agentsi.core.plist"
    if [ "$enable" = 1 ]; then
      do_ launchctl bootstrap "gui/$(id -u)" "$agent_dir/dev.agentsi.core.plist"
    else
      say "The core agent is not loaded. To load it: launchctl bootstrap gui/\$(id -u) $agent_dir/dev.agentsi.core.plist"
    fi
    ;;
  *) die "unsupported system $(uname -s)" ;;
esac

case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) say "warning: $bin_dir is not on your PATH. Add it to your shell profile." ;;
esac
