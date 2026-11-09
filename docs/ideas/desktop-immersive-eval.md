# Immersive desktop app: inputs and findings

**Date:** 2026-11-09 · **Brief:** `docs/ideas/desktop-immersive.md`

This file holds the inputs of the brief. It lists the events that the app can use, what exists today, the state map, the design of the four parts, the findings on GPUI and audio, and the questions for the owner.

## Terms

These terms are new. They are not in `CONTEXT.md` yet.

- **Alert:** what the app shows or plays when an event needs the owner. It is a glow, a sound, a toast or a row in the alert log. A protocol message keeps the name "event", as in `docs/specs/agentsi-desktop-protocol.md:85`.
- **Glow:** a border of 2 px in the color of a state, with a soft shadow outside it.
- **Toast:** a short message at the top right of the window that closes by itself.
- **Alert log:** the list of past alerts, newest first.
- **Agent card:** the small view of one agent: the initial in a circle, the name and a word for what the agent does now.

Registry paths below start at `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`. All other paths start at the repo root.

## Event inventory

The app subscribes to the topics `terminal`, `data`, `models` and `meeting` of the local core (`crates/agentsi-app/src/shell.rs:340`). It subscribes to `voice` only for the core of an SSH host (`crates/agentsi-app/src/hosts/conn.rs:476`).

