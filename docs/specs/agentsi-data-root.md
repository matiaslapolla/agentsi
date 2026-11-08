# Data root

**Status:** done · **Source:** unit 1 of `docs/plans/agentsi-open-source.md`, approved by the owner on 2026-10-21

## Problem

AgentSI keeps its data inside the code checkout. `workspace/` and `agentsi.config.json` live next to the code, and each meeting, mark and decision commits there. A public code repo would then carry the private data of every product.

## Goal

- The data lives in its own directory, the **data root**, with its own git history.
- The code checkout holds only code. A meeting, a mark or a story write never touches it.
- Without a configured data root, AgentSI works as today: the data root is the code checkout.
- The owner's data moves to a private repo with its full history.

## Non-goals

- No change to the layout inside the data root: `agentsi.config.json` and `workspace/<product>/...` stay as they are.
- No change to the product repos or to story sessions, which already write only in the product worktree.
- The guide reads the data root, but it keeps a real product as its demo until unit 4.

## Terms

**Data root**:
The git repo that holds the registry and the workspace. It gets the commits of minutes, marks, decisions, memory, reflections and stories. Without a configured data root, it is the code checkout.

**Code checkout**:
The AgentSI repo: packages, web app, plugin and global roles in `plugin/agents/`.

## Design

### 1. Resolution

The data root is the first of:
1. `AGENTSI_HOME`, when it is set and not empty.
2. The path in `${XDG_CONFIG_HOME:-~/.config}/agentsi/home`, one line.
3. The code checkout.

A relative path is relative to the home dir, so every process gets the same data root.

The file covers every process at once: the CLI, the MCP server that Claude Code starts, and the web app. `agentsi whoami --home` prints the data root and where it came from.

### 2. Core

- `paths.ts` exports `codeRoot` and `dataRoot`. Every `root = repoRoot` default for data becomes `dataRoot`. `configPath` is in the data root.
- Code paths stay in the code checkout: `sqlDir`, the voice project, `apps/web`, `docker compose`.
- Global roles: `pluginAgentsDir(root)` is `<root>/plugin/agents` when that directory exists, else the one of the code checkout. A test root carries its own copy. The data root has none.
- A reflection changes a global role only when the root holds `plugin/agents/`. With a separate data root, the change goes to the product overlay. So AgentSI never writes into the code checkout.

### 3. Web app

`apps/web/lib/env.ts` resolves the data root with the same function from the core. `CLI_BIN` and the guide routes stay in the code checkout. `agentsi web` passes `AGENTSI_HOME` to the web process.

### 4. Migration of the owner's data

1. Commit the pending workspace change.
2. Clone this repo to a private data directory, such as `~/agentsi-data`. Keep only `workspace/` and `agentsi.config.json` with `git filter-repo`, so the history stays.
3. Write the path to `~/.config/agentsi/home`.
4. Check with `agentsi whoami --home`, `agentsi status` and the web app on two products.
5. In the code repo, delete `workspace/` and `agentsi.config.json`, and add `agentsi.config.example.json`. The code repo does not ignore them, so the default data root can still commit there.

Step 5 runs only after step 4 passes.

## Acceptance criteria

- [x] With no `AGENTSI_HOME` and no home file, the data root is the code checkout, and all current tests pass.
- [x] `AGENTSI_HOME` wins over the home file, and the home file wins over the code checkout. An empty value or a missing file is skipped.
- [x] A test with a separate data root runs a write that commits (a mark or a story write). The commit lands in the data root, and the code checkout stays clean.
- [x] With a separate data root, roles load from the code checkout's `plugin/agents/`, and a reflection target is never a global role file.
- [x] `agentsi whoami --home` prints the data root and its source.
- [x] The web app reads products, minutes and stories from the data root.
- [x] The owner's data lives in a private data repo with its history. The code repo has no `workspace/` and no `agentsi.config.json`, and it ships `agentsi.config.example.json`.
- [x] Typecheck, lint and all tests pass.
