# AgentSI desktop: the core protocol

**Status:** approved 2026-11-02 · **Source:** `docs/specs/agentsi-desktop.md`, Herdr `schema/README.md` (Apache-2.0), P3 report

## Problem

The app, the pill, the CLI, the MCP server and the Claude Code hooks all talk to the core, locally and over SSH. Each client needs one contract that does not break when the core changes. The terminals need a byte stream with flow control next to the requests.

## Goal

- One protocol for each client of the core, on a Unix socket and over SSH.
- A schema file that a test keeps current. From M2, a second test fails on a change that breaks a client.
- A hook bridge that gives each Claude Code event to the core of its terminal.

## Non-goals

- Network sockets, TLS or a login. SSH gives the transport and the identity for a remote host.
- Windows named pipes.

## Terms

- **Line mode**: JSON-RPC 2.0, one message on each line.
- **Framed mode**: binary frames that carry JSON-RPC on channel 0 and terminal streams on the other channels.
- **Hook bridge**: the command `agentsi-core hook <event>` that a Claude Code hook runs.
- **First paint**: the bytes that bring the screen of a new pane to the screen of the core: `ESC c`, then the screen as VT sequences.
- **Observer**: a pane that only watches its terminal. It attaches with `observe: true`.

## Design

### Socket

- Linux: `$XDG_RUNTIME_DIR/agentsi/core.sock`. macOS (M2): `$TMPDIR/agentsi/core.sock`. `AGENTSI_CORE_SOCKET` overrides both.
- The core makes the default folder with mode 0700 and the socket with mode 0600.
- A folder from `AGENTSI_CORE_SOCKET` keeps its own mode. If the folder does not exist, the core makes it with mode 0700. The peer check protects the socket in each folder.
- The core checks the user id of each peer (`SO_PEERCRED` on Linux, `getpeereid` on macOS). It closes a connection from another user.
- The core takes the lock file `core.lock` next to the socket, and writes its pid in it. The log of a core that a client started is `core.log`, next to the socket.
- A second core decides that a core is alive from the lock first, then from a probe connect to the socket. If a core is alive, the second core stops with exit code 3.
- If `$XDG_RUNTIME_DIR` is not set, the core stops with a message that names `AGENTSI_CORE_SOCKET`.
- A core that stops removes its socket before it frees the lock, and it holds the lock for up to 10 seconds. If a client starts a core, and that core stops with exit code 3, a core starts or stops. The client then waits up to 15 seconds in all (the stop wait and the 5 seconds of a start). It starts a core again after a pause of 50 ms that doubles up to 1 second, and it connects as soon as a socket exists.

### Modes and frames

A connection starts in line mode. `core.hello` with `{"framed": true}` changes it to framed mode after the reply. The framing copies Herdr:

```
frame = [channel u32 LE][kind u8][len u32 LE][payload ≤ 16 MiB]
kind  0 DATA     channel 0: one JSON-RPC message; other channels: stream bytes
      1 CREDIT   u32 LE: bytes that the receiver consumed
      2 CLOSE    the channel ends; from the core, a JSON reason
      3 RESIZE   u16 LE cols, u16 LE rows (pane to core)
      4 FOCUS    1 or 0 (pane to core)
      5 AUTO     reply bytes that the pane's screen made, not typed keys
```

- The core sends at most 1 MiB on a stream channel before the client returns a CREDIT. Thus a slow pane cannot fill the memory of the core.
- `core.hello` returns `{"protocol": 1, "version": "<semver>", "features": [...]}`. A client needs the features that it uses.

### Methods

Each method that the CLI uses returns the same JSON as the `--json` output of that command today. `agentsi-desktop-parity.md` lists the commands.

| Namespace | Methods |
|---|---|
| `core` | `hello {framed?, client?}`, `shutdown`, `subscribe {topics}` |
| `rpc` | `discover` returns the OpenRPC schema |
| `terminal` | `create {cwd, argv, env, product?}`, `list`, `attach {id, observe?}` returns a channel, `input {id, text, submit?}`, `resize`, `kill` |
| `product` | `list`, `desk`, `status`, `whoami {cwd}` |
| `minutes` | `list`, `get`, `check` |
| `decision` | `list`, `decide`, `add`, `get {product, ids}` (desk conversation) |
| `mark` | `pending`, `set` |
| `ticket` | `list`, `get`, `add`, `update`, `move`, `promote` (tracker, `agentsi-desktop-tracker.md`) |
| `desk` | `history`, `send`, `stop`, `reset`, `show`, `auth` (desk conversation, below) |
| `story` | `config`, `list`, `get`, `new`, `suggest`, `approve`, `discard`, `changes`, `run`, `exec`, `review`, `accept`, `reject`, `result` (U23) |
| `meeting` | `start`, `ask`, `cancel` |
| `knowledge` | `index`, `search`, `get_doc` |
| `memory` | `list`, `retire` |
| `roles` | `list`, `validate`, `apply {stage_dir \| stage}`, `reset` |
| `registry` | `get`, `update` |
| `setup` | `scan`, `draft`, `outcome` |
| `reflect` | `run`, `revert`, `cancel` |
| `archive` | `archive`, `restore` |
| `data` | `backup`, `audit`, `snapshot` |
| `voice` | `mode {terminal, on?}`, `session {session, on?}`, `dictate {start\|stop}`, `transcript {text}` (U14), `relay {host, terminal, role?, text?}` (U17) |
| `vocabulary` | `get`, `set {entries}` (U14) |
| `hook` | `event {terminal, session, event, payload}` |
| `focus` | `report {terminal?}` |
| `models` | `list`, `download {id}`, `delete {id}`, `voices` (U15c) |

### Events

`core.subscribe {topics}` starts notifications on channel 0. The topics are the event namespaces: `terminal`, `data`, `meeting`, `voice`, `dictation`, `models` and `desk`. A second call adds topics. The events are:

- `terminal.state {id, state, kind?, msg?}`, `terminal.exit {id, code}`, `terminal.title {id, title}`.
- `data.changed {product, kind}` after each write. The kind `tickets` follows each write of a ticket. `ticket.promote` sends `stories` or `decision` first.
- `meeting.event {meeting, role, line}` carries each stream-json line of each role. `meeting.progress` carries the stages (U22).
- `voice.speak {terminal, role, text}` and `voice.stop {terminal}`.
- `dictation.target {terminal?}`, `dictation.start {at_us}` and `dictation.stop {at_us}` (U14). `at_us` is when the key edge happened, in microseconds since the Unix epoch.
- `desk.state {product, state, message?}`, `desk.delta {product, text}` and `desk.item {product, seq, at, item}` (desk conversation).
- `models.progress {id, bytes, total, verifying}`, `models.done {id}` and `models.error {id, error, message, file?}` (U15c).
- `core.lagged {missed}` goes to each subscriber. The core keeps 1024 events of the topics of each connection. Past that, it drops the events, and then sends `core.lagged` where they are missing. Then the client must read the state again.

### Additions of protocol 1

These additions are part of protocol 1, which is unstable until M2:

