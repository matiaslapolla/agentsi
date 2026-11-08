#!/usr/bin/env bash
# The Linux pill criterion of docs/specs/agentsi-desktop-voice.md:
# while the pill speaks, `hyprctl layers -j` lists it on the overlay layer (level 3),
# and `hyprctl activewindow` shows no focus change.
#   linux-criterion.sh [<pill binary>]   draws the speaking phase for 6 s, with no core and no sound.
#   linux-criterion.sh --live [--runs n] the real path: a core, a Stop hook, and the pill speaking.
#                                        Needs release builds of agentsi-core and agentsi-pill.
set -euo pipefail
if [[ "${1:-}" == "--live" ]]; then
    shift
    exec python3 "$(dirname "$0")/speech-bench.py" --criterion --runs 3 "$@"
fi
bin="${1:-${CARGO_TARGET_DIR:-$HOME/.cache/agentsi-target}/debug/agentsi-pill}"
before=$(hyprctl activewindow -j | python3 -c 'import json,sys; print(json.load(sys.stdin).get("address",""))')
"$bin" --show speaking --seconds 6 2>/dev/null &
pid=$!
sleep 3
layer=$(hyprctl layers -j | python3 -c '
import json, sys
for output, v in json.load(sys.stdin).items():
    for level, layers in v["levels"].items():
        for l in layers:
            if l["namespace"] == "agentsi-pill" and l["w"] > 0:
                print(output, "level", level, l["x"], l["y"], l["w"], l["h"])')
after=$(hyprctl activewindow -j | python3 -c 'import json,sys; print(json.load(sys.stdin).get("address",""))')
wait "$pid"
echo "layer:  ${layer:-none}"
echo "focus:  $before -> $after"
[[ "$layer" == *"level 3"* ]] || { echo "FAIL: the pill is not on the overlay layer"; exit 1; }
[[ "$before" == "$after" ]] || { echo "FAIL: the focus changed"; exit 1; }
echo "PASS"
