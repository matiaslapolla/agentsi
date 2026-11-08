#!/bin/sh
# Builds vendor/ghostty-vt-sys: the registry copy of ghostty-vt-sys 0.8.0 with a build.rs that links the
# system libghostty-vt.so (Arch package `ghostty`) in place of compiling Ghostty with Zig 0.17.
# Needed only when `zig` is not installed. Source crate: Apache-2.0, kept as LICENSE in the copy.
set -eu
cd "$(dirname "$0")/.."
src=$(ls -d "$HOME"/.cargo/registry/src/*/ghostty-vt-sys-0.8.0* | head -1)
rm -rf vendor/ghostty-vt-sys
mkdir -p vendor/ghostty-vt-sys/src
cp "$src"/src/*.rs vendor/ghostty-vt-sys/src/
cp "$src"/LICENSE "$src"/LICENSE-GHOSTTY vendor/ghostty-vt-sys/
cat > vendor/ghostty-vt-sys/Cargo.toml <<'TOML'
[package]
name = "ghostty-vt-sys"
version = "0.8.0+ghostty.1.4.0-main.83fb956"
edition = "2024"
build = "build.rs"
links = "ghostty-vt"
license = "Apache-2.0"
description = "ghostty-vt-sys 0.8.0 bindings, linked against the system libghostty-vt.so"
[lib]
path = "src/lib.rs"
TOML
cat > vendor/ghostty-vt-sys/build.rs <<'RS'
fn main() {
    println!("cargo:rustc-env=GHOSTTY_VT_COMMIT=83fb95624b0439ee660c2679b43b38c8b63769cd");
    println!("cargo:rustc-env=GHOSTTY_VT_VERSION=1.4.0-main+83fb956");
    println!("cargo::metadata=COMMIT=83fb95624b0439ee660c2679b43b38c8b63769cd");
    println!("cargo::metadata=VERSION=1.4.0-main+83fb956");
    println!("cargo:rustc-link-lib=dylib=ghostty-vt");
}
RS
