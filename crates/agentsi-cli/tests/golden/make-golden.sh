#!/usr/bin/env bash
# Writes the golden files of this folder with the TypeScript CLI (packages/cli).
#
#   crates/agentsi-cli/tests/golden/make-golden.sh
#
# It starts a throwaway Postgres container with its own name, port and volume,
# loads fixture/data.sql into it, runs each case of cases.tsv with the TypeScript
# CLI, and writes one <case>.golden file. At the end it removes the container and
# its volume. It never touches the container `agentsi-postgres`, its volume or the
# database of your config. Set GOLDEN_KEEP_PG=1 to keep the container for a re-run.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
name=${GOLDEN_PG_NAME:-agentsi-u20-golden-pg}
volume=${GOLDEN_PG_VOLUME:-agentsi-u20-golden-pgdata}
port=${GOLDEN_PG_PORT:-55620}
url="postgres://golden:golden@127.0.0.1:$port/golden"
work=$(mktemp -d "${TMPDIR:-/tmp}/agentsi-golden.XXXXXX")
work=$(cd "$work" && pwd -P)

cleanup() {
  rm -rf "$work"
  if [ "${GOLDEN_KEEP_PG:-}" != 1 ]; then
    docker rm -f "$name" >/dev/null 2>&1 || true
    docker volume rm "$volume" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

tool() { (cd "$repo" && cargo run -q -p agentsi-cli --example golden_tool -- "$@"); }
psql_() { docker exec -i -e PGOPTIONS='-c client_min_messages=warning' "$name" psql -U golden -d golden -v ON_ERROR_STOP=1 -q "$@"; }

if ! docker ps --format '{{.Names}}' | grep -qx "$name"; then
  docker run -d --name "$name" -e POSTGRES_USER=golden -e POSTGRES_PASSWORD=golden -e POSTGRES_DB=golden \
    -p "127.0.0.1:$port:5432" -v "$volume:/var/lib/postgresql/data" postgres:17-alpine >/dev/null
fi
for _ in $(seq 1 60); do
  docker exec "$name" pg_isready -U golden -d golden >/dev/null 2>&1 && break
  sleep 1
done

ts() {
  (cd "$1" && shift && env -i PATH="$work/bin:$PATH" HOME="$work/home" XDG_CONFIG_HOME="$work/config" TZ=UTC \
    AGENTSI_HOME="$work/root" AGENTSI_DATABASE_URL="$url" \
    GIT_AUTHOR_NAME=Golden GIT_AUTHOR_EMAIL=golden@example.invalid \
    GIT_COMMITTER_NAME=Golden GIT_COMMITTER_EMAIL=golden@example.invalid \
    GIT_AUTHOR_DATE=2099-01-01T10:00:00Z GIT_COMMITTER_DATE=2099-01-01T10:00:00Z \
    GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null \
    node "$repo/packages/cli/bin/agentsi.mjs" "$@" <"${TS_STDIN:-/dev/null}")
}

# The work folder and the database, new. A case that writes starts from here.
load() {
  find "$work" -mindepth 1 -delete
  tool prepare "$work"
  psql_ -c 'drop schema public cascade; create schema public;' </dev/null
  ts "$work/outside" migrate >/dev/null
  psql_ <"$work/data.sql"
  # The fixture gives explicit ids to the decisions, so the sequence moves past them, as after an import.
  psql_ -c "select setval(pg_get_serial_sequence('decisions', 'id'), (select max(id) from decisions))" </dev/null >/dev/null
}
load

# One line for each step: name, shared, fresh or rows, step number, step count, cwd, arguments.
# A `rows` case is a fresh case that also keeps the rows that it leaves.
# Pass 1 runs the shared cases. The MCP sessions and the documents dump need their index, so they
# come next. Pass 2 runs the cases that write: each one loads a new work folder and database.
run_cases() {
# The list comes first: `load` empties the work folder, and `tool cases` reads it.
local cases
cases=$(tool cases "$work")
while IFS=$'\t' read -r case mode index count cwd rest; do
  IFS=$'\t' read -ra argv <<<"$rest"
  if [ "$1" = shared ] && [ "$mode" != shared ]; then continue; fi
  if [ "$1" = fresh ] && [ "$mode" = shared ]; then continue; fi
  # GOLDEN_ONLY=<regex> writes only the cases whose name matches.
  if [ -n "${GOLDEN_ONLY:-}" ] && ! [[ "$case" =~ $GOLDEN_ONLY ]]; then continue; fi
  if [ "$mode" != shared ] && [ "$index" = 0 ]; then load; fi
  code=0
  if [[ "$rest" == *'{LAST_REFLECTION}'* ]]; then
    last=$(psql_ -At -c "select id from reflections where product = 'alpha' and entry->>'reverts' is null order by position desc limit 1" </dev/null)
    argv=("${argv[@]//\{LAST_REFLECTION\}/$last}")
  fi
  if [ "${argv[0]:-}" = @meeting-rows ]; then
    psql_ -At <"$here/meeting-rows.sql" >"$work/rows.json"
    tool meeting-rows "$work" "$work/rows.json" >"$work/stdout"
    : >"$work/stderr"
  elif [ "${argv[0]:-}" = @reflect-rows ]; then
    psql_ -At <"$here/reflect-rows.sql" >"$work/rows.json"
    tool reflect-rows "$work" "$work/rows.json" >"$work/stdout"
    : >"$work/stderr"
  elif [[ "${argv[0]:-}" == @sql-* ]]; then
    psql_ <"$here/fixture/sql/${argv[0]#@sql-}.sql"
    : >"$work/sql-loaded"
    : >"$work/stdout"
    : >"$work/stderr"
  elif [[ "${argv[0]:-}" == @* ]]; then
    tool harness "$work" "${argv[0]}" >"$work/stdout"
    : >"$work/stderr"
  else
    ts "$cwd" "${argv[@]}" >"$work/stdout" 2>"$work/stderr" || code=$?
  fi
  if [ "$count" = 1 ]; then
    tool render "$work" "$code" "$work/stdout" "$work/stderr" >"$here/$case.golden"
  else
    [ "$index" = 0 ] && : >"$here/$case.golden"
    tool render "$work" "$code" "$work/stdout" "$work/stderr" "${argv[*]}" >>"$here/$case.golden"
  fi
  if [ "$mode" = rows ] && [ "$index" = $((count - 1)) ]; then
    psql_ -At <"$here/rows.sql" >"$work/rows.json"
    tool rows "$work" "$work/rows.json" >"$here/$case.rows"
  fi
  if [ "$mode" = reflect-rows ] && [ "$index" = $((count - 1)) ]; then
    psql_ -At <"$here/reflect-rows.sql" >"$work/rows.json"
    tool reflect-rows "$work" "$work/rows.json" >"$here/$case.rows"
  fi
  echo "$case[$index]: exit $code"
done <<<"$cases"
}
run_cases shared

# The MCP sessions and the documents dump need all shared cases, so a filtered run skips them.
if [ -z "${GOLDEN_ONLY:-}" ]; then
# The MCP sessions run after the cases, so the index holds what `index` and `desk` wrote.
tool mcp-cases "$work" | while IFS=$'\t' read -r case cwd requests; do
  TS_STDIN="$requests" ts "$cwd" mcp >"$work/stdout" 2>/dev/null || true
  tool render-mcp "$work" "$work/stdout" >"$here/$case.golden"
  echo "$case: $(grep -c '"id"' "$work/stdout") answers"
done

# The documents table that the TypeScript indexer left, as the Rust test compares it.
psql_ -At -c "select row_to_json(t) from (select product, scope, kind, ref, title, body,
  to_char(updated_at at time zone 'utc', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') as updated_at from documents) t" >"$work/documents.jsonl"
tool documents "$work" "$work/documents.jsonl" >"$here/documents.golden"
echo "documents: $(wc -l <"$here/documents.golden") rows"
fi

run_cases fresh
