# AgentSI desktop: data in SQLite

**Status:** approved 2026-11-02 · **Source:** `docs/specs/agentsi-desktop.md`, `packages/core/sql/001_init.sql` to `008_minutes_logs_archive.sql`, ADR 0003

## Problem

Today a Postgres container holds the data of the agency. Each device and each SSH host needs the data without a container. The data model of the 8 Postgres migrations must move with no loss.

## Goal

- One SQLite file on each host holds the data of that host.
- The tables keep the names, the columns and the keys of Postgres.
- One import copies Postgres into SQLite with the rules of the TypeScript import of today, and proves the row count of each table.
- The core makes a backup each day.

## Non-goals

- A sync between hosts. A product lives on the host that has its repo.
- A new shape for the JSON columns. They stay JSON text.
- A port of `agentsi data import`, which read the old file data root. Postgres already holds that data.

## Terms

- **Database file**: `<data root>/agentsi.db`, the SQLite file of one host.

## Design

### File and connection

- `rusqlite` with the `bundled` feature gives the same SQLite and FTS5 on Linux and macOS.
- The core opens the database file with `journal_mode=WAL`, `foreign_keys=ON` and `busy_timeout=5000`.
- The core is the only process that opens the file. One writer thread does each write. Readers use a pool of connections.
- `PRAGMA user_version` holds the number of the last migration.

### Migrations

- `crates/agentsi-store/sql/0001_init.sql` holds the final shape of the 8 Postgres migrations in one file.
- Each later change is a new numbered file. The core applies the missing files at start, each one in a transaction.
- The registry `agentsi.config.json`, the workspace audio and the backups stay as files in the data root (ADR 0003).

### Type map

| Postgres | SQLite | Rule |
|---|---|---|
| `text` | `TEXT` | No change |
| `jsonb`, `json` | `TEXT` | `CHECK (json_valid(col))` |
| `timestamptz` | `TEXT` | ISO 8601 in UTC with 3 fraction digits, for example `2026-11-02T10:00:00.000Z`. A Postgres value with microseconds keeps 6 digits |
| `boolean` | `INTEGER` | `CHECK (col IN (0, 1))` |
| `int` | `INTEGER` | No change |
| `numeric` (`usage.cost_usd`) | `REAL` | A sum differs by less than 1e-6 |
| `bigserial` primary key | `INTEGER PRIMARY KEY AUTOINCREMENT` | The import keeps each id |
| `bigserial` position | `INTEGER NOT NULL` | The store writes `max + 1` |
| `check (x in (...))` | same `CHECK` | The final value lists of migrations 003 and 004 |

### Tables

These 17 tables keep their names, columns and primary keys: `products`, `documents`, `index_state`, `minutes`, `priorities`, `decisions`, `usage`, `stories`, `story_logs`, `memory`, `agent_overlays`, `briefs`, `meeting_log`, `snapshots`, `listens`, `reflections` and `archive_records`. Each b-tree index of Postgres has one SQLite index with the same columns. FTS5 takes the place of the GIN index.

A local table is a table that Postgres never had. The tracker adds `tickets` and its index `tickets_product_state_idx` (`agentsi-desktop-tracker.md`). `schema::LOCAL_TABLES` lists it. The import reads only the 17 tables, and `read::counts`, the restore drill and the schema tests read both lists.

### Search

- `documents.tsv` becomes an FTS5 table `documents_fts(title, body)` with `content='documents'`. Three triggers keep it current.
- The tokenizer is `unicode61 remove_diacritics 2`. It is close to the `simple` configuration of Postgres: lowercase words and no stemming.
- FTS5 finds the matches. A port of the Postgres `ts_rank` (`crates/agentsi-store/src/rank.rs`) orders them, with the weight 1.0 for the title and 0.4 for the body, as the `A` and `B` weights of Postgres.
- The port takes the place of `bm25`. On the owner's data, `bm25(documents_fts, 2.0, 1.0)` passed 6 of the 10 fixed queries, and the port passes 10 of 10. `bm25` puts short commits before long docs, and `ts_rank` has no length normalization.
- An `or` between two operands is OR. In other places, `or` is a word, as in `websearch_to_tsquery`.

