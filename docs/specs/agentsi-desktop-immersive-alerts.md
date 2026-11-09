# AgentSI desktop: tones and notifications

**Status:** draft · **Source:** `docs/ideas/desktop-immersive.md`, `docs/ideas/desktop-immersive-eval.md` and the owner decisions of 2026-11-09

## Problem

The app plays no sound and sends no notification. When the target of an alert is in a hidden tab or the window is not active, the owner does not see its glow.

## Goal

- An alert plays a tone when the owner cannot see its target, and it obeys the mute, the volume and the quiet hours.
- A toast or a system notification tells the alert, and a click opens the target.
- The alert log keeps the alerts of this run of the app.

`agentsi-desktop-immersive.md` defines the work states, the alerts and the shared surfaces of this module.

## Non-goals

- A recorded sound set or alerts in the voice of a role. The app has `cpal` and no audio decoder, so tones made in code come first (owner, 2026-11-09).
- An alert log that survives a restart.
- Alerts in the pill.

## Terms

- **Toast**: a short message at the top right of the window that closes by itself.
- **Alert log**: the alerts of this run of the app, newest first.
- **Tone**: a short sound that code makes at start, with no sound file.
- **Quiet hours**: a time range in which the app plays no tone and posts no system notification.
- **Alerts file**: `<config>/agentsi/alerts.toml`, the settings of the section "Alerts".

## Design

### Tones

`crates/agentsi-app/src/chime.rs` makes 4 tones at start, at the rate of the output device. Each note is a sine with a 5 ms rise and an exponential fall to -40 dB. The peak is 50 % of full scale, times the volume.

| Tone | Work state | Notes | Length |
|---|---|---|---|
| `attention` | Blocked, not for a bell | 880 Hz for 120 ms, 40 ms of silence, 1175 Hz for 120 ms | 280 ms |
| `ready` | Review, also "minutes written" | 660 Hz for 140 ms, 990 Hz for 160 ms | 300 ms |
| `error` | Failed | 392 Hz for 160 ms, 294 Hz for 200 ms | 360 ms |
| `done` | Done, off by default | 1320 Hz for 90 ms | 90 ms |

- `chime.rs` copies the `Output` pattern of `screens/call/player.rs`. It opens one `cpal` stream at the first tone and closes it after 10 s with no tone.
- A test output records each tone and makes no sound.
- The app plays no tone in these cases:
  - The target is visible in the active window.
  - A tone played less than 2 s before. The alerts of those 2 s share that tone.
  - A call plays, the pill speaks, or the microphone is open.
  - The mute is on, or the clock is in the quiet hours.
- The pill speaks from `voice.speak` until `voice.stop` or the next `voice.speak`. Without one of them, the speech ends after 0.4 s for each word of the text, 60 s at most.
- The microphone is open from `voice.mic {open: true}` to `voice.mic {open: false}`. It is also open from `converse.state` `started` to `stopped`.

### Toasts

- A toast is a `Notification` of gpui-component at the top right. It shows only when the target of the alert is not visible.
- The toast has the `StateMark` icon and the text of the alert.
- A click on the toast opens the target (`on_click`). The toast has no action button, because an action stops the autohide.
- Three toasts show at most. A toast of a target replaces its older toast (`Notification::id1`).
- The toast keeps the motion of gpui-component: 400 ms in and 200 ms out. Its timer stops after 5.2 s, so the idle rule holds 10 s after the alert.

### System notifications

- When the window is not active, the app also posts the alert with `NotificationDelivery::System`.
- The key `system = false` of the alerts file turns it off. In quiet hours, the app posts none.

### Alert log

- A bell button with the count of unseen alerts sits in the sidebar header. The bell button and the palette entry "Show alerts" open the log as a popover.
- Each row shows the time, the product, the agent or the pane, the `StateMark`, the text and "Open". "Open" opens the target.
- The log keeps 200 alerts. When the popover opens, the count goes back to 0.

### Settings

The section "Alerts" sits after "Appearance" on the settings screen. It has one control for each key of the alerts file.

- The app writes the alerts file atomically, as `appearance.toml`. The app does not overwrite a file that does not parse.
- A missing file gives the defaults below. An invalid key gives its default and one note on the settings screen.
- A change applies at once. A change of the volume plays `attention` one time at the new volume.

```toml
mute = false
volume = 60                 # 0 to 100
done = false                # the tone of Done
system = true               # system notifications
quiet = ["22:00", "08:00"]  # absent: no quiet hours
```

### Protocol

- The core sends the new event `voice.mic {open: bool}` on the topic `voice`. It sends `open: true` with each `dictation.start` and `open: false` with each `dictation.stop` (`crates/agentsi-core/src/dictate.rs`).
- The app subscribes to `voice` on the local core, for events 24 and 28 and `voice.mic`. A `voice` subscription does not set `core.status.pill`, because only the `dictation` topic sets it.
- The pill ignores `voice.mic`.
- `schema/core.openrpc.json` gets the event.

### Defaults taken

- Q4: the app posts a system notification when the window is not active. `system = false` turns it off.
- Q5: Blocked, Failed and Review make a tone. "Minutes written" is Review. The bell makes no tone. The tone of Done is off by default.
- Q8: the alert log keeps alerts for one run of the app.
- Event 27: the core sends `voice.mic`, so no tone plays during dictation.
- Event 24: no event tells the end of the speech of the pill, so the app estimates it.

## Acceptance criteria

- [ ] `cargo test -p agentsi-app chime` passes. Each tone is 400 ms or less, and each rule of "Tones" holds with the test output.
- [ ] `cargo test -p agentsi-app alerts_file` passes. A missing file gives the defaults, an invalid key gives one note, and a write keeps unknown keys.
- [ ] `cargo test -p agentsi-proto --test schema` passes, and `schema/core.openrpc.json` has `voice.mic`.
- [ ] `cargo test -p agentsi-core dictate` passes. Each `dictation.start` and `dictation.stop` also sends `voice.mic`.
- [ ] `agentsi-app --bench alert --seconds 10 --json` reports 0 frames in a valid run, with a toast shown.
- [ ] The owner hears `attention` one time for a blocked pane in a hidden tab, and opens the pane from the toast.
- [ ] During dictation, during a call, in quiet hours and with the mute on, no tone plays. The glow, the toast and the log row stay.
- [ ] When the window is not active, a system notification shows the alert.
