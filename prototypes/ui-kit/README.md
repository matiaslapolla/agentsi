# Prototype: the front desk on gpui-component (P4)

Throwaway prototype for the desktop app of `docs/ideas/agentsi-desktop.md`. It answers one question: which GPUI component library gives the premium look, on `gpui-pre` 0.3.11?

Answer: `gpui-component` 0.9.0, used through `gpui-kit` 0.9.0. Both pin `gpui-pre =0.3.11`, the version of `prototypes/gpui-pill/`. One `Cargo.lock` holds one `gpui-pre`.

## Run

```
cargo run --release     # first build takes about 8 minutes, then seconds
```

The window closes by itself after 25 s. Set `AGENTSI_LIFETIME=<seconds>` to change that.

- `Ctrl+K` opens the command palette.
- `AGENTSI_OPEN=palette` opens the palette 1.2 s after the start.
- `AGENTSI_BENCH=1` scrolls the minutes list at 4,000 px per second and prints frame times when the window closes. `AGENTSI_BENCH=idle` does the same without scrolling, as the baseline.

## What the screen shows

All text comes from the repo: the terms of `CONTEXT.md`, the mark wizard spec and the desktop idea brief.

- Sidebar with 4 products. `mobile app` shows "device off". The count in the violet tag is the number of pending decisions.
- 3 decision cards with approve and reject buttons.
- Minutes list: 1,000 rows on `v_virtual_list`.
- Story card with its 3 criteria.
- Empty terminal pane, with a blinking caret.
- Command palette in a dialog, with groups, icons, search and key hints.

Screenshots (taken with `grim`, 1600x1000): `screens/01-front-desk.png`, `screens/02-palette.png`, `screens/03-hover-card.png`.

## Result, 2026-11-02, Linux (Hyprland, RTX 3050, 100 Hz)

Verified:

- `cargo check` and `cargo build --release` pass against `gpui-pre` 0.3.11 with the `wayland`, `x11` and `font-kit` features that `gpui-kit` turns on.
- The sample runs on Wayland. The palette opens, the cards animate in, the hover state works, and the scroll works.
- Frame cadence: 24 to 25 fps (41 ms) both with and without scrolling. The GPU is at 1 % and the process at 15 % CPU. The limit is the frame pacing of this setup, not the list. `render()` takes 0.2 ms per frame.
- Scrolling 1,000 rows costs 6.3 ms of CPU per frame, against 5.0 ms for the idle baseline.

Not verified:

- The cause of the 41 ms cadence. Another GPUI app, in a focused window, may behave differently.
- macOS and the `gpui-fast` feature.
- Hover is instant. GPUI has no CSS transition, so a smooth hover needs an own animation.