### Import

`agentsi-core import-postgres <url> [--fresh] [--data-root <dir>]` copies the data. It reads Postgres in one read-only transaction, and it holds the lock of the core socket while it runs.

The rules copy `packages/core/src/data-import.ts`:

1. It reads each table from Postgres.
2. It inserts only the rows that the database lacks, by primary key. It never replaces a row.
3. A minutes id that another product already holds becomes `<id>-<product>`. The minutes log line, the snapshot and the listen of that minutes take the new id. Its priorities, usage rows, decisions, archive record and minutes document also take it.
   - `usage` has no primary key. If a minutes has usage rows in SQLite, it gets no usage rows from Postgres.
4. It counts the rows of each table on both sides and prints one line for each table: `<table> pg=<n> sqlite=<n> ok|FAIL`. A table with fewer rows in SQLite than in Postgres is `FAIL`, and the exit code is 1.

The import writes into a copy of the database file. Only when each table is `ok` does the copy take the place of `agentsi.db`.

The import is safe to run many times. A second run inserts nothing. Postgres stays the source of the data until the switch-over.

The import without `--fresh` only inserts. Thus it does not copy a later change to a row that SQLite already has, such as a new mark or a decided decision. Also, when Postgres saves a minutes again, its decisions get new ids. A later import then keeps the old decisions and adds the new ones.

`--fresh` imports into a new empty database file. If `agentsi.db` exists, it stops with exit code 1 and changes nothing. The switch-over uses only `--fresh`.

**Restore drill:** `agentsi-core backup` dumps the database. The drill restores the dump into an empty database and compares the row count of each table with the source.

### Backups

- `agentsi-core backup` (in M2 also `agentsi backup`, through `data.backup`) uses the SQLite online backup API. It writes `<data root>/backups/agentsi-<UTC date>.db` and a copy of the registry.
- The core makes the first backup of each day when it starts or at 03:00 local time. A day is the UTC date of the file name. A core that finds `backups/agentsi-<today>.db` makes none, and writes one line to its log. A core with no `agentsi.db` makes none, and tries again at 03:00 (U13).
- The core keeps 14 backups, as `KEEP_BACKUPS` does today. `--keep <n>` changes the number.

## Acceptance criteria

- [ ] `cargo test -p agentsi-store` passes, with one test for each of the 17 tables and each local table that writes and reads one row.
- [ ] `sqlite3 agentsi.db .schema` lists the 17 tables, the local tables, `documents_fts` and each b-tree index of the Postgres schema.
- [ ] `agentsi-core import-postgres postgres://…` on the owner's database prints `ok` for each table and stops with exit code 0.
- [ ] After the import, `select id from decisions` gives the same ids on both sides.
- [ ] After the import, the sum of `usage.cost_usd` differs by less than 1e-6 between the two sides.
- [ ] For 10 fixed queries, 3 of the first 5 results of the search equal the Postgres results (the same ref). The TypeScript CLI has no `agentsi search`, so the Rust CLI has none either. `cargo run -p agentsi-core --example fts_check -- <url> <db> [<queries file>]` runs the check, and `crates/agentsi-cli/tests/golden/fts-check.sh` runs it on synthetic data.
- [ ] A second import inserts 0 rows, replaces no row and stops with exit code 0.
- [ ] If `agentsi.db` exists, `import-postgres --fresh` stops with exit code 1 and leaves the file as it was. Into an empty data root, it prints `ok` for each table.
- [ ] A test inserts a newer row in SQLite. The import leaves it as it is.
- [ ] A test imports two products that hold the same minutes id. The second minutes becomes `<id>-<product>`, with its log line, snapshot and listen.
- [ ] The restore drill passes: the count of each table equals the source.
- [ ] `agentsi-core backup` (in M2 `agentsi backup` through `data.backup`) writes a file that `sqlite3 <file> 'pragma integrity_check'` reports as `ok`.
