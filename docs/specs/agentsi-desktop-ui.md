# AgentSI desktop: design system, performance and screens

**Status:** approved 2026-11-02 · **Source:** `docs/specs/agentsi-desktop.md`, P4 report, `prototypes/ui-kit/`, `apps/web/DESIGN.md`

## Problem

The owner wants the app to feel as good as Zed, Linear and Raycast. P4 found that the sample window paints every 41 ms when nothing changes. A public app also needs one set of tokens and components, so that contributors build each screen the same way.

## Goal

- One design system crate with tokens, a theme and the own components.
- A gallery that shows each component in each state.
- A performance bar with numbers, and a bench mode that measures it.
- The screens of the app, with the milestone of each one.

## Non-goals

- A light theme in M1. M1 ships one theme. A light theme comes later.
- Custom window decorations on Linux.

## Terms

- **Token**: one named value of the design, for example a color, a size or a duration.
- **Gallery**: the example program `cargo run -p agentsi-ui --example gallery`.
- **Bench mode**: `agentsi-app --bench <idle|scroll|terminal|start|attention> --seconds <n> --json`. It prints frames, frame intervals and CPU.

## Design

### Library

- `gpui-component` 0.9.0, through `gpui-kit` 0.9.0, gives the base components (ADR 0004).
- `agentsi-ui` sets the theme of `gpui-component` from the tokens. Screens use `agentsi-ui` and never set a color or a size by hand.
- `agentsi-ui` uses a part of `gpui-component` when the library has it, for example `Tag`, `Kbd` and the value transitions of `gpui-base`.
- These parts are own code on plain GPUI: the terminal pane, the pill, the hover and press transitions, and each icon that Lucide does not have.

### Tokens

`crates/agentsi-ui/src/tokens.rs` holds each token:

- Color: near-black surfaces (4 levels), 3 text levels, border, focus, one violet accent as in the P4 sample (`prototypes/ui-kit/`), and the status colors `ok`, `warn`, `danger`, `info` and `off`. A pressed control is not a fifth surface: it is its fill with a 4 % light press layer.
- Type: Inter for the UI and JetBrains Mono for the terminals and ids, both bundled under the SIL Open Font License (SIL OFL). Sizes 12, 13, 14, 16, 20 and 28 px.
- Space: a 4 px grid. Radius: 6, 8 and 12 px.
- Motion: 120 ms for hover and press, 180 ms for panels, 260 ms for dialogs. All use ease-out, also when a panel or a dialog closes.
- No screen uses a repeating animation. A repeating animation draws each frame at the full refresh rate. A caret or a pulse uses a timer that changes a value and asks for one frame. A caret stops blinking after 5 s with no typing.
- A unit test checks that each text and fill pair has a contrast of 4.5:1 or more.

### Performance bar and open measurements

The bench mode measures each line on the dev machine, in a release build, with a focused window. Each line has a method and a pass line.

A bench run is valid only when no input and no change of focus reach the window during the measurement. The bench counts pointer, wheel and key events and focus changes, and reports `valid`. A run that is not valid has no verdict. On Hyprland with `follow_mouse`, a pointer that crosses the window gives it focus.

| Case | Method | Pass line | P4 or P3 value |
|---|---|---|---|
| Idle repaint | `--bench idle`: count the frames that GPUI draws in 10 s with no input and no animation, after the first frame | 0 frames per second | 25 frames each second (P4) |
| Scroll of 1,000 minutes rows | `--bench scroll`: the minutes list of the first product, scrolled at 4,000 px per second for 10 s. The intervals between frames count (U13: the core needs a product with 1,000 minutes) | p95 interval ≤ 1.2 × refresh interval | 45.8 ms (P4) |
| Claude pane, PTY read to paint | `--bench terminal`: a script writes to the PTY, and the bench takes the time from the read to the paint | p95 ≤ 16 ms | 9 to 18 ms (P3) |
| Start to first paint of the front desk | `--bench start --runs 10`: start 10 new processes, one after the other, and take the time from each start to the first paint that shows the front desk with its data (U13). The file cache of the system stays warm, because to clear it needs root | p95 ≤ 400 ms | not measured |
| App memory with 8 panes | RSS after 60 s with 8 panes open | ≤ 300 MB | 165 MB with 1 pane (P3) |
| Attention marker | A script writes `OSC 7501` `blocked` into a pane. The bench takes the time to the paint of the marker. 20 runs | p95 ≤ 100 ms | not measured |
| Dictation accuracy | The `prototypes/stt-eval/` harness on the owner's 20 recordings, with the vocabulary on | WER ≤ 8 %, and each product name right in 18 of 20 | synthetic run only (P2) |

