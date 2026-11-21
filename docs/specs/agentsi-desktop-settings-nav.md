# AgentSI desktop: settings sections, fonts and the terminal

**Status:** approved 2026-11-21 · **Source:** owner request of 2026-11-21 (settings sidebar, terminal settings, font size and families)

## Problem

Settings is one long scroll (`screens/settings.rs:1146`), with 13 sections and no way to jump to one. The owner cannot change the font families, and the text size has only three steps. Each terminal value (font, size, line height, cursor, scrollback, shell, colors) is a constant in `tokens.rs` or `term-view/pane.rs`.

## Goal

- Settings has a left list of its sections. A click shows one section. The palette opens each section.
- Settings has a section "Fonts" and a section "Terminal". Each change applies at once, with no restart.
- The idle app still draws 0 frames.

## Non-goals

- Terminal color themes from files (iTerm, Ghostty). The owner picks from the built-in palettes only.
- A font download. The lists show the bundled fonts and the installed system fonts.
- The pill. It keeps its own sizes.

## Terms

- **Settings section:** one page of the settings screen, with an id (`appearance`, `terminal`…) and a title.
- **Terminal settings:** the values of the section "Terminal", kept in `<config>/agentsi/terminal.toml`.

## Design

### Section list

- `screens/settings.rs` gets a `Section` enum with an id, a title and a render function for each current section.
- The screen draws two columns. The left column (fixed width, `tokens`) lists the sections. The right column scrolls one section.
- The last section opened is saved in `layout.json` (`settings_section`). A bump of the layout version is not needed if the field is optional.
- Keys: Up and Down move in the list when it has focus. Page Up, Page Down, Home and End keep their current job in the right column.
- Hidden sections (Dictation key, Hosts) are hidden in the list too.
- `ScreenSpec` for settings takes an optional section. `nav.rs` and the palette use it.

### Fonts

- `appearance.toml` gets `ui_font`, `mono_font` and `text_offset`.
- `ui_font` and `mono_font`: a family name. The default is Inter and JetBrains Mono. A name that the text system does not know falls back to the default, with a note line, as the other bad keys do.
- The pickers list the bundled families first, then `cx.text_system().all_font_names()`, sorted, with a search field. A monospace filter is on for `mono_font`.
- `text_offset` replaces `TextSize`: an integer from −2 to +4 px, added to each step of `size::xs..xl`. The old `text = "small|medium|large"` reads as −1, 0, +2, so old files keep their look.
- `tokens::font::UI` and `MONO` become functions that read `current()`.
- The contrast and fit tests of the appearance spec run for each of the 7 offsets.

### Terminal

`<config>/agentsi/terminal.toml`, with the pattern of `appearance.rs` (`read`, `parse`, `install`, `change`, `keys::edit`, atomic write):

| Key | Values | Default |
|---|---|---|
| `font_size` | 9 to 24 px | 13 |
| `line_height` | 1.0 to 1.6, step 0.05 | 1.25 |
| `cursor_style` | `program`, `block`, `bar`, `underline` | `program` |
| `cursor_blink` | `program`, `on`, `off` | `program` |
| `scrollback_mb` | 1 to 100 | 10 |
| `shell` | empty (uses `$SHELL`) or a path | empty |
| `palette` | `agentsi-dark`, `agentsi-light`, `follow-theme` | `follow-theme` |
| `copy_on_select` | bool | false |
| `option_as_meta` | bool | true |
| `link_modifier` | `ctrl`, `none` (see `agentsi-desktop-terminal-links.md`) | `ctrl` |

- `tokens::terminal` becomes functions that read the terminal settings. `size::MONO` reads `font_size`.
- `agentsi-light` is a new 16-color palette with the contrast test of the light theme.
- A change of font size or line height resizes each open pane and sends the new size to the program, as a window resize does.
- `scrollback_mb` and `shell` apply to new panes. The section says so under each field.
- The section shows a live sample pane with 4 lines of colored text.

### Live apply

- A `TerminalSettings` global, like `Appearance`. `apply` calls `cx.refresh_windows()` only when a value changes.

## Acceptance criteria

- [ ] Settings shows the list of sections at the left. A click on "Git" shows only the Git section.
- [ ] The palette entry "Settings: Terminal" opens Settings at the Terminal section.
- [ ] A change of `mono_font` to another installed monospace family changes all open panes at once.
- [ ] `text_offset = 4` passes the fit tests of each screen. `text_offset = -2` passes the contrast tests.
- [ ] An old `appearance.toml` with `text = "large"` reads as `text_offset = 2`.
- [ ] A change of `font_size` from 13 to 16 resizes open panes, and `tput cols` in a pane prints the new width.
- [ ] `cursor_style = "bar"` draws a bar cursor in a pane whose program asks for a block.
- [ ] A bad value in `terminal.toml` falls back to its default and shows one note line.
- [ ] The idle bench draws 0 frames with the settings screen open.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass.
