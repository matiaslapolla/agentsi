# Build log of the four desktop modules

One entry for each merged branch of `desktop-modules.md`: what changed, the proof, the decisions, the follow-up work and the notes for later branches.

## 2026-11-09 — Start

- The owner approved building from the draft specs `docs/specs/agentsi-desktop-{tracker,actions,setup,conversation,conversation-views,immersive,immersive-alerts,immersive-theme}.md`.
- The defaults that each spec lists under "Defaults taken" stand until the owner overturns them.
- Voice on this M1 (`docs/research/voice-on-m1.md`): the CPU path of Kokoro is good enough. No GPU work is necessary.

## Contract changes for the owner

The final report lists each change to the protocol, the database schema and the MCP tools.

## 2026-11-09 — B1 `ui-tokens` merged (210a98b)

- **Changed:** `Palette` and `Palette::DARK` hold the colors. Each color constant is now a lowercase function (`surface::panel()`). About 390 lines in 35 files of `agentsi-ui`, `agentsi-app` and `agentsi-pill` moved. `crates/agentsi-ui/tokens-rename.sed` moves later code.
- **Proof:** `cargo test -p agentsi-ui -p agentsi-app` passed (346 and 33). The coordinator compared the hex values of `tokens.rs` before and after: no value changed. The palette is thread-local, so the palette test cannot change other tests.
- **Skipped:** the release bench and a screenshot comparison, because of memory. B4 runs the bench for the glow.
- **Decision:** immersive U5 (`voice.mic`) moves from B4 to B3 `setup-core`, because B2 also changes `crates/agentsi-proto/src/events.rs`.
- **Incident:** at 16:38 systemd-oomd killed both workers during two parallel GPUI builds. Since then, `.worktrees/.cargo/config.toml` (untracked) gives every worktree one shared target dir with 4 jobs, and a watcher reports memory pressure.
- **For later branches:** run the sed file on new code. `text_pairs()` gets a palette argument in B4.

## 2026-11-09 — Shared target dir removed

- **Found by B2:** cargo hashes a path crate relative to its workspace. Two worktrees that share one target dir get the same hash, so a build can reuse the other worktree's artifact. B2 got a false red in `agentsi-app`.
- **Fix:** each worktree has its own `target/`. `.worktrees/.cargo/config.toml` (untracked) sets only `jobs = 2`. The mixed workspace artifacts were cleaned in main and in B4.
- **B1 proof again:** on main after the clean, `cargo test --workspace` passed (1335 passed, 0 failed). Typecheck, lint, cargo check and clippy passed. The gate's `cargo test` step timed out at 300 s on the cold build with 2 jobs. It passed with no time limit.

## 2026-11-09 — B4 `immersive-theme` merged (5b59d3e)

- **Changed:** Settings > Appearance has "Color theme" (System, Dark, Light) and "Motion" (System, Reduced, Full). With no `appearance.toml`, the app stays Dark. A blocked pane glows on its pane, tab, sidebar item and product row: it fades in, pulses 3 times and stays static.
- **API for later screens:** `tokens::WorkState` (`color`, `tint`, `word`), `components::StateMark`, `style::glow(key, state)`, `ProductRow::state` and `ItemRow::state`.
- **Proof:** `cargo test -p agentsi-ui -p agentsi-app` passed (36 and 347). Contrast tests run on both palettes. Debug benches in a real window: idle (Light) and alert (Dark and Light) draw 0 frames after the glow ends.
- **Skipped:** the release benches, because of memory. The coordinator runs them at the end, with no other build running.
- **Decision:** the open items of B4 (the glow end rules, the Listening edge, the Blocked glow sample in settings) go to B11a.
- **Plan change:** B11 splits. B11a `immersive-alerts` (U6 to U8 and the B4 items) runs now in the idle app lane. B11b (U9, agent cards) stays last, because it changes the desk screen.

## 2026-11-09 — B2 `tracker-core` merged (03c80b6)

