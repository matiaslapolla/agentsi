# AgentSI desktop: key bindings

**Status:** approved 2026-11-07, amendment "Live apply" approved 2026-11-07 · **Source:** owner request of 2026-11-07, before the switch-over. It is an exception to the non-goal "a new feature waits" of `agentsi-desktop-parity.md`.

## Problem

The keys of the app are fixed in `crates/agentsi-app/src/actions.rs`. The settings screen shows one key, read only. The dictation key needs a hand edit of the compositor config, and `CONTRIBUTING.md` gives lines that do not work on a Lua Hyprland config.

## Goal

- The owner changes each key of the app on the settings screen. The new key works at once, with no restart.
- The owner picks the dictation key on the settings screen. After one include line in the compositor config, a later change needs no hand edit.
- The palette and the settings screen show the keys that are in effect.

## Non-goals

- macOS. U18 stands by. On macOS the dictation section does not show.
- Compositors other than Hyprland. They get the two commands as text.
- A key chord of more than one step.
- A change to the protocol. This feature lives in the app, as `hosts.toml` does.
- An edit of any file of the owner's compositor config. AgentSI writes only in its own config folder.

## Terms

- **Key file:** `<config>/agentsi/keys.toml`. It holds only the keys that differ from the defaults.
- **Bind file:** the file that AgentSI writes in `<config>/agentsi/` for the compositor. The owner includes it one time.
- **Include line:** the one line that loads the bind file from the compositor config.

## Design

### Key file

```toml
[app]
new-terminal = "ctrl-alt-y"

[dictation]
key = "SUPER + F10"
```

- Each action in `BINDINGS` gets a stable id, which is the name in `[app]`. The GPUI key form is the value.
- A missing file means the defaults. An unknown id or an invalid key is skipped, and the settings screen shows one line for each skipped entry. The app still starts.
- The app writes the file atomically and keeps the other entries.

### App keys

- `actions::bind` joins the defaults and the key file. The app replaces its bindings when the owner saves a key.
- The palette and `VOICE_KEY` hints read the keys in effect.
- The rule at the top of `actions.rs` stays. The editor refuses, with one line that gives the reason:
  - a key with no `Ctrl`, `Alt` or `Super`, except a function key,
  - a key of the pane (`Ctrl+Shift+C`, `Ctrl+Shift+V`),
  - `Ctrl` with one letter and no other modifier (`Ctrl+C`, `Ctrl+D`, `Ctrl+W`), because terminal programs read these keys. An action may keep its own default: `Ctrl+K` and `Ctrl+T` are defaults, so they stay,
  - a key that another action already has, naming that action.

### Settings screen

- The section "Hotkey" becomes "Keys". The text "Read only." goes.
- Each row shows the label of the action, its key, a Change button and, for a changed key, a Reset button.
- Change waits for the next key press. Escape cancels. While the row waits, no app action runs.
- A filter field narrows the rows.
- With no change in progress, the section draws no frame.

### Dictation key

A Lua Hyprland config has no release bind that works: `~/.config/hypr/bindings.lua` says that a release bind misses the release when the modifier goes up first. So the bind file stops the dictation on the first key release, as the Hex block does.

- The app finds the form of the config:
  - `hyprland.lua` in the Hyprland config folder gives a Lua bind file, `hypr-dictation.lua`,
  - `hyprland.conf` gives a `bindd` and `bindrnid` file, `hypr-dictation.conf`. A plain `bindr` misses the release when the modifier goes up first, so the release bind ignores the modifiers and does not consume the key,
  - no Hyprland session shows the two `agentsi-core dictate` commands as text,
  - no `agentsi-core` program shows one line that names the missing program.
- The bind file calls `agentsi-core` by its absolute path, the sibling of the running app or `AGENTSI_CORE_BIN`. If `AGENTSI_CORE_SOCKET` is set, the bind file sets it for the command.
- The settings section "Dictation key" shows:
  - the key, with Change and Remove buttons,
  - the include line, with a Copy button,
  - the state: "Active", "Add the include line", "The bind file is out of date" or "The key is taken by <description>". The state "The bind file is out of date" has a "Write again" button that saves the same key with no capture.
