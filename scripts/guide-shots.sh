#!/bin/bash
# Takes the screenshots of the guide (docs/guide/img/<name>.png).
#
#   scripts/guide-shots.sh [--bin-dir <dir>] [--out <dir>] [--only <name>[,<name>...]] [--keep] [--list]
#
#   --bin-dir <dir>  Folder with agentsi, agentsi-core and agentsi-app.
#                    Default: ${CARGO_TARGET_DIR:-<checkout>/target}/release.
#   --out <dir>      Where the PNG files go. Default: docs/guide/img.
#   --only <names>   Take only these shots. Default: all.
#   --keep           Keep the work folder, and print its path.
#   --list           Print the shot names and stop.
#
# The script never touches your AgentSI data, config or core. It starts these processes of its own:
#   - a nested Hyprland, as a window of the Wayland session that runs this script,
#   - a core and the app, in that nested Hyprland, on synthetic data (scripts/guide-shots/seed.sql).
# Each of them runs with a throwaway HOME, AGENTSI_HOME, AGENTSI_CORE_SOCKET, XDG_CONFIG_HOME,
# XDG_DATA_HOME and XDG_RUNTIME_DIR in one work folder. A shot is `grim -g` of the app window.
# Each `hyprctl` call and each key goes to the nested instance only. The core finds a stand-in for `claude` and for `codebase-memory-mcp` first on its PATH
# (scripts/guide-shots/*-stub.sh). They call no service. A shell with a short prompt runs the terminals.
# Before the shots, a first run of the app opens the terminals. The script then writes `look.toml` and the
# tab groups of `layout.json` with the terminal ids, and starts the app again.
# The script stops its processes by pid at the end. It needs: Hyprland, hyprctl, grim, wtype, jq, sqlite3, ffmpeg, git.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
bin_dir="${CARGO_TARGET_DIR:-$root/target}/release"
out="$root/docs/guide/img"
only=""
keep=0

# The shots in page order. Each name is the file name of an image in docs/guide/README.md.
SHOTS="window-tree palette front-desk minutes-list minute priorities call meeting stories story memory roles git settings settings-voice settings-keys settings-dictation settings-data"

while [ $# -gt 0 ]; do
  case "$1" in
    --bin-dir) bin_dir=${2:?--bin-dir needs a folder}; shift 2 ;;
    --out) out=${2:?--out needs a folder}; shift 2 ;;
    --only) only=$(echo "${2:?--only needs names}" | tr ',' ' '); shift 2 ;;
    --keep) keep=1; shift ;;
    --list) echo $SHOTS | tr ' ' '\n'; exit 0 ;;
    -h|--help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "guide-shots: unknown option $1" >&2; exit 2 ;;
  esac
done

die() { echo "guide-shots: $*" >&2; exit 1; }
for tool in Hyprland hyprctl grim wtype jq sqlite3 ffmpeg git; do
  command -v "$tool" >/dev/null || die "$tool is not installed."
done
for p in agentsi agentsi-core agentsi-app; do
  [ -x "$bin_dir/$p" ] || die "$bin_dir/$p is missing. Build first, or pass --bin-dir."
done
[ -n "${WAYLAND_DISPLAY:-}" ] && [ -n "${XDG_RUNTIME_DIR:-}" ] || die "no Wayland session to nest in."
for n in $only; do
  case " $SHOTS " in *" $n "*) ;; *) die "no shot named $n. Run --list." ;; esac
done