- `core.hello` takes `client`, a name for the log of the core. Its result `features` has `framed` and the namespace of each method.
- `rpc.discover` returns the schema of the running core.
- `core.lagged {missed}`, as above.
- In line mode the core answers the requests of one connection in sequence. In framed mode it answers them at the same time, up to 32 MiB of requests in flight.
- A line and a frame each take at most 16 MiB, without the newline.
- The types that the tables do not give: `terminal.exit.code` is an integer or null (null for a signal). `terminal.state.kind` and `meeting.event.line` are strings.

#### Terminals (U7)

These additions are also part of protocol 1:

- Each terminal id is `t<n>-<run>`. A core counts `n` from 1. `run` is 6 base-36 characters that the core makes at its start (U11). Thus a saved id of an earlier core never names a terminal of a new core.
- `terminal.create` also takes `cols` and `rows`, 80 and 24 by default. `env` is an object of names and values. `cwd` must be a folder that exists.
- `terminal.create` returns the terminal. `terminal.list` returns `{terminals}`, in the order of creation. A terminal has `id`, `argv`, `cwd`, `product`, `pid`, `cols`, `rows`, `title`, `state`, `kind`, `msg`, `running` and `code`.
- A terminal stays in the list after its program stops. Then `running` is false and `code` holds the exit code. `terminal.kill` removes it.
- `terminal.kill` sends SIGHUP and SIGTERM to the process group of the program. After 2 seconds it sends SIGKILL. The group gets the signals also after the program stopped, while a process of it keeps the PTY open.
- When the program stopped, the core removes the terminal from the list.
- The child environment gets `AGENTSI_TERMINAL` and `AGENTSI_CORE_SOCKET`. The core removes `CLAUDECODE*`, `CLAUDE_CODE*`, `CLAUDE_JOB*` and `CLAUDE_BG*`. A name in the `env` param of `terminal.create` stays.
- The `PATH` of the child finds the `agentsi-core` of the core first (U11). If the `PATH` already finds it first, the `PATH` stays as it is. Else the core puts the folder of its own program first, also when the `env` param gives a `PATH`. Thus the hook bridge runs in each Claude session of a terminal.
- `terminal.attach` needs framed mode. On a line-mode connection it returns `invalid`. It returns `{channel}`. Each connection counts its channels from 1.
- A stream starts with a RESIZE frame from the core, then the first paint. The first paint has the screen, its scrollback, the modes, the styles and the cursor. It has no palette, so the pane keeps its theme. It has the active screen only.
- A pane writes a first paint at the size of the RESIZE frame before it, until the next RESIZE frame, then goes back to its own size. Otherwise a first paint for more rows scrolls the earlier output out of view (U15b). Live bytes after a RESIZE frame keep the size of the pane.
- The core sends a RESIZE frame to each pane when the size of the terminal changes, before the bytes at the new size.
- A CREDIT gives the bytes that the pane read since its last CREDIT. The window is at most 1 MiB.
- The core keeps at most 1 MiB of live bytes for a pane. Past that, the core drops the bytes.
- When the window of that pane opens again, the core sends a RESIZE frame and a new first paint. A first paint does not count in the 1 MiB.
- At the end of the program, each pane gets a CLOSE frame with `{"reason":"exit","code":<integer or null>}`. A CLOSE frame from a pane ends its stream. The core sends no reply to it.
- A pane can send a RESIZE frame of 8 bytes: cols, rows, and the width and height of a cell in pixels. `terminal.resize` takes `cell_width` and `cell_height` for the same reason.
- With `observe: true`, the core ignores the DATA, AUTO, RESIZE and FOCUS frames of the pane.
- The core writes the AUTO bytes of one pane only: the pane that sent the last FOCUS 1, else the oldest pane that is not an observer.
- If the program set mode 1004, the core writes `CSI I` when the terminal gets its first focused pane. It writes `CSI O` when the focused pane loses the focus or goes away.
- `terminal.input` with `submit` writes the Return key with the keyboard modes of the program. No other input comes between the text and the Return key.
- At most 8 MiB of input waits for a program. Past that, `terminal.input` returns `busy`, and the core drops the DATA and AUTO bytes of a pane. A frame never stops the read of its connection.
- At most 1 MiB of replies of the screen waits for a program. Past that, the core drops the replies and writes a line to its log.
- The core sends `terminal.state` only when the state changes. It reads the record of the program itself, the record with the empty id. It ignores the records of the sub-tasks.
- When the program clears its state, the core sends `idle`. When the program stops in `working` or `blocked`, the core sends `idle`, then `terminal.exit`.
- Errors: `not_found` for an unknown terminal id, `invalid` for bad params or a program that does not start, `conflict` for input to a program that stopped, and `busy` for input past the budget.

#### Hook bridge and voice mode (U9)

These additions are also part of protocol 1:

- `hook.event {terminal, session, event, payload}` returns `{context?}`. `event` is one of `SessionStart`, `UserPromptSubmit`, `Notification`, `PermissionRequest`, `PostToolUse` and `Stop`. `payload` is the JSON that Claude Code gave to the hook. An unknown terminal returns `not_found`. An empty `terminal` names a Claude Code session outside the app (see "Voice mode outside the app").
- `context` is text that the bridge prints for Claude Code. It holds the voice style rules, on `UserPromptSubmit` with voice mode on. The bridge prints it as `{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":<context>}}`. The bridge prints nothing else.
- The state from a hook: `SessionStart` gives `idle`, `UserPromptSubmit` gives `working`, `Stop` gives `done`, `PermissionRequest` gives `blocked` with kind `permission` and `msg` from `tool_name`, and `Notification` gives `blocked` with kind `permission` (type with "permission") or `question` (type with "elicitation") and `msg` from `message`. A `Notification` of any other type, or with no type, changes nothing. `PostToolUse` changes `blocked` to `working`, and changes no other state (U11). The hooks give no event when the owner grants a permission, so the tool that runs after it ends the wait.
- A hook sets the state only if the program never sent an OSC 7501 record. After its first record, the hooks never change the state of that terminal.
- `voice.mode {terminal, on?}` returns `{terminal, on}`. Without `on` it only reads. Voice mode is off for a new terminal. It is per terminal and lives as long as the core runs.
- With voice mode on, `UserPromptSubmit` sends `voice.stop {terminal}` and saves the prompt. `Stop` saves the turn, then sends `voice.speak {terminal, role: "pm", text}`. `text` holds the spoken sentences, one on each line. A `Stop` with `stop_hook_active: true` is saved and not spoken.
- The transcript of a session is `<data root>/voice/transcripts/<session>.md`. The session is `session_id` of the hook, else the terminal id.

##### Voice mode outside the app

Decided 2026-11-07. `agentsi voice on` in a plain Claude Code session (no `AGENTSI_TERMINAL`) uses `CLAUDE_CODE_SESSION_ID`. The TypeScript CLI used the same variable. A pane of the app wins when both are set.

