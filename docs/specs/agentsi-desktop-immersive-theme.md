# AgentSI desktop: light theme and agent cards

**Status:** draft · **Source:** `docs/ideas/desktop-immersive.md`, `docs/ideas/desktop-immersive-eval.md` and the owner decisions of 2026-11-09

## Problem

The app has one dark theme, and each color is a constant in `crates/agentsi-ui/src/tokens.rs`. On 2026-11-09 the owner asked for a light theme. The meeting screen shows one static tag for each role, so the owner cannot see what each agent does now.

## Goal

- The section "Appearance" has a color theme (System, Dark, Light) and a motion choice (System, Reduced, Full).
- In both color themes, each text pair reaches 4.5:1 and each glow edge reaches 3:1.
- No code outside `tokens.rs` names a color constant. Each color comes from a function that reads the palette.
- The meeting, call and desk screens show a card for each agent.

`agentsi-desktop-immersive.md` defines the work states and the shared surfaces of this module.

## Non-goals

- A light terminal pane. Programs such as Claude Code pick their colors for a dark background, so a pane keeps `terminal::*` in both color themes.
- The pill. It never sets the palette, so it keeps the dark palette.

## Terms

- **Color theme**: the colors of the app, Dark or Light. The choice System follows the window appearance of the system.
- **Palette**: the struct of all the color values of one color theme. `Palette::DARK` holds the colors of today.
- **Motion**: the choice of the section "Appearance" that turns reduced motion on or off.
- **Agent card**: the small view of one agent: the initial in a circle, the name and the activity word. A specialist shows its own name.
- **Activity word**: the words of an agent card for its work now, for example "Reading" and a file name.

At approval, the entry "Appearance" of `CONTEXT.md` gets the color theme and motion.

## Design

### Palette and tokens

- `tokens.rs` gets `pub struct Palette`, with one field for each color in the table below and the alpha of `accent::subtle()`.
- A thread-local copy of the palette works as `appearance::current` does. `ColorTheme::apply` is its only writer.
- Each color constant becomes a function with the same name in lowercase. For example, `surface::PANEL` becomes `surface::panel()`.
- The derived colors read the palette: `accent::subtle()`, `state::pressed`, the `switch` colors, `Status::color`, `Status::tint`, `Hue::color`, `Hue::tint_over` and the danger fills.
- The alpha constants stay constants. `terminal::*` stays constant, and `terminal::BACKGROUND` keeps `0x0b0c0f`.
- `crates/agentsi-ui/tokens-rename.sed` has one rule for each renamed constant. Theme unit U2 runs it on each crate. A module that merges later runs it on its own files.
- `theme.rs` calls `Theme::change` with `ThemeMode::Dark` or `ThemeMode::Light`. Then it sets each theme field from the palette, as today.

### Light palette

The Dark column repeats the values of today. Each Light value passes the tests of "Contrast tests".

| Token | Dark | Light |
|---|---|---|
| `surface::base()` | `0b0c0f` | `eef0f3` |
| `surface::panel()` | `101115` | `f7f8fa` |
| `surface::raised()` | `16171c` | `ffffff` |
| `surface::overlay()` | `1e2027` | `e8eaee` |
| `text::primary()` | `e8e9ed` | `15171c` |
| `text::secondary()` | `a4a8b2` | `484d59` |
| `text::tertiary()` | `8a8f9a` | `5c6270` |
| `line::border()` | `23252c` | `dcdfe5` |
| `line::strong()` | `31343d` | `c3c7cf` |
| `accent::base()` | `8b7cf6` | `6248d8` |
| `accent::hover()` | `9d90f8` | `5539cc` |
| `accent::pressed()` | `7a6af0` | `4a2fb8` |
| `accent::on()` | `0b0c0f` | `ffffff` |
| alpha of `accent::subtle()` | 0.16 | 0.12 |
| `status::ok()` | `4ade80` | `146c33` |
| `status::warn()` | `f2b84b` | `8a4f00` |
| `status::danger()` | `f87171` | `b01f26` |
| `status::info()` | `60a5fa` | `1a54ad` |
| `status::off()` | `9a9faa` | `575d69` |
| `switch::thumb()` | `e8e9ed` | `ffffff` |
| label `rose` | `fb7185` | `a50f34` |
| label `orange` | `fb923c` | `9c370a` |
| label `amber` | `fbbf24` | `7a5000` |
| label `lime` | `a3e635` | `3f6110` |
| label `teal` | `2dd4bf` | `0c6159` |
| label `sky` | `38bdf8` | `075a8b` |
| label `violet` | `c084fc` | `7c2bc4` |
| label `pink` | `f472b6` | `a0164f` |

