# The data of the agency lives in Postgres, not in a second git repo

**Status:** accepted 2026-11-02 · The Postgres part is superseded by ADR 0006

## Context

Since `docs/specs/agentsi-data-root.md`, the data lives in its own git repo, so the code repo can be public. The owner wants one repo. Git history was the only backup and undo for the data.

## Decision

Postgres holds all the data of the agency. The registry, audio and backups stay as files in `data/`, which the code repo ignores. AgentSI makes no git commits of data. A daily `pg_dump` is the backup.

## Consequences

- One repo to clone, push and publish. The code stays safe to publish, because the data is never in it.
- The history of each item is gone. Only the daily dumps let the owner go back.
- Tests of data code need a database. PGlite runs one in the test process.
- Rejected: an ignored folder with its own nested git repo, which is still two repos. Plain files with no history, which keeps the file code and adds no backup. Data committed in the code repo, which keeps the code repo private forever.
