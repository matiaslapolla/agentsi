# Small fixes from the owner checks of 2026-11-19

**Status:** approved 2026-11-19 · **Source:** owner checks of 2026-11-19 on an isolated core and app, owner request of 2026-11-19

## Problem

The owner checks found small faults in the setup, the desk and the alert tones. A draft gives values that the setup form refuses, the form hides its errors, and the desk ignores the objectives of the brief. The volume preview keeps the audio device open, and no volume button shows the default.

## Goal

- A draft with Claude gives values that the form accepts, and it keeps the values that it does not fill.
- After a failed Review, the owner sees the errors without a search.
- A desk table keeps each cell inside its column.
- The desk knows the objectives of the brief.
- The audio device closes 10 s after the last preview tone, and one volume button shows the current volume.

## Non-goals

- The redraw after a move to a monitor with another scale. It is in gpui.
- New fields or new rules in the setup forms.

## Design

### Setup draft (`crates/agentsi-agency/src/setup.rs` and the setup screen of the app)

- F1. A source of truth of a draft has one of the kinds `issues`, `analytics`, `docs` and `other`. The prompt of the draft names these four kinds. The parse of the reply maps `file` and `doc` to `docs`, and each other unknown kind to `other`.
- F2. A draft field with no value leaves the field of the form as it was. The stage stays when the draft gives no stage.

### Setup form (`crates/agentsi-app/src/screens/setup*.rs`)

- F3. A Review with errors scrolls the form to the first error, and it shows one line next to the Review button: "Fix the N fields marked in red above."
- F4. The empty Due field of an objective shows the placeholder `YYYY-MM-DD`.

### Desk

- F5. A table view of the desk wraps the text of each cell inside its column. No text of a cell draws over the next cell.
- F6. The desk session reads the objectives of the brief of the product, through the MCP tool or the context that it already uses for the product. A question on the objectives gets the objectives of the brief.

### Tones (`crates/agentsi-app/src/chime.rs` and the alerts section of the settings)

- F7. A preview tone starts the same 10 s idle timer as an alert tone, so the device closes.
- F8. The default volume is 50, the value of a button.

## Acceptance criteria

- [ ] A draft reply with a source of kind `file` gives a form source of kind `docs`.
- [ ] A draft reply with no stage keeps the stage of the form.
- [ ] A Review with errors shows the line "Fix the N fields marked in red above." next to the Review button, and the form shows the first error.
- [ ] The Due field of an empty objective shows `YYYY-MM-DD`.
- [ ] A desk table with long cells draws no text over the next cell.
- [ ] The desk session can read the objectives of the brief.
- [ ] After a preview tone, the audio device closes within 11 s.
- [ ] With no `alerts.toml`, the volume is 50, and the button 50 shows as chosen.
