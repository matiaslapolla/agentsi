# Audit: repair baseline from the minutes

Decision 30 (minutes 2026-10-17-2335) approved this change.

## Problem

The o1 signal in `workspace/agentsi/brief.json` compares repair rounds per meeting against the first two meetings. The audit reads repairs only from `meetings.jsonl`. The first meeting (2026-10-11) has no ledger line, and its minutes say `repaired: true`. So the audit cannot show the baseline.

## Decisions

- The audit reads the repair result of every meeting from the minutes files (`check.repaired`). The minutes exist for every meeting, the ledger does not.
- The baseline is the first `REPAIR_BASELINE_MEETINGS` (2) minutes, in `created_at` order. "Since" is every later minutes file.
- A meeting has at most one repair round (`docs/ideas/agentsi.md`, verify and repair loop). So repair rounds per meeting is the count of repaired minutes divided by the count of minutes.
- The new section is `repairs` at the top level of the report. The `meetings` section keeps its ledger-only window.
- Minutes without a `check` field do not count, and they do not take a baseline place.
- Minutes with the status `invalid` count, as in the ledger `repair_rate`.
- No change to `meetings.jsonl`. No backfill.

## Plan

1. Core: add `check` to `MarkedMinutes`, add `repairBaseline()` and the `repairs` section in `buildAudit`, with tests.
2. CLI: one line in `renderAudit`.
3. Web: two tiles in `AuditSection`.
4. Update §8 of `docs/specs/agentsi-roles.md`.

## Acceptance criteria

- [x] `agentsi audit --json --product agentsi` has `repairs.baseline` with the ids of the minutes 2026-10-11 and 2026-10-13, and 1 repaired meeting of 2.
- [x] `repairs.since` counts every minutes file after the baseline, with its repaired count and its rounds per meeting.
- [x] With fewer than 2 minutes, `since.minutes` is empty and `per_meeting` is null where a group has no minutes.
- [x] Minutes without `check` do not count in either group.
- [x] `agentsi audit` prints the baseline and the later rate in one line under "Reuniones".
- [x] The roles page shows the two rates in the audit tiles.
- [x] Core tests cover the cases above. Typecheck, lint and the core, cli and web tests pass.