### Contrast tests

- `text_pairs` takes a palette. The 4.5:1 test and the label test run on `Palette::DARK` and on `Palette::LIGHT`.
- The terminal pairs check `terminal::FOREGROUND` and the ANSI colors on `terminal::BACKGROUND` only, because the terminal keeps its dark colors.
- A new test checks that each `WorkState` color reaches 3:1 on the 4 surfaces of both palettes.
- The test `the_text_and_fill_colors_do_not_depend_on_the_appearance` stays.

### Settings

- `appearance.toml` gets two keys: `theme` (`system`, `dark` or `light`) and `motion` (`system`, `reduced` or `full`). The defaults are `dark` and `system`, so an app with no file looks as today.
- `ColorTheme` and `Motion` are gpui globals beside `Appearance`. Thus the 27 appearance combinations stay.
- The section "Appearance" gets two more rows of three buttons. The sample row also shows a glow of Blocked.
- With `system`, the app reads `cx.window_appearance()` and follows `observe_window_appearance`.
- A change of the color theme refreshes each window one time.
- `reduced` calls `set_reduce_motion(true)`, and `full` calls `set_reduce_motion(false)`. `system` calls `gpui_kit::base::apply_system_reduce_motion`.

### Agent cards

- `components::AgentCard` in `agentsi-ui` takes a name, a `WorkState`, an activity word and an optional audio level. It replaces `speaker()` of `screens/call.rs`.
- The ring is 2 px in the color of the work state. For Speaking, a `BoxShadow` with a spread of up to 4 px follows the level. A shadow takes no layout space.
- Call screen: the level is the peak of the last 50 ms of samples. The call repaints at that rate already.
- Meeting screen: a row of cards above the panes, in the order of the roster. `screens/meeting.rs` gets `activity(line) -> Option<String>` beside `pane_lines`. It gives these words:
  - After `stage: role, phase: start`: "Starting".
  - A `tool_use` of `Read`, `Grep` or `Glob`: "Reading" and the file name.
  - A `tool_use` whose name starts with `mcp__`: "Searching".
  - A `text` block: "Writing".
  - A `tool_use` of `StructuredOutput`: "Answering".
  - After `phase: end`: "Done", the seconds and the cost. With `error`: "Failed".
- The PM card shows "Writing the minutes" in the synthesis, "Checking" in the check and "Repairing" in the repair.
- Desk screen: `agent_row` draws one card for each id of `DeskResult.roles`. A card shows a meeting of this window that runs the agent. Else it shows a story session with the agent as `owner_role`. Else it shows "Idle" in Off.
- A story session shows its state word only: "Queued", "Tests", "Work", "Checks", "Ready" or "Failed".
- While the pill speaks for a role (alerts spec, "Tones"), the card of that role shows Speaking with no level.

### Defaults taken

- Q6: story sessions keep `--output-format json`, so a card shows the session state only.
- Q9: the pill keeps the dark palette.

## Acceptance criteria

- [ ] `cargo test -p agentsi-ui tokens` passes on both palettes: text pairs at 4.5:1, labels at 4.5:1 and glow edges at 3:1.
- [ ] `rg -n 'surface::(BASE|PANEL|RAISED|OVERLAY)\b|text::(PRIMARY|SECONDARY|TERTIARY)\b|line::(BORDER|STRONG|FOCUS)\b|accent::(BASE|HOVER|PRESSED|ON|SUBTLE)\b|status::(OK|WARN|DANGER|INFO|OFF)\b' crates --type rust` finds lines only in `crates/agentsi-ui/src/tokens.rs`.
- [ ] `rg -n 'rgb\(|hsla\(|px\(' crates/agentsi-app/src` finds nothing.
- [ ] `cargo test -p agentsi-app appearance` passes. The keys `theme` and `motion` read, write and fall back to their defaults.
- [ ] `cargo test -p agentsi-app activity` passes. Each line kind of the list gives its activity word.
- [ ] `agentsi-app --bench idle --seconds 10 --json` reports 0 frames in a valid run with `theme = "light"`.
- [ ] With no `appearance.toml`, the owner sees the app in Dark, as before.
- [ ] The owner sets Light, sees each screen change at once, and the choice survives a restart.
- [ ] The owner runs a meeting and sees each card change its word. In a call, the ring of the speaker follows the voice.
