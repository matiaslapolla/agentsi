# Golden tests of the CLI

`cargo test -p agentsi-cli --test golden` runs each case of `cases.tsv` with the Rust CLI on a fixed database. It compares the output with the file `<case>.golden` of this folder.

It also runs the MCP sessions of `mcp-cases.tsv` (`agentsi mcp` on stdio) and compares the `documents` table that the Rust indexer left with `documents.golden`.

**The TypeScript CLI wrote every `.golden` file** on the same data. A file that the Rust CLI wrote would prove nothing. So `make-golden.sh` is the only way to write them.

## Row files

A `fresh` case with `rows` in the fifth column of `cases.tsv` also has a `<case>.rows` file. The TypeScript run writes it with `rows.sql` on Postgres after the last step. The Rust test writes the same shape from `agentsi.db` with `support.rs::sqlite_rows` and compares. A command that fails has a rows file too: it must leave the rows as they were.

The rows are the tables `minutes` (id and `data`), `priorities`, `decisions` and `archive_records`. `support.rs::rows_text` sorts the keys and prints the JSON. It has one normalization of time. A `decided_at` after 2026-10-31 was set by the run and prints as `<NOW>`. Fixture dates are all before that day.

The rows check leaves `documents` out. TypeScript refreshed the minutes documents after a mark. The Rust program leaves that to `knowledge.index`.

An argument in double quotes keeps its spaces in `cases.tsv`.

Known differences that no case covers:

- `backup`, `voice` and `web` have no golden case. TypeScript runs `pg_dump` (`backup`), writes a state file for a Claude Code session (`voice`) and starts a web server (`web`). The Rust commands make a SQLite online backup, switch voice mode of the pane in `AGENTSI_TERMINAL`, or of the session in `CLAUDE_CODE_SESSION_ID` and open `agentsi-app`. `tests/ops.rs` holds their expected text. The backup test restores the file and compares the rows of each table with the source. The `help` golden still holds the TypeScript summaries of these three commands. The switch-over (U28) writes it again.
- A decision whose minutes has no stored body. TypeScript writes the decision row and then fails. The Rust write fails as a whole, so no row changes.

A case with `reflect-rows` in the fifth column has a `.rows` file of another shape. It holds the tables `memory`, `reflections`, `decisions` and `agent_overlays`. It also holds the counts of the tables that a reflection must not touch. `reflect-rows.sql` writes it on Postgres. `support.rs::reflect_sqlite_rows` gives the same shape from SQLite. `@reflect-rows` prints the same text in the middle of a case.

## What a file holds

```
exit: <exit code>
--- stdout
<stdout>
--- stderr
<stderr>
```

The text has one more newline after each part. Both sides pass the output through `normalize` in `support.rs`.

## The data

All data is synthetic. The repo will be public.

- `fixture/root/` is the data root: the registry `agentsi.config.json` and three global role files.
- `fixture/repos/` holds the folders of the products. `alpha` has no git repo and rich data. `beta` is a git repo with docs, a journal and three commits, and thin data. The three commits get dates 3, 2 and 1 minutes before the fixed git date, so "the last commit" has no tie. `gamma` is an archived product.
- `fixture/data.sql` is the database. One file loads into Postgres for the TypeScript CLI and into SQLite for the Rust CLI.
- `{{WORK}}` in the registry and in `data.sql` is the work folder of one run. `dot-claude/` becomes `.claude/` in the work folder, because the repo ignores `.claude/`.

`support.rs::prepare` builds the work folder: it copies the fixture, fills `{{WORK}}`, makes the git repo of `beta` and loads `data.sql` into `root/agentsi.db`.

## Write the files again

```sh
crates/agentsi-cli/tests/golden/make-golden.sh
```

The script does these steps:

1. Start `postgres:17-alpine` as the container `agentsi-u20-golden-pg` on `127.0.0.1:55620`, with the volume `agentsi-u20-golden-pgdata`. `GOLDEN_PG_NAME`, `GOLDEN_PG_VOLUME` and `GOLDEN_PG_PORT` change them. It never uses `agentsi-postgres`, its volume, or the database of your config.
2. `golden_tool prepare <work>` builds the work folder (`cargo run -p agentsi-cli --example golden_tool`).
3. `agentsi migrate`, then `psql < data.sql`, both with the TypeScript CLI and `AGENTSI_DATABASE_URL` of that container.
4. For each case, run `node packages/cli/bin/agentsi.mjs <args>` in the folder of the case, with `AGENTSI_HOME=<work>/root`.
5. `golden_tool render` normalizes the output and writes `<case>.golden`.
6. Remove the container and the volume. `GOLDEN_KEEP_PG=1` keeps them for a second run.

