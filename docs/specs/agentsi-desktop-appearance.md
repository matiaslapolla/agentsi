# AgentSI desktop: appearance settings

**Status:** approved 2026-11-07 · **Source:** owner request of 2026-11-07, after the navigation mockup

## Problem

The sizes, the spacing and the corner radius of the app are fixed constants in `crates/agentsi-ui/src/tokens.rs`. The owner cannot make the app denser, rounder or easier to read.

## Goal

The settings screen has a section "Appearance" with three choices. Each change applies at once to the whole app, with no restart:

- **Density:** Compact, Normal, Comfortable. The model is Discord's message display.
- **Corners:** Square, Subtle, Round.
- **Text size:** Small, Medium, Large.

## Non-goals

- Colors and themes. The dark theme of `agentsi-ui` stays the only theme.
- A free number for each choice. Three fixed steps keep each step tested for contrast and fit.
- The text size of the terminal. A pane keeps its own font size.
- The pill. It keeps its own sizes.

## Terms

- **Appearance:** the three choices together, kept in `<config>/agentsi/appearance.toml`.

## Design

### Values

| Token group | Compact | Normal | Comfortable |
|---|---|---|---|
| `space()` step | 3 px | 4 px | 5 px |
| `control::ROW` | 32 px | 44 px | 52 px |
| `control::HEIGHT_SM`, `HEIGHT_MD` | 24, 28 px | 28, 32 px | 32, 36 px |
| sidebar and tree row padding | 4 px | 6 px | 8 px |

| Token group | Square | Subtle | Round |
|---|---|---|---|
| `radius::SM`, `MD`, `LG` | 0, 2, 2 px | 6, 8, 12 px | 8, 12, 16 px |

| Token group | Small | Medium | Large |
|---|---|---|---|
| `size::XS` to `XL` | each minus 1 px | as today | each plus 2 px |
| `control::ICON_SM`, `ICON_MD` | 12, 14 px | 14, 16 px | 16, 18 px |

- Normal, Subtle and Medium are the defaults, and they give the app of today, pixel for pixel.
- `radius::FULL` stays for pills and dots. The focus ring stays 2 px.

### Tokens

- The tokens of these groups change from constants to values that a gpui global `Appearance` gives. Screens read them with the same names through one call, for example `size::base(cx)`.
- Colors, motion and the window size stay constants.
- No screen sets a size by hand. The `rg` rule of `agentsi-desktop-ui.md` stays.

### Settings and file

- The section "Appearance" sits first on the settings screen. Each choice is a group of three buttons with the current one marked. A small sample row under them shows a sidebar row, a button and a tag in the chosen look.
- The app writes `appearance.toml` atomically, as `keys.toml`. A missing file or an unknown value gives the default and a note on the settings screen.
- A change asks one frame for each window. The idle app draws 0 frames.

## Acceptance criteria

- [ ] With no `appearance.toml`, a screenshot test of the settings screen and the sidebar equals the screenshot of today.
- [ ] A test sets each of the 27 combinations and checks that each screen lays out with no text cut and no overlap, at the window size of `control::WINDOW_WIDTH` by `WINDOW_HEIGHT`.
- [ ] Each combination passes the contrast rule (4.5:1) for text.
- [ ] A change applies at once and survives a restart.
- [ ] An invalid file starts the app with the defaults and shows one note.
- [ ] The idle app draws 0 frames, and `rg -n 'rgb\(|hsla\(|px\(' crates/agentsi-app/src` finds nothing.
- [ ] The owner sets Compact, Round and Large, and accepts the look.
