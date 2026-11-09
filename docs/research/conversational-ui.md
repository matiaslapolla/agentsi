# A conversational front desk with native views in the app

**Status:** research · **Date:** 2026-11-09 · **Questions:** (1) How can the desktop app show a front desk conversation in which Claude shows components? (2) Which software design pattern fits, and which design is best? (3) Does this need another way to connect the owner's Claude subscription?

All claims come from primary sources: official docs, the source of the repos, and this repo. Access date for every link: 2026-11-09. The T3 Code facts come from commit `5067b88` of `pingdotgg/t3code` (2026-11-09). The json-render facts come from commit `1819412` of `vercel-labs/json-render` (2026-11-04), package version 0.22.0. "Unknown" marks a claim I could not prove on a primary page. The readings of the Anthropic terms are not legal advice.

## 1. Summary

1. **The subscription question: no new connection is necessary.** The core already starts the owner's installed `claude` binary, and that binary uses the owner's own login. Anthropic permits this. AgentSI must never offer a Claude login of its own or read the OAuth token. Section 6 gives the quotes.
2. **The app can draw the components itself.** `gpui-component` 0.9.0 already has the parts: chat messages, a virtual message list, streamed Markdown, custom Markdown blocks, a diff view, charts and a questionnaire. No web view is necessary.
3. **Recommended design: the hybrid of section 5.6.** Claude calls typed MCP tools on an MCP server of the core. The core checks each call and sends a typed view to the app. The app draws each view natively from core data, from a closed catalog.
4. **Pattern names.** The tool call is a Command. The core is the Mediator. Each card is a view model that observes core data (Observer, MVVM). The catalog is a closed Registry: one Rust `enum` and one `match`.
5. **The owner's approval stays outside Claude.** A button in a view calls an existing core method, such as `decision.decide`, after a Confirm. Claude can only propose.
6. **First slice:** a desk session that shows the pending decisions as cards, and the owner approves one with a click. Section 7 gives the checks.

## 2. Terms

These terms are new. They are not in `CONTEXT.md`.

- **Desk session:** one long-lived `claude -p` process that the core keeps for the front desk of one product. The owner sends turns to it from the app.
- **View:** one native component that the app draws inside the front desk conversation, for example a decision card or a table.
- **Card:** a view that shows one domain object from core data: a decision, a priority, a story, a minutes or a diff.
- **Catalog:** the closed list of view kinds. Each kind has a props schema and a list of actions.
- **Permission prompt:** the request of Claude Code for approval before a tool call. It is not the AgentSI prompt of `CONTEXT.md`.

## 3. What other products do

### 3.1 json-render (Vercel Labs)

json-render is "The Generative UI framework". The model produces JSON, and a renderer draws only components from a catalog that the app defines. License: Apache-2.0. Sources: https://github.com/vercel-labs/json-render and `packages/core/README.md` in that repo.

- **Catalog.** `defineCatalog(schema, { components, actions })`. Each component has a zod `props` schema and a `description`. Each action has a `description`.
- **System prompt.** `catalog.prompt()` writes the system prompt from the catalog.
- **Spec.** The model output is a flat tree: a `root` key and an `elements` map. Each element has `type`, `props` and `children` (a list of element keys).
- **Streaming.** "SpecStream" is JSONL. Each line is one RFC 6902 JSON Patch operation. `createSpecStreamCompiler()` applies each line and gives a partial spec, so the UI fills in while the model writes.
- **Text and UI in one stream.** In "fence mode", the patch lines sit between a ` ```spec ` line and a ` ``` ` line, inside normal text (`packages/core/src/types.ts`, `SPEC_FENCE_OPEN`).
- **Rendering.** `defineRegistry(catalog, { components })` maps each type to a React component. `<Renderer spec registry />` draws the spec. Renderers exist for React, Vue, Svelte, Solid, React Native, Ink and others. No renderer exists for Rust or GPUI.
- **Actions.** An element has an `on` field. A binding names a catalog action, its `params`, and an optional `confirm` object (`title`, `message`, `confirmLabel`, `variant`) (`packages/core/src/actions.ts`). State bindings use `{"$state": "/path"}`.
- **Validation.** `validateSpec()` and `autoFixSpec()` check and repair a spec.
- **MCP.** `@json-render/mcp` serves a catalog as an MCP App. An MCP App is HTML in an iframe, so this part needs a web view.