**Gate:** U2 is a go/no-go gate. If the idle repaint cannot reach 0 frames per second, the build stops for a review before the screen units. If another line fails, the unit that owns it does not finish.

### Shell and layout

- A sidebar groups the products by host. A product of a host that does not answer shows "device off".
- A product of an SSH host opens the same screens and terminals as a local product. A tab of a host names the host. While the host is off, its screens show "device off" and its panes keep their last screen. The protocol spec holds the rules, in "Remote screens and panes (U17b)".
- A dock on `gpui-component` holds the panes, with tabs and splits.
- The command palette (`Ctrl+K`, `Cmd+K` on macOS) holds each action of each screen.
- The app saves its layout to `<config>/agentsi/layout.json` and restores it at start.
- The layout file holds the dock and the active product. A terminal panel holds its terminal id. A screen panel holds its kind and its product (U11).
- At start, the app drops each terminal panel whose id the core does not know, and shows a notice in the sidebar with the ids. A terminal id names one terminal of one core run, so a panel never attaches to another program.
- When the owner closes the window, the core and its terminals keep running. When the owner closes the tab of a terminal, the app sends `terminal.kill`.
- The app sends `focus.report` when a pane gets the focus. When the window loses the focus, the last pane stays the target of dictation.
- The header of a terminal pane shows the program state, the attention marker and the voice switch. The switch calls `voice.mode {terminal, on}` and shows the state of the reply.
- A bench run of the app uses no layout file. It opens its own panes and kills their terminals at the end.

### Screens

| Screen | Contents | Milestone |
|---|---|---|
| Front desk | State of the product, objectives, pending decisions, last minutes | M1 read-only, M2 decide |
| Terminals | Dock of panes, new shell or `claude` in a product path, attention marker, voice switch on each pane | M1 |
| Minutes | Virtual list, and one minutes with priorities, sources, disagreements, decisions, check result and usage | M1 read-only, M2 marks |
| Settings | Voice models, voices, keys, dictation key, hosts | M1 voice, M2 hosts |
| Meeting | One live pane for each role, from the stream-json events | M2 |
| Call | Player that shows the speaking role, listened mark. See `agentsi-desktop-calls.md` | M2 (U26) |
| Stories | List, and one story with criteria, session state, diff and the review actions | M2 |
| Tickets | The tickets of a product by state, with an add field and filters, and one ticket with its fields, links and actions. The front desk shows the tickets for the owner. See `agentsi-desktop-tracker.md` | M2 |
| Memory | Items of each agent with helpful and harmful counts, retire, reflections with revert and the decision of a proposal | M2 (U25) |
| Roles | Roster, overlays, specialists, validate, apply, reset | M2 (U21) |

`agentsi-desktop-parity.md` maps each web route of today to one of these screens.

#### Front desk and minutes (U13)

