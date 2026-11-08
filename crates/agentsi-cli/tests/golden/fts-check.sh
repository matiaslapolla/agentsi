#!/usr/bin/env bash
# The search criterion of the data spec on synthetic data: for 10 fixed queries, 3 of the
# first 5 results of the Rust search equal the results of the Postgres search.
#
#   crates/agentsi-cli/tests/golden/fts-check.sh
#
# It makes a git repo with 60 docs and 40 commits of made-up words. The TypeScript indexer
# fills a throwaway Postgres with it, and the Rust indexer fills a SQLite file with the same
# repo. Then `fts_check` runs the 10 queries on both. The container has its own name, port and
# volume, and the script removes them at the end. It never uses `agentsi-postgres`.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../../.." && pwd)
name=${FTS_PG_NAME:-agentsi-u19-fts-pg}
volume=${FTS_PG_VOLUME:-agentsi-u19-fts-pgdata}
port=${FTS_PG_PORT:-55620}
url="postgres://fts:fts@127.0.0.1:$port/fts"
work=$(mktemp -d "${TMPDIR:-/tmp}/agentsi-fts.XXXXXX")
work=$(cd "$work" && pwd -P)

cleanup() {
  # Stop the core of this run by the pid in its own lock file, never by name.
  pid=$(cat "$work/run/core.lock" 2>/dev/null || true)
  [ -n "$pid" ] && kill "$pid" 2>/dev/null || true
  rm -rf "$work"
  docker rm -f "$name" >/dev/null 2>&1 || true
  docker volume rm "$volume" >/dev/null 2>&1 || true
}
trap cleanup EXIT

cargo build -q -p agentsi-cli --bin agentsi --example golden_tool
cargo build -q -p agentsi-core --bin agentsi-core --example fts_check
target=$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')
bin="$target/debug"

# The corpus: 60 docs and 40 commits, with a fixed seed. Each doc and commit has its own time.
python3 - "$work" <<'PY'
import os, random, subprocess, sys, time, json
work = sys.argv[1]
rng = random.Random(19)
words = ("harbour tide anchor mooring cargo ferry lantern pilot chart shoal beacon "
         "gale swell keel hull mast rigging sail ballast quay dock pier customs ledger "
         "manifest voyage captain crew watch fog current channel buoy lighthouse reef cove").split()
repo = os.path.join(work, "repos/corpus")
os.makedirs(os.path.join(repo, "docs"))
env = dict(os.environ, GIT_CONFIG_GLOBAL="/dev/null", GIT_CONFIG_SYSTEM="/dev/null")
def git(*a, date=None):
    e = dict(env)
    if date:
        e["GIT_AUTHOR_DATE"] = e["GIT_COMMITTER_DATE"] = f"@{date} +0000"
    subprocess.run(["git", "-c", "user.name=F", "-c", "user.email=f@example.invalid",
                    "-c", "commit.gpgsign=false", *a], cwd=repo, env=e, check=True,
                   stdout=subprocess.DEVNULL)
git("init", "-q", "-b", "main")
now = int(time.time())
def sentence(n):
    return " ".join(rng.choice(words) for _ in range(n))
for i in range(60):
    path = os.path.join(repo, "docs", f"note-{i:02d}.md")
    body = f"# {sentence(rng.randint(2, 4)).title()}\n\n" + "\n\n".join(
        sentence(rng.randint(12, 40)) + "." for _ in range(rng.randint(2, 6))) + "\n"
    open(path, "w").write(body)
    stamp = now - 86400 * 20 - i * 1000
    os.utime(path, (stamp, stamp))
git("add", "-A")
git("commit", "-q", "-m", "docs: the first notes", date=now - 86400 * 30)
for i in range(40):
    path = os.path.join(repo, "docs", f"note-{i % 60:02d}.md")
    open(path, "a").write(f"\n{sentence(6)}.\n")
    git("commit", "-q", "-am", f"{rng.choice(['feat', 'fix', 'docs'])}: {sentence(rng.randint(3, 6))}",
        date=now - 86400 * 25 + i * 3600)
root = os.path.join(work, "root")
os.makedirs(root)
json.dump({"owner": {"name": "Ada", "language": "English"},
           "products": [{"id": "corpus", "name": "Corpus", "paths": [repo], "status": "active", "roles": ["pm"]}]},
          open(os.path.join(root, "agentsi.config.json"), "w"))
PY

queries="$work/queries.tsv"
printf 'corpus\t%s\n' "harbour tide" "lantern pilot" "cargo manifest" "keel hull mast" "fog current" \
  "customs ledger" "beacon or lighthouse" "voyage -captain" '"mooring cargo"' "swell" >"$queries"

docker run -d --name "$name" -e POSTGRES_USER=fts -e POSTGRES_PASSWORD=fts -e POSTGRES_DB=fts \
  -p "127.0.0.1:$port:5432" -v "$volume:/var/lib/postgresql/data" postgres:17-alpine >/dev/null
for _ in $(seq 1 60); do docker exec "$name" pg_isready -U fts -d fts >/dev/null 2>&1 && break; sleep 1; done

ts() {
  (cd "$work/repos/corpus" && env -i PATH="$PATH" HOME="$work/home" XDG_CONFIG_HOME="$work/config" \
    AGENTSI_HOME="$work/root" AGENTSI_DATABASE_URL="$url" node "$repo/packages/cli/bin/agentsi.mjs" "$@")
}
ts migrate >/dev/null
ts index --product corpus --force

"$bin/examples/golden_tool" empty-db "$work/root/agentsi.db"
export XDG_RUNTIME_DIR="$work/run" AGENTSI_HOME="$work/root" AGENTSI_CORE_SOCKET="$work/run/core.sock" \
  AGENTSI_CORE_BIN="$bin/agentsi-core" XDG_CONFIG_HOME="$work/config" XDG_DATA_HOME="$work/data"
mkdir -p "$work/run" "$work/config" "$work/data" "$work/home"
HOME="$work/home" "$bin/agentsi" index --product corpus --force

cd "$repo"
"$bin/examples/fts_check" "$url" "$work/root/agentsi.db" "$queries"