`agentsi index` and `agentsi desk` write the index into Postgres, so their cases come after the shared cases in `cases.tsv`. After the cases, the script runs each MCP session of `mcp-cases.tsv` with `node packages/cli/bin/agentsi.mjs mcp`, and dumps the `documents` table of Postgres into `documents.golden`. Both sides run with `TZ=UTC`, because the MCP text prints local times.

`GOLDEN_ONLY=<regex>` writes only the cases whose name matches. The MCP sessions and the `documents` dump always run. So a filter must also match `^(index-|desk-)`, or they write from an empty index.

## Cases that write

A case with `fresh` in the fourth column of `cases.tsv` writes. It runs on a new work folder and a new database, with its own core. The script also moves the sequence of `decisions.id` past the fixture ids, as an import would. Its steps are separated by ` ;; `, and the golden file holds `## <step>` before the output of each step. A step that starts with `@` is a harness step: `@registry` prints `agentsi.config.json`, `@commits` prints the subjects of the commits of the data root, and `@story-branch` makes the branch of the story S001, and `@tree` prints the files under `root` and `repos` (no `.git`, no `backups`, no database). The TypeScript run and the Rust run print them from their own files, so the golden file proves that both write the same files and nothing else.

The data root is a git repo, and both runs get the same git identity, because `roles apply` and `reset` commit the registry. They also get a fixed git date in 2099, so each commit hash is the same in each run: `story accept` prints the hash of its merge (U23). A date in the future keeps the commits in the 14 days of the snapshot.

`beta` has a stories block. `prepare` makes the commits of the branch `story/S001-filter-mail` and removes the branch, because the indexer reads each branch and the cases of other commands must not see it. The harness step `@story-branch` makes it again. `prepare` writes the hashes of these commits into `data.sql` in place of `{{BETA_MAIN}}`, `{{BETA_STORY_TESTS}}` and `{{BETA_STORY}}`. In `cases.tsv`, `{BETA_MAIN}` is the head of `main` of `beta`. No case starts a Claude session: `story new`, `suggest`, `changes` and `run` have cases only for the errors that come before the model call. `fixture/stages/` holds the stage folders of `roles apply`.

## Cases of a reflection

A reflection has no question, and its calls run at the same time. So these cases use three more steps:

- `@sql-<name>` loads `fixture/sql/<name>.sql` into the database of the run. `reflect.sql` replaces the data of `alpha` with made-up evidence. Its JSON keys are in Postgres `jsonb` order, because the order of the agents shows in the output.
- `@scenario-<name>` writes the file `claude-scenario`, which names the folder of `replies/` for the fake `claude`. With that file, the fake records each call under its key and a checksum of its system and its prompt. So two calls at the same time do not overwrite each other, and a different prompt gives another order of the calls.
- `{LAST_REFLECTION}` in a step is the id of the last reflection that is not a revert.
- `@git-hook-fail` makes each later commit of the data root fail, so `decide` prints `Commit skipped:`.

`decide 9` in these cases is the decision of the proposal: the fixture ends at id 8.

## What is normalized

These rules are in `support.rs`, the same on both sides:

1. The work folder is `<WORK>`.
2. A commit line of the snapshot (`- <hash> <date> <subject>`) keeps the subject only. Git makes the hash and the date at the time of the run.
3. The seconds at the end of the index line of `desk` and `index` are `<SECONDS>`.
4. `generated_at` of the audit is `<NOW>`.
5. The date after `Last commit:` of `status` is `<DATE>`. A commit of the run has the date of the run.
6. The minutes in `indexed <n> min ago` of `status` are `<N>`. The TypeScript CLI is slower than the Rust CLI.
7. The id of a reflection is `r-<STAMP>`, and a time after 2026-10-31 is `<NOW>`. In a run with an `@sql-` step, each memory id of hexadecimal digits is `m-<NEW>`. The fixture ids of `reflect.sql` have a letter that is not hexadecimal, so they stay. A run with no `@sql-` step keeps each id, because a new id can look like `m-0001`.
8. The checksum in the name of a call of a reflection goes away. The calls keep the order that it gave.

