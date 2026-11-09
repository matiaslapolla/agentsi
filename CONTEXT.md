# AgentSI

AgentSI is an agency of role agents that the owner opens inside any of their products from Claude Code. The roles read the product, meet, and give the owner priorities and decisions to approve. Nothing acts without the owner's approval.

## Language

### The agency

**Owner**:
The person who runs the products, approves the work of the agency, and gets the minutes and the call. The `owner` block of the registry gives their name and the language of the prompts.
_Avoid_: User, admin

**Product**:
One front that the owner runs and that AgentSI knows by its paths in the registry. Examples: a web app, a mobile app, a landing page, AgentSI itself.
_Avoid_: Project, client, repo

**Registry**:
The list of products and the paths that belong to each one, plus the owner block.
_Avoid_: Config, catalogue

**Instance**:
The agency bound to one product for one Claude Code session.
_Avoid_: Agency session, run

**Front desk**:
The conversation that `/agentsi` opens, held by the PM, from which the owner asks, convenes a meeting or consults one role.
_Avoid_: Lobby, chat, home

**Desk session**:
The one `claude -p` process of the core for the front desk of one product, with its saved session id. It plays the PM and reads only. It has no tool that writes product data.
_Avoid_: Chat session, bot

**Desk conversation**:
The stored items of one desk session, and the right column of the front desk screen that draws them.
_Avoid_: Chat, thread, front desk (when you mean the column)

**Owner message**:
One text that the owner sends to a desk session. It is not a turn of conversation mode.
_Avoid_: Prompt, turn

**Reply**:
What Claude writes for one owner message, up to its `result` line. It streams as Markdown.
_Avoid_: Answer, response

**View**:
One native component in the desk conversation. The closed `enum View` in `agentsi-proto` is the catalog. Claude shows a view with a `show_<kind>` tool of the MCP server.
_Avoid_: Widget, card (when you mean the catalog entry), generated UI

**Object view**:
A view that names stored objects by id and reads them from the core. It shows no text that Claude copied. The decisions view is one.
_Avoid_: Rich message

**Desk note**:
One line about a write of product data, which the core adds to the next owner message. An example is "The owner approved decision 14." The pane `/agentsi` and the app both cause desk notes.
_Avoid_: Sync message, system message

**Auth state**:
The result of the login check of `claude` (`claude auth status --json`): `ready`, `signed_out`, `missing`, `not_read` or `unknown`. The desk conversation shows the message of a state that is not `ready` in place of the input, with a "Check again" button.
_Avoid_: Login status, session expired

**Role**:
A fixed perspective with its own persona: PM, social, dev, QA, scout, or a custom role of one product such as SEO. A role lasts; the session that plays it does not.
_Avoid_: Bot, teammate, persona

**Agent**:
One seat of a role in a product's roster. It answers in meetings and has its own memory and its own credit for what it proposed.
_Avoid_: Bot, worker, role (when you mean the seat)

**Specialist**:
An agent with a focus inside its role, such as the backend dev and the mobile dev of the same product. Each specialist speaks with its own voice.
_Avoid_: Sub-role, clone

**Roster**:
The agents that attend the meetings of one product.
_Avoid_: Team, active roles

**Persona**:
The style and criteria of a role, the same in every product.
_Avoid_: Identity, character

**Overlay**:
What one product adds to or changes in a role for one of its agents: focus, sources, model. The role's persona stays underneath.
_Avoid_: Fork, custom prompt

**Brief**:
The product as the owner defined it in setup: what it is, for whom, its stage, its objectives, its north-star docs and what is out of scope.
_Avoid_: Profile, product config

**Setup screen**:
The screen of the app that registers a product or updates its brief in four steps: Folder, Forms, Review and Done. It writes only after the owner's Confirm, through `roles.apply`, `registry.update`, `decision.add` and `setup.outcome`, and never into the product repo.
_Avoid_: Wizard, onboarding

**Setup draft**:
The field values that one Claude call proposes from the files of a product folder (`setup.draft`, model `sonnet`, no tools, a cap of 0.50 USD). The owner confirms the cost, edits each field, and a field that the files do not answer shows "no data".
_Avoid_: Autofill, suggestion

**Inline stage**:
The stage files of `roles.apply` sent as text in the request (`stage`), in place of a folder path (`stage_dir`). It works for a product on an SSH host.
_Avoid_: Virtual stage

**Objective**:
One goal of a brief, with the signal that shows it and a date. Every priority points to one.
_Avoid_: OKR, KPI, target

### Meetings

**Meeting**:
One round in which the PM asks every agent of the roster one question, each agent answers once, and the PM writes the minutes.
_Avoid_: Call, standup, sync

**Minutes**:
The written result of a meeting: ranked priorities with their sources, disagreements between roles, and decisions to confirm.
_Avoid_: Report, summary, acta