- `voice.session {session, on?}` returns `{session, on}`. Without `on` it only reads. An id that is not 1 to 100 ASCII letters, digits, `_` or `-`, with one letter or digit at least, returns `invalid`. Two ids never share a file.
- The state is the empty file `<data root>/voice/sessions/<session>`. It survives a restart of the core. The folder is not the one that the TypeScript hook read (`<data root>/voice/<session>.json`), so the two never act on the same session.
- Voice mode applies from the next prompt, as the voice skill says. The core keeps the prompt of a turn only if its `UserPromptSubmit` came with the file present. A `Stop` with no kept prompt is neither spoken nor saved. Turning voice mode off, or on from off, drops the kept prompt. Thus a reply is never paired with the prompt of an earlier turn. A forced second `Stop` of a turn that was kept is saved and not spoken.
- The core removes files that no hook touched for 14 days when a session turns voice mode on. Each hook of a session with the file touches it.
- The bridge with no `AGENTSI_TERMINAL` handles only `UserPromptSubmit` and `Stop`. It reads the folder first. With no file in it, it stops with exit code 0 before it reads stdin. With a file, it reads `session_id` from stdin and stops unless that session has its file. Otherwise it sends `hook.event` with an empty `terminal`. It never starts a core.
- For an empty `terminal` the core does nothing unless the file exists. Then it does what it does for a pane with voice mode on, with the name `session:<session>` in place of the terminal id in `voice.speak` and `voice.stop`. The pill speaks that text. The core keeps no terminal state for such a session.
- The pill speaks the reply, so `agentsi-pill` and the core must run. With no core, the bridge writes one line to `hook.log` and stops with exit code 0. `core.status` has the field `pill`, true while a client is subscribed to the `dictation` topic (only the pill does that). `agentsi voice on` prints one warning line on stderr when `pill` is false.
- `voice.dictate {action: start|stop, at_us?}` returns `{}` and sends `dictation.start {at_us}` or `dictation.stop {at_us}` to the subscribers of the `dictation` topic. `agentsi-core dictate` sends `at_us` from its own clock. The core uses its clock when `at_us` is absent. The bind runs `start` and `stop` as two processes, so they can reach the core in the wrong order. The pill drops a `start` that is older than a `stop` it has seen. The pill is that subscriber. With no subscriber, the core returns `unavailable` ("the pill is not running"). A second `start` while the pill listens changes nothing in the pill.
- `agentsi-core dictate start|stop` is the command-line form of `voice.dictate`. A compositor bind runs it. It never starts a core, and it exits with 1 and one line on stderr when the call fails.
- `voice.transcript {text}` returns `{text, terminal}`. The pill calls it with the text of the model. The core applies the vocabulary, replaces each control character (Return, Escape, Tab) with a space, and types the result into the pane of the last `focus.report` with `terminal.input`. No Return follows, unless `send_return = true` is in `<config>/agentsi/dictation.toml`. `text` is the typed text. `terminal` is the pane that got it, or null when no pane has the focus (nothing is typed, and the pill copies the text).
- `vocabulary.get {}` returns `{entries, path}`. `entries` is a list of `{term, heard}`. When `<config>/agentsi/vocabulary.toml` does not exist, the core builds the list from the product names of the registry. It writes the file only when it read the registry, and never writes the seed again. A file that does not parse returns `unavailable`, and the core leaves it as it is.
- `vocabulary.set {entries}` replaces the list and returns the saved `{entries, path}`. The core trims each text, drops empty texts and repeats, and merges entries with the same term (ignoring case).
- `agentsi-core hook <event>` accepts the event name in any case. It stops after 170 ms at the latest, and it never starts a core.

#### App shell (U11)

These additions are also part of protocol 1:

- `product.list` returns `{products}`, in the order of their ids. A product has `id`, `name` and `path`. `name` is the id when the store has no name. `path` is the first folder of the product, or null. `archived` is true when the archive record holds the product. The core leaves the field out when it is false, and a client reads a missing field as false. The app lists no archived product in its sidebar. It reads the list again at `data.changed` with `kind` `product` (an archive or a restore of a product, also by the CLI), and when a host connects again. If the active product leaves the list, the first product of the sidebar becomes active. The start does the same for the saved active product.
- If `<data root>/agentsi.db` does not exist, `product.list` returns no products, and the core does not make the file. Thus `import-postgres --fresh` can still make it. Without a data root, `product.list` returns `unavailable`.
- `focus.report {terminal?}` returns `{}`. The app sends it when the focused pane changes, and without `terminal` when no pane has the focus. When the terminal changes, the core sends `dictation.target {terminal}`. An unknown terminal returns `not_found`.

#### Voice models (U15c)

These additions are also part of protocol 1. The rules of the files are in `agentsi-desktop-voice.md`, "Model downloads".

- `models.list` returns `{root, models}`, in the order of the catalog. `root` is the models folder. A model has `id`, `name`, `kind` (`speech-out` or `speech-in`), `license`, `description`, `size` (bytes of all its files) and `state`.
- `state` is one of these, with its fields next to it:
  - `missing`: no byte of the model is on disk.
  - `partial {bytes}`: finished files and `.partial` files hold `bytes`.
  - `downloading {bytes, total, verifying}`: a download of this core runs.
  - `downloaded`: a receipt for this catalog entry, and each file at its pinned size.
  - `conflict {file, message}`: a file with a catalog name is not the pinned file. The owner put it there, or an older catalog entry pinned it.
  - `error {error, message}`: the last download of this core failed. `error` is the kind of the failure: `hash_mismatch`, `network`, `bad_response`, `busy`, `canceled` or `io`.
- `conflict` comes from the last download, or from a catalog file with another size than the pinned size. The list never hashes a file. So a foreign file with the pinned size shows as `partial` until a download finds the conflict.
- A file that appears at a catalog path during a download stays. The download then fails with `conflict`.
- The core keeps the last failure of each model while it runs. A new download or a delete clears it.
- `models.download {id}` returns `{id, started}` at once. `started` is false when a download of the model already runs, or when the model is downloaded. Then nothing changes.
- A download sends `models.progress` at most 10 times each second, then `models.done` or `models.error`. `verifying` is true while the core checks the hash of a file.
- `models.delete {id}` deletes the files of the model that the download wrote, and returns the model. A catalog file counts as written by the download when a receipt lists it, also a receipt of an older catalog entry, or when it has the pinned size and hash. It returns `busy` while a download of the model runs, in this core or another process.
- `models.voices` returns `{source, lang, default, default_note?, roles}`. `source` is the `voices.toml` of the data root, or `built-in`. `roles` lists `{role, voice, note?}` in the order of the role names.
- A `note` tells that a voice of another engine than Kokoro falls back to the default voice. A `default_note` tells that a default of another engine falls back to the built-in default.
- A file that does not parse, or a voice that is not `<kokoro|piper|chatterbox>:<name>`, returns `invalid`. The pill rejects the same file.
- Errors: `not_found` for an unknown model id, `busy` as above, `internal` for a disk error.
- Tests and manual runs can change two inputs. `AGENTSI_MODELS_CATALOG` names a catalog file that replaces the built-in catalog. `AGENTSI_MODELS_URL` names a server with the Hugging Face URL layout, and it may use `http://`. The models folder follows `XDG_DATA_HOME`.

#### Front desk and minutes (U13)

These additions are also part of protocol 1. The methods only read. The M2 methods `decision.*` and `mark.*` write.