| # | Event | Carrier today | Source | Gap |
|---|---|---|---|---|
| 1 | Meeting started | `meeting.progress {stage: start}` | `crates/agentsi-proto/src/meeting_api.rs:154` | None |
| 2 | A role starts its run | `meeting.progress {stage: role, phase: start, role}` | `crates/agentsi-proto/src/meeting_api.rs:155` | None |
| 3 | A role reads, searches or writes | `meeting.event {meeting, role, line}`, one stream-json line | `crates/agentsi-proto/src/events.rs:116`, parsed in `crates/agentsi-app/src/screens/meeting.rs:212` | One line for each message, so the status changes every few seconds, not each token |
| 4 | A role answers | `meeting.progress {stage: role, phase: end, seconds, cost_usd}` | `crates/agentsi-proto/src/meeting_api.rs:155` | None |
| 5 | A role fails | `meeting.progress {stage: role, phase: end, error}` | `crates/agentsi-proto/src/meeting_api.rs:173` | None |
| 6 | The PM writes the minutes | `meeting.progress {stage: synthesis}` and `meeting.event` with role `synthesis` | `docs/specs/agentsi-desktop-protocol.md:335` | None |
| 7 | Check failed | `meeting.progress {stage: check, phase: end, note: "<n> failures: ..."}` | `crates/agentsi-agency/src/meeting/run.rs:279` | No boolean. The app must read the `note` prefix. The repair then runs as role `synthesis-repair` |
| 8 | Meeting warning | `meeting.progress {stage: warning, note}` | `crates/agentsi-proto/src/meeting_api.rs:156` | None |
| 9 | Minutes written | `meeting.progress {stage: done, note: <minutes id>}` and `data.changed {kind: minutes}` | `crates/agentsi-core/src/methods/meeting.rs:176` | None |
| 10 | Meeting failed or canceled | `meeting.progress {stage: done, error}` | `crates/agentsi-proto/src/meeting_api.rs:157` | None |
| 11 | Decision pending | `data.changed {kind: decision}` or `{kind: minutes}` | `crates/agentsi-core/src/methods/marks.rs:152` | No event names the new decision. The app reads `product.desk` again and compares |
| 12 | Story session queued, tests, work, checks | `data.changed {kind: stories}`, then `story.list` gives `session.state` | `crates/agentsi-core/src/methods/story.rs:124`, `crates/agentsi-proto/src/story_api.rs:74` | No event carries the state. The app reads the list again |
| 13 | Story needs review | `session.state = ready`, same carrier as 12 | `crates/agentsi-app/src/tree.rs:122` | Same as 12 |
| 14 | Story session failed | `session.state = failed` with `failure`, same carrier as 12 | `crates/agentsi-proto/src/story_api.rs:87` | Same as 12 |
| 15 | Story session live work | None | `crates/agentsi-agency/src/story/claude.rs:199` uses `--output-format json` | No stream. The agent card can show only the state of 12 |
| 16 | Claude waits for input | `terminal.state {state: blocked, kind: permission, question or auth, msg}` | `crates/agentsi-proto/src/events.rs:52`, hooks in `docs/specs/agentsi-desktop-protocol.md:143` | None |
| 17 | Claude works | `terminal.state {state: working}` | `crates/agentsi-proto/src/events.rs:44` | None |
| 18 | Claude ended a turn | `terminal.state {state: done}` from the `Stop` hook | `docs/specs/agentsi-desktop-protocol.md:143` | None |
| 19 | Program error | `terminal.state {state: error}` | `crates/agentsi-proto/src/events.rs:49` | Only a program that writes OSC 7501 sends it |
| 20 | Program stopped | `terminal.exit {id, code}` | `crates/agentsi-proto/src/events.rs:72` | None |
| 21 | Terminal bell | None from the core. `agentsi-vt` gives `Event::Bell`, the pane gives `PaneEvent::Bell` | `crates/agentsi-vt/src/terminal.rs:1077`, `crates/agentsi-term-view/src/pane.rs:387` | The app drops it at `crates/agentsi-app/src/pane.rs:477` |
| 22 | Terminal title | `terminal.title {id, title}` | `crates/agentsi-proto/src/events.rs:87` | None |
| 23 | A call plays, a role speaks | None. The call screen finds the role from `timings.json` | `crates/agentsi-app/src/screens/call.rs:95` | Local to the call screen |
| 24 | The pill speaks a reply | `voice.speak {terminal, role, text}` and `voice.stop` | `crates/agentsi-proto/src/events.rs:142` | The local app does not subscribe to `voice`. No event tells the end of the speech |
| 25 | Call render | `meeting.render {stage: start, segment, encode, done, error}` | `crates/agentsi-proto/src/call_api.rs:229` | None |
| 26 | Call listened | `data.changed {kind: call}` | `crates/agentsi-core/src/methods/call.rs:178` | None |
| 27 | Dictation starts or stops | `dictation.start`, `dictation.stop` | `crates/agentsi-proto/src/events.rs:186` | A client on the `dictation` topic makes `core.status.pill` true (`docs/specs/agentsi-desktop-protocol.md:161`). The app must not subscribe to it |
| 28 | Conversation mode | `converse.state {started, stopped, silent_reply, no_pane}` | `crates/agentsi-proto/src/events.rs:201` | The app does not subscribe to `voice` |
| 29 | Reflection runs and ends | None. `reflect.run` answers at the end, then `data.changed {kind: memory}` | `crates/agentsi-core/src/methods/reflect.rs:162`, `crates/agentsi-app/src/model.rs:248` | The app knows only a reflection that this window started |
| 30 | Model download | `models.progress`, `models.done`, `models.error` | `crates/agentsi-proto/src/events.rs:245` | None |
| 31 | Git, knowledge or clear write | `data.changed {kind: git, knowledge or clear}` | `crates/agentsi-core/src/methods/git.rs:61`, `knowledge.rs:90`, `data.rs:386` | None |
| 32 | Events lost | `core.lagged {missed}` | `crates/agentsi-proto/src/events.rs:35` | The app must read the state again |
| 33 | SSH host lost or back | None. The app link sets `HostState::Off` | `crates/agentsi-app/src/hosts/mod.rs:76` | Local to the app |

Count: 33 events. No recorded event stream exists yet.

## What exists today

