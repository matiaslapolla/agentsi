# Terminals use libghostty-vt through raw FFI, built from source with a pinned Zig

**Status:** accepted 2026-11-02

## Context

The app runs Claude Code in terminal panes. A pane needs a correct VT parser, a key encoder and the OSC 7501 program state. P3 showed that libghostty-vt renders Claude Code in a GPUI pane and answers `OSC 7501;?` itself when the option `OPT_PROGRAM_STATUS` is set.

## Decision

- `agentsi-vt` binds libghostty-vt through raw FFI and wraps it in a small safe API, with `OPT_PROGRAM_STATUS`.
- `build.rs` builds libghostty-vt from one pinned Ghostty commit with Zig 0.17.0. `mise.toml` and CI pin it, and `scripts/zig.sh` caches a download.
- If `AGENTSI_GHOSTTY_VT_LIB` names a library file, `build.rs` links that file and does not run Zig.
- The PTY is own code in `agentsi-pty`, after the shape of Herdr's `spawn_pty`.

## Consequences

- A fresh clone needs no manual Zig step. P3 expects a wrong Zig version to be the most frequent build failure.
- A Ghostty upgrade changes the commit, the bindings and maybe the Zig version together.
- Rejected: the safe crate `ghostty-vt` 0.8.0 alone, because it does not expose `OPT_PROGRAM_STATUS`. A prebuilt library in the repo, because binary files are hard to audit. The env var keeps that option open. `alacritty_terminal` and `vt100`, because they have no program state protocol and P3 did not test them. `portable-pty`, because the core needs control of the signals and the session of each child.