- The state comes from `hyprctl binds -j`, a read. The bind file is out of date when it differs from the text that the app writes now, for example after `agentsi-core` moved.
- The key follows the rules of the app keys, and no app action may hold it. A key that another bind holds is refused, and the bind file stays as it was. `hyprctl binds -j` lists a `code:N` bind with an empty key, so a digit key is also refused when such a bind has the same modifiers.
- The app checks that `keys.toml` parses before it writes the bind file. If a write fails after that, the old bind file comes back.
- A path with a line break is refused. A `#` in a path of the conf form is escaped as `##`.
- After a save, the app applies the key with no reload (see "Live apply"). A test never runs `hyprctl` on the owner's session: a trait carries `hyprctl`, and the tests use a fake.
- The Remove button writes an empty bind file, so the include line stays valid.

### Live apply (amendment of 2026-11-07)

A `hyprctl reload` makes Omarchy's `monitor-workspaces.lua` move the active workspace, so each key save moved it.


- On a save, the app writes the bind file as today. Then it runs `hyprctl eval 'dofile("<bind file>")'` in place of `hyprctl reload`. Remove does the same with the empty bind file.
- The Lua bind file must be safe to load more than once in one Lua state, because Hyprland keeps its Lua state across reloads (Omarchy's `bootstrap.lua` clears `package.loaded` for this reason):
  - It keeps its state in one global table, `agentsi_dictation`.
  - It removes its old bind only if the stored bind handle is alive (`tostring(handle) ~= "HL.Keybind(expired)"`, as Omarchy's `bindings/utilities.lua` does). So a bind of the owner on an old key stays after a reload.
  - Each load gets a new token. A key listener of an older load does nothing when its token is not the current token.
- The conf form keeps `hyprctl reload`, because `hyprctl keyword` cannot remove one bind of the owner. The settings section says so in one line.
- Before the code, a worker proves these facts in a nested Hyprland 0.56.2 that does not hold the owner's session, and writes them in the log:
  1. A write of a file that `dofile` loads does not start a reload. If it does, the worker stops and reports.
  2. `eval` of the bind file applies the new key and removes the old key.
  3. After a reload, one press gives one `dictate start` and one release gives one `dictate stop`.
- The fake `Hyprctl` gets `eval`.

### Docs

`CONTRIBUTING.md` replaces the section "Dictation hotkey" with the settings path, and keeps the hand lines for each form. `agentsi-desktop-voice.md` and `agentsi-desktop-ui.md` change their "read only" and key lines. `CONTEXT.md` gets the three terms.

## Acceptance criteria

- [ ] With no key file, `BINDINGS` gives the same keys as before, and each existing test of `actions.rs` passes. The test of `VOICE_KEY` goes with that constant, because the key rows now read the bindings.
- [ ] A test of the real screen changes a key, and the new key runs the action and the old key does nothing, with no restart.
- [ ] The palette hint and the settings row show the new key.
- [ ] A key file with an unknown id and an invalid key starts the app, keeps the good entries and shows one line for each bad entry.
- [ ] The editor refuses each of the four refused key kinds, with its reason.
- [ ] Escape during a change leaves the key as it was, and a key press during a change runs no action.
- [ ] Reset removes the entry from the key file.
- [ ] The Lua bind file loads in a real Hyprland of the installed version that does not hold the owner's session, or the report says why this was not possible. A press runs `dictate start`, and the release runs `dictate stop`.
- [ ] Unit tests give the expected text of both bind forms, with and without `AGENTSI_CORE_SOCKET`.
- [ ] The three states of the dictation section come from a fake `hyprctl`, and no test runs the real `hyprctl reload`.
- [ ] A taken key leaves the bind file unchanged.
- [ ] The idle app draws 0 frames, and `rg -n 'rgb\(|hsla\(|px\(' crates/agentsi-app/src` finds nothing.
- [ ] `git diff --stat` shows no change to `schema/core.openrpc.json`.
- [ ] The owner sets the dictation key on the settings screen, adds the include line, and dictates with the key.
- [ ] In a nested Hyprland 0.56.2, a key save applies the new key with no reload, and the log records the three facts of "Live apply".
- [ ] Loading the Lua bind file twice gives one start and one stop for each press and release.
- [ ] The fake `Hyprctl` records `eval` for the Lua form and `reload` for the conf form.
- [ ] The owner changes the dictation key, and the active workspace stays in place.