- **Changed:** the table `tickets` (migration 0002) and `schema::LOCAL_TABLES`. Six methods: `ticket.list`, `get`, `add`, `update`, `move`, `promote`. Each write sends `data.changed {kind: "tickets"}`. Four MCP tools: `list_tickets`, `get_ticket`, `add_ticket`, `update_ticket`. Meetings see `## Open tickets`, and a priority can cite a ticket.
- **Proof:** each commit passed the gate. No `.golden` or `.rows` file changed. The 5 old MCP tools keep their schemas. The tests cover the refusals of the session rule.
- **Accepted outside the plan:** `956c3ac` fixes a `push_guard` test that failed 1 run in 5 on the base (broken pipe).
- **Decisions accepted:** the 14 defaults in the worker report. Among them: the session rule applies only to `by: session`. Promote takes an open or a waiting ticket. Promote to a story with stories mode off returns `invalid`.
- **Ticket ids:** `C001` and up. `T` is taken by tasks (`T015`), so the prefix stays `C`. This goes to the owner as a contract point.
- **Follow-up:** `agentsi-core --test call` hit its 30 s timeout twice under load. A promote test on a real core with a fake `claude`. A ticket can link to itself.
- **For later branches:** desk-core takes migration 0003 and adds its tables to `LOCAL_TABLES` and `clear::TABLES`. Its `show_*` tools come after `update_ticket`.
- **Main after the merge:** `cargo test --workspace --no-fail-fast` passed (1368 passed, 0 failed). A first run failed `regressions.rs` `a_client_waits_for_a_long_stop_and_does_not_start_cores_in_a_loop` at load 6. It uses real sleeps, and it passed 3 times alone. Follow-up: timing tests under load (this one and `--test call`).

## 2026-11-09 — B11a `immersive-alerts` merged (ea89c20)

- **Changed:** an alert center. Panes, meetings, story sessions and decisions raise alerts. The target glows until the owner opens it. A hidden target shows a toast (3 at most). A tone plays when the owner cannot see the target. The sidebar has a bell with the count and the alert log. A system notification goes out when the window is not active. Settings has a section "Alerts" (`alerts.toml`: mute, volume, done, system, quiet). A pane that listens shows a 1 px edge.
- **Proof:** `cargo test -p agentsi-app -p agentsi-ui` passed (380 and 36). The replay test reads `crates/agentsi-app/tests/fixtures/alerts-replay.jsonl`. Debug benches: alert and idle draw 0 frames after the alert.
- **Decisions accepted:** the tones reuse the call `Player` at 24 kHz, and the device resamples. The spec asks for the device rate, and the sound is the same. System notifications use gpui-component, not `notify-send`. A Done alert plays its tone only, with no log entry and no glow.
- **For the owner to check by hand:** a real tone on the speaker, and a real system notification.
- **Flake:** `agentsi-ui bench::tests::latency_samples_of_the_warm_up_do_not_count` failed once in a gate (B4 code). Follow-up with the other timing tests.
- **Follow-up:** alerts for a reflection end and a `meeting.render` error. Opening Settings does not end `Model` alerts. A repeat alert on a sidebar row does not pulse again.
- **For B11b (agent cards):** `WorkState::Speaking` is not set anywhere yet. `AlertCenter::hear` is the place.

## 2026-11-09 — B6 `tracker-app` merged (e3d4035)

- **Changed:** `Ctrl+Shift+I` opens the Tickets screen of the active product. The owner adds, filters, opens, edits, moves and promotes tickets. Promote to story asks one Confirm. The product menu has "New ticket" and "Open tickets". The front desk shows open tickets for the owner and the session tickets of the last 24 h. The docs of the shared surfaces and `CONTEXT.md` name the tickets.
- **Proof:** each commit passed the gate. On main after the merge, `cargo test -p agentsi-app -p agentsi-ui` passed (430). A test shows that `data.changed` kind `tickets` makes the open screen read again.
- **For the owner to check by hand:** a Claude pane calls `add_ticket`, and the ticket shows on the open Tickets screen within 1 s.
- **Code review:** one `code-review` runs at the end over all the merged branches, in place of one for each branch.
- **Plan change:** B7 `actions-app` starts now from main plus a merge of the first three commits of `setup-core` (4e638a5: doctor, `core.doctor`, `decision.archived`). B7 merges after B3.

## 2026-11-09 — B3 `setup-core` merged (987df0e)

