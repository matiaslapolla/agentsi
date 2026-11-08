# agentsi-app

The window of AgentSI: a sidebar with the products of this device, a dock of panes and screens, and a command palette. The app is a client of the core. The core owns the terminals, so they keep running when the window closes, and the next start opens the same panes again. Spec: `docs/specs/agentsi-desktop-ui.md` (Shell and layout) and `docs/specs/agentsi-desktop.md`.

## Run

```sh
export CARGO_TARGET_DIR=~/.cache/agentsi-target
cargo build -p agentsi-core -p agentsi-app --release
~/.cache/agentsi-target/release/agentsi-app
cargo test -p agentsi-app
```

- The app finds `agentsi-core` in `AGENTSI_CORE_BIN`, next to its own program, or on the `PATH`. If no core listens on the socket, the app starts one.
- `AGENTSI_CORE_SOCKET`, `AGENTSI_HOME` and `XDG_CONFIG_HOME` give a separate socket, data root and layout file, for a test.
- `AGENTSI_TRACE_ATTACH=1` writes the time of each feed after `terminal.attach` to stderr.
- `--size WxH` of a bench is the one size that does not come from `agentsi_ui::tokens`: a flag of the bench run gives it.

## Keys

| Key | Action |
|---|---|
| `Ctrl+K` | The command palette with each action |
| `Ctrl+T` | A shell in the folder of the active product |
| `Ctrl+Shift+T` | `claude` in the folder of the active product |
| `Ctrl+1` to `Ctrl+9` | Select a product and show its front desk |
| `Ctrl+Shift+D` | The front desk of the active product |
| `Ctrl+Shift+M` | Voice mode of the focused pane on or off |
| `Ctrl+Shift+L` | The minutes of the active product |

macOS uses `Cmd` in place of `Ctrl`. The palette also has "Close the window": the core and its terminals keep running. Closing a tab of a terminal ends its terminal (`terminal.kill`).

## Layout

The app saves the dock and the active product to `<config>/agentsi/layout.json`, 300 ms after each change and at the close. A terminal panel saves its terminal id. A terminal id names one terminal of one core run (`t<n>-<run>`). At start, the app drops each panel whose terminal the core does not know, and writes a notice in the sidebar.

## Bench mode

The modes of `agentsi_ui::bench`, with these additions. A bench uses no layout file. It opens its own panes in a grid and kills their terminals at the end.

```sh
agentsi-app --bench idle --seconds 10 --json
agentsi-app --bench scroll --seconds 10 --json
agentsi-app --bench terminal --seconds 10 --json
agentsi-app --bench attention --seconds 10 --json
agentsi-app --bench start --runs 10 --json
agentsi-app --bench idle --seconds 60 --panes 8 --program "<sh command>" --json
```

- `--screen desk|minutes` shows that screen of the first product in place of panes, for example `--bench idle --screen desk`. The scroll bench shows the minutes by default.
- `--panes <n>` opens `n` panes, 1 to 16. `--program` gives their shell command. `--size WxH` fixes the area of the panes in pixels, so runs on different monitors compare.
- `scroll`: the minutes list of the first product. The core needs a product with about 1,000 minutes. The bench moves the list at `--speed` px per second.
- `attention`: the program turns `blocked` on a line `b` and `idle` on a blank line. Every 400 ms the bench sends `b`, and the marker records the time from the send to its paint. The pass line needs 20 samples.
- `start`: the normal start, with the layout file read and never written. The marker starts when the first screen has its data, so the first paint is the front desk with its data.

## Screens

- `src/screens/mod.rs` has `ScreenKind` (`Desk`, `Minutes`, `Minute`), `ScreenSpec` (kind, product and an optional argument such as a minutes id) and `ScreenPanel`. The layout file saves `{"kind", "product"}` and `"arg"` when the screen has one.
- `desk.rs` is the front desk, `minutes.rs` the virtual list and `minute.rs` one minutes. `parts.rs` holds what they share: `Load`, the section title, the stat tiles and the status tags.
- To add a screen: write its view, add a `ScreenKind` variant with its `name`, `title` and a `ScreenView` arm, then add an action and a palette entry if the owner opens it with a key.
- A screen reads the core through `core.read(cx).link.call::<M>(&params)`, a future. Spawn it with `cx.spawn`, and set a `Load` in the reply. `reload` runs the call again, and the shell calls it for "Reload the open screens".
- A screen opens another screen with `Nav::open(cx, ScreenRequest::minute(product, id))`. The shell reads the requests (`Shell::pump_requests`) and calls `show_screen`.
- A list takes focus as one element: its container tracks one focus handle, handles the keys (`cursor_after`) and draws rows with no focus handle.
- `screens::parts::FIRST_SCREEN_READY` is set when the first screen has its data. The start bench shows its marker only from then on.
- `CoreState` holds the products, the terminals with their state, the active product and the notices. Observe it with `cx.observe(&core, …)`, and add the events of a new topic to `CoreState::apply` and the topic to the `core.subscribe` call in `Shell::start`. A screen that does not need the products observes nothing, so it does not redraw on each terminal event.

## Rules and traps

- Take each color and size from `agentsi_ui::tokens`. `rg -n 'rgb\(|hsla\(|px\(' crates/agentsi-app/src` must find nothing, also in tests. A new value is a new token in `agentsi-ui`.
- The terminal pane stops each key down event. A shortcut must be an action with a key binding in `src/actions.rs`, with a palette entry in `ENTRIES`.
- Do not add a key that a terminal program needs, for example `Ctrl+W` or `Ctrl+R`. The keys of the spec (`Ctrl+K`, `Ctrl+T`) do not reach the program of a pane.
- A palette entry with an action dispatches it. An entry without an action runs in `on_confirm` of `Shell::palette`.
- The core sends no stream frame before the reply of `terminal.attach`. `Link::attach` opens the queue of the channel when it reads the reply, so no frame is lost.
- A test module must not write `use super::*` in a file that has `use gpui_kit::*`: the glob brings in the `test` macro of GPUI, and `#[test]` then expands without end. Import each name.
- A shell that starts tmux (for example a `.bashrc` that runs `exec tmux attach-session`) hides `AGENTSI_TERMINAL` and the OSC 7501 state from the pane.
