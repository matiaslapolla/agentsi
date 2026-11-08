# Contributing

AgentSI is a personal tool. Issues and pull requests are welcome, but there is no promised response time. For a large change, open an issue first.

## Setup

Follow the README up to `agentsi doctor`. The tests need no database and no `claude` CLI.

- The tests make git commits, so git needs `user.name` and `user.email`.
- The story session tests need a working `bwrap`. Without it, they skip.

## Checks

Run these from the repo root:

```sh
pnpm typecheck
pnpm lint
pnpm test
```

For a change in `packages/voice`, also run:

```sh
cd packages/voice
uv run pytest
```

The CI runs the same commands.

## Build the Rust workspace (desktop app)

The Rust rebuild lives in `crates/`. `crates/README.md` lists the crates. The TypeScript code stays until the switch-over.

### Linux

1. Install the system packages. On Ubuntu 24.04:

   ```sh
   sudo apt-get install -y build-essential pkg-config clang cmake curl xz-utils \
     libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev libx11-xcb-dev libxcb1-dev \
     libfontconfig-dev libfreetype-dev libasound2-dev libvulkan-dev libssl-dev
   ```

   On Arch: `base-devel clang cmake wayland libxkbcommon-x11 fontconfig freetype2 alsa-lib vulkan-icd-loader`.

2. Install Rust with `rustup`. `rust-toolchain.toml` pins Rust 1.99.1.
3. Get Zig 0.17.0. `mise install` reads `mise.toml`. Without mise, run `scripts/zig.sh`. It prints a Zig 0.17.0 path, and it downloads a checksummed copy into `~/.cache/agentsi/zig/0.17.0/` when none is on the machine.
4. Share one target folder between worktrees: `export CARGO_TARGET_DIR=~/.cache/agentsi-target`. Cargo locks the folder, so two builds wait for each other.
5. Build and test:

   ```sh
   export AGENTSI_ZIG=$(scripts/zig.sh)
   cargo build --workspace
   cargo test --workspace
   ```

The first build compiles GPUI and takes several minutes. Later builds take seconds.

`agentsi-vt` builds libghostty-vt from one pinned Ghostty commit with that Zig. The first build downloads the source tarball into `~/.cache/agentsi/ghostty/` and compiles it. This takes about 3 minutes. To link a system `libghostty-vt` and skip Zig, set `AGENTSI_GHOSTTY_VT_LIB=/usr/lib/libghostty-vt.so`. `crates/agentsi-vt/README.md` has the details.

`scripts/fresh-clone.sh` builds the committed HEAD in a clean `ubuntu:24.04` container with Docker.

### Dictation key (Hyprland)

The hold-to-talk key needs a bind in your Hyprland config. A Wayland client cannot read a global key. So the compositor runs one command on press and another on release.

AgentSI never edits your Hyprland config. It writes a bind file in its own config folder. You include that file one time.

To set the key in the app:

1. Open Settings (`Ctrl+,`) and find the section "Dictation key".
2. Press Change, then press the key.
3. If another bind holds the key, AgentSI refuses it and names that bind.
4. Press Copy on the include line, and add the line to your Hyprland config.
5. When Hyprland holds the bind, the section shows "Active".
6. To dictate with no key held, choose "Press to start, press to stop" in the row "Mode". AgentSI writes the bind file again in the toggle form, and reloads Hyprland.

After the include line is in place, a new key needs no hand edit. AgentSI writes the bind file and applies it at once:

