# Stories mode in setup

**Status:** done · **Source:** The owner's request on 2026-10-21, after the setup review of the same day

## Problem

`/agentsi setup` does not know Stories mode. It does not ask for the test and gate commands, and no command writes the `stories` block of `agentsi.config.json`. The blocks of `agentsi` and a second product were written by hand.

## Goal

- Setup asks whether Stories mode is on, and recommends the test and gate commands from the product manifest.
- `agentsi roles apply` writes the `stories` block, with the same dry run, `--yes` and single commit as the roster.
- Update mode checks that the block still matches the product.

## Non-goals

- No change to the story commands or the sandbox.
- No automatic run of the gate during setup. Setup only reads the manifest.

## Design

### 1. Stage file

The stage dir may hold `stories.json`:
- an object `{ "test": "...", "gate": ["..."], "repo": "/abs" }` turns Stories mode on or changes it. `repo` is optional.
- `null` turns Stories mode off.
- no file leaves the current block as it is.

The plan validates the block with `storyConfig`: `test` has `{files}`, and `gate` is a non-empty list of non-empty commands. `repo`, when present, must be an existing absolute path to a directory. Unknown keys fail. The plan also runs the registry edit in memory, so a line it cannot rewrite fails the dry run.

### 2. Plan and apply

The plan holds the block before and after. The dry run prints one `Stories:` line when the block changes. Apply writes the block on the product's own line of the registry, in the same commit as the roster.

### 3. Setup flow

- Section 2 also notes the test runner, the typecheck and lint scripts, and tests that need a database or the network.
- Round 5 (optional) asks: Stories mode on or off, the test command, the gate commands. Each has a recommended answer. Setup checks that `bwrap` and `codebase-memory-mcp` are on the PATH before it recommends on.
- Section 7 stages `stories.json` when the answer changes the block.
- Update mode compares the block with the manifest scripts.
- The close says that `/agentsi story` works when the block is on.

## Acceptance criteria

- [x] `agentsi roles apply` with a valid `stories.json` prints a `Stories:` line and, with `--yes`, writes the block on the product's line in one commit with the roster.
- [x] `stories.json` with `null` removes the block. No `stories.json` keeps the current block unchanged.
- [x] An invalid block (no `{files}`, empty gate, relative or missing `repo`) fails the dry run and writes nothing.
- [x] `--register` with `stories.json` registers the product with its block.
- [x] `plugin/skills/agentsi/setup.md` has the Stories round, the staging of `stories.json`, the update-mode check and the close line.
- [x] Typecheck, lint and the core and CLI tests pass.