What AgentSI can take: the closed catalog, the flat tree, JSON Patch for streaming, and the `confirm` field on an action. What AgentSI cannot take: the code, because it is TypeScript and React.

### 3.2 T3 Code (pingdotgg/t3code)

T3 Code is an "agent harness control surface" that controls the coding agents on the owner's own machine. License: MIT. Source: https://github.com/pingdotgg/t3code.

- **Stack.** A local server (`apps/server`), a web app, mobile apps, and an Electron desktop app (`apps/desktop/package.json` depends on `electron` 44.10.1).
- **How it drives Claude.** The server uses `@anthropic-ai/claude-agent-sdk` (`apps/server/package.json`). It calls `query()` with `pathToClaudeCodeExecutable` set to the installed binary (`apps/server/src/provider/ClaudeProvider.ts`). It sets `includePartialMessages: true`, `resume`, `canUseTool` and `mcpServers` (`apps/server/src/orchestration-v2/Adapters/ClaudeAdapterV2.ts`).
- **How it drives Codex.** It starts `codex app-server` (`apps/server/src/provider/codexLaunchArgs.ts`, package `effect-codex-app-server`).
- **Subscription.** The README says that T3 Code works with existing subscriptions: "If they're set up on your computer, T3 Code can control them." The owner runs `claude auth login` first. The Claude guide says: "T3 Code uses Claude Code's login and configuration" (`docs/user/providers-claude.md`). T3 Code has no Claude login of its own.
- **Tool calls.** The server converts the events of each agent into common item types: `command_execution`, `file_change`, `mcp_tool_call`, `dynamic_tool_call`, `collab_agent_tool_call`, `web_search`, `image_view` (`packages/contracts/src/providerRuntime.ts`). The web app has one view for each type, plus a diff panel (`apps/web/src/components/DiffPanel.tsx`).
- **Approvals.** The thread shows each approval request, and the owner approves or rejects it there (`docs/user/permission-modes.md`).
- **Visual replies.** T3 Code adds its own MCP server, `t3-code`, of type `http`, to each Claude session (`ClaudeAdapterV2.ts`). That server has the tools `html_preview` and `html_render` (`apps/server/src/mcp/toolkits/html/tools.ts`). `html_render` takes a full HTML page. The thread shows the page in a sandboxed frame "above its written reply" (`docs/user/html-renders.md`).
- **MCP Apps.** T3 Code draws MCP Apps in place of the tool call. "T3 Code asks first unless the server marks the tool as read-only, and it asks before every message" (`docs/user/html-renders.md`).

What AgentSI can take: typed items with one native view each, an MCP server of its own for the session, and approvals inside the conversation. What AgentSI cannot take as is: the HTML frame, because it needs a web view.

### 3.3 Claude Code on the web

Source: https://code.claude.com/docs/en/claude-code-on-the-web

- A **Changes** card beside the conversation shows the lines added and removed. A click opens the diff view. The owner can comment on lines and "send the comments to Claude with your next message".
- A plugin suggestion is a card in the conversation: "A card in the conversation that lists one or more plugins, with an **Add** button, means Claude is suggesting plugins for your task."
- Both are typed cards that the product draws. Claude proposes, and a person clicks. The page does not describe HTML artifacts for cloud sessions.

### 3.4 Codex web and the Codex app server

- Codex cloud: "Inspect changed files and check results, request follow-up changes, and commit or open a pull request when you're ready." Source: https://learn.chatgpt.com/docs/cloud (redirect from developers.openai.com/codex/cloud). The page gives no more detail about artifacts. Unknown: how Codex web draws other components.
- The Codex app server is JSON-RPC 2.0 over stdio JSONL. A thread has turns, and a turn has items. The item types include `agentMessage`, `plan`, `reasoning`, `commandExecution`, `fileChange`, `mcpToolCall`, `webSearch` and `imageView`. Source: https://learn.chatgpt.com/docs/app-server
- The server asks the client for approval with `item/commandExecution/requestApproval` and `item/fileChange/requestApproval`. The answers are `accept`, `acceptForSession`, `decline` and `cancel`. Same source.
- The pattern is the same as T3 Code: the client gets typed items and draws one view for each type.

