# Each host keeps the agency data in one SQLite file, and Postgres goes away

**Status:** accepted 2026-11-02 · Supersedes the Postgres part of ADR 0003

## Context

ADR 0003 put the data in a Postgres container. The app runs on laptops and SSH hosts, where a container is a burden. The owner accepts one database on each host and no sync.

## Decision

- The core of each host keeps the data in `<data root>/agentsi.db`, with `rusqlite` and the bundled SQLite.
- Only the core opens the file. Clients read and write through the core protocol.
- The schema keeps the 17 tables of Postgres, with the type map of `docs/specs/agentsi-desktop-data.md`.
- A local table is a table that Postgres never had. The tracker adds `tickets` (`docs/specs/agentsi-desktop-tracker.md`). `schema::LOCAL_TABLES` lists local tables. The import from Postgres reads only the 17 tables.
- Full-text search uses FTS5 to find the matches. A port of the Postgres `ts_rank` orders them. On the owner's data, `bm25` gave the Postgres results for 6 of 10 fixed queries, and the port for 10 of 10.
- The rest of ADR 0003 stays: the registry, the audio and the backups are files in the data root, and AgentSI makes no git commits of data.

## Consequences

- A fresh install needs no Docker and no database server.
- The import from Postgres only inserts. At the switch-over, `import-postgres --fresh` writes a new database file.
- Tests use a SQLite file in a temp folder, with no PGlite.
- A product lives on the host that has its repo. If that host is off, the app shows "device off" and cannot read the product.
- Search ranks differ a little from Postgres, because the tokenizers differ. The rank reads each match, so a query with many matches costs more.
- Rejected: Postgres on each host, because it needs a server. A sync layer such as CRDTs or Litestream, because the owner does not need the same data on two devices. Plain files, because ADR 0003 already rejected them.