- `product.desk {product}` returns the front desk of one product:
  - `product` (`id`, `name`, `path`), `status` (`active` or `planned`) and `roles` from `agentsi.config.json` of the data root (empty with no registry);
  - `index` with `docs`, `commits`, `issues`, `indexed_at` and `last_commit {date, title}`;
  - `objectives` of the brief, and `pending_decisions` in the order of `agentsi desk` (the decisions of each valid minutes, oldest minutes first, then the others, each group in id order);
  - `minutes_total`, `last_minutes` (the 3 newest) and `usage` totals.
- The pending decisions and the minutes follow `agentsi status`: a decision or a minutes in `archive_records` is left out, and the marks count come from the `priorities` table. A decision has `id`, `kind`, `minutes_id`, `reflection_id`, `text`, `why` and `action`.
- `minutes.list {product}` returns `{minutes}`, newest first, with `id`, `question`, `status`, `created_at`, `scope`, `marks` (`accepted`, `changed`, `rejected`, `unmarked`) and `heard`. It includes a minutes row that has no stored body. An unknown product gives an empty list.
- `minutes.get {id}` returns one minutes: `priorities` with `sources`, `disagreements`, `decisions`, `roles`, `check`, `usage` and `usage_totals`.
  - The mark and `is_new` of a priority come from the `priorities` table, as the web app shows them.
  - The status of a decision comes from the `decisions` row with the same text. `id` is null when no row has that text.
  - A minutes row with no stored body (from before migration 008) returns `not_found`.
- Errors: `not_found` for an unknown product or a minutes without a body. `unavailable` when `agentsi.db` does not exist (`minutes.list` gives an empty list then) or cannot open.

#### Agency port (U16)

These additions are also part of protocol 1. The methods only read.

- The output of the CLI is the same as the TypeScript CLI prints on the same data. The golden tests of `agentsi-cli` prove it for `status`, `snapshot`, `audit`, `whoami` and `desk`.
- The core reads its own data root, from its own environment. A client does not send a root. So a CLI that talks to a running core shows the data of that core, and `whoami --home` shows the root of that core.

- `product.whoami {cwd?, product?, list?, home?}` returns `{product, products?, home?}`. The registry is `agentsi.config.json` of the data root.
  - With `product`, it returns that product. An unknown id returns `not_found`.
  - Else it returns the product whose folder holds `cwd`. The longest folder wins. `product` is null when no folder matches.
  - A product has `product` (the id), `name`, `scope` (null today), `roles`, `paths` and `owner {name, language}`.
  - With `list`, `products` holds `{id, name, status, paths, archived}` for each product of the registry. With `home`, `home` holds `{root, from}`, and `from` is `AGENTSI_HOME` or `home file`.
  - A data root with no registry returns `unavailable`.
- `product.status {product}` returns `{markdown, index}`. `markdown` is the text of `agentsi status`. `index` holds `repos`, `docs`, `commits`, `issues`, `minutes` and `memory`: what the store holds, for the index line of `agentsi desk`.
- `data.snapshot {product, scope?}` returns `{markdown}`: the context of a meeting, in 60 KiB or less. It reads the repos of the product with `git`, and the store.
- `data.audit {product}` returns the report of `agentsi audit --json`. The `marks` keys keep the order of first appearance. A result is an object whose keys keep the order of the fields, because the core uses `serde_json` with `preserve_order`.
- `product.desk` also gives `priorities` for each pending decision: the unmarked priorities of its minutes that it links.
- `product.status`, `data.snapshot` and `data.audit` return `not_found` for an unknown product, and `unavailable` when `agentsi.db` does not exist.
- The global role files are found by one rule (U21): `<data root>/plugin/agents/` when it exists, else `$AGENTSI_PLUGIN_DIR/agents/` when the variable is set, else the five role files that the build holds. After the switch-over the plugin stays in Claude Code and the programs are on the `PATH`, so a program has no checkout to read. The built-in files are the files of `plugin/agents/` of this repo at build time.
- Roles (U21). `roles.list {product, agent?}` returns `{product, roster?, agent?, overlays, globals, warnings}`: the roster as `{id, role, source, model, effort, error}` for each agent, or the resolved definition of one agent, `source` being `global`, `overlay` or `custom`. `roles.validate {product}` returns `{product, valid, failures, agents, warnings}`. `warnings` holds what the TypeScript code prints on stderr: tools that a meeting does not allow.
- `roles.apply {product, stage_dir?, stage?, yes?, register?: {name, path}}` and `roles.reset {product, agent?, memory?, yes?}` plan first. Without `yes` they write nothing and return the plan. With `yes` they write the database in one transaction (a failed registry edit rolls it back), write the roster in `agentsi.config.json`, and commit that file alone with git. The result holds `applied` and `commit {committed, error?}`. A failed commit does not undo the write. `roles.apply` returns `plan: null` and `failures` for a stage that is not valid. `roles.reset` returns `invalid` for an unknown agent and for a reset that would empty the roster.
- `roles.apply` and `roles.reset` write only these files: the rows of `briefs`, `agent_overlays` and `memory` of the product, and `agentsi.config.json` of the data root. The golden cases compare the registry, the commits of the data root and the list of files after each write.

#### Setup and registry
- `roles.apply` takes `stage`, a map from the name of a stage file to its text, in place of `stage_dir`. Send one of the two. Both or neither returns `invalid`. The keys are `brief.json`, `roster.json`, `stories.json` and `agents/<id>.md`. With `yes`, the call also writes the `products` row and sends `data.changed {product, kind: "product"}`.
- `setup.scan {path}` reads a product folder with code only: the proposed id and name, the registered product, the manifest commands, the north-star docs and the tools on the `PATH`. It returns `invalid` for a path that is not an existing absolute folder.
- `setup.draft {path, previous?, failures?}` drafts the five forms with one `claude -p` call: model `sonnet`, no tools, a cap of 0.50 USD. It returns `{draft, fields, no_data, cost_usd, seconds}`. The app asks one Confirm before it calls it. A repair sends `previous` and `failures` and asks its own Confirm.
- `setup.outcome {draft, product, fields}` records `kept`, `edited` or `cleared` for each drafted field. The app calls it after an apply that a draft filled.
- `registry.get {}` returns `{owner, products}`. `registry.update {owner?, product?}` changes the owner block, or the name and the paths of one product, keeps each other key of `agentsi.config.json`, and makes no commit.
- The app sequence for a new product: `setup.scan`, then optionally `setup.draft`, then `roles.apply` without `yes` (the plan), then on Confirm `roles.apply` with `yes` and `register`, then `registry.update` for more paths, `decision.add` for the north-star decision and `setup.outcome`.

#### Knowledge base (U19)

These additions are also part of protocol 1. `knowledge.index` writes. The other methods only read.

- `knowledge.index {product, force?}` updates the index of one product and returns `{product, repos, minutes, memory, ms}`. The rules are those of `agentsi index` today:
  - For each git repo of the product it reads the docs (six root files and `docs/**/*.md`, 200 KiB or less), `.claude/JOURNAL.md`, the commits of the last 90 days and, with a remote, the 50 newest open issues from `gh`.
  - A repo with the same HEAD and an index time under 10 minutes old is skipped (`skipped: true`), unless `force` is true.
  - A text that did not change keeps its row and its time.
  - A document that left the repo leaves the index. A product with one repo also drops the commits that no branch holds.
  - It indexes the minutes and the memory of the product, and drops the ones that the store no longer holds.
  - `repos[].issues` is null when the repo has no remote or `gh` failed.