The MCP golden files pass through `mcp_golden`. It sorts the answers by id and sorts the keys. It also removes the snippet line of each search hit, because Postgres `ts_headline` and FTS5 `snippet` cut other words. It replaces the 12-character hash of a commit ref, and the time after `updated=`.

`documents.golden` holds one JSON object for each row, sorted. The hash and the date of a commit, and the time of a file, are replaced, because they come from the run. The golden test fails when the Rust indexer leaves another title, body, scope or ref than the TypeScript indexer.

Nothing else is normalized. The dates in the data are fixed. The objective that is overdue has the due date 2020-01-01, so it stays overdue.

## Add a command

1. Add the cases to `cases.tsv`: `name<TAB>cwd<TAB>arguments`. The `cwd` is `alpha`, `beta` or `outside`. `{WORK}` in an argument is the work folder.
2. Add the rows that the command reads to `fixture/data.sql`, in SQL that Postgres and SQLite both accept.
3. Run `make-golden.sh`. Read each new file. Check that it holds what the command should print, and not an error.
4. Port the command. Run `cargo test -p agentsi-cli --test golden`.
5. If a field changes at each run, fix it in the data. Add a rule to `normalize` only if you cannot, and write it in the list above.

To see how the Rust output differs, run the test with `AGENTSI_BLESS_RUST=1`. It writes `<case>.rust` next to the golden file. Delete those files. They are not golden files.

## Known differences from the TypeScript CLI

- `decide <id> approve` on a proposal with no target (id 7 of the fixture): TypeScript fails with a V8 text. The Rust CLI prints that the proposal may not write its target.
- `reflect` writes the memory, the entry and the decision in one transaction. TypeScript uses several writes. A second `reflect` of one product returns a conflict while the first runs.
- `decide` checks the allowlist of the target before its hash. TypeScript checks the hash first, so a target outside the list ends as `stale` there.
- A stored reflection line without `signals` or `memory` reads as empty. TypeScript fails on such a line with a V8 text, so no case runs `reflect` on `data.sql` without `@sql-reflect`.
- An add may cite the whole line of a signal as well as its bare ref. TypeScript takes the bare ref only. A real model cites the line, so TypeScript drops each real lesson. The prompt stays equal.
- `sharedLesson` breaks a tie by bytes, not with `localeCompare`.
- A marked priority with no `owner_role`: TypeScript runs a curation call for `undefined`, and fails if that call adds an item. The Rust code keeps the signal and runs no call for it.
- A proposal stores the hash of the target that it read before its model call. TypeScript reads the hash after the call, so an edit during the call is lost on approval.
- `reflect` takes the id of its entry in its write. TypeScript takes it before, and a revert in the same minute overwrites the line.

- The tool `role_memory` of the TypeScript server tries `|| "sin memoria todavía"` on a promise, which is never falsy. So it answers with an empty text for a role with no memory. The Rust tool does the same.
- Search hits with the same rank and the same time come in an order that no engine defines. The MCP sessions use queries without such ties.
- `agentsi search` does not exist in the TypeScript CLI, so the Rust CLI has none. The search is the core method `knowledge.search` and the MCP tool `search`.
- The TypeScript text of an unknown option comes from Node (`util.parseArgs`). The Rust CLI prints `Unknown option '--x'` after `agentsi <command>: `. No golden file covers it.
- The TypeScript code prints a warning on stderr for an overlay that lists tools that a meeting does not allow. `roles`, `roles validate` and `roles apply` print it in Rust too. The Rust `audit` prints none, so no case runs `audit` on a product with such an overlay.
- A V8 text, such as the reason that `JSON.parse` gives for a stage file that is not JSON, cannot match. No golden file covers it. The Rust text is the reason of `serde_json`.
- The Rust core copies the database and the registry into `backups/` of the data root once a day. `@tree` leaves that folder out.
- `agentsi meet --audio` calls `call.render` in Rust (U26). The TypeScript CLI needs Python and a voice model, so no case runs it. The `render-*` cases cover only the errors that come before any voice runs.
- A meeting of the same minute with the same question on the same product reuses the id and overwrites, as in TypeScript. The Rust write is one transaction, so a failed meeting writes no row. TypeScript uses several writes.
- `minutesFileOf` of `meeting.ts` is not ported: the old minutes files do not exist after `import-postgres`.
- `Ctrl+C` during `agentsi meet` or `agentsi ask` calls `meeting.cancel` and exits 130. The role runs have their own process group. No case sends a signal.
- The docs index of the snapshot sorts names without case, close to `localeCompare`. A name with an underscore can sort in another place.
