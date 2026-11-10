# Build order of the four desktop modules

**Status:** approved 2026-11-09 · **Source:** the module plans in `docs/plans/agentsi-desktop-*.md`

The owner chose these rules on 2026-11-09:
- The build runs to the end, with 2 build sessions at the same time.
- Opus builds the core and data branches. Sonnet builds the screen branches.
- The coordinator decides small points and records them in `desktop-modules-log.md`.
- A change to a public contract (protocol, database schema, MCP tools) goes to the owner in the final report.

Each branch below is one build session in `.worktrees/<branch>/`. The module plans give the commits inside each branch.

## Lanes

Two lanes run at the same time. The core lane changes `agentsi-proto`, `agentsi-store`, `agentsi-agency`, `agentsi-core` and `agentsi-cli`. The app lane changes `agentsi-ui` and `agentsi-app`. A branch starts only after each branch that it needs has merged.

| # | Branch | Lane | Plan units | Needs | Model |
|---|---|---|---|---|---|
| B1 | `ui-tokens` | app | immersive U1, U2 | none | sonnet |
| B2 | `tracker-core` | core | tracker U1 to U6 | none | opus |
| B3 | `setup-core` | core | actions U1 to U3, setup U1 to U3, U7, immersive U5 | B2 | opus |
| B4 | `immersive-theme` | app | immersive U3, U4 | B1 | sonnet |
| B5 | `desk-core` | core | conversation U1 to U5 | B3 | opus |
| B6 | `tracker-app` | app | tracker U7 to U9 | B2, B11a | sonnet |
| B7 | `actions-app` | app | actions U4 to U10 | B3, B6 | sonnet |
| B8 | `setup-app` | app | setup U4 to U6, U8 | B7 | sonnet |
| B9 | `desk-app` | app | conversation U6, U7 | B5, B8 | sonnet |
| B10 | `desk-views` | app | conversation U8 to U10 | B9 | sonnet |
| B11a | `immersive-alerts` | app | immersive U6 to U8 | B4 | sonnet |
| B11b | `agent-cards` | app | immersive U9 | B10 | sonnet |

## Rules at each merge

- The first branch that adds a migration keeps number 0002. The next one takes 0003.
- The first branch that merges adds `schema::LOCAL_TABLES`. Later branches add their tables to it.
- Nobody merges `schema/core.openrpc.json` by hand. After a rebase, the branch writes it again with the schema test.
- After B1 merges, each later branch runs `sed -i -f crates/agentsi-ui/tokens-rename.sed` on its new files. New screens take state colors only from `WorkState`.
- B4 changes `crates/agentsi-proto/src/events.rs` and `crates/agentsi-core/src/dictate.rs`. It must not run beside a core branch that changes the same files.