- `knowledge.search {product, query, kind?, limit?}` returns `{hits, any_term}`. `limit` is 5 by default, at most 50. A hit has `kind`, `ref`, `scope`, `title`, `rank` and `snippet`. When no document has all the terms, the core searches for any term and sets `any_term`. The ranking is in the data spec. `memory` is searched only when `kind` names it.
- `knowledge.get_doc {product, kind, ref}` returns the document. A hex ref of 7 to 40 characters finds a commit by prefix.
- `knowledge.recent_minutes {product, limit, include_archived?}`, `knowledge.pending_decisions {product, limit}` and `knowledge.role_memory {product, role}` give the data of the MCP tools `list_minutes`, `pending_decisions` and `role_memory`. Their client is `agentsi mcp`. A screen reads the same data from `minutes.*` and `decision.*`.
- Errors: `not_found` for an unknown product or document, `invalid` for an unknown `kind` or an agent id that does not match `pm` or `dev.backend`, and `unavailable` when `agentsi.db` does not exist.
- `agentsi desk` now runs `knowledge.index` first, as the TypeScript `desk` does. It sends no `force`.
- `agentsi mcp` is a client. It starts no core of its own and reads no database. `initialize` and `tools/list` need no core. It joins the core at the first `tools/call`.

#### Marks, decisions and archive (U20)

These additions are also part of protocol 1. The methods write. Each write runs in one SQLite transaction on the writer thread of the store. An error rolls it back, so a failed write leaves no row. After the commit, the core sends `data.changed {product, kind}` for each product that changed. `kind` is `minutes` for a mark, `decision` for `decision.decide` and `decision.add` (also `minutes` when the decision is in a minutes), and the kind of the record for an archive change.

- `core.doctor {}` returns `{checks}`. A check has `name`, `level` (`required`, `stories` or `optional`), `status` (`ok`, `missing` or `fail`), `detail` and `blocks`. The rows are those of `agentsi doctor`, in the same order. It runs on the host of the core and writes nothing.
- `decision.archived {product}` returns `{decisions}` with `id`, `kind`, `minutes_id`, `text`, `why`, `action` and `status`, newest first. It only reads. An unknown product returns `not_found`.
- `mark.pending {product}` returns `{items, decisions_error?}`, as `agentsi mark --pending --json` prints. An item has `kind` `priority` (`minutes_id`, `idx`, `title`, `why`, `owner_role`, `objective`, `sources`) or `decision` (`id`, `minutes_id`, `text`, `why`, `action`, `priorities`).
  - The priorities are the unmarked ones of the valid minutes, oldest minutes first. The pending decisions of a minutes follow its priorities. Product decisions come last.
  - A decision in `archive_records` is left out. The priorities of an archived minutes stay, as in TypeScript.
- `mark.set {entries, unmarked_only?}` returns `{marked}`. An entry has `minutes_id`, `idx`, `mark` (`accepted`, `changed` or `rejected`) and `is_new`.
  - The `priorities` row and the priority inside `minutes.data` change together.
  - A missing minutes or priority returns `not_found`. A priority twice in one call returns `invalid`.
  - With `unmarked_only`, a priority that has another mark returns `conflict`. The same mark again passes, so a retry works.
- `decision.decide {decision, status}` returns `{product, minutes_id, status, action}`. `status` in the params is `approved` or `rejected`. The `decisions` row (`status`, `decided_at`) and the decision at the same position inside `minutes.data` change together.
  - An unknown id returns `not_found`. An archived decision returns `conflict` with the text `Decision <id> is archived. Run agentsi restore <id> first, then decide.`.
  - A decision may be decided again, as in TypeScript.
  - A reflection decision settles its reflection in the same transaction (U25). The result has `minutes_id` and `action` null, and `status` can be `stale`. A role file and its commit come after the last row, so a failed file write rolls each row back.
  - A reflection decision on a proposal that is settled in another way returns `invalid`.
  - A reflection with no proposal can never be applied. The decision is rejected and stays so. The result has `status` `rejected` and `refused`, the text of the reason, and a client reports that text as an error.
  - A commit of a global role that fails does not undo the approval. The result holds its text in `commit_error`.
  - If the minutes of the decision has no stored body, the whole write fails with `not_found`. TypeScript wrote the decision row first and then failed.
- `decision.add {product, text, why, action?}` returns `{id}`. It records a pending decision of kind `product`. An empty text or why returns `invalid`. An unknown product returns `not_found`.
- `archive.archive {product, kind, id?}` and `archive.restore` return `{product, kind, id}`. `kind` is `minutes`, `decision`, `story` or `product`. A product needs no `id`.
  - Each call writes or deletes one `archive_records` row. A record that already has the wanted state returns `conflict`, for example `Minutes <id> is already archived.`.
  - A minutes id with a path part, or a decision id below 1, returns `invalid`. A minutes that does not exist for the product returns `not_found`.
  - A story that has a running session cannot be archived.
- The writes do not re-index. TypeScript refreshed the `documents` rows of the minutes after a mark. `knowledge.index` (U19) does it in the Rust program.

#### Stories (U23)

These additions are also part of protocol 1.

- `story.*` returns what `agentsi story` prints, from `packages/core/src/story*.ts`. A `Story` keeps the shape of the story JSON. The CLI prints a story with its keys in Postgres `jsonb` order, as the TypeScript CLI does.
- `story.config {product, run?}` returns the stories block of the registry with its defaults. It returns `invalid` when stories mode is off. With `run`, it first refuses an archived product. The CLI calls it before each subcommand, so the errors come in the TypeScript order.
- `story.get` also returns `markdown` (the view of `story show`), `actions` (the legal actions now), `log` and `archived`. `story.review` also returns `diff`, for the stories screen.
- `story.discard` is the discard of `story discard`. `story.changes {rewrite}`: with `rewrite` the PM rewrites the story (`agentsi story changes`). Without it only the note is recorded, as the changes action of the web does.
- `story.run` queues the session and starts it on a thread of the core. It returns at once. `story.exec` runs a queued session and returns when it ends (`agentsi story _exec`). There is no detached `_exec` process.
- The session records the pid of the core. `story.reject {force}` stops a session that runs in this core: it kills the child process group (claude, a test or a gate), waits up to 5 seconds for the session to end, and the session writes no failure. `story.run` refuses while that session still stops. Another pid counts only if `/proc/<pid>/cmdline` shows the `_exec` of this story, so a reused pid is never killed. When a core stops, it stops its sessions first, as "Meetings (U22)" says. Then `reject` works without `force`.
- Each method that writes sends `data.changed {product, kind: "stories"}`. A running session sends one after each step.
- The runner sandbox (`crates/agentsi-agency/src/sandbox.rs`) gives bubblewrap the arguments of `sandbox.ts` plus `--unshare-pid`, after `--unshare-net` (owner decision, 2026-11-07). A child that leaves the process group then ends with the sandbox. `sandbox_parity` compares both lists for the same worktree and mask, and allows this one difference.
- The runner is stricter than `story-run.ts` in four points, because git runs on the host:
  - The `.git` file of the worktree is read-only in the sandbox. `sandboxed` adds it to the read-only list, and `bwrap_args` stays equal to the TypeScript function except for `--unshare-pid`.
  - The escape check also runs after each test and gate command, not only after each Claude session.
  - The commits of the runner and the merge of `story.accept` run with `core.hooksPath=/dev/null`, so no hook of the repo runs.
  - `story.accept` and `story.reject` check that the story can be saved before they change git.