- **Changed:** `core.doctor`, `decision.archived`, `roles.apply {stage}` (inline stage, and it writes the `products` row), `registry.get` and `registry.update`, `setup.scan`, `setup.draft` (one `claude -p` call, sonnet, no tools, 0.50 USD cap) and `setup.outcome`, and the event `voice.mic {open}`.
- **Proof:** each commit passed the gate. No golden file changed. A failed registry write leaves the file and the `products` row as they were. Five rounds of a concurrent `roles.apply` and `registry.update` keep both edits. The schema test passes on main after the merge.
- **Decisions accepted:** the doctor probes live in `agentsi-proto`, because the CLI depends only on it. `stage_dir` is optional, with a 2-line change in `screens/roles.rs`. `roles.apply` and `roles.reset` keep their direct registry write, for golden parity.
- **Follow-up:** the flaky `agentsi-core --test clear` test with a running pane. `cargo fmt` changes `crates/agentsi-store/tests/tables.rs`. A failed `setup.draft` writes no log line. `setup.scan` reads only the first manifest.
- **Watcher:** the build watcher now runs as the user service `agentsi-build-watch` (log `~/.local/state/agentsi-build-watch.log`). It notifies the desktop for memory and kill events.
- **Main after the merge:** `cargo test --workspace --no-fail-fast` passed (1440 passed, 0 failed).

## 2026-11-09 — B7 `actions-app` merged (47ad1f3)

- **Changed:** the owner does in the app what needed a command before. Settings has Health ("Check again") and "Back up now". The front desk has "Update the index", mark keys on pending items, "New decision", archived decisions with Restore, and "Run in Claude" for an approved action. The meeting screen picks the roles, asks one role and shows the context. The story screens have New story, Suggest, Archive, the change note and Record a merge. The roles and memory screens have the role definition, the memory box of a reset, Reflect and Cancel. The sidebar shows archived products with Restore.
- **Proof:** each commit passed the gate. On main after the merge: the schema test passed, and `cargo test -p agentsi-app -p agentsi-ui` passed (449). The worker ran `cargo test -p agentsi-core -p agentsi-cli` (287 passed).
- **Fix asked by the coordinator (14ecf2e):** "Run in Claude" passed the action text as a bare argument of `claude`. A role writes that text, so a leading `-` could read as an option such as `--dangerously-skip-permissions`. `nav::claude_prompt` now adds `Action: ` before such a text, with a unit test.
- **Decisions accepted:** the Confirm of `story.new` and `meeting.ask` is a two-click form. "Run in Claude" opens `claude <prompt>` in the product folder, and the app runs nothing by itself.
- **For the owner to try by hand:** rows 4, 13, 16, 18, 37 and 41 of `docs/ideas/desktop-actions-eval.md`.

## 2026-11-09 — B5 `desk-core` merged (1628c8d)

- **Changed:** the core keeps one desk session for each product: one `claude -p` process with `--input-format stream-json`, read-only tools (`Read`, `Grep`, `Glob`, the read tools of `agentsi mcp` and the show tools), `--permission-mode dontAsk`, and never `--bare`. Tables `desk_sessions` and `desk_items` (migration 0003). Methods `desk.*`, the topic `desk` and its events. `decision.decide`, `decision.add`, `mark.set` and `archive.*` write a desk note, so the `/agentsi` pane and the desk stay in sync. `agentsi mcp` lists `show_decisions` only with `AGENTSI_DESK_SESSION`.
- **Proof:** each commit passed the gate. No golden file changed. The schema test passes on main after the merge. The tests use a fake `claude` that reads stream-json.
- **Failure states:** a reply over 10 min, 30 min idle, a crash, a lost session id and no `claude` each give a stored item and a state. The core never restarts a process by itself.
- **Auth states:** `ready`, `signed_out`, `missing`, `not_read` (the bare-mode risk) and `unknown`. The check runs `claude auth status` and never reads the credentials file.
- **Decision accepted:** the login check runs at the first desk request, not at core start, because a check at start changed 5 golden files. The app sends that request when it opens the front desk.
- **Contract point for the owner:** `desk.*` methods, the `desk` topic, migration 0003.
- **Main after the merge:** `cargo test --workspace --no-fail-fast` passed (1500 passed, 0 failed).

