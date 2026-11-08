# Security

AgentSI runs on one machine for one person. It runs model-written code in story sessions, so read this file before you turn on Stories mode.

## Report a vulnerability

Open a private security advisory on the GitHub repo: the "Security" tab, then "Report a vulnerability". Do not open a public issue for a vulnerability. This is a personal project, so there is no promised response time.

## Threat model

The main threat is a prompt injection: text in a product repo, an issue or a web page that makes a model do something the owner did not ask for. The owner of the machine is trusted. The registry is trusted, because only the owner writes it.

### Story sessions

A story session is the one place where a model writes code. Risk for a user: medium.

What protects you:

- The session writes into a git worktree on a `story/*` branch. Only `agentsi story accept` merges, and only after you review the diff.
- Claude Code runs with `--restricted`. The file tools refuse each path outside the worktree. This covers `..` paths, a sibling folder with the same name prefix and a symlink that points outside. A real test with Claude Code 2.1.306 proved it. See `docs/specs/agentsi-hardening.md`.
- The Claude Code Bash sandbox has no network. It has `failIfUnavailable`, so a session does not start without the sandbox.
- A deny list blocks git, `gh`, network tools, deploy and publish commands, `WebFetch` and `WebSearch`.
- Bash allows only the test command, the gate commands and a few read commands of the product.
- The tests and the gates run in bubblewrap: a read-only root, a writable worktree only, a private `/tmp` and no network.
- The runner fails the session when a session changes the `.git` file or leaves files in `.claude/` or `.worktrees/`.
- Each session has a cost cap, USD 5 by default (`stories.budget_usd`).

What does not protect you:

- bubblewrap binds `/` read-only. A test or a gate can read `~/.ssh`, `~/.claude` and your other repos. It cannot send them out, but a session can copy what it reads into the diff. Read every diff before `accept`.
- The dependency install runs outside any sandbox, with network, before the first session. It runs the install scripts of the product, from the base commit. This is the same trust as a manual install on your main branch.
- `stories.test` and `stories.gate` run through `sh -c`. They come from your own registry. Keep the data root out of shared or synced folders.

### Meetings and roles

Risk for a user: low.

- Role calls get `Read`, `Grep` and `Glob`, and the scout gets `WebSearch`. They run with `--restricted` in the product directory, so they cannot read files outside it.
- A prompt injection in the product docs can still steer the minutes. The minutes stay on your machine. Nothing acts on a decision until you approve it.
- The snapshot of a meeting goes to Anthropic. The README lists what it holds.

### Web app

Risk for a user: low to medium.

- The web app binds to `127.0.0.1:3361`. It has no authentication.
- A proxy rejects each request whose `Host` header is not `127.0.0.1`, `localhost` or `[::1]`. This blocks DNS rebinding.
- Server actions check that `Origin` matches `Host`.
- The web app can approve, change or discard a story and log a listen. It cannot start a story session. Only the CLI can.
- `agentsi web` runs `next dev`, not a production build.
- Any other local user or process on the machine can reach the web app.

### MCP server

Risk for a user: low.

- The server runs over stdio and only reads. Its SQL uses parameters. `role_memory` checks the agent id before it builds a path.
- It answers for any registered product, whatever the current directory. A prompt injection in one repo can make Claude read the minutes or the memory of another product.

### The push guard hook

Risk for a user: low.

- While the plugin is on, the hook runs before each Bash call in every Claude Code session. It asks before `git push` and `gh pr create`.
- A script file that pushes passes the guard. The hook is a prompt, not a security boundary.
- To turn it off, disable the plugin or remove the hook from `plugin/hooks/hooks.json`.

### Postgres

Risk for a user: low.

- `docker-compose.yml` uses the user `agentsi` with the password `agentsi`.
- The port binds to `127.0.0.1:5561` only. Any local process can connect.
- The database holds an index of your products and the meeting usage. Do not expose the port.
