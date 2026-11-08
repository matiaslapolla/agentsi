# AgentSI desktop app

**Status:** approved 2026-11-02 · **Source:** `docs/ideas/agentsi-desktop.md`, `docs/plans/agentsi-desktop-log.md` (P1, P3, P4)

## Problem

AgentSI is five parts in three languages: a Node CLI, a web app, a Postgres container, a Python voice package and a Claude Code plugin. Voice mode works only on local Linux. The owner works from several devices and SSH hosts, and has no single place for the agency, the terminals and the voice.

## Goal

- Rust programs in `crates/` hold all of AgentSI on Linux and macOS, under Apache-2.0. M1 is Linux only.
- The owner talks to Claude Code in terminals of the app, hears each reply and dictates into the focused terminal.
- The app shows the agency of each product, from the local device and from SSH hosts.
- At parity, one import moves the Postgres data to SQLite, and the TypeScript code leaves the repo.

## Non-goals

- Windows, project creation, and a sync of data between devices.
- Holder processes. A restart of the core ends its terminals.
- A change to the skills and agents of the plugin, except new hook entries.

## Terms

- **Host**: a machine that runs a core. It is the local device or an SSH host.
- **Device**: a machine where the owner sits. It runs the app and the pill.
- **Core**: the headless process `agentsi-core`, one for each user on each host. It owns the data, the terminals and the Claude runs of that host.
- **App**: the GPUI window process `agentsi-app`. It is a client of one or more cores.
- **Pill**: the process `agentsi-pill`. It owns the microphone, the speakers, speech in, speech out and the small overlay window.
- **Terminal**: a PTY and its program, owned by a core. A **pane** is the view of one terminal in the app.
- **Program state**: `idle`, `working`, `done`, `blocked` or `error`, as a program reports with OSC 7501.

## Design

### Processes

```
device                                      SSH host
┌───────────┐  socket  ┌──────────────┐      ┌──────────────┐
│ agentsi-  │─────────▶│ agentsi-core │      │ agentsi-core │
│ app       │          │ SQLite, PTYs │      │ SQLite, PTYs │
│           │──── ssh host agentsi-core connect ───▶        │
└───────────┘          └──────────────┘      └──────────────┘
┌───────────┐  socket        ▲   ▲ socket
│ agentsi-  │────────────────┘   │
│ pill      │        agentsi CLI, agentsi mcp, Claude Code hooks
└───────────┘
```

- The core listens on a Unix socket and speaks JSON-RPC 2.0 (`agentsi-desktop-protocol.md`). Each other process is its client.
- Only the core opens the SQLite file. A client reads data through the core.
- For an SSH host, the app runs `ssh -T <host> agentsi-core connect`. That command carries the same protocol over stdin and stdout.
- If a local client finds no socket, it starts the core. A systemd user unit or a launchd agent can start it at login.
- The pill is a separate process, so voice works when the app is closed.

### Crate layout

| Crate | Output | Contents |
|---|---|---|
| `agentsi-proto` | lib | Protocol types, framing, OpenRPC schema |
| `agentsi-store` | lib | SQLite schema, migrations, stores, Postgres import |
| `agentsi-agency` | lib | Port of `packages/core`: registry, meetings, minutes check, marks, stories, memory, reflection, knowledge base |
| `agentsi-pty` | lib | Own PTY code |
| `agentsi-vt` | lib | libghostty-vt FFI, `build.rs` with Zig, safe wrapper |
| `agentsi-core` | bin `agentsi-core` | Daemon: socket server, terminals, Claude runs, hooks, events. Subcommands `serve`, `connect`, `hook`, `dictate`, `import-postgres` |
| `agentsi-cli` | bin `agentsi` | CLI client with the commands of today, and `agentsi mcp` |
| `agentsi-models` | lib | Model catalog and downloads |
| `agentsi-voice` | lib | Speech in and speech out on ONNX Runtime, audio input and output |
| `agentsi-ui` | lib | Design system on `gpui-component` |
| `agentsi-term-view` | lib | GPUI terminal pane |
| `agentsi-app` | bin `agentsi-app` | Window and screens |
| `agentsi-pill` | bin `agentsi-pill` | Pill window and voice |