### 3.5 MCP Apps

An MCP tool can name a `ui://` resource in `_meta.ui.resourceUri`. The host fetches an HTML page and draws it in a sandboxed iframe. The page talks to the host with JSON-RPC over `postMessage`. Source: https://modelcontextprotocol.io/docs/extensions/apps

Claude Code treats these resources as "pages for a host application to render rather than content for Claude to read". Source: https://code.claude.com/docs/en/mcp. Thus a host such as T3 Code must draw them. The `claude` CLI in `-p` mode does not draw them.

## 4. What AgentSI has today

### 4.1 How the core runs Claude

- `crates/agentsi-agency/src/story/claude.rs`, `call_args`: each call is `claude -p` with `--system-prompt`, `--output-format json`, `--no-session-persistence`, `--restricted`, `--strict-mcp-config`, `--permission-mode dontAsk`, `--permission-prompts none`, `--max-budget-usd`, `--tools` and `--allowedTools`. A call with MCP adds `--mcp-config`.
- `crates/agentsi-agency/src/meeting/claude.rs`, `stream_args`: a meeting changes the format to `stream-json` and adds `--verbose`. The prompt goes in on stdin. Each call is one turn, and the process then stops.
- The core sends each stream line to the app as `meeting.event {meeting, role, line}` (`docs/specs/agentsi-desktop-protocol.md`, Events).
- `crates/agentsi-cli/src/mcp.rs` is the AgentSI MCP server. It serves `list_minutes`, `pending_decisions`, `role_memory`, `search` and `get_doc`.

### 4.2 How the meeting screen draws the stream

`crates/agentsi-app/src/screens/meeting.rs`, `pane_lines`:

1. The app reads one `meeting.event` line as JSON.
2. If `type` is not `assistant`, the line shows nothing.
3. A `text` block becomes one plain line for each line of text.
4. A `tool_use` block becomes one line, such as `Read: docs/a.md`.
5. The `StructuredOutput` tool becomes the answer text and the numbered titles of the priorities.
6. Each pane keeps its last 400 lines (`PANE_LINES`) as plain text in a `div`.

Thus the meeting screen already reads raw stream JSON in the app. It draws no Markdown, no cards and no actions.

### 4.3 What `gpui-component` 0.9.0 gives

Source: `~/.cargo/registry/src/*/gpui-component-0.9.0` and `gpui-base-0.9.0`, through `gpui-kit` 0.9.0.

| Need | Part | Notes |
|---|---|---|
| Conversation layout | `message.rs` (`MessageGroup`, `Message`, `MessageHeader`, `MessageContent`), `bubble.rs` | Alignment and grouping by sender. |
| Long conversation | `message_scroller.rs` (`MessageScroller`) | Virtual list. The caller keeps the data. |
| Streamed Markdown | `TextViewState::push_str`, `stream_fade.rs` | Appends text and fades in the new glyphs. |
| Custom blocks in Markdown | `TextView::markdown_block_parser`, `markdown_block_renderer`, `TextViewPlugin` | A block parser makes a `MarkdownNode`. A renderer draws it by name. |
| MDX | `TextView::markdown_mdx` | Parses JSX-like tags. Turns off raw HTML. |
| Diff | `diff` module (`Diff`, `DiffFile`, `DiffState`) | The git screen already uses it (`screens/git.rs`). |
| Charts | `chart` (bar, line, area, pie, radar, sankey, candlestick), `plot` | |
| Questions | `questionnaire` | Answers, checks, navigation and focus. |
| Other | `table`, `list`, `tag`, `badge`, `alert`, `description_list`, `stepper`, `dialog`, `attachment`, `skeleton` | |
| Web view | none | No `wry` and no WebKit in the crate or in `Cargo.lock`. |

