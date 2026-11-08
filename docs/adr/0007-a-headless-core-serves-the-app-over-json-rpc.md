# A headless core on each host owns the terminals and the data, and clients talk to it with JSON-RPC on a Unix socket

**Status:** accepted 2026-11-02

## Context

The owner works from several devices and from SSH hosts. A Claude session must keep running when the app window closes. The app, the pill, the CLI, the MCP server and the Claude Code hooks all need the same data and the same terminals.

## Decision

- `agentsi-core` runs on each host for each user. It owns the PTYs, the Claude runs, the SQLite file and the knowledge base index.
- The core listens on a Unix socket with mode 0600, and speaks JSON-RPC 2.0 with a framed mode for terminal streams. The format copies Herdr's `herdr-core` (Apache-2.0).
- `agentsi-core connect` carries the protocol over stdin and stdout, so `ssh <host> agentsi-core connect` reaches a remote core.
- An OpenRPC schema file and a compatibility test protect the clients.
- The core owns the PTY from the first version.

## Consequences

- One protocol serves the local device and SSH hosts. SSH gives the identity and the encryption.
- Each feature needs a method and a schema entry.
- A restart of the core ends its terminals.
- Rejected: the app owns the PTY first and the core comes later, because the pane code is the same and a move later costs more. gRPC, HTTP or TCP, because a Unix socket with JSON lines needs no port and no login.