- **Appearance:** density, corners and text size (`docs/specs/agentsi-desktop-appearance.md:11`). Colors and motion stay constants (`docs/specs/agentsi-desktop-appearance.md:54`).
- **Theme:** one dark theme (`crates/agentsi-ui/src/theme.rs:42`). A light theme is out of M1 (`docs/specs/agentsi-desktop-ui.md:18`).
- **Color tokens:** `status::OK`, `WARN`, `DANGER`, `INFO`, `OFF`, with `TINT_ALPHA` 0.14 and `BORDER_ALPHA` 0.35 (`crates/agentsi-ui/src/tokens.rs:137`). `accent::BASE` (`tokens.rs:123`). Eight label colors (`tokens.rs:347`).
- **Motion tokens:** 120 ms hover and press, 180 ms panel, 260 ms dialog, one ease-out curve (`crates/agentsi-ui/src/tokens.rs:509`).
- **Hover transitions:** `fade` moves a value in 120 ms with ease-out (`crates/agentsi-ui/src/components/mod.rs:136`).
- **Frame rule:** no repeating animation. A pulse uses a timer that asks for one frame (`docs/specs/agentsi-desktop-ui.md:44`). The idle app draws 0 frames (`docs/specs/agentsi-desktop-ui.md:55`).
- **Program state:** the pane maps each state to a status (`crates/agentsi-app/src/pane.rs:76`). The attention marker does not pulse (`crates/agentsi-ui/src/components/display.rs:72`). It shows in the pane header and the product row (`crates/agentsi-app/src/sidebar.rs:444`).
- **Tabs:** a tab shows the 3 px bar of its group color and no state (`crates/agentsi-app/src/tab_bar.rs:481`).
- **Meeting screen:** one text pane for each role, with a tag `running`, `done` or `failed` (`crates/agentsi-app/src/screens/meeting.rs:385`).
- **Call screen:** a circle with the initial of each role. The circle of the role that speaks is lit in `status::OK` (`crates/agentsi-app/src/screens/call.rs:557`). A playing call repaints each 50 ms (`call.rs:37`).
- **Audio playback:** the call player on `cpal` (`crates/agentsi-app/src/screens/call/player.rs:249`). The pill plays speech on `cpal` (`crates/agentsi-pill/src/speak.rs:497`).
- **Pill:** a layer-shell overlay with the states hidden, listening, transcribing, speaking, waiting and error. It keeps its own sizes.

## State map

One element shows one state. When two states apply, the first in this table wins.

