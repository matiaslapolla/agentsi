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
# agentsi-pill.service when agentsi-pill is installed).
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
    -h|--help) sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "install.sh: unknown option $1" >&2; exit 2 ;;
  esac
done

die() { echo "install.sh: $*" >&2; exit 1; }
say() { echo "$*"; }
# Runs a command, or only prints it with --dry-run.
do_() { if [ "$dry" = 1 ]; then echo "would run: $*"; else "$@"; fi; }
# Reports a finished step. With --dry-run the step did not happen.
done_() { if [ "$dry" = 1 ]; then say "would install $1"; else say "installed $1"; fi; }

[ -d "$from" ] || die "no folder $from. Build first (cargo build --release) or pass --from."
from=$(cd "$from" && pwd)
case "$bin_dir" in /*) ;; *) die "--bin-dir must be an absolute path" ;; esac
case "$bin_dir/" in
  "$HOME"/*) ;;
  *) die "--bin-dir $bin_dir is outside $HOME. This script changes nothing outside your home." ;;
esac
case "$bin_dir" in *..*) die "--bin-dir must not hold .." ;; esac
# render() puts these paths into sed commands.
case "$bin_dir$HOME" in *'|'*|*'&'*|*'\'*) die "a path holds |, & or a backslash" ;; esac

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

do_ mkdir -p "$bin_dir"
for p in $programs; do
  # Copy to a new name, then rename: a running program keeps working.
  do_ cp "$from/$p" "$bin_dir/.$p.new"
  do_ chmod 755 "$bin_dir/.$p.new"
  do_ mv -f "$bin_dir/.$p.new" "$bin_dir/$p"
  done_ "$bin_dir/$p"
done

# The data root of a first install: the home file names it, and it holds an empty registry.
config_home="${XDG_CONFIG_HOME:-$HOME/.config}"
home_file="$config_home/agentsi/home"
if [ -n "${AGENTSI_HOME:-}" ]; then
  say "AGENTSI_HOME is set ($AGENTSI_HOME). The data root stays as it is."
elif [ -s "$home_file" ]; then
  say "The data root stays as $home_file names it: $(head -1 "$home_file")"
else
  [ -n "$data_root" ] || data_root="${XDG_DATA_HOME:-$HOME/.local/share}/agentsi/data"
  case "$data_root" in /*) ;; *) die "--data-root must be an absolute path" ;; esac
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

case "$(uname -s)" in
  Linux)
    unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}"
    case "$unit_dir/" in "$HOME"/*) ;; *) die "XDG_CONFIG_HOME is outside $HOME." ;; esac
    case "$unit_dir" in *..*) die "XDG_CONFIG_HOME must not hold .." ;; esac
    unit_dir="$unit_dir/systemd/user"
    do_ mkdir -p "$unit_dir"
    [ "$dry" = 1 ] || { render "$here/systemd/agentsi-core.service" > "$unit_dir/.unit.new" && mv -f "$unit_dir/.unit.new" "$unit_dir/agentsi-core.service"; }
    done_ "$unit_dir/agentsi-core.service"
    if [ "$have_pill" = 1 ]; then
      [ "$dry" = 1 ] || { render "$here/systemd/agentsi-pill.service" > "$unit_dir/.unit.new" && mv -f "$unit_dir/.unit.new" "$unit_dir/agentsi-pill.service"; }
      done_ "$unit_dir/agentsi-pill.service"
    fi
    if [ "$enable" = 1 ]; then
      do_ systemctl --user daemon-reload
      do_ systemctl --user enable --now agentsi-core.service
      if [ "$have_pill" = 1 ]; then do_ systemctl --user enable --now agentsi-pill.service; fi
    else
      say "The core service is not enabled. To enable it: systemctl --user enable --now agentsi-core"
      if [ "$have_pill" = 1 ]; then say "The pill service is not enabled. It starts with the graphical session. To enable it: systemctl --user enable --now agentsi-pill"; fi
    fi
    ;;
  Darwin)
    agent_dir="$HOME/Library/LaunchAgents"
    do_ mkdir -p "$agent_dir" "$HOME/Library/Logs"
    [ "$dry" = 1 ] || { render "$here/launchd/dev.agentsi.core.plist" > "$agent_dir/.agent.new" && mv -f "$agent_dir/.agent.new" "$agent_dir/dev.agentsi.core.plist"; }
    done_ "$agent_dir/dev.agentsi.core.plist"
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
