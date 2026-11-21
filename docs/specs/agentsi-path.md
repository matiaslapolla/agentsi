# The core finds the owner's programs on the login shell PATH

**Status:** approved 2026-11-21 · **Source:** owner report of 2026-11-21 ("no detecta claude"), local diagnosis of the same day

## Problem

The systemd unit `packaging/systemd/agentsi-core.service` sets a fixed `PATH` (`@BINDIR@`, `~/.cargo/bin`, `~/.local/bin`, `/usr/local/bin`, `/usr/bin`, `/bin`). On the owner's machine, `claude` is in `~/.local/share/mise/command-wrappers/bin`, which only the login shell adds. The core started by systemd cannot find `claude`, so meetings, reflections, the desk and the doctor fail. The launchd plist has the same fixed `PATH`.

## Goal

- A core started by systemd, by launchd or by the app finds each program that the owner's login shell finds.
- One function in the code finds a program on the `PATH`. All callers use it.
- The doctor shows where each program was found, and which `PATH` the core uses.

## Non-goals

- No setting for a path to `claude` in the app. `AGENTSI_CLAUDE_BIN` is enough for an exception.
- No change to how the app passes `PATH` to a terminal pane, except that the pane gets the merged `PATH`.

## Terms

- **Login shell PATH:** the `PATH` that `$SHELL -l -i` prints. The core reads it once at start.
- **Merged PATH:** the `PATH` of the core process, followed by each folder of the login shell PATH that is not in it yet.

## Design

### Shared lookup

- `crates/agentsi-proto` gets a module `path` with `fn which(name) -> Option<PathBuf>` and `fn is_executable_file(path) -> bool`. It moves `is_executable_file` from `client.rs`.
- These callers use it: `agentsi-agency/src/setup.rs` `on_path`, `agentsi-cli/src/commands/web.rs`, `agentsi-proto/src/client.rs` `find_core_bin`, `agentsi-proto/src/doctor.rs`, `agentsi-voice/src/tts/phonemes.rs`.

### Login shell PATH

- At start, `agentsi-core serve` runs `$SHELL -l -i -c` with a command that prints the `PATH` between two marker lines. The core reads only the text between the markers, because an interactive shell can print other text.
- The run has a time limit of 5 s. On a timeout, an error or no `$SHELL`, the core keeps its own `PATH` and logs one line.
- The core sets its own `PATH` to the merged PATH with `std::env::set_var`, before it starts any thread. All later `Command::new` calls and terminal panes use it.
- `AGENTSI_NO_SHELL_PATH=1` turns off the read. Tests set it.

### claude

- `meeting.rs`, `reflect.rs`, `desk/mod.rs` and `story/claude.rs` start `claude` through one function. It uses `AGENTSI_CLAUDE_BIN` when that is set, else `path::which("claude")`.
- When `claude` is not found, the error says: "cannot find `claude` on the PATH of the core" and gives the merged PATH.

### Service files

- `packaging/systemd/agentsi-core.service` and `packaging/launchd/dev.agentsi.core.plist` keep the fixed `PATH`. It is the base when the shell read fails.
- The comment above the line says that the core adds the login shell PATH at start.

### Doctor

- `agentsi doctor` shows the full path of each tool it found, and the source of the core `PATH`: `process`, `merged` or `shell read failed`.

## Acceptance criteria

- [ ] A test with a fake `$SHELL` that prints noise, then the markers and a `PATH`, gives the merged PATH, with the process folders first and no duplicate folders.
- [ ] A test with a fake `$SHELL` that sleeps longer than the limit keeps the process `PATH`, and the start takes less than 6 s.
- [ ] A test with a fake `claude` in a folder that only the fake shell adds: `meeting` finds and starts it.
- [ ] `rg 'var_os\("PATH"\)' crates` finds only `agentsi-proto/src/path.rs`, the shell read and the terminal `child_path`.
- [ ] `agentsi doctor` prints the full path of `claude` and the `PATH` source.
- [ ] On the owner's machine, after `make install` and `systemctl --user restart agentsi-core`, a meeting starts `claude` (owner check).
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass.