- Stories run only on Linux hosts. On another system `story.run` returns `invalid` with the text "Story sessions run only on a Linux host". macOS has no bubblewrap. The candidate is `sandbox-exec` with a profile: deny `network*`, allow `file-write*` only under the worktree and the temp dir, and deny `file-read*` of the masked paths. U18 tests it on the MacBook before stories run on macOS. Claude Code itself already uses Seatbelt for the Bash sandbox of the session on macOS.

#### Reflection and memory (U25)

These additions are also part of protocol 1. The rules are those of `reflect.ts`, `replay.ts`, `proposal.ts` and `reflection-decision.ts`, and ADR 0001.

- `reflect.run {product, memory_only?}` returns `{entry, candidates, warnings}`. `entry` is null when nothing was written. `warnings` holds the lines that TypeScript prints on stderr. It costs money.
  - The model calls run first and only read. One transaction then writes the memory, the reflection entry and the decision. A failed curation call or a cancel leaves no row.
  - The deltas apply on a fresh read in that transaction, so a fact that a meeting added during the calls stays.
  - A second `reflect.run` of a product returns `conflict` while the first runs. Two runs would apply the same evidence twice.
  - A core that stops cancels a running reflection, as it cancels a meeting, and the reflection writes nothing. A new `reflect.run` then returns `unavailable`.
- `reflect.revert {product, reflection}` returns `{entry}`. It applies the inverse of the memory changes on the items of today and appends a revert line, in one transaction. An unknown id returns `not_found`. A revert line, an entry that is reverted, and an entry that changed no memory return `conflict`.
- `reflect.cancel {product}` returns `{canceled}`. It stops the calls of the running reflection, which then writes nothing. `agentsi reflect` calls it on `Ctrl+C` and exits with code 130.
- `memory.list {product}` returns `{agents, reflections}`. `agents` has the roster first, then each other agent with memory. The items of an agent come newest first. `reflections` come newest first. A stored entry that does not fit the type is left out.
- `memory.retire {product, agent, id, why}` returns `{item}`. An unknown item returns `not_found`. An item that is retired already and an empty reason return `conflict`.
- Replay runs one agent, the synthesis and the check on each held-out case, one thread for each case. It does not call `run_meeting`, so it has no write path: no minutes, no memory, no usage, no log line and no snapshot. `reflect::run::tests::a_replay_writes_no_row` compares each table before and after.
- A proposal writes a file only for a global role, under `plugin/agents/` of the data root, and only after approval. `reflect::decision::write_role_file` refuses each other path. An overlay is a row.
- After each reflection entry and each retire, the core runs `knowledge.index` for the product and sends `data.changed {product, kind: "memory"}`. A proposal with a decision also sends `kind: "decision"`.

#### Meetings (U22)

These additions are also part of protocol 1. A meeting runs one `claude -p` for each role, so it costs money and takes minutes.

- `meeting.start {product, scope?, question, roles?, meeting?, frozen?}` returns `{meeting, minutes, usage, total_cost_usd}` at the end of the meeting. The rules are those of `runMeeting` today:
  - Each agent of `roles`, by default the roster, runs at the same time. Then the synthesis runs as the `pm` agent, then the check, and one repair when the check fails.
  - One transaction writes the rows of `minutes`, `priorities`, `decisions`, `usage`, `memory`, `meeting_log` and `snapshots`. Then the core updates the index and sends `data.changed {product, kind: "minutes"}`.
  - `usage` lists the runs in the order in which they ended.
- `meeting` is a name for the events, which the client picks. Without it, the core makes one. A name that runs already returns `conflict`.
- `frozen` is the id of stored minutes of the product. The meeting then reads their frozen snapshot, does no index before or after the meeting, and uses no code graph. The CLI has no flag for it. The check of the 23 snapshots uses it. Replay does not: it never calls `run_meeting` (U25).
- `meeting.ask {product, scope?, role, question, meeting?}` returns `{answer, usage, warnings}`. It asks one agent in plain text, with no code graph, and writes no minutes. `warnings` holds the lines that TypeScript prints on stderr.
- `meeting.cancel {meeting}` returns `{canceled}`. It kills the process groups of the runs. A meeting that gets the cancel before its write writes nothing, and its `meeting.start` returns `conflict`. `canceled` is false when no meeting of that name runs. `agentsi meet` and `agentsi ask` call it on `Ctrl+C`.
- `minutes.check {minutes, roles?}` returns `{passed, failures}`: the JSON of `agentsi check`. `minutes` is the JSON of the minutes. The check reads `git`, `gh` and the store, and writes nothing.
- Each run uses `--output-format stream-json --verbose`. Its last line is the envelope of `--output-format json`, which `claude.ts` reads.
- `meeting.event {meeting, role, line}` carries each stream-json line of each run. `role` is the agent id, `synthesis` or `synthesis-repair`. A line over 1 MiB stays out of the events.
- `meeting.progress {meeting, product, stage, phase, role?, seconds?, cost_usd?, input_tokens?, error?, note?}` tells the stages. `stage` is one of these:
  - `start`: `note` holds the question.
  - `index`, `role`, `synthesis` and `check`, with `phase` `start` or `end`, as `MeetingEvent` of today.
  - `warning`: `note` holds a line for stderr.
  - `done`: the last event. `note` holds the minutes id and `cost_usd` the total, or `error` holds the reason.
- A subscriber of the topic `meeting` gets the events of each meeting, also of one that another client started.
- A core that stops, on SIGTERM, SIGINT or `core.shutdown`, stops its running work before it exits:
  - It keeps the lock of the socket until the work has ended, also after it removed the socket. A second core exits with code 3 until then.
  - It cancels each meeting and each story session, and kills their process groups. A new `meeting.start` or `meeting.ask` returns `unavailable`.
  - It closes each client link when the stop starts. An idle link closes at once. A request in flight gets 1 second to answer, and the core stays up for that time, also when no meeting or session runs.
  - It waits up to 10 seconds for the meetings, the story sessions and the other work of requests, such as `knowledge.index`. A canceled meeting writes no minutes.
  - A story session records no failure. It keeps its last saved state and the pid of the core, its log gets the line `The story session of <id> stopped with the core.`, and `story.reject` works on it with no `force`.
  - When work still runs after 10 seconds, the core stops each of its child processes, then exits with code 1. Thus no process of the core writes after the lock is free. An open transaction rolls back. A step that runs `git`, `gh` or a reindex can hold a meeting or a session this long, because no cancel reaches it.