- The front desk reads `product.desk` and shows, from top to bottom: the name, the id, the status and the roles of the product, a row of figures, the pending decisions, the objectives of the brief, the usage, the last 3 minutes with a button to the list, and the index line. A pending decision shows its text, its why, its action, the command `agentsi decide <id>` and a button to its minutes. U20 adds the decide buttons (below).
- The minutes list reads `minutes.list` and draws a virtual list. A row shows the question, the id, the date, the marks (`3/5 marked`), `heard` and the status. A click or `Enter` opens the minutes.
- One minutes reads `minutes.get` and shows the id, the status, the check result, the question, four figures and five tabs: Priorities (with marks, objective, sources), Disagreements, Decisions (with their status), Roles (answer, concerns, notes) and Usage. An empty Disagreements or Roles tab is left out.
- U20 adds the writes of the owner. Each write calls the core (`decision.decide`, `archive.archive`, `mark.set`, `archive.restore`), shows its outcome in one line above the content, and then reads each open screen again. The app also subscribes to the `data` topic: each `data.changed` event, and each `core.lagged`, makes each open screen except the settings read again. So a write by the CLI shows in the app. A second write waits while one runs.
  - A pending decision has the buttons Approve, Reject and Archive. A reflection decision has them too. A decision that the core did not apply, for example a stale one, shows the reason in place of "approved".
  - A priority has the buttons Accept, Change and Reject, and a New button that flips its "new" flag once it has a mark. The button of the current mark has the filled look. A mark may be changed, as `agentsi mark <id> <idx> <mark>` does.
  - One minutes has a button that archives it, or restores it when it is archived. The minutes list leaves archived minutes out, so the restore button is on the screen that stays open.
  - The front desk and one minutes take focus as one element, with a cursor on the pending decisions or on the priorities. `Up`, `Down`, `Home` and `End` move it. The card under the cursor has the accent border.
  - Keys of the focused screen: on the front desk `A` approves, `R` rejects, `X` archives and `Enter` opens the minutes of the decision. On one minutes (Priorities tab) `A` accepts, `C` changes, `R` rejects and `N` flips the "new" flag. A key with a modifier does nothing here.
  - The palette has an entry for each action. The palette closes with the focus on a pane, so an entry acts on the screen that the owner used last: the last key press or click in it.
  - A decision that the owner archives from the desk comes back with `agentsi restore <id>`. The app has no list of archived decisions.
- The roles screen reads `roles.list` (U21). It shows the roster, the specialists (ids with a dot) and the overlays that no roster agent uses. A row shows the label, the id, the source tag (`global`, `overlay` or `custom`), the model and the effort, and the error of an agent that does not resolve. Validate runs `roles.validate` and lists the failures. Reset (one agent or all) and Apply plan first and show the plan. Confirm writes it. Cancel drops it. There is no text field: Apply takes the path of the stage folder from the clipboard.
- The meeting screen (U22) shows the last meeting of the product since the app started. The app subscribes to the `meeting` topic, so a meeting that `agentsi meet` started shows too.
  - One pane for each role and for the synthesis, three panes in a row. A pane shows the state, the seconds, the cost, the text of the run, its tool calls and its structured answer. A pane keeps its last 400 lines and follows its last line.
  - The toolbar shows the stage and the cost so far. It has Cancel while a meeting runs, and Open the minutes at the end.
  - A meeting starts from one question field with the roster of the product. The screen says that a meeting costs money. The field stays hidden while a meeting runs. A meeting that started in the CLI can choose other roles.
  - The newest meeting of a product owns the screen. Later events of an older meeting change nothing, and Cancel stops the meeting that the screen shows.
  - A meeting ends with its `done` event, the reply of `meeting.start`, a `meeting.cancel` that finds no such meeting, or a lost link. `core.lagged` adds a line that says a pane can miss lines.
- The memory screen (U25) reads `memory.list`. It shows the active items of each agent, newest first, then the retired items of that agent, then the reflections of the product, newest first.
  - An item shows its id, its text, its date, its source, and tags for a lesson and for helpful and harmful counts above 0.
  - A reflection shows its signals, one line each, its memory changes with their diffs, and its proposal with the rationale, the diff and the replay table. The replay table keeps the Spanish text of the decision.
  - An applied reflection that changed memory has Revert. A pending proposal has Approve and Reject, which call `decision.decide`, as the front desk does.
  - Retire shows the text of the item first. Revert shows first which items it retires and restores, and how many counts it undoes. Confirm writes, Cancel drops it. The screen has no text field, so a retired item gets the reason "Retired by the owner in the app.".
  - A cursor moves over the active items of all the agents. `X` asks to retire the item under the cursor, `Enter` confirms and `Escape` cancels. The palette has entries that open the screen, ask to retire the item, ask to revert the newest reflection that can be reverted, and approve or reject the newest pending proposal. They act on the screen of the active product. A hidden tab ignores them.
  - An approval whose target changed says that nothing was applied, as `agentsi decide` does.
