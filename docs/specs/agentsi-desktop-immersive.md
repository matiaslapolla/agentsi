# AgentSI desktop: work states and glow

**Status:** draft · **Source:** `docs/ideas/desktop-immersive.md`, `docs/ideas/desktop-immersive-eval.md` and the owner decisions of 2026-11-09

## Problem

A blocked pane shows a static marker, and its tab shows nothing (`crates/agentsi-app/src/tab_bar.rs:481`). Each screen shows the state of its work in its own way. Thus the owner finds a blocked pane, a failed run or a ready story only when they look.

## Goal

- Each pane, meeting role, story session and agent shows one work state, with a color, an icon and a word.
- A work state that needs the owner makes an alert, and the target of the alert glows.
- The idle app draws 0 frames again 10 s after the last alert.

The immersive app is one module in three specs. This spec holds the work states, the glow and the shared surfaces. `agentsi-desktop-immersive-alerts.md` holds the tones and the notifications. `agentsi-desktop-immersive-theme.md` holds the light theme and the agent cards.

## Non-goals

- The pill. It keeps its states, its sizes and the dark colors.
- A change of the CLI, the MCP tools or the golden files of `agentsi-desktop-parity.md`. The module lands in the Rust programs only, before the switch-over (the exception under Non-goals of that spec).

## Terms

- **Work state**: the one state that the app shows for a pane, a meeting role, a story session or an agent. `tokens::WorkState` holds the 9 states of the state map.
- **Blocked**: the work state of a program that waits for the input of the owner. It is not the pill state "Waiting".
- **Alert**: what the app shows or plays when a work state needs the owner. A protocol message keeps the name "event".
- **Glow**: a 2 px border in the color of a work state, with no fill and no shadow.
- **Pulse**: one rise and fall of the alpha of the glow border in 1.2 s, from rest (`control::GLOW_REST_ALPHA`, 0.75) to 1.0.

## Design

### State map

When two work states apply to one element, the first row wins. The event numbers point to the event inventory of `docs/ideas/desktop-immersive-eval.md`, which gives the carrier of each event.

| Work state | Events | Color | Glow on | Icon | Word | Motion |
|---|---|---|---|---|---|---|
| Blocked | 16, 21 | `status::warn()` | pane, tab, item row, product row | `TriangleAlert` | "Blocked: `msg`" | glow in 180 ms, 3 pulses, then static |
| Failed | 5, 10, 14, 19, 25 error, 30 error | `status::danger()` | tab, item row, product row | `CircleX` | "Failed: reason" | glow in 180 ms, 1 pulse, then static |
| Review | 9, 11, 13 | `accent::base()` | item row, product row | `Eye` | "Review" | glow in 180 ms, 1 pulse, then static |
| Repairing | 7 | `status::warn()` | none | `RefreshCw` | "Repairing the minutes" | text change in 120 ms |
| Speaking | 23, 24 | `status::ok()` | ring of the agent card | `Play` | "Speaking" | theme spec, "Agent cards" |
| Listening | 28 | `accent::base()` | 1 px edge of the target pane | `Mic` | "Listening" | edge in 120 ms |
| Working | 2, 3, 6, 12, 17, 25, 30 | `status::info()` | none | status dot | activity word | text change in 120 ms |
| Done | 4, 18, 20 with code 0 | `status::ok()` | none | `CircleCheck` | "Done" | dot change in 120 ms |
| Off | 22, 33 | `status::off()` | none | status dot | "Off" or "device off" | none |

- `WorkState` lives in `crates/agentsi-ui/src/tokens.rs`. It gives `color()`, `tint()`, `word()` and `rank()`, and `rank()` keeps the order of the table.
- `components::StateMark` in `crates/agentsi-ui/src/components/display.rs` draws the icon and the word. The icons are in the default icon set of gpui-component.
- `pane::status_of` maps through `WorkState`. The attention marker becomes a `StateMark` of Blocked.
- Two carriers have no state field:
  - Event 7: the app reads `meeting.progress {stage: check, phase: end}` with a `note` that starts with a number and " failures:".
  - Events 11 to 14: on `data.changed` of kind `stories`, `decision` or `minutes`, the app reads `story.list` or `product.desk` again and compares the two lists.

### Alert center

- `crates/agentsi-app/src/alerts.rs` holds the gpui global `Alerts`. It keeps the work state of each target: a pane, a meeting, a story, the decisions of a product, a model or a host.
- The pure function `alerts::apply(&mut AlertState, &Notification) -> Vec<Alert>` does the map. Tests call it with no window.
- An alert starts when a target enters Blocked, Failed or Review. A new alert of a target replaces its older alert.
- The glow of Blocked ends when the program state changes. The glow of a bell ends when its pane gets the focus.
- The glow of Failed and of Review ends when the owner opens the target.
- A closed product row shows the work state of its items with the lowest rank.
- `pane.rs` keeps `PaneEvent::Bell` and gives it to `Alerts` as event 21.

