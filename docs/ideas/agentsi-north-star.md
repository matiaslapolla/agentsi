# AgentSI north star

**Status:** shaped · **Date:** 2026-11-23 · **AI level:** Level 0, no AI

## Idea
AgentSI gets a north star in `docs/product.md` and becomes one more product in its own registry. The PM then ranks the sections of `ROADMAP.md` against two objectives. The PM can then say why one section goes before another.

## Problem
- **Who:** the owner, the only developer. AgentSI is a personal tool. The owner shows it on X sometimes.
- **Today:** the owner picks the next work by hand, in each session. `ROADMAP.md` has 12 sections with no order and no signal. PRs #17, #18 and #19 came from no minutes.

## Data it reads
- `docs/product.md`: the vision, the audience, the objectives and the out-of-scope list.
- The brief of AgentSI in the registry, with `north_stars: ["docs/product.md"]`.
- `desk_items` (owner messages by day) and `priorities` (the mark of each priority), for the signals.

## AI level
Level 0. The owner and Claude write a document and a registry entry. The product has no AI step for this idea.

## Decisions
- **Vision:** the owner opens a product, and AgentSI says what to do next and why. The owner approves, and the roles do it. The means is one place for each product.
- **Objective A, the app is the daily workplace:** sections 1, 8, 9 and the switch-over. Signals:
  - 20 days each month with an owner message at the front desk,
  - the list of tasks that the owner does outside the app gets shorter at each review.
- **Objective B, the roles know the product and are correct:** sections 5, 10, and the issue trackers and docs of section 2. Signal: 75% or more of the marks of the month are `accepted` (a guess).
- **`due`:** a monthly review date. The first one is 2026-12-31.
- **Out of scope:** platforms other than Linux (macOS, a mobile app), a remote client (Claude Code remote control covers it), and any action without the approval of the owner.
- **Owner override:** the owner's top 3 sections were 4, 5 and 6. Sections 4 and 6 have no objective. The owner kept A and B.

## Pass/fail check
1. `agentsi roles validate` passes, and `north_stars` resolves to `docs/product.md`.
2. Each of the 12 sections maps to A, B, "no objective" or "out of scope" (`agentsi-north-star-eval.md`).
3. The order from the map is not the owner's top 3.

## Conditions to stop
| Metric | Value | Date |
|---|---|---|
| Meetings on AgentSI | less than 2 | 2026-12-31 |
| Share of `accepted` marks | less than 50% | 2026-12-31 |
| Order from the map | same as the owner's top 3 | 2026-11-23 (done: the order changed) |

The cheapest test was the map of the 12 sections. It ran on 2026-11-23, and the order changed.

## Open tests
None.

## Next step
Write the spec at `docs/specs/agentsi-north-star.md`.
