#!/bin/sh
# Prints the path of a Zig 0.17.0 binary. Order: $AGENTSI_ZIG, a `zig` on PATH that reports 0.17.0,
# else a checksummed download into ~/.cache/agentsi/zig/0.17.0/. Used by fresh clones and CI.
set -eu
VERSION=0.17.0
if [ -n "${AGENTSI_ZIG:-}" ]; then echo "$AGENTSI_ZIG"; exit 0; fi
if command -v zig >/dev/null 2>&1 && [ "$(zig version)" = "$VERSION" ]; then command -v zig; exit 0; fi
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) arch=x86_64-linux; sha=1cbe9df9f27e6b78d14ccbca43b6703a404ef79ef1c463de901d7f088d4e2026 ;;
  Linux-aarch64) arch=aarch64-linux; sha=9e8d11661d4ae3bd57702a3832781e23ad151dde5798e16a5ccd503f65234ff8 ;;
  Darwin-arm64) arch=aarch64-macos; sha=b607e9b9234790a008116ae5bdb71c6243b84b9fb42a53a9e70fde41c06c536a ;;
  Darwin-x86_64) arch=x86_64-macos; sha=4f9a1c5269aa17ebda5e6d3c2b89d6cbf36f7d2b22a0306e9ab98f25f95529c6 ;;
  *) echo "zig.sh: no Zig $VERSION build for $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
dir="${XDG_CACHE_HOME:-$HOME/.cache}/agentsi/zig/$VERSION"
dirname="zig-$arch-$VERSION"
if [ ! -x "$dir/$dirname/zig" ]; then
  mkdir -p "$dir"
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  curl -fsSL "https://ziglang.org/download/$VERSION/$dirname.tar.xz" -o "$tmp/zig.tar.xz"
  if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum "$tmp/zig.tar.xz" | cut -d' ' -f1); else got=$(shasum -a 256 "$tmp/zig.tar.xz" | cut -d' ' -f1); fi
  [ "$got" = "$sha" ] || { echo "zig.sh: checksum mismatch (got $got)" >&2; exit 1; }
  tar -xJf "$tmp/zig.tar.xz" -C "$dir"
fi
echo "$dir/$dirname/zig"
