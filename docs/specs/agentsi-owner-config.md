---
status: done
date: 2026-10-21
---

# Owner config and Notion removal

Unit 2 of `docs/plans/agentsi-open-source.md`. It fixes should-fix 2 and 3 of `docs/reviews/2026-10-21-open-source-readiness.md`.

## Problem

AgentSI assumes one owner. The prompts name one person and fix Rioplatense Spanish. The story schema stores the owner's first name in `proposed_by`. The plugin text, the push guard, the MCP tool descriptions and the web title name that person. The plugin also sends work to skills and to a Notion workspace that the repo does not ship. A stranger who installs AgentSI gets prompts and text that do not fit.

## Decision

- **Owner block.** The registry `agentsi.config.json` gets an optional top-level block `"owner": { "name": "...", "language": "..." }`. `language` is free text for the prompts, for example `"English"` or `"Rioplatense Spanish"`.
  - With no block, the name is `"the owner"` and the language is `"English"`.
  - A missing key takes its default.
  - A block that is not an object, or a key that is not a non-empty string, makes the registry read fail with a message that names the key.
- **Core.** `packages/core/src/registry.ts` exports `Owner`, `DEFAULT_OWNER`, `ownerOf(registry)` and `loadOwner(root)`. `loadOwner` reads the registry of a data root. It gives the defaults when that root has no registry file.
- **whoami.** `agentsi whoami` prints an `owner` key with the name and the language. The front desk reads it and talks in that language.
- **Prompts.** The role answer, the synthesis, the call script, the story proposal and the reflect proposal take the owner's name and language from the owner block. No prompt names a person or a fixed human language. Identifiers, paths and code stay in English.
- **No-data marker.** The role prompt asks for the fixed marker `[no data]` when data is missing. The meeting log counts `[no data]` and the old phrase "sin datos", in any case. The log field keeps its name `sin_datos`.
- **Story schema.** `proposed_by` is `"owner" | "pm"`. When a story file holds that legacy name, the read gives `"owner"`. The file on disk changes only at the next save. The web fixtures use `"owner"`.
- **Notion.** The `sync-notion` command, `packages/core/src/notion-payload.ts` and their tests go away. The plugin, the CLI help and the web app do not mention Notion. The brief schema keeps `notion` as a source kind, so old briefs still load.
- **Plugin text.** The plugin says "the owner". The front desk talks in the owner's language. Multi-unit work becomes "propose a plan of units and ask the owner". `setup.md` describes the question rounds inline. `social.md` drops the X reference. The push guard message is neutral English.
- **Other text.** The MCP tool descriptions, the web metadata, the code comments and `CONTEXT.md` say "the owner". `CONTEXT.md` gives the product list as an example. The plugin and marketplace author is "AgentSI contributors". `agentsi.config.example.json` shows the owner block with the default values.
- The web UI stays Spanish.

## Out of scope

- `docs/` and `docs/guide/`, which unit 6 curates.
- `story-run.ts`, `sandbox.ts`, `apps/web/proxy.ts` and `doctor`, which unit 3 owns.
- `packages/voice/`, which unit 5 owns.
- A translation of the web UI or of the CLI output.
- The owner's own data root. The coordinator writes the owner's block there after the merge.

## Acceptance criteria

- [x] A registry with a valid owner block gives that name and language. A registry with no block gives "the owner" and "English". An invalid block fails with a message that names the key. Unit tests cover the three cases.
- [x] `agentsi whoami` prints the owner name and language.
- [x] With a custom owner, the role prompt, the synthesis prompt, the story prompt and the proposal system prompt contain that name and that language, and no "Rioplatense" or "Spanish". A unit test proves it.
- [x] The meeting log counts `[no data]` and "sin datos" in an answer. A unit test proves it.
- [x] A story file with the legacy name in `proposed_by` loads as `"owner"` and passes validation. A unit test proves it. A new story from `story new` has `proposed_by: "owner"`.
- [x] `agentsi help` does not list `sync-notion`. `git grep -i notion` outside `docs/` returns only the brief source kind in `packages/core/src/brief.ts`.
- [x] A case-insensitive `git grep` for the owner's name returns only `docs/` files and the read-time migration with its test.
- [x] The plugin has no reference to `orchestrate-background-sessions`, the grilling skill, the `notion` skill or the X product. The push guard message is in English.
- [x] Typecheck, lint and the tests pass.