| State | Events (#) | Color token | Border and glow | Icon and text | Sound | Motion |
|---|---|---|---|---|---|---|
| Waiting | 16, 21 | `status::WARN` | Glow on the pane, its tab, its item row and its product row | Alert icon, "Waiting: `msg`" | `attention` | Glow in 180 ms, 3 pulses of 1.2 s, then static |
| Failed | 5, 10, 14, 19, 25 error, 30 error | `status::DANGER` | Glow on the item row, the tab and the product row until the owner opens it | Cross icon, "Failed: reason" | `error` | Glow in 180 ms, 1 pulse, then static |
| Review | 9, 11, 13 | `accent::BASE` | Glow on the item row and the product row until the owner opens it | Eye icon, "Review" | `ready` | Glow in 180 ms, 1 pulse, then static |
| Repairing | 7 | `status::WARN` | No glow | Wrench icon, "Repairing the minutes" | None | Text change in 120 ms |
| Speaking | 23, 24 | `status::OK` | Ring on the agent card | Wave icon, "Speaking" | None | Ring width follows the audio level at the 50 ms tick of the call |
| Listening | 28 | `accent::BASE` | 1 px edge on the target pane | Microphone icon, "Listening" | None, and all sounds stop | Edge in 120 ms |
| Working | 2, 3, 6, 12, 17, 25, 30 | `status::INFO` | No glow | Dot and the activity word | None | Text change in 120 ms, no spinner |
| Done | 4, 18, 20 code 0 | `status::OK` | No glow | Check icon, "Done" | `done`, off by default | Dot color change in 120 ms |
| Off | 22, 33 | `status::OFF` | No glow | Dot, "Off" or "device off" | None | None |

## Design of the four parts

### Motion

- Motion shows a change of state. It never decorates a still screen.
- Only opacity, color, border and shadow move. No size or position in a list changes, so the layout tests of the appearance spec stay valid. The toast is the one exception: it moves 8 px down when it opens.
- Each duration comes from `tokens::motion`. Each transition uses `EASE_OUT`. The new tokens are `PULSE` (1.2 s), `PULSES` (3) and `PULSE_FPS` (30).
- A pulse is a chain of oneshot animations with `with_max_fps(30)`. After the last step the element is static and asks for no frame. Thus the idle bench reads 0 frames again 10 s after the last alert at most.
- Terminal content, text and scroll never move.
- Reduced motion: `gpui_kit::init` already reads the setting of the system (see GPUI findings). A new setting "Motion" has the values System, Reduced and Full. Reduced calls `set_reduce_motion(true)`. With reduced motion, a glow shows at once at full strength, a toast opens with no move, and each state change is instant.

### Notification sounds

- One set of 4 sounds: `attention`, `ready`, `error` and `done`. Each sound is 400 ms or less.
- Rules for each sound:
  - If the target of the alert is visible in the active window, the app plays no sound.
  - The app plays one sound each 2 s at most. Alerts in that time share the sound.
  - The app plays no sound while a call plays, while the pill speaks, or while the microphone is open.
  - Each sound has a glow or a toast and a row in the alert log.
- Settings: one mute switch, a volume from 0 to 100 % (default 60 %), and quiet hours with a start and an end time. In quiet hours, the app plays no sound and sends no system notification. Glows, toasts and the log stay.
- The settings go in a new section "Alerts" and a new file `<config>/agentsi/alerts.toml`. The appearance file does not fit, because appearance never changes colors (`CONTEXT.md`, "Appearance").
- Gap: the app cannot see dictation without the `dictation` topic (event 27). The core must also send the dictation edges on the `voice` topic, or the app must accept that a sound can play during dictation.

### Live agents on screen

- The agent card reuses the circle of the call screen (`crates/agentsi-app/src/screens/call.rs:557`). A specialist shows its own name.
- The meeting screen gets a row of agent cards above the panes, in the order of the roster. The desk screen gets the same row for the roster of the product, with the work of each agent now: a meeting, a story session or nothing.
- The activity word of a meeting agent comes from its stream-json lines (event 3):
  - After `stage: role, phase: start`: "Starting".
  - A `tool_use` of `Read`, `Grep` or `Glob`: "Reading" and the file name.
  - A `tool_use` of a search tool of the knowledge base: "Searching".
  - A `text` block: "Writing".
  - A `tool_use` of `StructuredOutput`: "Answering".
  - After `phase: end`: "Done", the seconds and the cost. With `error`: "Failed".
- The PM card shows "Writing the minutes" in the synthesis, "Checking" in the check and "Repairing" in the repair.
- A story session shows only its state: "Queued", "Tests", "Work", "Checks", "Ready" or "Failed" (event 12). A live word needs a stream from the core (question 6).
- Speaking: in a call, the card of the role that speaks gets a ring. The ring width follows the level of the samples of the last 50 ms. The call already repaints at that rate, so it adds no frame. For the pill, the app must subscribe to `voice` (event 24).

### Visual alerts

- A glow is a full-color 2 px border plus a shadow with a blur of 8 px at `BORDER_ALPHA`. GPUI draws the shadow with `BoxShadow` (see GPUI findings).
- On a tab, the glow is the outline of the tab. The left 3 px bar stays for the group color.
- A closed product row shows the strongest state of its items.
- Highlight: when the owner opens an alert from a toast or the log, the app scrolls to the target. Then the target fills with the tint of its state and fades in 260 ms.
- Toasts use the `Notification` of gpui-component. A toast shows only when the target is not visible. A toast has an "Open" action. Three toasts show at most, and alerts of one target replace each other.
- System notification: when the window is not active, the app can post the alert to the system through `NotificationDelivery::System` (question 4).
- Alert log: the palette entry "Show alerts" and a count in the sidebar header open it. Each row has the time, the product, the agent or the pane, the state icon, the text and "Open". It keeps 200 alerts.
- Clear rules: the glow of a waiting pane ends when the state changes. The glow of a failure or a review ends when the owner opens the target.

## GPUI findings

GPUI is `gpui-pre` 0.3.11. The app reaches it and the libraries on top of it through `gpui-kit` 0.9.0 (`Cargo.toml:12`, `Cargo.toml:15`).

- `Animation` has a duration, a oneshot flag, an easing function and an optional `max_fps` (`gpui-pre-0.3.11/src/elements/animation.rs:15`). `repeat()` loops it (`animation.rs:44`). `with_max_fps` limits the redraws (`animation.rs:69`).
- `with_max_fps` schedules the next frame with a timer of `1 / max_fps` s. Without it, the animation asks for each frame (`animation.rs:452`).
- `AnimationExt` gives `with_animation`, `with_animations` for a chain, and `with_spring` (`animation.rs:82`).
- Easing functions: `linear`, `quadratic`, `ease_in_out`, `ease_out_quint`, `bounce` and `pulsating_between` (`animation.rs:506` to `animation.rs:542`).
- Reduced motion: `App::reduce_motion` and `set_reduce_motion` (`gpui-pre-0.3.11/src/app.rs:1405`). With the flag set, a oneshot animation draws its end state, a repeating one draws its start state, and neither asks for frames (`animation.rs:76`).
- `gpui-base` 0.9.0 reads the system setting into that flag at init: `NSWorkspace` on macOS and the XDG portal key `reduced-motion` on Linux, followed live on Linux (`gpui-base-0.9.0/src/reduce_motion.rs:1`). The app already calls `gpui_kit::init` (`crates/agentsi-ui/src/lib.rs:20`), so this works today.
- `gpui-base` also gives `Transition`, `transition`, springs, keyframes, sequences, stagger and `Easing::cubic_bezier` (`gpui-base-0.9.0/src/motion.rs:112`, `motion/easing.rs:114`). They obey reduced motion.
- `BoxShadow` has a blur, a spread and an inset flag (`gpui-pre-0.3.11/src/style.rs:349`). It draws the glow.
- gpui-component `Notification` gives toasts with a type, autohide, an action and a placement. Its delivery is in-app, system or both (`gpui-component-0.9.0/src/notification.rs:60`). Its enter and exit take 400 ms and 200 ms (`notification.rs:30`), which differ from the motion tokens.
- System notifications on Linux go through `notify-rust` on the XDG D-Bus interface (`gpui-pre-linux-0.3.11/src/linux/system_notifications.rs:1`).
- `request_attention` works on X11 (`gpui-pre-linux-0.3.11/src/linux/x11/window.rs:1550`) and does nothing on Wayland (`linux/wayland/window.rs:1889`).
- The gpui-component spinner repeats forever (`gpui-component-0.9.0/src/spinner.rs:69`). It breaks the idle rule, so the working state uses a word in place of a spinner.

## Audio findings

- The workspace has `cpal` 0.18 (`Cargo.toml:24`), locked at 0.18.2 with `alsa` and `coreaudio-rs` (`Cargo.lock:1447`). `crates/agentsi-app` already depends on it.
- `Cargo.lock` has no PipeWire or PulseAudio crate. On Linux, `cpal` uses ALSA. A PipeWire system mixes several ALSA streams through its ALSA plugin.
- The call player shows the pattern for a short sound: an `Output` trait (`crates/agentsi-app/src/screens/call/player.rs:149`), a device that opens at the first play (`player.rs:249`), and an output that makes no sound for tests (`crates/agentsi-app/src/screens/call.rs:980`).
- The app has no audio decoder. `ogg` and `opusic-sys` are only in `crates/agentsi-agency`. Thus a sound is either a tone that code makes at start, or 16-bit PCM at 24 kHz (`player.rs:11`) embedded in the program. A sound of 0.4 s takes 19 KB as PCM.
- `crates/agentsi-voice` holds Kokoro speech out and speech in on ONNX Runtime. It can speak an alert in the voice of a role, but the voice runs in the pill (question 3).
- Unknown: the time that `cpal` takes to open the default output on ALSA and on Core Audio. A slow open delays each sound. One stream that stays open while sounds play solves it.
- Unknown: a raw ALSA hardware device can refuse a second stream while a call plays. The rule "no sound while a call plays" avoids the case.

## Accessibility and contrast

- Each state has a color, an icon and a word. A glow never shows a state alone. The tab also shows the icon of a waiting or failed pane.
- Each sound has a glow or a toast and a row in the alert log. A sound never shows a state alone.
- `status::OK` and `status::WARN` have almost the same luminance (10.83:1 and 10.54:1 on `surface::PANEL`). A person with deuteranopia can confuse them, so their icons must have different shapes.
- A non-text element needs 3:1 against its neighbors (WCAG 1.4.11). The table gives the contrast of each glow color. The values come from the formula of `crates/agentsi-ui/src/tokens.rs:526`.

| Color | On `surface::BASE` | On `surface::PANEL` | On `surface::OVERLAY` | At `BORDER_ALPHA` on `PANEL` | On white |
|---|---|---|---|---|---|
| `status::OK` | 11.22 | 10.83 | 9.33 | 2.30 | 1.74 |
| `status::WARN` | 10.93 | 10.54 | 9.09 | 2.27 | 1.79 |
| `status::DANGER` | 7.07 | 6.82 | 5.88 | 1.83 | 2.77 |
| `status::INFO` | 7.69 | 7.42 | 6.40 | 1.93 | 2.54 |
| `status::OFF` | 7.37 | 7.11 | 6.13 | 1.90 | 2.65 |
| `accent::BASE` | 5.88 | 5.67 | 4.89 | 1.70 | 3.33 |

- On the dark theme, each full-color edge passes 3:1. The shadow at `BORDER_ALPHA` does not pass, so the shadow is decoration and the 2 px edge carries the state.
- On a white surface, each status color fails 3:1. A light theme needs its own status colors. The contrast test must then check each glow edge on the surfaces of both themes.

## Open questions for the owner

1. Do you accept this restatement: the app shows the work of the agency through motion, sounds, live agent cards and glowing alerts, with no AI?
2. The app has one dark theme. Does this module add a light theme, or only status colors that are ready for one?
3. Which sound set do you want: tones that code makes, a recorded set with a free license, or short phrases in the voice of each role?
4. When the window is not active, does the app also post a system notification?
5. Which events make a sound? The proposal is waiting, failed, review and minutes written. Is the bell silent and is "done" off by default?
6. Story sessions run with `--output-format json`. Can the core change them to stream-json and send their lines, so the agent card shows live work?
7. The call screen lights the speaking role in green. Does speaking keep green, or does it get its own color?
8. Does the alert log keep alerts after a restart, or only for one run of the app?
9. Does the pill show alerts or agents, or does it stay as it is?

## Shaping steps that did not run

- Step 1: the owner did not accept the restatement (question 1).
- Step 3: `skeptic` did not run. The brief gives its own conditions to stop.
- Step 4: `grilling` did not run. The questions above are its first round.

## Owner decisions (2026-11-09)

- Q2, light theme: this module adds a light theme. Every state color must pass contrast on light and dark surfaces.
- Q3, sound set: no answer yet. The spec starts with tones made in code, because the app has no audio decoder.