The guide screen already draws Markdown with `TextView` (`screens/guide.rs`).

### 4.4 The `claude` CLI options for a long-lived desk session

Sources: https://code.claude.com/docs/en/cli-reference and https://code.claude.com/docs/en/headless

| Option | What it does |
|---|---|
| `--input-format stream-json` | "Specify input format for print mode (options: `text`, `stream-json`)". The process stays open and reads user messages on stdin. |
| `--output-format stream-json --verbose` | One JSON event for each line. The last line of a turn is a `result` message. |
| `--include-partial-messages` | Adds `stream_event` lines with token deltas. "Requires `--print` and `--output-format stream-json`". |
| `--replay-user-messages` | Writes each stdin user message back on stdout, as an acknowledgment. |
| `--session-id <uuid>` | Gives the session a fixed id. |
| `--resume <id>` | Continues a saved session. `--no-session-persistence` turns saving off, so a desk session must not use it. |
| `--mcp-config` with `--strict-mcp-config` | Loads only the MCP servers that AgentSI gives. With `-p`, the CLI waits for them before the first turn, up to 30 s. |
| `--permission-prompt-tool mcp__<server>__<tool>` | Sends each permission prompt to an MCP tool. The tool answers allow or deny. |
| `--permission-mode` | `default`, `acceptEdits`, `plan`, `auto`, `dontAsk` or `bypassPermissions`. |

A user message on stdin has this shape (https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode):

```json
{"type":"user","message":{"role":"user","content":"What should we focus on?"},"parent_tool_use_id":null}
```

Two limits matter:

