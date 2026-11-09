# AgentSI desktop: the immersive app, build plan 2 of 2

**Specs:** `docs/specs/agentsi-desktop-immersive.md`, `agentsi-desktop-immersive-alerts.md`, `agentsi-desktop-immersive-theme.md` · **Branch:** `devflow wt new desktop-immersive`

This plan holds U6 to U9. They start after U5 of `docs/plans/agentsi-desktop-immersive.md`. Each unit is one commit. Each Check also runs `python3 ~/dev/personal/productos/claudeconfig/scripts/devflow check --fast`.

## U6 — Alert center
- **Files:** `crates/agentsi-app/src/{alerts,main,shell,pane,tab_bar,sidebar}.rs`, `crates/agentsi-app/tests/fixtures/alerts-replay.jsonl`
- **Steps:**
  1. Write `AlertState`, `Alert` and the pure `alerts::apply` for each row of the state map.
  2. Read the `note` of a failed check, and read `story.list` and `product.desk` again on `data.changed`.
  3. Write the fixture: a meeting with a failed check, a story session that becomes ready, a blocked pane.
  4. Add the gpui global `Alerts`. Subscribe to `Topic::Voice` on the local core.
  5. Give `PaneEvent::Bell` to `Alerts`.
  6. Draw the glows of Failed and Review, and apply the clear rules.
- **Check:** `cargo test -p agentsi-app alerts`.

## U7 — Tones and the alerts file
- **Files:** `crates/agentsi-app/src/{chime,alerts_file,alerts,main}.rs`, `crates/agentsi-app/src/screens/settings.rs`, `crates/agentsi-app/src/screens/settings/alerts.rs`
- **Steps:**
  1. Make the 4 tones of the alerts spec at the rate of the device.
  2. Copy the `Output` pattern of the call player. Close the stream after 10 s with no tone.
  3. Apply the four rules of "Tones", with the estimate of the pill speech.
  4. Read and write `alerts.toml` with `toml_edit`, as `appearance.rs` does.
  5. Add the section "Alerts" after "Appearance".
- **Check:** `cargo test -p agentsi-app chime` and `cargo test -p agentsi-app alerts_file`.

## U8 — Toasts, system notifications and the alert log
- **Files:** `crates/agentsi-app/src/{alerts,shell,sidebar,actions}.rs`
- **Steps:**
  1. Push a toast with `Notification::id1` when the target is not visible.
  2. Open the target from `on_click`, and fade its tint in `motion::DIALOG`.
  3. Add `NotificationDelivery::System` when the window is not active.
  4. Add the bell button, the count and the log popover of 200 rows.
  5. Add the palette entry "Show alerts".
- **Check:** `cargo test -p agentsi-app alerts`, and `cargo run --release -p agentsi-app -- --bench alert --seconds 10 --json` reports 0 frames. The owner opens a blocked pane from a toast.

## U9 — Agent cards
- **Files:** `crates/agentsi-ui/src/components/{display,mod}.rs`, `crates/agentsi-app/src/screens/{meeting,call,desk}.rs`
- **Steps:**
  1. Add `AgentCard`. Replace `speaker()` of the call screen with it.
  2. Give the speaking card the peak level of the last 50 ms.
  3. Write `activity(line)` beside `pane_lines`, with one test for each line kind.
  4. Draw the row of cards above the panes of the meeting screen.
  5. Add `agent_row` at the top of the desk screen.
  6. Show Speaking on the card of the role of `voice.speak`.
- **Check:** `cargo test -p agentsi-app activity` and `cargo test -p agentsi-ui components`. The owner runs a meeting and sees each word change.
