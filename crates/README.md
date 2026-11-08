# crates

The Rust rebuild of AgentSI (the desktop app). Spec: `docs/specs/agentsi-desktop.md`. Plan: `docs/plans/agentsi-desktop.md`.

## Licenses

Each crate declares `license = "Apache-2.0"` (ADR 0008). Until the switch-over, the rest of the repo is MIT, and `packages/voice` is GPL-3.0-or-later. No crate links GPL code.

## Crates

| Crate | Output | Contents |
|---|---|---|
| `agentsi-proto` | lib | Protocol types, framing, OpenRPC schema |
| `agentsi-store` | lib | SQLite schema, migrations, stores, Postgres import |
| `agentsi-agency` | lib | Port of `packages/core` |
| `agentsi-pty` | lib | Own PTY code |
| `agentsi-vt` | lib | libghostty-vt FFI, `build.rs`, safe wrapper |
| `agentsi-core` | bin `agentsi-core` | Daemon |
| `agentsi-cli` | bin `agentsi` | CLI client |
| `agentsi-models` | lib | Model catalog and downloads |
| `agentsi-voice` | lib | Speech in and out |
| `agentsi-ui` | lib | Design system on `gpui-component` |
| `agentsi-term-view` | lib | GPUI terminal pane |
| `agentsi-app` | bin `agentsi-app` | Window and screens |
| `agentsi-pill` | bin `agentsi-pill` | Pill window and voice |

Each binary prints `<name> <version>` for `--version`. Until the switch-over, the TypeScript `agentsi` keeps the name on the `PATH`. The Rust one stays in `target/`.

## Rules

- Pin shared crates in `[workspace.dependencies]` of the root `Cargo.toml`. A crate uses `dep = { workspace = true }`.
- `gpui-pre`, `gpui-pre-platform` and `gpui-kit` stay pinned with `=`. The `wayland` feature is required.
- Set `CARGO_TARGET_DIR=~/.cache/agentsi-target`.
- Before a commit, run `cargo check`, `cargo clippy --workspace -- -D warnings` and `cargo test --workspace`.