**Priority**:
One item of the minutes that names what to work on, why, and the source that supports it.
_Avoid_: Task, recommendation

**Decision**:
A point of the minutes that waits for the owner's approval before anything changes. It can carry an **action**: one sentence that says what an agent does after the approval.
_Avoid_: Action item, approval

**Mark**:
The owner's verdict on one priority: accepted, accepted with changes, or rejected, plus "new" when the owner did not have it in mind.
_Avoid_: Rating, score, feedback

**Pending item**:
A priority with no mark, or a decision with status `pending`, in a product. `agentsi mark` lists them and walks through them.
_Avoid_: Todo, open item

**Form**:
A group of text fields and choices inside a screen, with one button that writes and one Cancel button. `Enter` in the last field writes and `Escape` cancels.
_Avoid_: Dialog, wizard

**Health section**:
The section of the settings screen that shows the result of `core.doctor`: one row for each tool check, with its status and what it blocks.
_Avoid_: Doctor screen

**Synthesis**:
The last step of a meeting, in which the PM reads the answers of the roles and writes the minutes.
_Avoid_: Summary, wrap-up

**Frozen snapshot**:
The stored product state of an earlier meeting. A meeting that reads it runs no index and no code graph, so a replay sees the same state.
_Avoid_: Cache, fixture

**Call script**:
The meeting told as a conversation, one segment per role, from which the audio is rendered. It uses Simplified Technical English (STE) style in the owner's language.
_Avoid_: Transcript, podcast script

**Call**:
The audio of a call script, played with the speaking role shown.
_Avoid_: Recording, podcast, meeting

### Stories

**Stories mode**:
A way to work on one product in which an approved story goes straight to a story session. It lives beside the spec circuit and each product turns it on.
_Avoid_: Run mode, lite mode

**Story**:
One unit of work that says who wants what and why, with 2 to 5 criteria. It is not implemented, implemented or discarded.
_Avoid_: Issue, task (when you mean a story)

**Task**:
A story with no user, for work such as debt, infra or a refactor.
_Avoid_: Chore, tech story

**Ticket**:
One note, to-do or question of a product that the owner or Claude writes and the owner closes. It gates nothing and runs nothing. A ticket that becomes work turns into a story.
_Avoid_: Card, issue, item, entry, task (when you mean a ticket)

**Open ticket**:
A ticket in the state `open` or `waiting`.
_Avoid_: Active ticket, live ticket

**Tickets screen**:
The screen of kind `Tickets` that lists the tickets of one product. Its key is `Ctrl+Shift+I`.
_Avoid_: Ticket list, board

**Ticket screen**:
The screen of kind `Ticket` that shows one ticket. Its id is the argument of the panel.
_Avoid_: Ticket detail, ticket card

**Ticket line**:
The one-line text of a ticket: `<id> [<kind>[, waiting on <waiting_on>]] <title>[ (for <assignee>)]`.
_Avoid_: Ticket summary

**Criterion**:
One "given, when, then" line of a story, with the user-facing test that QA writes for it before the work starts.
_Avoid_: Requirement, acceptance check

**Story session**:
The one background session in which the owner role of a story does its work, apart from the main branch.
_Avoid_: Run, job, worker

**Result**:
The owner's verdict on the diff of a story: merged unchanged, merged with the owner's changes, or discarded.
_Avoid_: Outcome, mark (a mark is for a priority)

### Knowledge

**Knowledge base**:
The searchable index of each product's documents, journal, history, issues and minutes.
_Avoid_: Corpus, RAG, memory

**Memory**:
What one agent learned about one product, kept as separate items that the owner can audit, each with how often it helped or hurt.
_Avoid_: Knowledge base, context, notes

**Workspace**:
The `workspace/` folder of the data root. Since ADR 0003 it holds only the audio of each meeting. The minutes, the memory and the stories are in Postgres.
_Avoid_: Data dir, storage

**Data root**:
The folder that holds the registry, the workspace, the voice files and the backups. `AGENTSI_HOME` or `~/.config/agentsi/home` names it. Without either, it is the data folder. It is not a git repo, and AgentSI makes no commits in it (ADR 0003).
_Avoid_: Data dir, data repo

**Data folder**:
The ignored `data/` folder of the code checkout, the default data root.
_Avoid_: Data repo

**Store**:
The module that reads and writes one kind of data in Postgres, such as the story store or the memory store.
_Avoid_: Repository, DAO

**Code checkout**:
The AgentSI repo: packages, web app, plugin and the global roles in `plugin/agents/`.
_Avoid_: Repo root, install dir

**Code graph**:
The index of one product's code that links each file to its symbols, its callers and the docs that describe it. Agents search it in place of reading files one by one.
_Avoid_: Knowledge base, repo map, deep scan