Until the switch-over, the TypeScript CLI keeps the name `agentsi`. The Rust binary stays in `target/`.

### Terminals

- The core spawns each terminal with own PTY code: `openpty`, `setsid`, `TIOCSCTTY` and `TIOCSWINSZ`. The child starts with default signals.
- The core sets `AGENTSI_TERMINAL=<id>` and `AGENTSI_CORE_SOCKET` in the environment of each terminal, and keeps one libghostty-vt screen for it. The `PATH` of each terminal finds this `agentsi-core` first. The screen gives the first paint of a new pane and the program state.
- The core sets the libghostty-vt option `OPT_PROGRAM_STATUS`. Then libghostty-vt answers `OSC 7501;?` before the DA1 reply, and reports each change of state.
- The core removes the queries that it answered from the stream to the panes, so no query gets two answers.
- The app keeps a second libghostty-vt screen for each visible pane, encodes the keys and draws the cells.
- `blocked` with `kind=permission`, `question` or `auth` puts an attention marker on the pane and on its product. The marker shows the `msg` text.
- If a program does not answer the probe, the core takes the state from the Claude Code hooks, never from screen text.
- When a client sends text with a Return key, the core writes the text, then the Return key 100 ms later. In P3, text and `\r` in one write became a newline. In U11, Claude Code 2.1.317 took a Return 30 ms after the text as a new line in 4 of 10 runs, and 60 ms as a submit in 10 of 10.

### Milestones

- **M1** runs on Linux only. Speech in is optional: its model waits for open test 2 with the owner's recordings. M1 has no macOS criterion.
- **M2** adds parity, SSH hosts and macOS. Each macOS criterion moves here, after a run on the owner's MacBook. The protocol schema becomes stable.

### Data, screens, voice

`agentsi-desktop-data.md`, `agentsi-desktop-ui.md` and `agentsi-desktop-voice.md` give the details.

### Build

- `rust-toolchain.toml` pins Rust. `mise.toml` pins Zig 0.17.0, which builds libghostty-vt from one pinned Ghostty commit.
- If the pinned Zig is not on the `PATH`, `scripts/zig.sh` downloads it into `~/.cache/agentsi/zig/<version>/` and checks its hash. A fresh clone and CI need no manual step.
- If `AGENTSI_GHOSTTY_VT_LIB` names a library file, `agentsi-vt` links it and runs no Zig.
- `gpui-pre` is pinned to `=0.3.11`. `gpui-kit` and `gpui-component` are pinned to `=0.9.0`. The three move together.
- `CONTRIBUTING.md` lists the system packages and the commands from a fresh clone to a running app.
- CI builds and tests on Linux, and on macOS from M2. Its cache key has the Zig version.

### Switch-over

`agentsi-desktop-parity.md` maps each CLI command and web screen of today to the new programs and gives the switch-over steps. ADR 0008 records the change of license.

## Acceptance criteria

- [ ] On a fresh clone on Linux with no Zig installed, the commands of `CONTRIBUTING.md` build `agentsi-core`, `agentsi-app`, `agentsi-pill` and `agentsi` with no other step.
- [ ] `cargo test --workspace` passes on Linux.
- [ ] M2: the two criteria above also pass on the owner's MacBook.
- [ ] With no core running, `agentsi-app` starts one and shows the front desk.
- [ ] While the app and the pill run, `lsof <data root>/agentsi.db` lists only the `agentsi-core` process.
- [ ] A `claude` session in a pane asks for a permission. Within 100 ms, the pane and its product show the attention marker.
- [ ] When the owner closes the app, `agentsi-core` and its terminals keep running, and the app opens again with the same panes.
- [ ] `printenv AGENTSI_TERMINAL` in a pane prints the id of its terminal.
- [ ] The acceptance criteria of the protocol, data, UI, voice and parity specs pass.