- The errors of these methods:
  - `not_found`: an unknown product, or a `frozen` id with no snapshot.
  - `invalid`: an agent that does not resolve, or a product with no folder.
  - `conflict`: an archived product in `meeting.start`, a name that runs, or a cancel. `meeting.ask` works on an archived product, as today.
  - `unavailable`: `agentsi.db` does not exist.
  - `internal`: a failed run of `meeting.ask`. A failed run of a role in a meeting is no error. Its answer holds the concern, as today.

#### Calls (U26)

These additions are also part of protocol 1. The rules and the audio format are in `agentsi-desktop-calls.md`.

- `call.get {product, minutes_id}` returns `{product, minutes_id, question, segments, timed, audio?, heard, render?}`. `segments` are `{idx, role, text, start, end}`. `audio` is `{format, bytes}`, with `format` `opus` or `webm`. `render` is `{render, done, of}` while a render runs in this core.
- `call.audio {product, minutes_id, from, count}` returns `{from, total, pcm}`. `pcm` is base64 of 16-bit little-endian samples, mono, at 24 kHz. The core returns at most 2 minutes. It decodes the file at each request, whatever its format.
- `call.render {product, minutes_id, render?}` returns `{render, out, duration, bytes, segments}` at the end. It takes minutes. The errors are `not_found` (unknown minutes), `invalid` (no script), `conflict` (this call renders already, a name that runs, or a cancel) and `unavailable` (no database, no Kokoro model, no `espeak-ng`).
- `call.cancel {render}` returns `{canceled}`. A canceled render writes nothing. `agentsi render` and `agentsi meet --audio` call it on `Ctrl+C`, through `meeting.cancel`, which finds a render by its name too.
- `call.heard {product, minutes_id, heard}` returns `{changed}`. A call with no audio file cannot be marked.
- `meeting.render {render, product, minutes_id, stage, done, of, error?, note?}` tells the steps `start`, `segment`, `encode` and `done`. A subscriber of the topic `meeting` gets it.
- A write sends `data.changed {product, kind: "call"}`. A core that stops cancels each render, as it does for meetings.

### Errors

Each error is a JSON-RPC error with a stable `data.kind`: `not_found`, `invalid`, `conflict`, `busy`, `unavailable` or `internal`. `data.hint` gives one sentence for a person.

### Schema and versions

- `agentsi-proto` makes `schema/core.openrpc.json` from the Rust types. A test fails when the file is not current.
- `schema/core.openrpc.json` is stable at M2, not at M1. During M1 it can change, and only the first test runs.
- At M2, `schema/released.openrpc.json` holds the schema of the last release. A test fails on a change that breaks a client of it:
  - a method, a parameter or a result field that goes away,
  - a new required parameter,
  - a parameter that accepts less,
  - a result field that changes its type.
- A break that the project wants gets protocol number 2, on purpose.

### SSH

- `agentsi-core connect` joins its stdin and stdout to the local socket. If no core runs, it starts one.
- The app keeps its hosts in `<config>/agentsi/hosts.toml`, with the last product list of each host.
- If `connect` fails or stops, the app shows each product of that host as "device off". Then it tries again after 1, 2, 4 and up to 60 seconds.
- Voice events of a remote core go to the app. The app sends them to the local core, and the local pill speaks them.

#### SSH hosts (U17)

These additions are also part of protocol 1:

- `hosts.toml` has one `[[host]]` table for each host:
  - `name` is the name in the sidebar. It is unique. It holds no `/` and no control character.
  - `ssh` is the SSH destination. The default is `name`.
  - `args` adds options to `ssh`. `command` replaces the remote command `agentsi-core connect`.
  - `products` is the product list of the last connection. The app writes it, and keeps the comments and the other keys.
- The app reads `hosts.toml` at start. A file that does not parse gives no hosts, and the settings screen shows the reason. An unknown key is not an error.
- The app runs `ssh -T -o BatchMode=yes -o ConnectTimeout=10 -o ServerAliveInterval=5 -o ServerAliveCountMax=3 <args> <ssh> <command>`. One end of a socket pair is its stdin and stdout.
- Each host has its own thread. A slow or dead host never blocks the window or the local panes.
- A host that gives no `core.hello` reply in 20 seconds is "device off". The app kills its `ssh` process.
- When the app quits, it kills the `ssh` process of each host, also an `ssh` that starts after the quit. Then the threads of the hosts end. An `ssh` whose remote command never ends does not stop at the end of its input.
- If `ssh` stops with a code other than 0, the reason of "device off" is the last line that it wrote to stderr. Otherwise the reason is the reason of the link. Thus an old warning of `ssh` never hides the real reason.
- After a connection that read the products and stayed up for 10 seconds, the waits start at 1 second again. A host that closes each connection at once still waits up to 60 seconds.
- Between two tries, the window draws nothing. It draws only when the state or the reason changes.
- `voice.relay {host, terminal, role?, text?}` returns `{}`. With `text`, the core sends `voice.speak {terminal: "<host>/<terminal>", role, text}` to its subscribers. Without `text`, it sends `voice.stop {terminal: "<host>/<terminal>"}`. `role` is `pm` by default.
- The pill reads the voice of the role from its own data root. Thus the voice of a role comes from the local device.
- The app relays only the events whose terminal holds no `/`. Thus two hosts that list each other do not loop.
- `voice.relay` of a terminal of this core returns `conflict`. Thus a host that is this device speaks each reply once. The settings screen tells it.
- Errors: `invalid` for a host that breaks the rule of `name`, and for an empty terminal or one with `/`. `conflict` as above.

#### Remote screens and panes (U17b)

These rules add no method. Each screen and pane uses the methods above on the core of its product's host.