### Improvement

**Meeting log**:
The record of how each meeting went: which checks failed before repair, which sources each agent got wrong, what it cost.
_Avoid_: Run log, telemetry

**Lesson**:
A memory item an agent learned from a failure or from the owner's marks, not from reading the product.
_Avoid_: Insight, tip

**Reflection**:
One pass that reads the meeting log and the marks since the last pass, curates memory, and may propose a change to an agent's prompt for the owner to approve.
_Avoid_: Retro, self-improvement run, training

**Signal**:
One grounded fact that a reflection learns from: a check failure, an unresolved source, an error, or a mark of the owner, with the minutes it came from. Not the signal of an objective.
_Avoid_: Feedback, event, metric

**Delta**:
One change to memory that a curation call returns: add an item with evidence, retire an item by id, or mark an item helpful or harmful. Plain code validates and applies it.
_Avoid_: Patch, edit, rewrite

**Proposal**:
One change to an agent's prompt that a reflection makes after the agent crossed a threshold. Replay judges it, and only the owner's approval applies it.
_Avoid_: Suggestion, prompt patch

**Held-out**:
The half of the replay cases that the proposal call never sees and replay judges it on. The hash of the minutes id picks the half, so a case stays in its half.
_Avoid_: Test set, validation set

**Replay**:
Re-running one agent and the minutes on past meetings, with their inputs frozen, to compare a proposed prompt against the current one.
_Avoid_: Eval run, backtest

### The desktop app

**Key file**:
`<config>/agentsi/keys.toml`. It holds only the keys of the app that differ from the defaults, the dictation key, the dictation mode and the conversation key.
_Avoid_: Keymap, shortcuts file

**Bind file**:
The file that AgentSI writes in `<config>/agentsi/` for the compositor. It runs `agentsi-core dictate start` on press and `dictate stop` on release, or only `dictate toggle` on press in the toggle form. It also holds the conversation key, which runs `agentsi-core converse toggle` on press. The owner includes it one time. After a save, AgentSI applies the Lua form with `hyprctl eval` and no reload, and the conf form with `hyprctl reload`.
_Avoid_: Hotkey config, Hyprland config

**Toggle dictation**:
Dictation that starts on one press of the dictation key and stops on the next press. `[dictation] mode = "toggle"` in the key file sets it, and the bind file runs `agentsi-core dictate toggle`. The default is hold dictation.
_Avoid_: Continuous dictation, open mic

**Voice detector**:
The model that marks each 32 ms of audio as speech or no speech. It is Silero VAD v5, the catalog entry `silero-vad-v5`.
_Avoid_: VAD (in text for the owner), silence detector

**Piece**:
The speech of toggle dictation between two pauses of 0.6 s. The pill transcribes each piece at once and sends its text with `voice.transcript`. A piece is never longer than 30 s.
_Avoid_: Chunk, segment

**Conversation mode**:
A state of the pill in which it listens, sends each turn to one pane, and speaks the reply. `voice.converse` starts and stops it. The target is the pane that has the focus at the start. A start turns on voice mode of that pane, and the stop puts it back as it was. The mode ends on the word "stop" or "para" alone, on a click on the pill, on the conversation key, when the pane closes, when the core stops, and after 10 min with no turn.
_Avoid_: Hands-free mode, voice chat

**Turn**:
The speech between the start of the speech and a silence of 1.2 s in conversation mode. The pill transcribes it, and the core submits the text to the target pane with a Return. A turn with no text sends nothing.
_Avoid_: Utterance, message

**Waiting**:
The state of the pill after a turn went to the pane. The microphone is closed, and the pill draws no frame. The reply of the pane ends it: the pill speaks the reply and then listens again, or it listens again at once when the reply has nothing to speak.
_Avoid_: Thinking, busy

**Conversation key**:
The second key of the section "Dictation key" in the settings. One press starts conversation mode and the next press stops it. `[conversation] key` in the key file holds it, and the bind file runs `agentsi-core converse toggle`.
_Avoid_: Talk key, push to talk

**Mode mark**:
The small mark of the pill that tells the owner the microphone is open with no key held. Toggle dictation draws a ring around the red dot. Conversation mode draws a small speech bubble before the state.
_Avoid_: Badge, indicator

**Include line**:
The one line in the compositor config that loads the bind file. AgentSI shows it and never writes it.
_Avoid_: Source line, snippet

**Git screen**:
The screen of kind `Git` for one product and one repo. It shows the branch, the changed files and their diff. The owner stages files and commits from it. It has no push, pull or fetch.
_Avoid_: Git panel, source control

