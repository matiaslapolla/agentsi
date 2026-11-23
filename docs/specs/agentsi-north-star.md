# AgentSI north star

**Status:** approved 2026-11-23 · **Source:** docs/ideas/agentsi-north-star.md

## Problem
The owner picks the next work by hand in each session. `ROADMAP.md` has no order and no signal. The PM of AgentSI has no north star to rank the roadmap against.

## Goal
- `docs/product.md` holds the north star of AgentSI: the vision, the audience, two objectives with their signals, the later sections and what is out of scope.
- `ROADMAP.md` agrees with the north star:
  - the switch-over is its own section,
  - macOS, the mobile app and the remote client are not in it.
- The owner can register AgentSI as one more product in its own registry with a brief that passes `validateBrief`.
- The PM then ranks the sections of `ROADMAP.md` against `o1` and `o2`.

## Non-goals
- A code change. The core, the app and the plugin stay as they are.
- A signal view in the app. The owner uses `sqlite3`.
- An order of the sections inside `ROADMAP.md`. The PM gives the order in the minutes.
- An objective for the second audience, the devs who see AgentSI on X.
- The registration itself. The registry is on the owner's machine, so the owner runs it.

## Terms
**North star**: The doc that `north_stars` of a brief names. The PM ranks the priorities against it. For AgentSI it is `docs/product.md`.

**Review date**: The `due` of an objective of AgentSI. It is the date of the monthly review, not a delivery date.

**Outside list**: The list of the tasks that the owner still does outside the app. `docs/product.md` keeps it.

## Design

### The north star, `docs/product.md`
- The file has these sections in this order:
  1. `# Product — AgentSI`, then `**Stage:** building · **Updated:** 2026-11-23`,
  2. `## What it is`, `## Audience` and `## Vision`,
  3. `## Objectives`, `## Later` and `## Out of scope`,
  4. `## Review` and `## Sources of truth`.
- Objective 1, "The app is the daily workplace", covers these sections of `ROADMAP.md`:
  - UI polish,
  - File management and day-to-day tasks,
  - More git,
  - The switch-over.
- Objective 1 has two signals:
  - 20 days or more in each month with an owner message at the front desk (`desk_items`, kind `owner`),
  - an outside list that gets shorter at each review.
- Objective 2, "The roles know the product and are correct", covers these sections:
  - Context injection into agents,
  - More audits,
  - the issue trackers and the docs of Third-party integrations.
- Objective 2 has one signal: 75% or more of the marks of the month are `accepted` (table `priorities`, column `mark`). The doc says that 75% is a guess.
- Each signal has a `sqlite3` query on `~/.local/share/agentsi/data/agentsi.db`. The month of a mark comes from `minutes.created_at` of its minutes.
- Each objective has the review date 2026-12-31.
- `## Later` lists the sections with no objective:
  - VPS setup and Cost monitoring and optimization,
  - Claude Agent SDK on the Claude subscription and Other agent harnesses,
  - more devices of More connectivity,
  - the chat and the calendars of Third-party integrations.
- `## Later` also records the owner override. The owner's top 3 was Cost monitoring, Context injection and the Agent SDK. The owner kept objective 1 and objective 2.
- `## Out of scope` lists these items:
  - platforms other than Linux (macOS, a mobile app),
  - a remote client,
  - any action without the approval of the owner.
- `## Review` gives the monthly check, the conditions to stop and the outside list.
- The conditions to stop are fewer than 2 meetings on AgentSI by 2026-12-31, or less than 50% `accepted` marks at that review.
- The PM reads this file, because `north_stars` names it.

### The roadmap, `ROADMAP.md`
- The section "macOS and the switch-over" becomes the section "The switch-over". It keeps the switch-over item, and the section says that the work is in progress.
- The item "Port the app to macOS" goes away.
- The items "Control the agency from a remote client" and "Add a mobile app" leave "More connectivity".
- The other sections keep their text and their order.

### The brief, `docs/plans/agentsi-north-star-brief.json`
- The file is the `brief.json` of `setup.md` section 4 for the product `agentsi`.
- It has `stage` `building`, the objectives `o1` and `o2`, `north_stars: ["docs/product.md"]` and the out-of-scope items of `docs/product.md`.
- The roster is `pm`, `dev` and `qa`. The `roles` field has one line for each.
- The plan gives the `roster.json` and the commands of `setup.md` section 7: the dry run, the diff, the owner's yes, the apply and `agentsi roles validate`.

### The terms, `CONTEXT.md`
- `CONTEXT.md` gets the term **North star**. The code and `setup.md` use it.

## Acceptance criteria
- [ ] `docs/product.md` exists and has the ten headings of the design in that order.
- [ ] `docs/product.md` maps each section of `ROADMAP.md` to objective 1, objective 2, `## Later` or `## Out of scope`, as in `docs/ideas/agentsi-north-star-eval.md`.
- [ ] `docs/product.md` gives a `sqlite3` query for each signal, the review date 2026-12-31, the owner override and the conditions to stop.
- [ ] `grep -niE "macos|mobile app|remote client" ROADMAP.md` prints nothing.
- [ ] `ROADMAP.md` has a section "The switch-over" that says the work is in progress. The other sections keep their text and their order.
- [ ] A `node --import tsx` script that calls `validateBrief` on the brief JSON, with the repo root as the product path and the roster `pm`, `dev` and `qa`, prints no failures.
- [ ] The brief JSON has `stage` `building`, the objective ids `o1` and `o2`, each with `due` 2026-12-31, and `north_stars: ["docs/product.md"]`.
- [ ] On the owner's machine, the owner runs the dry run, reads the diff, applies with `--yes`, and `agentsi roles validate` passes for `agentsi`.
- [ ] `CONTEXT.md` defines **North star**.
- [ ] `wc -w` gives 1000 or less for the spec and 600 or less for the plan.