## 2026-11-09 — B8 `setup-app` merged (672c4a9)

- **Changed:** a setup screen creates a product from the app: folder, facts from `setup.scan`, an optional draft by one capped Claude call, five forms, a dry run, then Confirm. Settings has a section Owner. The places that open setup are in the sidebar and the menus. The roles screen no longer reads the clipboard.
- **Proof:** each commit passed the gate. `cargo test -p agentsi-app` passed (430). The schema test passes on main after the merge.
- **Fix asked by the coordinator (bd77e34):** update mode replaced the product's `sources_of_truth` with an empty box, because `BriefView` did not carry them. `BriefView` now has the field, `roles.brief` fills it, and the box is prefilled. A core test and an app test cover it. No golden file changed.
- **Decision accepted:** the repair draft asks its own Confirm, because it spends money.
- **For the owner to try by hand:** create a real product with the real `claude`.
- **Contract point for the owner:** `BriefView.sources_of_truth` (additive).
- **Plan change:** B12 `shots` part 1 starts now in the idle core lane. It extends `scripts/guide-shots.sh` and shoots the finished screens into the scratchpad. The desk shots wait for B9 and B10.

## 2026-11-09 — B9 `desk-app` merged (8d5ece6)

- **Changed:** the front desk has a conversation column. The owner asks, and the reply streams in as Markdown. Tool uses and desk notes are muted lines. The decisions view shows the stored decision with Approve and Reject. Each asks a Confirm, then calls `decision.decide`, and the view changes with no new turn. A decision in the `/agentsi` pane changes the view after `data.changed`. Each login state shows its message and "Check again". Stop, New conversation (with a Confirm) and the palette entry "Ask the front desk".
- **Proof:** both commits passed the gate. On main after the merge, `cargo test -p agentsi-app -p agentsi-ui` passed (477). The worker ran the schema test and clippy, and found no `wry` or `webkit`. The debug idle bench after a streamed reply and a decisions view drew 0 frames.
- **Decisions accepted:** a second `EventEmitter<DeskNotice>` on `CoreState` carries the desk events. The history shows the newest 200 items, with no paging. Stop has no Confirm, because it changes no product data.
- **For the owner to try by hand:** a desk conversation with the real `claude`.

## 2026-11-09 — B12 `shots` part 1 merged

- **Changed:** `scripts/guide-shots.sh` has 14 new shots, and the seed has tickets for Lumen and Harbor, archived decisions and an archived product. The PNG files (1024x680, Dark and Light pairs) are in the coordinator scratchpad, not in `docs/guide/img`.
- **Not ideal, accepted:** the setup shot shows update mode. The tickets part of the front desk shows on Harbor. Health shows "fail" for voice models and audio devices, because the demo has none. The temp path shows in the sidebar, as in the old guide shots.
- **Part 2:** the desk conversation shots, after B10, at the comment "Reserved for the desk conversation".

## 2026-11-09 — B11b `agent-cards` merged

- **Changed:** agent cards on the meeting, call and desk screens. Each agent of the roster has a card with its live state from the meeting events. `WorkState::Speaking` comes from `voice.speak`. No card shows a state that no event set.
- **Proof:** the commit passed the gate. The worker ran `cargo test -p agentsi-ui -p agentsi-app` (37 and 448). The commit adds no animation. The debug idle bench drew 0 frames, but with no product, so the cards did not draw.
- **Decision:** a silent call participant shows "Off", the word of `WorkState::Off` in the spec. A visual detail for the owner's review.
- **Flake fixed by the coordinator:** `bench::tests::latency_samples_of_the_warm_up_do_not_count` failed 3 times. Two calls of `Instant::now()` can return the same value, so a warm-up sample could count (`t >= from`). The test now uses explicit times. It passed 5 of 5.
- **Incident at 21:35:** an out-of-memory kill stopped the coordinator session and the B10 worker. The coordinator's commit gate ran in the main checkout with the default 8 jobs, beside a worktree build. Rule since then: each cargo command in the main checkout runs with `CARGO_BUILD_JOBS=2`.