- The app subscribes to `voice`, `terminal`, `data`, `meeting` and `desk` on each host. It relays a `voice.*` event to the local core. It keeps each other event with the state of that host.
- Each host has its own state: its link, its products and its terminals. A product id and a terminal id name something on one core only, so a local product and a product of a host can have the same id. A product of the app is the pair of host and id.
- A screen or a pane holds the state of its host and no other. It reads the link at each call. So a write of a screen reaches only the core of the product on that screen. The app never uses the local core in place of a host that `hosts.toml` does not list.
- A screen opened from a screen stays on the same host.
- Each core has its own meeting feed. A `meeting.*` event of a core reaches only the meeting screens of that core, so a local product and a product of a host with the same id share no meeting. When the link breaks, the running meetings of that host show a failure. The meeting can still run on the host.
- A palette write acts on the screen in view that the owner used last. A hidden tab is no target, because it can be on another host. With no such screen in view, the action does nothing.
- A tab of a terminal of a host starts with the name of the host.
- The links of the hosts write through a thread, so a stalled host never blocks the window. The link of this device writes at once, so its last frames leave before a quit.
- `data.changed` and `lagged` of a core reload the open screens of that host only. So does a write of a screen, and the return of the host. The settings screen belongs to this device and follows the local core.
- Before the first connection, each call of a host fails at once.
- While a host is not up, each of its screens shows "connecting" or "device off" with the reason, in place of its content. A call fails at once with the reason of the loss. Local screens and panes do not change.
- When the link breaks, each pane of the host keeps its last screen, takes no input and shows "device off". The terminal keeps running on the host while its core runs.
- When the host answers again, the app takes the new link, reads `terminal.list` and reloads the screens of the host. Each pane of the host attaches again. A pane whose terminal the core no longer knows shows "the terminal ended".
- A tab that the owner closes while the host is off ends its terminal when the host answers: the app sends `terminal.kill` for each listed terminal of a closed tab.
- A stream channel belongs to one link. A pane never sends a channel number on a later link.
- A new terminal of a product of a host starts in the folder of the product on that host. Only that core checks the folder. The program is `/bin/sh -c 'exec "${SHELL:-/bin/sh}"'`, because the `SHELL` of this device may not exist on the host. A product with no folder gives a notice and no terminal.
- `layout.json` saves `host` in each panel of a host and `active_host` next to `active_product`. A panel of a listed host stays at start, also when the host is off. A panel of a host that `hosts.toml` no longer lists goes, and the sidebar says so. A `hosts.toml` that does not parse drops no panel: those panels stay off.
- The keys `Ctrl+1` to `Ctrl+9` and the palette follow the order of the sidebar: the local products, then the products of each host.
- Limits:
  - The settings screen shows this device only.
  - The local pill cannot dictate into a pane of a host. `focus.report` of such a pane reaches the core of the host. That core does not reach the local pill.
  - `roles.apply` with `stage_dir` reads the stage folder on the host. The setup screen sends the stage as text in `stage`, so it works for a product on a host too. Its folder dialog opens on this device only.
  - The app keeps the closed tabs of a host in memory. After a quit while the host is off, the terminal of such a tab keeps running.

### The desk conversation

The desk conversation is the right column of the front desk. Its spec is `agentsi-desktop-conversation.md`. These additions are also part of protocol 1.

- The core keeps at most one desk session for each product: one `claude -p` process that plays the `pm` agent. The core stores the items in the tables `desk_sessions` and `desk_items` (`agentsi-desktop-data.md`).
- The first desk request (`desk.history`, `desk.auth` or `desk.send`) runs `claude auth status --json` with a limit of 5 s. The core never reads the credentials.

| Method | Params | Result | Errors |
|---|---|---|---|
| `desk.history` | `product, before?, limit?` (default 200) | `{session?, state, message?, items: [{seq, at, item}]}`, oldest first | `not_found`, `unavailable` |
| `desk.send` | `product, text` | `{seq}` of the owner item | `not_found`, `invalid`, `busy`, `unavailable` |
| `desk.stop` | `product` | `{stopped}`. False when no process ran | `not_found` |
| `desk.reset` | `product` | `{session}`: a new uuid. The items go | `not_found`, `unavailable` |
| `desk.show` | `session, view` | `{view_id}`: the `seq` of the `view` item | `not_found`, `invalid`, `unavailable` |
| `desk.auth` | `recheck?` | `{state, message?}` | none |
| `desk.check` | `product, seq, checked` | `{seq}`. Stores the checked items of a checklist view and a desk note | `not_found`, `invalid`, `unavailable` |
| `desk.page` | `product, seq` | `{title, html}` of a page view | `not_found`, `invalid`, `unavailable` |
| `decision.get` | `product, ids` | `{decisions: [{id, kind, minutes_id, text, why, action, status, archived}]}` in the order of `ids`. An unknown id is left out | `not_found`, `unavailable` |

- `item.kind` is `owner`, `reply`, `tool`, `view`, `note`, `end` or `error`. A field that the catalog does not know gives `invalid`.
- `view.kind` is `decisions`, `priorities`, `story`, `diff`, `table`, `chart`, `checklist`, `page` or `tickets`. `agentsi-desktop-conversation-views.md` gives the fields and the limits of each. A `view` item of a checklist also holds `checked`.
- `desk.item` and `desk.history` give a page with an empty `html` and its length in `html_len`. `desk.page` gives the HTML.
- `state` is `stopped`, `starting`, `idle`, `working`, `auth` or `failed`. `seq` counts from 1 for each product, and starts again after `desk.reset`.
- `desk.send` returns `busy` while a reply runs. It returns `unavailable`, sends `desk.state auth` and starts no process while the login is not `ready`.
- `desk.delta` carries one text delta of the reply that streams. The `reply` item that follows holds the whole text. One assistant message can give several `reply` items.
- `decision.get` only reads. A view of the desk conversation reads the decisions with it, and reads them again after `data.changed` of kind `decision` or `minutes`.
- Auth states: `ready`, `signed_out`, `missing`, `not_read` and `unknown`. `agentsi-desktop-conversation.md` lists their messages.
- With `AGENTSI_DESK_SESSION` set, `agentsi mcp` lists one `show_<kind>` tool for each variant of `View`. Each tool calls `desk.show`. The desk session also gets `list_tickets` and `get_ticket`. A desk session has no tool that writes product data.
- After `decision.decide`, `decision.add`, `mark.set` or `archive.*`, the core stores a desk note if the product has a desk session. The next `desk.send` carries the unsent notes before the owner text.

### Hook bridge

- In M1 the plugin gets 6 hook entries, one for each event: `SessionStart`, `UserPromptSubmit`, `Notification`, `PermissionRequest`, `PostToolUse` and `Stop`. Each entry runs `agentsi-core hook <event>` if `agentsi-core` is on the `PATH`.
- If `AGENTSI_TERMINAL` is not set, the bridge stops at once with exit code 0, unless a session outside the app has voice mode on (see "Voice mode outside the app"). Thus a Claude session outside the app is not changed.
- Otherwise the bridge reads the hook JSON from stdin and sends `hook.event` to `AGENTSI_CORE_SOCKET`.
- The bridge always stops with exit code 0, in 200 ms or less. It writes each error to `<data root>/logs/hook.log`.
- `plugin/hooks/voice-mode.mjs` does nothing when `AGENTSI_TERMINAL` is set. Thus a reply is not spoken two times. The file leaves the plugin at the switch-over, and the bridge takes over its work for a plain session.

## Acceptance criteria

- [ ] `printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"core.hello","params":{}}' | socat - UNIX-CONNECT:$XDG_RUNTIME_DIR/agentsi/core.sock` prints a result with `"protocol":1`.
- [ ] The same line through `ssh -T localhost agentsi-core connect` prints the same result.
- [ ] `stat -c %a` gives `700` for the socket folder and `600` for the socket.
- [ ] A connection from a second user account is closed before any reply.
- [ ] `cargo test -p agentsi-proto` passes. After a manual edit of `schema/core.openrpc.json`, it fails.
- [ ] M2: a test that removes a result field from a released method makes the compatibility test fail.
- [ ] A test pane prints 100 MB with no reader. The memory of the core grows by less than 4 MB.
- [ ] `echo '{}' | agentsi-core hook stop` with no `AGENTSI_TERMINAL` stops with exit code 0 in less than 50 ms.
- [ ] With the core stopped, `agentsi-core hook stop` in a pane stops with exit code 0 in less than 200 ms and writes one line to `hook.log`.
- [ ] Stop the SSH host's core. Within 2 seconds, the app shows its products as "device off". Start it again. Within 60 seconds, the products come back.
