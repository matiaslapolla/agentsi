# Prototype: the voice pill in GPUI (open test 1)

Throwaway prototype for open test 1 of `docs/ideas/agentsi-desktop.md`.

Run it on Linux with Wayland: `cargo run --release`. The pill shows the preparing wave for 2 s, then the speaking bars for 4 s, then the program stops. A click stops it at once.

## Result, 2026-11-02

Linux (Hyprland, Omarchy), `gpui-pre` 0.3.11: pass.

- `WindowKind::LayerShell` makes a surface on the overlay layer (level 3), at the top center of the focused output (DP-1, at 4196,82, 168x40).
- The keyboard focus stays on the terminal: the pill takes no focus.
- The background is transparent and the corners are round.
- The process stops by itself after 6 s with exit code 0.
- `gpui-pre-platform` needs the `wayland` feature. Without it, the program opens no window and prints nothing.

macOS: not run. `WindowKind::PopUp` in `gpui-pre-macos` makes a nonactivating NSPanel at `NSPopUpWindowLevel`, which is the kind of window that Hex uses for its pill. A check for `aarch64-apple-darwin` fails on Linux because Metal shaders need the macOS SDK. Run it on the MacBook to close the test.
