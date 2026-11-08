# The app draws with GPUI from `gpui-pre`, and its components come from `gpui-component`

**Status:** accepted 2026-11-02

## Context

The app needs native windows on Linux and macOS, a layer-shell overlay for the pill, and fast terminal panes. The owner wants one Rust codebase. P1 drew the pill and P4 built a front desk sample on the owner's machine.

## Decision

- The app and the pill use `gpui-pre` `=0.3.11` on wgpu, with the platform features `wayland`, `x11` and `font-kit`.
- The base components come from `gpui-component` `=0.9.0`, through `gpui-kit` `=0.9.0`.
- The terminal pane, the pill and the hover transitions are own code on plain GPUI.
- The three pins move together, in one commit, after the gallery and the bench mode pass.

## Consequences

- Herdr's desktop app uses the same stack, so its code is a reference.
- `gpui-component` is before 1.0, and each release pins one patch of `gpui-pre`. An upgrade moves the whole app. If an upgrade blocks the project, vendor the crate, as Herdr does.
- The first build takes about 8 minutes. CI needs a cache.
- Rejected: Tauri or Electron, because a web view adds a second UI stack. `gpui-cn` and `ely-core`, because they pin `gpui-pre` 0.3.10. `bezel`, because it uses its own GPUI fork. Iced and egui, because the open tests did not cover them, and GPUI passed P1 and P4.
