# Do every action from the app

**Status:** shaped (draft: the owner did not accept the restatement yet) · **Date:** 2026-11-09 · **AI level:** level 0 for the action gaps, level 2 (workflow) for the setup draft

## Idea
The owner does each AgentSI action in the desktop app, with no CLI command and no Claude conversation. The app also creates each element: product, brief, roster, overlay, story, decision and meeting. Product docs (ADRs, specs, idea briefs) wait for conflict C1 of `docs/ideas/desktop-actions-eval.md`.

## Problem
- **Who:** the owner.
- **Today:** 17 actions have no place in the app, and 7 are partial (eval file). Setup is only a Claude conversation (`plugin/skills/agentsi/setup.md`). Apply and Changes read a path or a note from the clipboard (`crates/agentsi-app/src/screens/roles.rs:193-210`).

## Data it reads
The README, `CLAUDE.md`, the manifest, the docs index and `git log` of the product (`setup.md:41-61`). Code collects them, as `data.snapshot` does.

## Where the output shows
A new **setup screen** registers a product or updates its brief:
1. Folder: the owner selects a folder. Code proposes the id, the name and the path.
2. Draft: the owner confirms the cost. Then the core runs Claude.
3. Forms: product, objectives, roster, models, Stories mode. The draft fills each field. A field with no data stays empty and marked.
4. Review: the screen shows the `roles.apply` plan. Confirm writes it.

## AI level
Level 0 gives the id, the path, the manifest commands and the north-star files. Level 0 fails for "what", the audience, the stage and the objectives, because the README holds them as prose. Level 1 is one call with `--json-schema` (`crates/agentsi-agency/src/meeting/claude.rs:245-247`). Level 2 adds one repair call, as `story.new` does. Unknown: the level 1 pass rate.

## Pass/fail check
`roles.apply` with `yes: false` returns a plan with no failures, and each `north_stars` path exists.

## Check and repair
If the dry run fails, the core sends the failures to Claude one time. If the second draft fails, the form marks the failed fields.

## Learn from use
At apply, the core logs each field as kept, edited or cleared. If the owner edits more than half of the fields, change the prompt.

## Conditions to stop
1. On 2026-11-20, fewer than 4 of 5 seed products pass the dry run.
2. The owner edits more than half of the drafted fields in 3 drafts.
3. One draft costs more than 0.50 USD.

Cheapest test: draft 3 product repos with `claude -p --json-schema`, then run `agentsi roles apply` without `--yes`. If 2 of 3 fail, keep empty forms.

## Open tests
- The level 1 pass rate. The registry on this machine holds 0 products, so 0 seed cases exist.
- Can `data.snapshot` read a folder that the registry does not hold?
- The cost and the time of one draft.

## Next step
Prototype: the cheapest test, on 3 product repos that the owner names.
