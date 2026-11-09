# AgentSI desktop: the immersive app, build plan 1 of 2

**Specs:** `docs/specs/agentsi-desktop-immersive.md`, `agentsi-desktop-immersive-theme.md`, `agentsi-desktop-immersive-alerts.md` · **Branch:** `devflow wt new desktop-immersive`

This plan holds U1 to U5, the shared base. `docs/plans/agentsi-desktop-immersive-alerts.md` holds U6 to U9. Each unit is one commit. Each Check also runs `python3 ~/dev/personal/productos/claudeconfig/scripts/devflow check --fast`.

## U1 — Palette and color functions
- **Files:** `crates/agentsi-ui/src/{tokens,appearance,lib}.rs`
- **Steps:**
  1. Add `Palette` with the fields of the theme spec, and `Palette::DARK` with the values of today.
  2. Add the thread-local palette, `current_palette()` and `set_palette_for_test`, as `appearance::current` does.
  3. Add a lowercase function for each color constant. Each function reads the palette.
  4. Keep the constants for now, so that no caller changes.
- **Check:** `cargo test -p agentsi-ui tokens`.

## U2 — Move each crate to the color functions
- **Files:** `crates/agentsi-ui/tokens-rename.sed`, each `.rs` file that names a color constant
- **Steps:**
  1. Write one `sed` rule for each constant, for example `s/\bsurface::PANEL\b/surface::panel()/g`.
  2. Run `sed -i -f crates/agentsi-ui/tokens-rename.sed` on each file that the `rg` of the theme spec finds.
  3. Make `Status::color`, `Status::tint`, `Hue::color`, `Hue::tint_over` and the derived colors plain functions.
  4. Repair each `const` that now calls a function, by hand.
  5. Delete the color constants. Keep `terminal::*` and the alpha constants.
- **Check:** the `rg` of the theme spec finds lines only in `tokens.rs`, and `cargo test --workspace` passes.

## U3 — Light palette, color theme and motion
- **Files:** `crates/agentsi-ui/src/{tokens,theme,appearance}.rs`, `crates/agentsi-app/src/appearance.rs`, `crates/agentsi-app/src/screens/settings/appearance.rs`
- **Steps:**
  1. Add `Palette::LIGHT` with the values of the theme spec.
  2. Make `text_pairs` take a palette, and run the contrast tests on both palettes.
  3. Add the globals `ColorTheme` and `Motion`, and make `theme.rs` apply a palette.
  4. Read and write `theme` and `motion` in `appearance.toml`.
  5. Follow `observe_window_appearance` for `system`.
  6. Add the two button rows to the section "Appearance".
- **Check:** `cargo test -p agentsi-ui tokens`, `cargo test -p agentsi-app appearance`, and `cargo run --release -p agentsi-app -- --bench idle --seconds 10 --json` in Light reports 0 frames.

## U4 — Work states and glow
- **Files:** `crates/agentsi-ui/src/{tokens,style,bench}.rs`, `crates/agentsi-ui/src/components/{display,tree}.rs`, `crates/agentsi-app/src/{pane,tab_bar,sidebar,bench}.rs`
- **Steps:**
  1. Add `WorkState`, the motion tokens and the glow tokens.
  2. Add the 3:1 test of the glow edges on both palettes.
  3. Add `StateMark` and `StyleExt::glow` with the pulse chain.
  4. Give `ProductRow` and `ItemRow` an `Option<WorkState>`.
  5. Map `pane::status_of` through `WorkState`. Replace `AttentionMarker` with `StateMark`.
  6. Make a Blocked pane glow on its pane, tab, item row and product row.
  7. Add `BenchKind::Alert`.
- **Check:** `cargo test -p agentsi-ui` passes, and `cargo run --release -p agentsi-app -- --bench alert --seconds 10 --json` reports 0 frames.

## U5 — The event `voice.mic`
- **Files:** `crates/agentsi-proto/src/events.rs`, `schema/core.openrpc.json`, `crates/agentsi-core/src/dictate.rs`
- **Steps:**
  1. Declare `VoiceMic` with `open: bool` and register it.
  2. Publish it next to each `DictationStart` and `DictationStop`.
  3. Run `AGENTSI_UPDATE_SCHEMA=1 cargo test -p agentsi-proto --test schema`.
- **Check:** `cargo test -p agentsi-proto --test schema` and `cargo test -p agentsi-core dictate`.