### Glow and motion

- `StyleExt::glow(state)` in `crates/agentsi-ui/src/style.rs` draws the edge and a `BoxShadow` at `status::BORDER_ALPHA`.
- The 2 px edge carries the state. At rest it reaches 3:1 on each surface. The owner chose the border only, because a shadow showed through the transparent overlay as a fill.
- A tab draws the glow as its outline, and the 3 px bar keeps the group color. A tab of a Blocked or Failed pane also shows the state icon.
- Only opacity, color and border move. No size or position changes, so the layout tests of the appearance spec stay valid.
- A pulse is a chain of oneshot animations with `with_max_fps(motion::PULSE_FPS)`. After the last pulse, the element asks for no frame.
- New tokens: `motion::PULSE` (1.2 s), `motion::PULSES` (3), `motion::PULSE_FPS` (30), `control::GLOW_EDGE` (2 px) and `control::GLOW_REST_ALPHA` (0.75).
- When the owner opens an alert, the app shows the target. The target fills with its tint, and the tint fades in `motion::DIALOG`.
- With reduced motion, the glow shows at once at full strength, and no pulse runs.
- The bench mode `--bench alert` asks for a `blocked` state in a pane that is not in view. Then it waits 10 s and counts the frames for `--seconds`.

### Shared surfaces

The tracker, actions and conversational desk modules build at the same time. This table lists each shared contract that the three immersive specs change.

| Surface | Change | Rule for the other modules |
|---|---|---|
| Theme tokens, `tokens.rs` | Each color constant becomes a function: `surface::PANEL` becomes `surface::panel()` (theme spec). New `WorkState`, `motion::PULSE*` and `control::GLOW_*` | State colors come only from `WorkState::<State>.color()` (edge, icon, text) and `.tint()` (fill). A highlight uses `.glow(state)`, and the icon and word use `StateMark`. A plain status dot uses `Status`. Never write `Color(0x…)`. After the merge of theme unit U2, run `sed -i -f crates/agentsi-ui/tokens-rename.sed` on your files |
| `look.rs`, `look.toml` | No change of the file. `Hue::color()` reads the palette and is no longer `const` | Do not use `Hue::color()` in a `const` |
| Protocol events | New `voice.mic`. The app subscribes to `voice` on the local core (alerts spec) | None |
| `schema/core.openrpc.json` | Gets `voice.mic` | On a conflict, run `AGENTSI_UPDATE_SCHEMA=1 cargo test -p agentsi-proto --test schema`. Do not merge the file by hand |
| Settings keys | `appearance.toml` gets `theme` and `motion`. New file `alerts.toml` and section "Alerts" after "Appearance" | Put a new section after "Alerts" |
| Sidebar | A bell button in the header. Glow and work state on product rows and item rows (`ProductRow`, `ItemRow` of `components/tree.rs` take `Option<WorkState>`) | Give each new item row its `WorkState` |
| Tabs and pane chrome | Tab outline glow and state icon. Pane glow and the Listening edge | Keep the 3 px left bar for color only |
| Palette | Entry "Show alerts" | None |
| Desk screen | A row of agent cards at the top (theme spec) | If the desk module replaces the screen, keep `agent_row` at the top |
| Golden tests | No change | None |

### Defaults taken

- Q1: the owner decisions of 2026-11-09 accept the four parts.
- Q6: story sessions keep `--output-format json`. A card shows the state of the session only.
- Q7: Speaking keeps `status::ok()`.
- Q9: the pill stays as it is.

## Acceptance criteria

- [ ] `cargo test -p agentsi-ui` passes. Each `WorkState` has a word, an icon and a color, and each glow edge reaches 3:1 on each surface of both palettes.
- [ ] `cargo test -p agentsi-app alerts` passes. It replays `crates/agentsi-app/tests/fixtures/alerts-replay.jsonl`: a meeting with a failed check, a story session that becomes ready, and a blocked pane. Each alert has the state, the icon and the word of the state map.
- [ ] `cargo test -p agentsi-cli --test golden` passes with no changed golden file.
- [ ] `agentsi-app --bench alert --seconds 10 --json` reports 0 frames in a valid run.
- [ ] `agentsi-app --bench idle --seconds 10 --json` reports 0 frames in a valid run.
- [ ] The owner sees a blocked pane in a hidden tab glow and pulse 3 times, and then the glow stays static.
- [ ] With reduced motion, the owner sees the glow at full strength at once, with no pulse.
