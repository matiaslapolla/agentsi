#!/bin/sh
# Packs AgentSI for another Linux machine: one tarball that installs with no Rust, Zig or build.
#
#   scripts/dist.sh [--portable] [--out <dir>]
#
#   (default)    Packs target/release of this machine. The programs need the glibc of this
#                machine or newer.
#   --portable   Builds the committed HEAD in an ubuntu:24.04 container first (Docker), so the
#                programs need glibc 2.39 or newer, which most current distros have. The
#                volumes agentsi-dist-cargo, agentsi-dist-target and agentsi-dist-cache keep the
#                build between runs.
#   --out <dir>  Where the tarball goes. Default: dist/ of this checkout.
#
# The tarball holds: bin/ (the 4 programs), install.sh, systemd/, applications/, icons/, the
# Claude Code plugin with the hooks that need no Node, INSTALL.md, VERSION and SHA256SUMS.
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
portable=0
out="$root/dist"
while [ $# -gt 0 ]; do
  case "$1" in
    --portable) portable=1; shift ;;
    --out) out=${2:?--out needs a folder}; shift 2 ;;
    -h|--help) sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "dist.sh: unknown option $1" >&2; exit 2 ;;
  esac
done
die() { echo "dist.sh: $*" >&2; exit 1; }
programs="agentsi agentsi-core agentsi-app agentsi-pill"

sha=$(git -C "$root" rev-parse --short HEAD)
dirty=$(git -C "$root" status --porcelain --untracked-files=no | head -c1)
[ -z "$dirty" ] || sha="$sha-dirty"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if [ "$portable" = 1 ]; then
  [ -z "$dirty" ] || echo "dist.sh: warning: --portable builds the committed HEAD, not your changes." >&2
  command -v docker >/dev/null || die "--portable needs Docker."
  sha=$(git -C "$root" rev-parse --short HEAD)
  git clone -q "$root" "$work/src"
  mkdir -p "$work/built"
  inner='set -eu
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends ca-certificates curl xz-utils git build-essential pkg-config clang cmake \
  libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev libfreetype-dev \
  libasound2-dev libvulkan-dev libssl-dev >/dev/null
[ -x "$CARGO_HOME/bin/cargo" ] || curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --no-modify-path >/dev/null
export PATH="$CARGO_HOME/bin:$PATH"
cd /src
export AGENTSI_ZIG=$(scripts/zig.sh)
cargo build --release --locked -p agentsi-cli -p agentsi-core -p agentsi-app -p agentsi-pill
cp /target/release/agentsi /target/release/agentsi-core /target/release/agentsi-app /target/release/agentsi-pill /built/
chown -R '"$(id -u):$(id -g)"' /built'
  docker run --rm \
    -e CARGO_HOME=/cargo -e RUSTUP_HOME=/cargo/rustup -e CARGO_TARGET_DIR=/target \
    -v agentsi-dist-cargo:/cargo -v agentsi-dist-target:/target -v agentsi-dist-cache:/root/.cache \
    -v "$work/src:/src" -v "$work/built:/built" \
    ubuntu:24.04 sh -c "$inner"
  from="$work/built"
else
  from="$root/target/release"
  for p in $programs; do
    [ -x "$from/$p" ] || die "$from/$p is missing. Run make release first."
  done
fi

name="agentsi-$sha-linux-$(uname -m)"
pkg="$work/$name"
mkdir -p "$pkg/bin" "$pkg/systemd" "$pkg/applications" "$pkg/icons" "$pkg/.claude-plugin"
for p in $programs; do
  cp "$from/$p" "$pkg/bin/$p"
  strip "$pkg/bin/$p" 2>/dev/null || true
done
cp "$root/packaging/install.sh" "$pkg/install.sh"
cp "$root/packaging/systemd/"*.service "$pkg/systemd/"
cp "$root/packaging/applications/agentsi.desktop" "$pkg/applications/"
cp "$root/packaging/icons/agentsi.svg" "$pkg/icons/"

# The plugin with the hooks of the switch-over: no Node, no test files.
cp -R "$root/plugin" "$pkg/plugin"
cp "$root/packaging/hooks.json" "$pkg/plugin/hooks/hooks.json"
find "$pkg/plugin/hooks" -type f ! -name hooks.json -delete
cp "$root/.claude-plugin/marketplace.json" "$pkg/.claude-plugin/marketplace.json"

cp "$root/packaging/INSTALL.md" "$pkg/INSTALL.md"
cp "$root/LICENSE" "$pkg/LICENSE" 2>/dev/null || true

glibc=$(for p in $programs; do objdump -T "$pkg/bin/$p"; done | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1 | sed 's/GLIBC_//')
{
  echo "commit $sha"
  echo "built $(date -u +%Y-%m-%dT%H:%MZ)"
  echo "glibc $glibc or newer"
  [ "$portable" = 1 ] && echo "build ubuntu:24.04 container" || echo "build $(uname -n)"
} > "$pkg/VERSION"
(cd "$pkg" && sha256sum bin/* > SHA256SUMS)

mkdir -p "$out"
tar -C "$work" -czf "$out/$name.tar.gz" "$name"
(cd "$out" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "dist.sh: $out/$name.tar.gz ($(du -h "$out/$name.tar.gz" | cut -f1), glibc $glibc or newer)"