parent_display="$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
case "$WAYLAND_DISPLAY" in /*) parent_display=$WAYLAND_DISPLAY ;; esac
[ -S "$parent_display" ] || die "$parent_display is not a socket."

# A short path: a Unix socket path holds at most 107 bytes.
work=$(mktemp -d "${TMPDIR:-/tmp}/demo.XXXXXX")
home=$work/home
run=$work/run
mkdir -p -m 700 "$run"
mkdir -p "$home/.config/agentsi" "$home/data" "$home/.local/share" "$work/repos" "$out"
core_sock=$work/core.sock
core_pid=""
app_pid=""
meet_pid=""

# Every hyprctl call goes to the nested instance: its own runtime folder and signature, and -i.
# An empty signature, or the signature of the session that runs this script, stops the script.
sig=""
hc() {
  [ -n "$sig" ] && [ "$sig" != "${HYPRLAND_INSTANCE_SIGNATURE:-}" ] || die "no nested Hyprland to talk to."
  XDG_RUNTIME_DIR=$run HYPRLAND_INSTANCE_SIGNATURE=$sig hyprctl -i "$sig" "$@"
}

stop_all() {
  [ -z "$app_pid" ] || kill "$app_pid" 2>/dev/null || true
  [ -z "$meet_pid" ] || kill "$meet_pid" 2>/dev/null || true
  [ -z "$core_pid" ] || kill "$core_pid" 2>/dev/null || true
  # The nested Hyprland: by the pid of the process that was started with this work folder.
  for p in $(pgrep -f "^Hyprland -c $work/" 2>/dev/null || true); do kill "$p" 2>/dev/null || true; done
  sleep 1
  if [ "$keep" = 1 ]; then echo "work folder: $work"; else rm -rf "$work"; fi
}
trap stop_all EXIT

# --- Synthetic data -------------------------------------------------------------------------

for p in lumen orchard harbor; do
  mkdir -p "$work/repos/$p/docs"
  (cd "$work/repos/$p" && git init -q -b main \
    && git config user.name "Demo" && git config user.email "demo@example.invalid" \
    && echo "# $p" > README.md && echo "A made-up product." > docs/vision.md \
    && git add . && git commit -q -m "feat: first commit" \
    && echo ".worktrees/" >> .git/info/exclude \
    && echo "change" >> README.md && echo "draft" > docs/notes.md)
done
# The session of the story S001 of Lumen has a real worktree.
(cd "$work/repos/lumen" && git stash -q -u \
  && git worktree add -q -b story/S001-send-a-reminder-the-day-before \
    .worktrees/S001-send-a-reminder-the-day-before main && git stash pop -q)

# A shell with a short prompt, and a stand-in for `claude` that calls no service.
printf "PS1='\\w \\$ '\nHISTFILE=/dev/null\n" > "$home/.bashrc"
printf 'pcm.!default { type null }\nctl.!default { type null }\n' > "$work/asound.conf"
# The Settings screen prints the path of the core program: a link keeps the path of this checkout out of the shot.
mkdir -p "$home/.local/bin" "$work/stub"
ln -s "$bin_dir/agentsi-core" "$home/.local/bin/agentsi-core"
cp "$here/guide-shots/claude-stub.sh" "$work/stub/claude"
cp "$here/guide-shots/code-graph-stub.sh" "$work/stub/codebase-memory-mcp"
chmod +x "$work/stub/claude" "$work/stub/codebase-memory-mcp"
cat > "$home/data/agentsi.config.json" <<EOF
{
  "owner": { "name": "Ana", "language": "English" },
  "products": [
    { "id": "lumen", "name": "Lumen", "paths": ["$work/repos/lumen"], "status": "active",
      "roles": ["pm", "dev", "qa"],
      "stories": { "test": "true {files}", "gate": ["true"] } },
    { "id": "orchard", "name": "Orchard", "paths": ["$work/repos/orchard"], "status": "active",
      "roles": ["pm", "dev"] },
    { "id": "harbor", "name": "Harbor", "paths": ["$work/repos/harbor"], "status": "active",
      "roles": ["pm", "dev"] }
  ]
}
EOF
echo "$home/data" > "$home/.config/agentsi/home"

# One call of 24 s: a tone, and the times of its 4 segments (the text is in seed.sql).
call_dir=$home/data/workspace/lumen/audio/lumen-001
mkdir -p "$call_dir"
ffmpeg -loglevel error -y -f lavfi -i "sine=frequency=220:sample_rate=24000:duration=24" \
  -ac 1 -c:a libopus -b:a 32k -f ogg "$call_dir/call.opus"
cat > "$call_dir/timings.json" <<'EOF'
[
  { "idx": 0, "role": "pm", "text": "Good morning. The goal is the first paying clinic. Three items matter this week.", "start": 0.0, "end": 5.5 },
  { "idx": 1, "role": "dev", "text": "The reminder queue drops a job after one failure. I propose one retry rule.", "start": 5.85, "end": 11.5 },
  { "idx": 2, "role": "qa", "text": "Two paths have no test. A late reminder and a canceled booking.", "start": 11.85, "end": 17.5 },
  { "idx": 3, "role": "pm", "text": "I rank the retry rule first. Then the two tests. Then the launch note.", "start": 17.85, "end": 23.5 }
]
EOF

# Throwaway environment for each AgentSI process. Nothing of the owner's passes through.
agentsi_env() {
  exec env -i PATH="$work/stub:/usr/local/bin:/usr/bin:/bin" ALSA_CONFIG_PATH="$work/asound.conf" SHELL=/bin/bash TERM=xterm-256color HOME="$home" AGENTSI_HOME="$home/data" AGENTSI_CORE_SOCKET="$core_sock" \
    AGENTSI_CORE_BIN="$home/.local/bin/agentsi-core" XDG_CONFIG_HOME="$home/.config" \
    XDG_DATA_HOME="$home/.local/share" XDG_RUNTIME_DIR="$run" \
    WAYLAND_DISPLAY=wayland-1 "$@"
}

start_core() {
  agentsi_env "$bin_dir/agentsi-core" serve > "$work/core.log" 2>&1 &
  core_pid=$!
  for _ in $(seq 1 50); do [ -S "$core_sock" ] && return 0; sleep 0.2; done
  die "the core did not start. See $work/core.log."
}

# The schema is the migrations of the store, in order. The core opens the file lazily and
# applies the migrations after the last one in `user_version`. The seed goes in before it starts.
db=$home/data/agentsi.db
version=0
for f in "$root"/crates/agentsi-store/sql/[0-9]*.sql; do
  sqlite3 "$db" < "$f"
  version=$((10#$(basename "$f" | cut -d_ -f1)))
done
sqlite3 "$db" "pragma user_version = $version;"
sed "s|{{WORK}}|$work|g" "$here/guide-shots/seed.sql" | sqlite3 "$db"
start_core

# --- Nested Hyprland ---------------------------------------------------------------------------

cat > "$work/hypr.conf" <<'EOF'
monitor = , 1600x1000@60, 0x0, 1
input {
    kb_layout = us
    resolve_binds_by_sym = true
}
general {
    gaps_in = 0
    gaps_out = 0
    border_size = 0
}
decoration {
    rounding = 0
}
animations {
    enabled = false
}
misc {
    disable_hyprland_logo = true
    disable_splash_rendering = true
}
EOF
env -i HOME="$home" PATH="$PATH" XDG_RUNTIME_DIR="$run" XDG_CONFIG_HOME="$home/.config" \
  WAYLAND_DISPLAY="$parent_display" \
  setsid Hyprland -c "$work/hypr.conf" --i-am-really-stupid > "$work/hypr.out" 2>&1 < /dev/null &
for _ in $(seq 1 50); do
  sig=$(ls "$run/hypr" 2>/dev/null | head -1 || true)
  [ -n "$sig" ] && [ -S "$run/hypr/$sig/.socket.sock" ] && break
  sleep 0.2
done
[ -n "$sig" ] || die "the nested Hyprland did not start. See $work/hypr.out."
sleep 2

# The geometry of the window of the app, as `x,y WxH`, when it is up.
geometry() {
  hc clients -j 2>/dev/null | jq -r '[.[] | select(.pid == '"$app_pid"')][0] | select(. != null) | "\(.at[0]),\(.at[1]) \(.size[0])x\(.size[1])"'
}

geo=""
start_app() {
  agentsi_env "$bin_dir/agentsi-app" >> "$work/app.log" 2>&1 &
  app_pid=$!
  geo=""
  for _ in $(seq 1 100); do
    geo=$(geometry || true)
    [ -n "$geo" ] && break
    sleep 0.2
  done
  if [ -z "$geo" ]; then
    hc clients -j > "$work/clients.json" 2>&1 || true
    die "the app window did not open. See $work/app.log and $work/clients.json."
  fi
  sleep 3
}

stop_app() {
  kill "$app_pid" 2>/dev/null || true
  wait "$app_pid" 2>/dev/null || true
  app_pid=""
  sleep 1
}

# --- Keys and shots ----------------------------------------------------------------------------

# Keys and grim go to the nested instance only: its runtime folder holds its own wayland-1.
nested() {
  [ -S "$run/wayland-1" ] || die "the nested Wayland socket is missing."
  WAYLAND_DISPLAY=wayland-1 XDG_RUNTIME_DIR=$run "$@"
}

# key ctrl+shift+d   key Return   key ctrl+comma
key() {
  local spec=$1 k
  IFS='+' read -ra parts <<< "$spec"
  k=${parts[${#parts[@]}-1]}
  local args=()
  for m in "${parts[@]:0:${#parts[@]}-1}"; do args+=(-M "$m"); done
  case "$k" in
    ?) args+=("$k") ;;
    *) args+=(-k "$k") ;;
  esac
  for m in "${parts[@]:0:${#parts[@]}-1}"; do args+=(-m "$m"); done
  nested wtype "${args[@]}"
  sleep 0.6
}

# type "text"
typed() { nested wtype "$1"; sleep 0.4; }

# The palette: ctrl+k, the words, Return.
palette() { key ctrl+k; sleep 1; typed "$1"; key Return; sleep 0.8; }

shot() {
  local name=$1
  if [ -n "$only" ]; then
    case " $only " in *" $name "*) ;; *) return 0 ;; esac
  fi
  geo=$(geometry)
  park_pointer
  nested grim -g "$geo" "$out/$name.png"
  echo "took $name"
}

# Move the pointer out of the window, so a shot shows no cursor.
park_pointer() { hc dispatch movecursor 1599 999 >/dev/null 2>&1 || true; }

# A shell command in the focused terminal.
run_in_terminal() { typed "$1"; key Return; sleep 0.6; }

# --- Prep: panes, labels, colors, one product group and two tab groups ----------------------

start_app
key ctrl+2                      # Lumen. The tree lists the products that have no group first.
for cmd in "git log --oneline" "cat docs/vision.md" "git status --short"; do
  key ctrl+t; sleep 1.5; run_in_terminal "$cmd"
done
key ctrl+3                      # Orchard
for cmd in "ls docs" "git diff --stat"; do
  key ctrl+t; sleep 1.5; run_in_terminal "$cmd"
done
sleep 1
stop_app

# The terminal ids belong to this run of the core: read them from the saved layout.
layout=$home/.config/agentsi/layout.json
ids=$(jq -r '[.. | objects | select(.panel_name == "terminal") | .info.panel.terminal] | .[]' "$layout")
set -- $ids
[ $# -eq 5 ] || die "expected 5 terminals in $layout, found $#."
lumen_a=$1 lumen_b=$2 lumen_c=$3 orchard_a=$4 orchard_b=$5

cat > "$home/.config/agentsi/look.toml" <<EOF
[products.lumen]
label = "pilot"
color = "teal"

[products.orchard]
label = "beta"
color = "violet"

[products.harbor]
label = "idea"
color = "amber"

[panes.$lumen_a]
label = "history"
color = "teal"

[panes.$lumen_b]
label = "vision"

[panes.$lumen_c]
label = "changes"
color = "lime"

[panes.$orchard_a]
label = "docs"
color = "violet"

[panes.$orchard_b]
label = "diff"

[[groups]]
name = "Clinics"
color = "sky"
collapsed = false
products = ["lumen", "orchard"]
EOF

key_of() { printf 'terminal:{"terminal":"%s"}' "$1"; }
jq --arg a "$(key_of "$lumen_a")" --arg b "$(key_of "$lumen_b")" --arg c "$(key_of "$lumen_c")" \
   --arg d "$(key_of "$orchard_a")" --arg e "$(key_of "$orchard_b")" \
  '.active_product = "lumen"
   | .dock.center.children[0].children |= map(select(.panel_name != "screen" or .info.panel.product == "lumen"))
   | .dock.center.children[0].info.tabs.active_index = 0
   | .tab_groups = [
       {"name": "Lumen", "color": "teal", "tabs": [$a, $b, $c]},
       {"name": "Orchard", "color": "violet", "collapsed": true, "tabs": [$d, $e]}
     ]' "$layout" > "$layout.new" && mv "$layout.new" "$layout"

start_app
park_pointer
key ctrl+2
# A meeting: the stand-in for claude prints a short stream and waits. The CLI starts it: after the
# form of the Meeting screen hides, the window takes no key.
agentsi_env "$bin_dir/agentsi" meet --product lumen --question "What should Lumen focus on this week?" \
  > "$work/meet.log" 2>&1 &
meet_pid=$!
sleep 8

# Each step opens its screen, then takes the shot. The steps assume the app starts on its default layout.
# TUNE: a screen that needs a click or a scroll gets its lines here, once the final app exists.
key ctrl+shift+d; sleep 1
shot window-tree
key ctrl+k; sleep 1; typed "meet"; sleep 1; shot palette; key Escape; key Escape
key ctrl+shift+d; sleep 1
shot front-desk
key ctrl+shift+l; sleep 2
shot minutes-list
# The list is newest first: the second row is lumen-001, which has the call.
key Down; key Return; sleep 2
shot minute
key c; sleep 1                   # mark the first priority as changed
shot priorities
key ctrl+shift+p; sleep 2
# The null audio device plays at once: the shot comes while the first role still speaks.
nested wtype " "; sleep 0.3
shot call
key ctrl+shift+s; sleep 2
shot stories
key Return; sleep 2
shot story
key ctrl+shift+y; sleep 2
shot memory
key ctrl+shift+r; sleep 2
shot roles
key ctrl+shift+b; sleep 2
shot git
key ctrl+comma; sleep 2
shot settings
# The page keys scroll the Settings screen by one page: voice models, keys, dictation key, data.
key Next
shot settings-voice
key Next
shot settings-keys
key Next; key Next
shot settings-dictation
key End; sleep 1
shot settings-data

# Last: a screen of a live meeting takes no key until the next click.
key ctrl+shift+g; sleep 2
shot meeting

# A PNG optimiser is optional. The shots are 100 KB or less without one.
if command -v oxipng >/dev/null; then
  for name in ${only:-$SHOTS}; do oxipng -q -o 4 --strip safe "$out/$name.png"; done
fi

echo "done: $out"