- The CLI records the system prompt at the first request. "If you pass different system prompt flag text, or none, on that later launch, it takes effect once the conversation is compacted or when you start a new conversation" (cli-reference). Thus a catalog change reaches an old desk session only after compaction.
- A tool marked `_meta["anthropic/requiresUserInteraction"]` cannot be approved through `--permission-prompt-tool`. The CLI changes an `allow` to a deny (https://code.claude.com/docs/en/mcp, "Require approval for a specific tool").

The Agent SDK has a `canUseTool` callback, but the SDK exists only for Python and TypeScript. The SDK talks to the binary through a control protocol on stdio. Unknown: a public spec of that protocol. The CLI route above uses only documented flags.

## 5. Candidates

Each candidate needs the same base first. The base is not repeated in each section.

**The base (cost M, about 1 to 2 weeks):**

1. The core starts one desk session for each product with the options of section 4.4.
2. The core keeps the session id in SQLite and resumes the session after a restart.
3. New protocol entries: `desk.session.start`, `desk.session.send`, `desk.session.stop`, and the event `desk.session.event {session, line}`.
4. A new pane of the front desk screen draws the conversation with `MessageScroller`, `Message` and a streamed `TextView`.
5. Tool calls of Claude Code (Read, Grep, Edit) show as one-line rows, as in the meeting screen today.

### 5.1 A: A component catalog that the app interprets (json-render in Rust)

**Pattern:** Interpreter over a view tree, with a Registry (an Abstract Factory) of native views. This is server-driven UI.

**How it works:**

1. AgentSI defines the catalog in Rust: one `enum` with `serde` and `schemars`. Each variant has its props and its actions.
2. The core writes the catalog into the system prompt, as `catalog.prompt()` does.
3. Claude writes a ` ```spec ` fence with JSON Patch lines, or calls one tool `render {spec}`.
4. The core applies each patch line to a flat tree and checks it against the catalog.
5. The core sends `desk.view {session, view_id, tree}` to the app.
6. The app walks the tree. A `match` on each element type builds one GPUI element.

**What Claude emits:**

```json
{"op":"add","path":"/root","value":"t1"}
{"op":"add","path":"/elements/t1","value":{"type":"Table","props":{"columns":["Priority","Owner"]},"children":["r1"]}}
{"op":"add","path":"/elements/r1","value":{"type":"Row","props":{"cells":["Ship U31","dev"]},"children":[]}}
```

**Streaming:** good. Each patch line adds one element, so the table grows while Claude writes.

**Owner approval:** a button names a catalog action, for example `decide`. The app maps the action name to a core method. A write action always opens a Confirm. The core checks the parameters again before it writes.

**Cost:** L, about 2 to 3 weeks after the base. Patch apply, tree checks, the prompt text, about 10 views, the action map and tests.

**ADR 0004:** fits. All views are GPUI.

**ADR 0007:** fits when the core applies and checks the patches. Each new view kind needs a schema entry.

**Risks:**

- Claude can build a large free-form tree. The model must learn the catalog from the prompt alone.
- Free-form props can copy domain data into the view. A copy can be out of date or wrong, for example a decision text that differs from the stored text.
- A fence in text gets no check that Claude sees. A bad tree fails in silence unless the core sends an error back in the next turn.

### 5.2 B: One typed MCP tool for each component

**Pattern:** Command. Each tool call is a command object with typed parameters. The app has one handler for each command name.

**How it works:**

1. The MCP server of the core lists one tool for each view kind: `show_table`, `show_chart`, `show_checklist`, `show_diff`.
2. Each tool has a JSON Schema for its input.
3. Claude calls the tool. The MCP server checks the input.
4. If the input is bad, the server returns an error result. Claude reads the error and calls again.
5. If the input is good, the core sends `desk.view` to the app and returns "shown" to Claude.
6. The app draws the view from the tool input.

**What Claude emits:**

```json
{"type":"tool_use","name":"mcp__agentsi__show_chart","input":{"kind":"bar","title":"Cost per meeting","labels":["M1","M2"],"values":[0.31,0.29]}}
```

**Streaming:** medium. The view shows when the tool call is complete. With `--include-partial-messages`, the input arrives as JSON deltas, and the app can show a `skeleton` until the call ends.

**Owner approval:** a view tool only shows. A button in the view calls a core method after a Confirm. A tool that acts (for example `run_story`) must not exist, or it must go through `--permission-prompt-tool` to a Confirm in the app.

**Cost:** M, about 1 to 2 weeks after the base. The MCP server already exists, and each tool is small.

**ADR 0004:** fits.

**ADR 0007:** fits well. The core owns the MCP server, the check and the event. A CLI client or a remote host gets the same events.

**Risks:**

- Each tool description costs context tokens in each turn. About 10 tools is a reasonable limit.
- The view shows after the call, not during it.
- The props still carry copies of domain data, as in A.

### 5.3 C: Markdown with fenced directive blocks

**Pattern:** a Parser with one Strategy for each block name. The `TextViewPlugin` of `gpui-component` is the plug point.

**How it works:**

1. The system prompt tells Claude the block names, for example ` ```agentsi-decision `.
2. Claude writes the block in its normal text.
3. The app streams the text into `TextViewState::push_str`.
4. A `markdown_block_parser` finds the fenced block and parses its body into a `MarkdownNode`.
5. A `markdown_block_renderer` for that name draws the native view.
6. A block that does not parse shows as a plain code block.

**What Claude emits:**

````markdown
Two decisions wait for you.

```agentsi-decision
id: 14
```
````

**Streaming:** good for the text. A block draws when its fence closes. Before that, it shows as code.

**Owner approval:** as in A. A button in the block calls a core method after a Confirm.

**Cost:** S to M, about 3 to 5 days after the base. `gpui-component` already has the hooks.

**ADR 0004:** fits.

**ADR 0007:** weak. The app parses model text. The core does not see the blocks, so it cannot check them, log them or send them to other clients.

**Risks:**

- Nobody tells Claude that a block is wrong. The owner sees a code block, and Claude does not learn.
- Claude can write a block in a quoted example by mistake.
- Text that Claude copies into a block can differ from the stored data.

### 5.4 D: Domain cards by reference

**Pattern:** Observer with MVVM. Claude sends only a reference. The view model reads the object from the core and observes `data.changed`. The card is a Proxy of the stored object.

**How it works:**

1. The MCP server lists tools that take ids: `show_decisions {ids}`, `show_priorities {minutes_id}`, `show_story {id}`, `show_diff {story_id}`.
2. Claude calls a tool with ids that it found with `pending_decisions` or `search`.
3. The core checks that each id exists for the product. A bad id returns an error to Claude.
4. The core sends `desk.view {session, view_id, kind, ids}` to the app.
5. The app makes one view model for each card. The view model reads the object with an existing method, such as `mark.pending` or `story.get`.
6. When `data.changed` names that product and kind, the view model reads again, and the card changes.

**What Claude emits:**

```json
{"type":"tool_use","name":"mcp__agentsi__show_decisions","input":{"ids":[14,15]}}
```

**Streaming:** the call is small, so the card shows almost at once. The data comes from the core, not from tokens.

**Owner approval:** the card has the buttons of the desk screen today: Approve, Reject, Archive. Each button calls `decision.decide` or `archive.archive`. The context menu rules about a Confirm apply. Claude has no tool that writes.

**Cost:** M, about 1 week after the base for decision, priority and story cards. The desk, story and git screens already draw these objects.

**ADR 0004:** fits.

**ADR 0007:** fits best. The core owns the data and the events. A card on a remote host reads the same methods.

**Risks:**

- Claude can show only what has a card. A free table or a chart needs another path.
- An id that Claude invents fails. The error result makes Claude search again.

### 5.5 E: An embedded web view for HTML artifacts

**Pattern:** a sandboxed Plugin host, the micro-frontend pattern. This is the T3 Code `html_render` path and the MCP Apps path.

**How it works:**

1. The MCP server lists `html_render {html, title, height}`, as T3 Code does.
2. Claude writes a full HTML page.
3. The core stores the page and sends `desk.view` with its id.
4. The app opens a web view and loads the page.
5. A bridge on `postMessage` takes action requests from the page and asks the owner.

**What Claude emits:** a full HTML page, up to 512,000 characters in T3 Code.

**Streaming:** none. The page arrives whole.

**Owner approval:** the bridge must ask before each action, as T3 Code does. The page is code from the model, so the bridge is the only guard.

**Cost:** XL, 3 weeks or more. `wry` needs WebKitGTK on Linux. The `wry` README says child web views work on "Linux (X11 Only)". For Wayland, it asks for a GTK container (`build_gtk`, `gtk::Fixed`). GPUI does not draw with GTK. Source: https://github.com/tauri-apps/wry

**ADR 0004:** against it. The ADR rejected Tauri and Electron "because a web view adds a second UI stack".

**ADR 0007:** fits in the core. The problem is in the app.

**Risks:**

- A second UI stack, a new system dependency, and a Wayland gap on the owner's Hyprland desktop.
- Model code runs in the app process tree.
- Theme and text size do not follow the appearance choices.

**A cheap variant (E2):** the core writes the page to a file, and a card has an "Open in browser" button. Cost: S, 1 to 2 days. No web view enters the app. ADR 0004 holds.

### 5.6 F: The hybrid (recommended)

**Pattern:** Command for the transport (B), Mediator in the core, Observer with MVVM for the cards (D), and a small closed Registry of generic views (A without the free tree). The Rust `enum` is the Registry. The compiler checks that each kind has a view.

**How it works:**

1. The MCP server of the core lists two groups of tools.
   - Card tools take ids (D): `show_decisions`, `show_priorities`, `show_story`, `show_diff`.
   - View tools take small typed data (B): `show_table`, `show_chart`, `show_checklist`.
2. Each tool input is a variant of one Rust `enum View`. The core checks the input with `serde` and returns any error to Claude.
3. The core sends `desk.view {session, view_id, view}` to the app and stores it with the turn.
4. The app draws each view with one `match`. Each card is a view model that observes `data.changed`.
5. Buttons call existing core methods after a Confirm. Claude gets no tool that writes product data.
6. After an owner action, the app adds one short note to the next turn, for example "The owner approved decision 14". T3 Code does the same for MCP Apps.
7. HTML pages use E2: a file and an "Open in browser" button.

```rust
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum View {
    Decisions { ids: Vec<i64> },
    Priorities { minutes_id: String },
    Story { id: String },
    Table { columns: Vec<String>, rows: Vec<Vec<String>> },
    Chart { chart: ChartKind, title: String, labels: Vec<String>, values: Vec<f64> },
}
```

**What Claude emits:** a tool call with a small input, as in B and D.

**Streaming:** text streams with `push_str`. A view shows when its call ends. A view shows a `skeleton` while the input streams.

**Owner approval:** as in D. The owner's click on a core method is the only path to a change.

**Cost:** M for the first slice after the base. Each new card or view is S.

**ADR 0004:** fits. All views are GPUI.

**ADR 0007:** fits. Each view kind is one schema entry, and the core is the only reader of the stream.

**Risks:**

- Claude may not call the tools when it should. The system prompt and the tool descriptions must say when.
- Ten tools or more cost context in each turn.
- The base work (a desk session) is new code in the core, with its own failure modes: a hung process, a lost session id.

### 5.7 Comparison

| | A: catalog | B: tool per view | C: Markdown blocks | D: domain cards | E: web view | F: hybrid |
|---|---|---|---|---|---|---|
| Pattern | Interpreter, Registry, server-driven UI | Command | Parser, Strategy, Plugin | Observer, MVVM, Proxy | Plugin host, micro-frontend | Command, Mediator, Observer, closed Registry |
| What Claude emits | JSON Patch lines in a fence or a tool | One tool call with typed props | A fenced block in text | One tool call with ids | A full HTML page | One tool call with ids or small typed data |
| Who checks | Core, if it applies the patches | MCP server in the core | Nobody | MCP server in the core | Nobody | MCP server in the core |
| Claude learns of an error | Only if the core sends it in the next turn | Yes, in the tool result | No | Yes, in the tool result | No | Yes, in the tool result |
| Streaming | Element by element | At call end, skeleton before | Text yes, block at fence end | At once (small call) | Whole page | Text yes, view at call end |
| Data source | Copied by Claude | Copied by Claude | Copied by Claude | Core data, live | Copied by Claude | Core data for cards, Claude for tables and charts |
| Owner approval | Action map and Confirm | Button and Confirm | Button and Confirm | Existing buttons and Confirm | Bridge must ask | Existing buttons and Confirm |
| Cost after the base | L (2 to 3 weeks) | M (1 to 2 weeks) | S to M (3 to 5 days) | M (1 week) | XL (3 weeks or more), E2: S | M first slice, S per view |
| ADR 0004 | Fits | Fits | Fits | Fits | Against (E2 fits) | Fits |
| ADR 0007 | Fits if the core checks | Fits well | Weak | Fits best | Fits in the core | Fits |
| Main risk | Free trees, stale copies | Context cost, no live data | Silent failures | Only known objects | Second UI stack, Wayland | Claude does not call the tools |

## 6. The subscription question

**Answer: no.** AgentSI needs no new way to connect the owner's subscription. The core already starts the owner's own `claude` binary, and the binary signs in with the owner's own login. A long-lived desk session uses the same binary with other flags.

The rules, from https://code.claude.com/docs/en/legal-and-compliance ("Authentication and credential use"):

> "Anthropic does not permit third-party developers to offer Claude.ai login into their own applications, or to route requests through Free, Pro, Max, or Ultra plan credentials on behalf of their users. Moreover, developers may not collect, store, or intermediate Claude.ai credentials or session tokens — sign-in to a Claude account must complete through Anthropic's own flow."

> "Nor does it prevent an end user from signing in to the unmodified Claude Code binary with their own Claude subscription, including where a platform hosts Claude Code as described under *Can customers offer Claude Code in their products?* above."

> "Advertised usage limits for Pro, Max, and Ultra plans assume ordinary, individual usage of Claude Code and the Agent SDK."

From https://code.claude.com/docs/en/agent-sdk/overview:

> "Unless previously approved, Anthropic does not allow third party developers to offer claude.ai login or rate limits for their products, including agents built on the Claude Agent SDK. Use the API key authentication methods described in the Quickstart instead."

What this means for AgentSI:

- **Permitted:** the core starts the installed, unmodified `claude` binary. The owner signed in to it with `claude auth login`. T3 Code works the same way.
- **Not permitted:** a "Sign in with Claude" button in AgentSI. Reading `~/.claude/.credentials.json` and calling the API with that token. Running Claude for other people on the owner's plan.
- **When another approach is necessary:** if AgentSI calls the Claude API directly, or if a hosted AgentSI runs Claude for other people. Then each person brings an API key from the Claude Console. A hosted product must also follow "Can customers offer Claude Code in their products?" on the same page.

**A risk to watch.** The headless page says: "`--bare` is the recommended mode for scripted and SDK calls, and will become the default for `-p` in a future release." It also says: "In bare mode, Claude Code never reads OAuth credentials or the system keychain." Source: https://code.claude.com/docs/en/headless. Unknown: whether a default bare mode will keep the subscription login for `-p`. If not, each `-p` call of AgentSI and of the Agent SDK can lose the subscription. The core should check the auth state at start and show a clear message. The installed version today is 2.1.319.

## 7. Recommendation and first slice

**Recommendation: F, the hybrid.** The product rule is "nothing changes without the owner's explicit word". In F, every change goes through a core method that the owner starts with a click. Claude has no write tool, and a card shows stored data, not a copy that Claude wrote. The core checks each tool call and returns errors to Claude, so the design repairs itself. All views are GPUI, so ADR 0004 holds. The core owns the stream and the views, so ADR 0007 holds.

C is cheaper, but its failures are silent and the core cannot see the blocks. A gives more freedom, but it costs more, and the free tree copies domain data. E breaks ADR 0004 and has a Wayland gap.

**First slice: approve a pending decision from the conversation.**

1. The core starts a desk session for one product with `--input-format stream-json`, `--output-format stream-json`, `--verbose`, `--include-partial-messages`, `--session-id`, `--mcp-config` and `--strict-mcp-config`.
2. The desk session runs read-only: `--permission-mode dontAsk` and the tools `Read`, `Grep`, `Glob` and the AgentSI MCP tools.
3. The MCP server of the core adds one tool: `show_decisions {ids}`.
4. The front desk screen gets a conversation pane with streamed Markdown and a decision card.
5. The card has Approve and Reject. Each button opens a Confirm and calls `decision.decide`.
6. The next turn carries a note of the owner's verdict.

**Checks for the slice:**

- [ ] With one pending decision, the owner asks "What waits for me?". The conversation shows a decision card with the stored text, `why` and `action`.
- [ ] A click on Approve and a Confirm sets the decision to `approved` in SQLite. The card shows the new status with no new turn.
- [ ] When Claude calls `show_decisions` with an id that does not exist, the tool result is an error, and no card shows.
- [ ] A second message goes to the same `claude` process. The process count does not grow.
- [ ] After a restart of the core, the desk session resumes with the same session id, and Claude remembers the last turn.
- [ ] The app crate gets no web view dependency. `Cargo.lock` has no `wry` and no `webkit2gtk`.

## 8. Open questions for the owner

1. Does the desk conversation replace the Claude Code terminal pane of `/agentsi`, or does it live beside it?
2. Can Claude record a pending decision itself with `decision.add`, or only show a draft that the owner adds with a click?
3. Is the desk session read-only, or can it edit files? If it can edit, each permission prompt goes to a Confirm in the app through `--permission-prompt-tool`.
4. Are HTML pages (charts, mockups) wanted? If yes, is "Open in browser" (E2) enough?
5. Should the meeting screen move to the same views later, so that the minutes show as priority cards?

## 9. Open gaps

- I did not run a `claude` process with `--input-format stream-json` on this machine. The flags come from the docs.
- The Codex web docs give no detail about how Codex web draws artifacts.
- The control protocol between the Agent SDK and the binary has no public spec that I found.
- I did not measure the context cost of 10 MCP tool descriptions.
- The cost figures are estimates for one developer. They are not measured.

## 10. Owner decisions (2026-11-09)

- Design: F, the hybrid.
- Question 1: the desk conversation and the Claude Code pane of `/agentsi` live together and stay in sync. Both read and change the same core data, so an action in one shows in the other.
- The tracker term is **Ticket**. In this doc, "card" names a view of a stored object. It is not a domain term.