**Guide screen**:
The screen of kind `Guide`. It belongs to no product. It draws `docs/guide/README.md` as Markdown, with its images. The program embeds the page and the PNG files, so an installed app with no checkout shows the guide. GitHub renders the same file. The palette and the line "Guide" of the Settings screen open it.
_Avoid_: Help page, manual, user guide page in the browser

**Git config file**:
`<config>/agentsi/git.toml`. It holds the `[shared]` git config and one `[product.<id>]` table for each override. AgentSI gives the result to git through the `GIT_CONFIG_COUNT` variables. AgentSI never writes a `.git/config` or the global git config of the owner.
_Avoid_: Git settings, gitconfig

**Appearance**:
The three choices of the settings screen section "Appearance": density, corners and text size. They live in `<config>/agentsi/appearance.toml`. They change sizes, space and radii. They never change colors.
_Avoid_: Theme, skin, look settings

**Density**:
The appearance choice for the room that rows, controls and gaps take: Compact, Normal or Comfortable.
_Avoid_: Spacing, padding

**Corners**:
The appearance choice for the radius of buttons, cards and panels: Square, Subtle or Round.
_Avoid_: Border radius, roundness

**Text size**:
The appearance choice for the size of the text and of the icons of controls: Small, Medium or Large. A terminal pane keeps its own font size.
_Avoid_: Zoom, font scale

**Item row**:
A sidebar row under a product for one running or open thing: a pane, a meeting, a story session or a reflection. Its meta shows the state or the phase. A closed product shows the count of running items in a chip.
_Avoid_: Child row, sub-item

**Product group**:
A named, ordered set of products in the sidebar, inside one host group. A product with no group stays above the groups. A group collapses. Removing a group keeps its products.
_Avoid_: Folder, workspace

**Look file**:
`<config>/agentsi/look.toml`. It holds the labels, the colors and the product groups. It keeps product data by product id and pane data by terminal id. The app writes it in place and keeps each entry it does not know. The open and closed state of the tree is not in it: that state is in `layout.json`.
_Avoid_: Theme file, style file

**Label**:
Free text of 40 characters or less that the owner gives to a product or a pane. A product label shows next to its name. A pane label replaces the program title in the tab and in the item row, and the title stays in the tooltip.
_Avoid_: Alias, nickname, tag

**Label color**:
One of the 8 colors of `tokens::label` (`Hue`) that the owner gives to a product, a pane or a product group. It shows as a 3 px bar at the left of a product row, an item row and a tab. Each color reaches 4.5:1 on every surface.
_Avoid_: Tint, theme color

**Prompt**:
The small question over the window that takes one line of text or one choice. The palette entries "Label the active product", "Color the active product", "Move the active product to a group", "Rename the active pane" and "Color the active pane" open it. The context menus open the same prompt, and a Confirm is a prompt with two choices.
_Avoid_: Dialog, modal

**Clear plan**:
The list of rows and files that a delete of the "Data" section removes, with counts and bytes. `data.clear_plan` makes it and writes nothing. `data.clear` deletes only a plan of the last 5 minutes whose counts still match.
_Avoid_: Preview, dry run

**Clear data**:
The delete of the "Data" section of the settings screen and of `agentsi clear`. It runs a backup first. It never deletes a repo, a branch, a worktree or a model. `agentsi clear` is not in `agentsi help` until the switch-over writes the help text again.
_Avoid_: Purge, wipe, reset (`roles.reset` is the reset of role files)

**Tab group**:
A named, colored set of tabs in one tab bar. It draws as a chip before its tabs, and its tabs carry the color of the group as a 3 px bar at the left edge. A click on the chip collapses the group to the chip, which shows the count of its tabs, or opens it. A tab is in one group at most, and a tab that leaves the bar of its group leaves the group. `layout.json` keeps the groups (`tab_groups`, from layout version 2) and the dock keeps the order of the tabs.
_Avoid_: Tab folder, tab stack, workspace

**Chip**:
The pill of a tab group in the tab bar. It shows the arrow, the color, the name of the group and, when the group is collapsed, the count of its tabs. A dropped tab joins the group.
_Avoid_: Badge, header

**Context menu**:
The menu that opens with a right click, or with Shift+F10 or the Menu key, on a product row, an item row, a product group header, a tab, a chip, the story screen, the meeting screen, a ticket row or the Ticket screen. It lists the actions of its object (`menus.rs`). Each entry has a palette entry that acts on the active product, the focused pane, the story screen in view or the group of the active tab. A command that stops a program, spends money or hides a product asks one Confirm first. Accept and Reject of a story keep the Confirm of the story screen.
_Avoid_: Right-click menu, popup, dropdown

**Menu key**:
Shift+F10 (the action "Context menu" in `keys.toml`) and the Menu key (fixed). In the sidebar they open the menu of the row under the cursor. Elsewhere they open the menu of the focused pane. A terminal program does not get these two keys.
