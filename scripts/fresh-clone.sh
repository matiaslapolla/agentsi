#!/bin/sh
# Builds the Rust workspace in a clean ubuntu:24.04 container from the committed HEAD (a fresh clone).
# Usage: scripts/fresh-clone.sh [command...]   Default: cargo build --workspace, then each --version.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
git clone -q "$root" "$tmp/src"
inner='set -eu
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends ca-certificates curl xz-utils git build-essential pkg-config clang cmake \
  libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev \
  libasound2-dev libvulkan-dev libssl-dev >/dev/null
curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null
. "$HOME/.cargo/env"
cd /src
export AGENTSI_ZIG=$(scripts/zig.sh)
"$AGENTSI_ZIG" version
'
if [ "$#" -gt 0 ]; then inner="$inner$*"; else inner="$inner
cargo build --workspace --locked
for b in agentsi agentsi-core agentsi-app agentsi-pill; do /target/debug/\$b --version; done"; fi
start=$(date +%s)
docker run --rm -e CARGO_TARGET_DIR=/target -v "$tmp/src:/src" ubuntu:24.04 sh -c "$inner"
echo "fresh-clone: ok in $(( $(date +%s) - start )) s"