- In the Lua form, AgentSI runs `hyprctl eval 'dofile("<bind file>")'`. Hyprland does not reload, so the active workspace stays in place. (A `hyprctl reload` makes Omarchy's `monitor-workspaces.lua` move the workspaces.) The bind file removes its old bind and adds the new one, and it is safe to load more than once.
- In the conf form, AgentSI runs `hyprctl reload`, because `hyprctl keyword` cannot remove one bind of yours. On Omarchy, a reload moves the active workspace.

Remove writes a bind file with no dictation bind, so the include line stays valid. The conversation bind stays until you remove it.

If the section shows "The bind file is out of date", press Write again. This happens when `agentsi-core` moved since AgentSI wrote the file.

The include line depends on the form of your config. `<config>` is `$XDG_CONFIG_HOME`. If that is not set, it is `~/.config`. Write the full path, because `dofile` and `source` do not expand `~`.

| Config | Bind file | Include line |
|---|---|---|
| `hyprland.lua` | `<config>/agentsi/hypr-dictation.lua` | `pcall(dofile, "<config>/agentsi/hypr-dictation.lua")` |
| `hyprland.conf` | `<config>/agentsi/hypr-dictation.conf` | `source = <config>/agentsi/hypr-dictation.conf` |

- The Lua line does not need the `o` helper of Omarchy. It works in any Lua config.
- Put the Lua line after your own binds.
- If the bind file is missing or broken, `pcall` keeps the rest of your config. A bare `dofile` of a missing file stops it.
- The bind file calls `agentsi-core` by its absolute path.
- If `AGENTSI_CORE_SOCKET` is set when you save the key, the bind file sets it for the command.
- If a path has a line break, AgentSI refuses to write the file.
- In the `hyprland.conf` form, AgentSI writes `##` for each `#` in a path, because Hyprland reads `#` as a comment.

#### By hand

Use this if you do not use the settings screen. A Lua config has no release bind that works, because it misses the release when the modifier goes up first. So the Lua block stops the dictation on the first key release:

```lua
-- Hold to dictate into the focused AgentSI pane. Release to send the text.
do
  local holding = false
  hl.bind("SUPER + F10", function()
    if holding then return end
    holding = true
    hl.exec_cmd("/path/to/agentsi-core dictate start")
  end, { description = "AgentSI: hold to dictate" })
  hl.on("input.keyboard.key", function(_, _, state)
    if state == 0 and holding then
      holding = false
      hl.exec_cmd("/path/to/agentsi-core dictate stop")
    end
  end)
end
```

A `hyprland.conf` config uses a press bind and a release bind. The release bind ignores the modifiers. It does not consume the key.

```ini
bindd    = SUPER, F10, AgentSI: hold to dictate, exec, /path/to/agentsi-core dictate start
bindrnid = SUPER, F10, AgentSI: hold to dictate, exec, /path/to/agentsi-core dictate stop
```

For another compositor, bind the first command to the press and the second to the release.

The toggle form needs one bind and no release. The command `agentsi-core dictate toggle` starts the dictation when none runs, and stops it when one runs. Add `mode = "toggle"` under `[dictation]` in `~/.config/agentsi/keys.toml`, so that the pill knows the mode.

```lua
hl.bind("SUPER + F10", function()
  hl.exec_cmd("/path/to/agentsi-core dictate toggle")
end, { description = "AgentSI: press to dictate" })
```

```ini
bindd = SUPER, F10, AgentSI: press to dictate, exec, /path/to/agentsi-core dictate toggle
```

#### Requirements

- `agentsi-pill` must run. The command prints "the pill is not running" and exits with 1 when it does not.
- The app must have the model: open Settings (`Ctrl+,`) and download "Parakeet TDT 0.6B v3, int8".
- The text goes into the pane that the app reports as focused. It ends with no Return. To press Return for you, write `send_return = true` in `~/.config/agentsi/dictation.toml`.
- With no focused pane, the pill copies the text with `wl-copy` (package `wl-clipboard`).
- Toggle dictation also needs the model "Silero VAD v5" (2 MB). The pill uses it to find the pauses. With no model, a press in the toggle form shows "Download the voice detector in the settings", and the hold form works as before.
- Toggle dictation has no 120 s limit. The pill sends the text at each pause of 0.6 s, and it stops after 10 min with no speech.

To try the commands with no key, run `agentsi-core dictate start`, speak, then run `agentsi-core dictate stop`.

#### Conversation mode

Conversation mode lets you talk to one Claude pane with no hands. The pill listens, sends each turn to the pane, and speaks the reply.

- To start it, run the palette entry "Talk to this pane", or press the conversation key. To set the key, open Settings (`Ctrl+,`), find the row "Conversation key" in the section "Dictation key", press Change, then press the key. The key goes into the same bind file as the dictation key. One press starts the mode, and the next press stops it. Without the Hyprland bind, run `agentsi-core converse toggle`.
- The target is the pane that has the focus at the start. A later focus change does not move it. With no focused pane, the pill shows "Focus a pane first".
- The pill shows "Listening". A turn ends after 1.2 s of silence. The pill transcribes the turn, the core submits the text to the pane with a Return, and the pill shows "Waiting".
- While the pane works and while the voice speaks, the pill ignores the microphone. After the last sentence of the reply, the pill listens again. The voice does not stop when you speak. A click on the pill ends the mode and the voice.
- Say "stop" or "para" alone to end the mode. A turn with no text sends nothing.
- The mode also ends when the pane closes, when the core stops, and after 10 min with no turn. If the pane gives no reply for 30 min, the pill ends the mode too.
- A start turns on voice mode of the pane, so the Stop hook speaks the reply. The stop puts voice mode back as it was.
- A dictation ends conversation mode, and conversation mode cannot start while a dictation runs.
- The mode needs the same two models as toggle dictation: "Parakeet TDT 0.6B v3, int8" and "Silero VAD v5".

### macOS (M2)

Not supported in M1. M2 adds the Xcode command line tools step, a macOS CI job, and the macOS check of this section.

## Commits

- Keep each commit small and about one change.
- Before each commit, typecheck, lint and the tests of the change must pass.
- The tests use a throwaway data root. Never point a test at your real data root.

## Specs before code

- A new feature starts with a spec in `docs/specs/<slug>.md`. The spec ends with `## Acceptance criteria`, a list of `- [ ]` items.
- An architecture decision gets an ADR in `docs/adr/`.
- Use the terms of `CONTEXT.md`. Add a new term there when a spec needs one.

## Docs style

The docs use Simplified Technical English:

- One topic in each sentence.
- The active voice, with the agent named.
- The condition first.
- One name for each thing, from `CONTEXT.md`.
- No semicolons and no arrows in place of verbs.

Code, comments, commit messages and docs are in English. The web UI is in Spanish for now.

## The guide

`docs/guide/README.md` is the guide of the desktop app. It is one hand-written Markdown page, in English. GitHub renders it, and the app draws it on the Guide screen. The program embeds the page and the PNG files of `docs/guide/img/` at compile time. The app opens the Guide screen from the palette and from the Settings screen.

- Edit the page by hand. Write a key as an inline code span, for example `Ctrl+K`. Draw a diagram as numbered steps, because the Guide screen shows no HTML and no SVG.
- Do not number a heading. The heading gives the anchor of its section, as on GitHub.
- Each link and image must resolve to a file in the repo, a heading of the page or an embedded image. `cargo test -p agentsi-app guide` checks it.
- A screenshot is `![alt text](img/<name>.png)`. `scripts/guide-shots.sh` takes `docs/guide/img/<name>.png` in a nested Hyprland, on synthetic data from `scripts/guide-shots/seed.sql`. It never touches your data or your session. A new image also goes into the list `IMAGES` of `crates/agentsi-app/src/guide.rs`.

## License

By contributing, you agree that your contribution uses the license of the part you change: MIT for the repo, GPL-3.0-or-later for `packages/voice`.