- A screen shows "Reading …" while it waits, and its error with a hint when the call fails. The palette entry "Reload the open screens from the core" reads each open screen again.
- The start bench shows no marker until the first screen has its data. Its first paint is thus the first paint of the front desk with its data, and not of an empty window.

### Keyboard

- Each action has an entry in the command palette.
- The keys of this list are the defaults. The owner changes each one in the settings screen, section "Keys", and the palette and the screen show the keys in effect (`agentsi-desktop-keys.md`).
- `Ctrl+Shift+R` opens the roles of the active product (U21).
- `Ctrl+Shift+S` opens the stories of the active product (U23). In the list, the arrow keys move the cursor and `Enter` opens the story.
- `Ctrl+Shift+Y` opens the memory of the active product (U25).
- `Ctrl+Shift+G` opens the meeting of the active product (U22). On that screen `Enter` in the question field starts the meeting. The palette has entries that start the meeting in the question field, cancel a running meeting and open its minutes.
- `Ctrl+Shift+P` opens the call of the open minutes, or of the last meeting (U26). On that screen `Space` plays or pauses, `Left` and `Right` move between segments and `L` flips the listened mark.
- `Ctrl+T` opens a terminal in the active product. `Ctrl+1` to `Ctrl+9` select a product. macOS uses `Cmd` in place of `Ctrl`.
- `Ctrl+Shift+T` opens `claude` in the active product. `Ctrl+Shift+D` shows its front desk. `Ctrl+Shift+L` shows its minutes (U13). `Ctrl+Shift+M` turns voice mode of the focused pane on or off (U11). `Ctrl+,` opens the settings (U15c). The palette closes the window.
- The writes of U20 bind no global key. Their keys work only in the focused screen, and the palette has the same actions.
- The app binds no key of the pane (`Ctrl+Shift+C` and `Ctrl+Shift+V` copy and paste). It binds no other key that a terminal program needs, for example `Ctrl+W`, `Ctrl+R` or `Ctrl+Q`. The keys of this list are the exceptions: `Ctrl+K` and `Ctrl+T` do not reach the program of a pane.
- Focus moves with `Tab` and shows a 2 px focus ring.
- A list takes focus as one element and moves a cursor with `Up`, `Down`, `Page Up`, `Page Down`, `Home` and `End`. `Enter` opens the row of the cursor. Its rows have no focus handles. In the gallery, a focus handle in each row cost 3 ms of CPU in each frame of a scroll.

## Acceptance criteria

- [ ] `cargo run -p agentsi-ui --example gallery` shows each component in its default, hover, pressed, focused and disabled states.
- [ ] `cargo test -p agentsi-ui` passes, with the contrast test.
- [ ] `rg -n 'rgb\(|hsla\(|px\(' crates/agentsi-app/src` finds no color or size outside `agentsi-ui`.
- [ ] `agentsi-app --bench idle --seconds 10 --json` reports 0 frames after the first, with a focused window, in a valid run.
- [ ] `agentsi-app --bench scroll --seconds 10 --json` reports a p95 frame interval of 1.2 × the refresh interval or less.
- [ ] `agentsi-app --bench terminal --seconds 10 --json` reports a p95 PTY-to-paint time of 16 ms or less.
- [ ] With 8 panes open, the RSS of `agentsi-app` is 300 MB or less after 60 s.
- [ ] With a scripted `blocked` state, the attention marker paints within 100 ms (p95 over 20 runs).
- [ ] `agentsi-app --bench start --runs 10 --json` reports a p95 first paint of 400 ms or less.
- [ ] The owner opens each M1 screen and accepts the look in writing in the log.
