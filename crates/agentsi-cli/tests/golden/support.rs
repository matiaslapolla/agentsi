//! What the golden test and `examples/golden_tool.rs` share: the fixture, the
//! cases and the one normalization of the output. The TypeScript side and the
//! Rust side use the same code, so a golden file proves the same thing for both.
//! See `tests/golden/README.md`.

#![allow(dead_code)]

use regex::Regex;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn golden_dir() -> PathBuf {
    // Read at run time: the shared CARGO_TARGET_DIR reuses a binary across worktrees.
    PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("run with cargo")).join("tests/golden")
}

pub struct Case {
    pub name: String,
    /// `alpha`, `beta` or `outside`: the folder to run in.
    pub cwd: String,
    /// true: the case runs on a work folder of its own, with fresh data. A case that writes is fresh.
    pub fresh: bool,
    /// The rows that the case leaves are compared too (`<case>.rows`). Only a fresh case has them.
    pub rows: bool,
    /// The `.rows` file holds the rows of `reflect-rows.sql`, not those of `rows.sql`.
    pub reflect_rows: bool,
    /// The steps, in order. Each step is the arguments of one `agentsi` run, or a harness step that starts with `@`.
    /// `{WORK}` is the work folder. Take them with `steps`.
    raw_steps: Vec<Vec<String>>,
}

impl Case {
    /// `{BETA_MAIN}` is the head of `main` in the repo of `beta`.
    pub fn steps(&self, work: &Path) -> Vec<Vec<String>> {
        let main = rev(&work.join("repos/beta"), "main");
        self.raw_steps
            .iter()
            .map(|step| {
                step.iter()
                    .map(|a| {
                        a.replace("{WORK}", &work.to_string_lossy())
                            .replace("{BETA_MAIN}", &main)
                    })
                    .collect()
            })
            .collect()
    }
}

/// The cases of `cases.tsv`: `name`, `cwd`, steps, and `fresh` as an optional fourth column.
/// A step is separated from the next by `;;`.
pub fn cases() -> Vec<Case> {
    let text = std::fs::read_to_string(golden_dir().join("cases.tsv")).unwrap();
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|line| {
            let mut cols = line.split('\t');
            let (name, cwd, steps) = (
                cols.next().unwrap(),
                cols.next().unwrap(),
                cols.next().unwrap(),
            );
            let flags: Vec<&str> = cols.collect();
            Case {
                name: name.into(),
                cwd: cwd.into(),
                fresh: flags.first() == Some(&"fresh"),
                rows: cols_rows(&flags),
                reflect_rows: flags.contains(&"reflect-rows"),
                raw_steps: steps
                    .split(";;")
                    .map(|step| words(step).into_iter().filter(|a| a != "-").collect())
                    .collect(),
            }
        })
        .collect()
}

/// Splits on white space. A word in double quotes keeps its spaces.
fn words(text: &str) -> Vec<String> {
    let (mut out, mut word, mut quoted, mut started) = (Vec::new(), String::new(), false, false);
    for ch in text.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            c if c.is_whitespace() && !quoted => {
                if started {
                    out.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => {
                word.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(word);
    }
    out
}

fn cols_rows(flags: &[&str]) -> bool {
    flags.get(1) == Some(&"rows")
}

pub fn cwd_of(work: &Path, key: &str) -> PathBuf {
    match key {
        "alpha" | "beta" => work.join("repos").join(key),
        "outside" => work.join("outside"),
        other => panic!("unknown cwd {other}"),
    }
}

/// The `PATH` of a golden run: the fake `claude`, `gh` and code graph come first.
pub fn path_of(work: &Path) -> std::ffi::OsString {
    let old = std::env::var_os("PATH").unwrap_or_default();
    let dirs = std::iter::once(work.join("bin")).chain(std::env::split_paths(&old));
    std::env::join_paths(dirs).unwrap()
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name();
        // `.claude/` is ignored by the repo, so the fixture keeps it as `dot-claude/`.
        let target = if name == "dot-claude" {
            to.join(".claude")
        } else {
            to.join(&name)
        };
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn git(dir: &Path, args: &[&str]) {
    git_at(dir, args, None);
}

/// The branch of the ready story S001 of `beta`.
const STORY_BRANCH: &str = "story/S001-filter-mail";

/// The time of `GIT_ENV`, as seconds since the epoch.
const GIT_TIME: u64 = 4_070_944_800;

/// `git`, with the author and commit date `secs_ago` seconds before the fixed time of
/// `GIT_ENV`. Commits in one second would tie in "the last commit", and the two engines
/// could pick another one. The time is fixed, so the commit hashes repeat.
fn git_at(dir: &Path, args: &[&str], secs_ago: Option<u64>) {
    let mut command = Command::new("git");
    command
        .args([
            "-c",
            "user.name=Golden",
            "-c",
            "user.email=golden@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .envs(GIT_ENV);
    if let Some(ago) = secs_ago {
        let date = format!("@{} +0000", GIT_TIME - ago);
        command
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", &date);
    }
    let status = command.status().expect("git runs");
    assert!(status.success(), "git {args:?}");
}

/// The full hash of `rev` in the repo `dir`.
pub fn rev(dir: &Path, rev: &str) -> String {
    let out = Command::new("git")
        .args(["rev-parse", rev])
        .current_dir(dir)
        .output()
        .expect("git runs");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// `{{WORK}}` in each file under `dir` is the work folder.
fn fill_work(dir: &Path, work: &str) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            fill_work(&path, work);
        } else {
            let text = std::fs::read_to_string(&path).unwrap();
            std::fs::write(&path, text.replace("{{WORK}}", work)).unwrap();
        }
    }
}

/// The git variables of a run. The data root is a git repo, and a command that
/// writes the registry commits it. `make-golden.sh` sets the same ones. The
/// fixed dates make each commit hash the same in each run, so a golden file
/// can hold one (`story result`, `story accept`). The date is in the future, so
/// the commits stay in the 14 days of the snapshot (`git log --since`).
pub const GIT_ENV: [(&str, &str); 8] = [
    ("GIT_AUTHOR_NAME", "Golden"),
    ("GIT_AUTHOR_EMAIL", "golden@example.invalid"),
    ("GIT_COMMITTER_NAME", "Golden"),
    ("GIT_COMMITTER_EMAIL", "golden@example.invalid"),
    ("GIT_AUTHOR_DATE", "2099-01-01T10:00:00Z"),
    ("GIT_COMMITTER_DATE", "2099-01-01T10:00:00Z"),
    ("GIT_CONFIG_GLOBAL", "/dev/null"),
    ("GIT_CONFIG_SYSTEM", "/dev/null"),
];

fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // The Rust core copies the database and the registry into `backups/` once a day. That is not an effect of a command.
        if name == ".git" || name == "backups" || name.starts_with("agentsi.db") {
            continue;
        }
        if path.is_dir() {
            walk(&path, base, out);
        } else {
            out.push(
                path.strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
}

/// A step of the harness, not of the CLI: it shows a change on disk. It runs
/// the same way after the TypeScript run and after the Rust run.
///
/// - `@registry`: the text of `agentsi.config.json`;
/// - `@commits`: the subjects of the commits of the data root, newest first;
/// - `@story-branch`: makes the branch of the ready story S001 in the repo of `beta`;
/// - `@tree`: the files under `root` and `repos`, with no `.git`, no `backups` and no database file;
/// - `@claude-calls`: each call that the fake `claude` got, in the order of its key;
/// - `@meeting-rows`: the rows that a meeting writes. The script reads them from Postgres, the test from SQLite.
pub fn harness(work: &Path, step: &str) -> (i32, String, String) {
    match step {
        "@claude-calls" => (0, claude_calls(work), String::new()),
        "@reflect-rows" => (
            0,
            reflect_rows_text(work, reflect_sqlite_rows(work)),
            String::new(),
        ),
        "@git-hook-fail" => {
            use std::os::unix::fs::PermissionsExt as _;
            let hook = work.join("root/.git/hooks/pre-commit");
            std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
            std::fs::write(&hook, "#!/bin/sh\necho 'the hook says no' >&2\nexit 1\n").unwrap();
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
            (0, String::new(), String::new())
        }
        _ if step.starts_with("@scenario-") => {
            std::fs::write(work.join("claude-scenario"), &step["@scenario-".len()..]).unwrap();
            (0, String::new(), String::new())
        }
        _ if step.starts_with("@sql-") => {
            load_sql(work, &step["@sql-".len()..]);
            (0, String::new(), String::new())
        }
        "@meeting-rows" => (
            0,
            meeting_rows_text(work, meeting_sqlite_rows(work)),
            String::new(),
        ),
        "@registry" => (
            0,
            std::fs::read_to_string(work.join("root/agentsi.config.json")).unwrap_or_default(),
            String::new(),
        ),
        "@commits" => {
            let out = Command::new("git")
                .args(["log", "--format=%s"])
                .current_dir(work.join("root"))
                .output()
                .expect("git runs");
            (
                0,
                String::from_utf8_lossy(&out.stdout).into_owned(),
                String::new(),
            )
        }
        "@story-branch" => {
            let head = std::fs::read_to_string(work.join("story-branch.txt")).unwrap();
            git(&work.join("repos/beta"), &["branch", STORY_BRANCH, &head]);
            (0, String::new(), String::new())
        }
        "@tree" => {
            let mut files = Vec::new();
            for top in ["root", "repos"] {
                walk(&work.join(top), work, &mut files);
            }
            files.sort();
            (0, files.join("\n") + "\n", String::new())
        }
        other => panic!("unknown harness step {other}"),
    }
}

/// Makes the work folder: the data root with the registry, the repos, `data.sql`
/// with the work path in it, and `root/agentsi.db` loaded from the same SQL.
pub fn prepare(work: &Path) {
    let fixture = golden_dir().join("fixture");
    copy_dir(&fixture.join("root"), &work.join("root"));
    copy_dir(&fixture.join("repos"), &work.join("repos"));
    copy_dir(&fixture.join("stages"), &work.join("stages"));
    copy_dir(&fixture.join("bin"), &work.join("bin"));
    copy_dir(&fixture.join("replies"), &work.join("replies"));
    copy_dir(&fixture.join("minutes"), &work.join("minutes"));
    std::fs::create_dir_all(work.join("outside")).unwrap();
    let w = work.to_string_lossy();
    let registry = work.join("root/agentsi.config.json");
    let text = std::fs::read_to_string(&registry)
        .unwrap()
        .replace("{{WORK}}", &w);
    std::fs::write(&registry, text).unwrap();
    fill_work(&work.join("stages"), &w);
    fill_work(&work.join("minutes"), &w);
    let root = work.join("root");
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "chore: the registry"]);
    let beta = work.join("repos/beta");
    git(&beta, &["init", "-q", "-b", "main"]);
    git(&beta, &["add", "-A"]);
    git_at(
        &beta,
        &["commit", "-q", "-m", "feat: first route"],
        Some(180),
    );
    std::fs::write(
        beta.join("docs/plans/launch.md"),
        "# Launch plan\n\nShip on a Friday, after the review.\n",
    )
    .unwrap();
    git_at(
        &beta,
        &["commit", "-q", "-am", "docs: write the launch plan"],
        Some(120),
    );
    std::fs::write(beta.join("notes.txt"), "bookkeeping\n").unwrap();
    git(&beta, &["add", "notes.txt"]);
    git_at(
        &beta,
        &["commit", "-q", "-m", "minutes(beta-001): add the minutes"],
        Some(60),
    );
    // The branch of the ready story S001, with its tests commit and its work commit.
    git(&beta, &["checkout", "-q", "-b", STORY_BRANCH]);
    std::fs::create_dir_all(beta.join("test")).unwrap();
    for n in [1, 2] {
        std::fs::write(
            beta.join(format!("test/list-{n}.test.sh")),
            format!("test \"$(sh src/filter.sh {n})\" = {n}\n"),
        )
        .unwrap();
    }
    git(&beta, &["add", "test"]);
    git_at(
        &beta,
        &["commit", "-q", "-m", "test(S001): criteria tests"],
        Some(40),
    );
    let tests = rev(&beta, "HEAD");
    std::fs::create_dir_all(beta.join("src")).unwrap();
    std::fs::write(beta.join("src/filter.sh"), "echo \"$1\"\n").unwrap();
    git(&beta, &["add", "src"]);
    git_at(
        &beta,
        &["commit", "-q", "-m", "feat(S001): Filter mail"],
        Some(20),
    );
    let story = rev(&beta, "HEAD");
    git(&beta, &["checkout", "-q", "main"]);
    // The indexer reads each branch, so the commits stay out of the refs until `@story-branch`.
    git(&beta, &["branch", "-q", "-D", STORY_BRANCH]);
    std::fs::write(work.join("story-branch.txt"), &story).unwrap();
    let main = rev(&beta, "main");

    let sql = std::fs::read_to_string(fixture.join("data.sql"))
        .unwrap()
        .replace("{{WORK}}", &w)
        .replace("{{BETA_MAIN}}", &main)
        .replace("{{BETA_STORY_TESTS}}", &tests)
        .replace("{{BETA_STORY}}", &story);
    std::fs::write(work.join("data.sql"), &sql).unwrap();

    let conn = agentsi_store::db::open_writer(&work.join("root/agentsi.db")).unwrap();
    conn.execute_batch(&sql).unwrap();
}

/// A case with `rows` is compared on its rows too. These are the tables that a
/// write changes. A row is a JSON object. Both engines give the same shape:
/// Postgres through `rows.sql`, SQLite through `sqlite_rows`.
pub const ROWS_TABLES: [&str; 4] = ["minutes", "priorities", "decisions", "archive_records"];

/// The rows of a SQLite database file, in the shape of `rows.sql`.
pub fn sqlite_rows(db: &Path) -> serde_json::Value {
    use rusqlite::{Connection, OpenFlags};
    use serde_json::{Map, Value, json};
    let c = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let query = |sql: &str, f: &dyn Fn(&rusqlite::Row) -> Value| -> Value {
        let mut stmt = c.prepare(sql).unwrap();
        let rows = stmt.query_map([], |r| Ok(f(r))).unwrap();
        Value::Array(rows.map(|r| r.unwrap()).collect())
    };
    let parsed = |text: Option<String>| -> Value {
        text.map_or(Value::Null, |t| serde_json::from_str(&t).unwrap())
    };
    let mut out = Map::new();
    out.insert(
        "minutes".into(),
        query(
            "select id, data from minutes order by id",
            &|r| json!({"id": r.get::<_, String>(0).unwrap(), "data": parsed(r.get(1).unwrap())}),
        ),
    );
    out.insert(
        "priorities".into(),
        query(
            "select minutes_id, idx, mark, is_new from priorities order by minutes_id, idx",
            &|r| {
                json!({"minutes_id": r.get::<_, String>(0).unwrap(), "idx": r.get::<_, i64>(1).unwrap(),
                       "mark": r.get::<_, Option<String>>(2).unwrap(),
                       "is_new": r.get::<_, Option<i64>>(3).unwrap().map(|v| v != 0)})
            },
        ),
    );
    out.insert(
        "decisions".into(),
        query(
            "select id, product, minutes_id, text, why, status, decided_at, kind, reflection_id, action from decisions order by id",
            &|r| {
                json!({"id": r.get::<_, i64>(0).unwrap(), "product": r.get::<_, String>(1).unwrap(),
                       "minutes_id": r.get::<_, Option<String>>(2).unwrap(), "text": r.get::<_, Option<String>>(3).unwrap(),
                       "why": r.get::<_, Option<String>>(4).unwrap(), "status": r.get::<_, Option<String>>(5).unwrap(),
                       "decided_at": r.get::<_, Option<String>>(6).unwrap(), "kind": r.get::<_, String>(7).unwrap(),
                       "reflection_id": r.get::<_, Option<String>>(8).unwrap(), "action": r.get::<_, Option<String>>(9).unwrap()})
            },
        ),
    );
    out.insert(
        "archive_records".into(),
        query(
            "select product, kind, ref from archive_records order by product, kind, ref",
            &|r| {
                json!({"product": r.get::<_, String>(0).unwrap(), "kind": r.get::<_, String>(1).unwrap(),
                       "ref": r.get::<_, String>(2).unwrap()})
            },
        ),
    );
    Value::Object(out)
}

/// The text of a `.rows` file: the rows with sorted keys and the one
/// normalization of time. A `decided_at` after the fixture dates (all are
/// before 2026-10-31) was set by the run, so it is `<NOW>`; an earlier one is
/// written as `Date.toISOString()` writes it, because Postgres and SQLite
/// print an instant in other ways.
pub fn rows_text(rows: serde_json::Value) -> String {
    use serde_json::Value;
    const FIXTURE_END_MS: i64 = 1_793_404_800_000; // 2026-10-31T00:00:00Z
    fn walk(v: Value) -> Value {
        match v {
            Value::Object(m) => {
                let mut keys: Vec<String> = m.keys().cloned().collect();
                keys.sort();
                let mut out = serde_json::Map::new();
                for k in keys {
                    let child = walk(m[&k].clone());
                    let child = match (k.as_str(), &child) {
                        ("decided_at", Value::String(t)) => {
                            let ms = agentsi_agency::clock::parse_iso_ms(t)
                                .unwrap_or_else(|| panic!("decided_at {t}"));
                            Value::String(if ms > FIXTURE_END_MS {
                                "<NOW>".into()
                            } else {
                                agentsi_agency::clock::iso(ms)
                            })
                        }
                        _ => child,
                    };
                    out.insert(k, child);
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(a.into_iter().map(walk).collect()),
            other => other,
        }
    }
    let mut text = serde_json::to_string_pretty(&walk(rows)).unwrap();
    text.push('\n');
    text
}

/// The calls that the fake `claude` wrote. The TypeScript code asks for `json` and
/// the Rust code for `stream-json` with `--verbose`. Everything else must be equal.
fn claude_calls(work: &Path) -> String {
    let dir = work.join("claude-calls");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .map(|entries| entries.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default();
    names.retain(|p: &PathBuf| p.extension().is_some_and(|e| e == "txt"));
    names.sort();
    let mut out = String::new();
    for file in names {
        let name = file.file_stem().unwrap().to_string_lossy();
        out.push_str(&format!("=== {name}\n"));
        let text = std::fs::read_to_string(&file).unwrap();
        let mut lines = text.lines().peekable();
        while let Some(line) = lines.next() {
            if line == "arg: --verbose" {
                continue;
            }
            if line == "arg: --output-format" {
                lines.next();
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Sorts the keys of a JSON value at each level.
fn sorted(v: &serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|k| (k.clone(), sorted(&map[k])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// The rows of a meeting in a shape that both engines give, as text. These
/// fields differ between runs, and so are replaced:
///
/// 1. the date part of a new minutes id;
/// 2. a time after 2026-10-31, which only a run makes;
/// 3. the id of a new memory item;
/// 4. `duration_ms` of an agent in the log line, which the clock of the run gives;
/// 5. what `normalize` replaces, and the hash of a commit.
pub fn meeting_rows_text(work: &Path, rows: serde_json::Value) -> String {
    let mut rows = sorted(&rows);
    for line in rows["meeting_log"].as_array_mut().unwrap() {
        let agents = line["entry"]["agents"].as_object_mut().unwrap();
        for agent in agents.values_mut() {
            agent["duration_ms"] = serde_json::json!(0);
        }
    }
    let new_ids: Vec<String> = rows["memory"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap().to_string())
        .collect();
    let mut text = serde_json::to_string_pretty(&rows).unwrap();
    for id in &new_ids {
        text = text.replace(id, "<MEMORY-ID>");
    }
    let time = Regex::new(r"20\d\d-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z").unwrap();
    let text = time.replace_all(&text, |c: &regex::Captures| {
        if &c[0] > "2026-10-31" {
            "<NOW>".to_string()
        } else {
            c[0].to_string()
        }
    });
    let stamp = Regex::new(r"20\d\d-\d\d-\d\d-\d{4}-").unwrap();
    let commit = Regex::new(r"\b[0-9a-f]{40}\b").unwrap();
    let text = stamp.replace_all(&text, "<STAMP>-");
    let text = commit.replace_all(&text, "<HASH>");
    normalize(&text, work) + "\n"
}

/// The rows of `agentsi.db` in the shape of `meeting-rows.sql`.
pub fn meeting_sqlite_rows(work: &Path) -> serde_json::Value {
    use serde_json::{Map, json};
    let c = read_db(work);
    let query = |sql: &str, json_columns: &[&str], booleans: &[&str]| {
        query_rows(&c, sql, json_columns, booleans)
    };
    let mut counts = Map::new();
    for table in MEETING_TABLES {
        let n: i64 = c
            .query_row(&format!("select count(*) from {table}"), [], |r| r.get(0))
            .unwrap();
        counts.insert(table.into(), json!(n));
    }
    json!({
        "counts": counts,
        "minutes": query(
            "select id, product, scope, question, status, created_at, data, usage from minutes where id like '20%' order by id",
            &["data", "usage"], &[]),
        "priorities": query(
            "select minutes_id, idx, title, why, sources, owner_role, objective, mark, is_new from priorities where minutes_id like '20%' order by minutes_id, idx",
            &["sources"], &["is_new"]),
        "decisions": query(
            "select product, minutes_id, text, why, action, status, decided_at, kind, reflection_id from decisions where minutes_id like '20%' order by minutes_id, text",
            &[], &[]),
        "usage": query(
            "select minutes_id, role, model, input_tokens, output_tokens, cache_read, cache_write, cost_usd, duration_ms from usage where minutes_id like '20%' order by minutes_id, role",
            &[], &[]),
        "memory": query(
            "select product, agent, id, position, text, kind, helpful, harmful, source, created_at, retired_at, retired_why from memory where source like '20%' order by product, agent, position",
            &[], &[]),
        "meeting_log": query(
            "select product, minutes_id, entry from meeting_log where minutes_id like '20%' order by minutes_id",
            &["entry"], &[]),
        "snapshots": query(
            "select product, minutes_id, body from snapshots where minutes_id like '20%' order by minutes_id",
            &[], &[]),
        "documents": query(
            "select kind, ref, title, body from documents where (kind = 'minutes' and ref like '20%') or kind = 'memory' order by kind, ref",
            &[], &[]),
    })
}

/// The database of a run, read only.
fn read_db(work: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open_with_flags(
        work.join("root/agentsi.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap()
}

/// The rows of `sql` as a JSON array.
fn query_rows(
    c: &rusqlite::Connection,
    sql: &str,
    json_columns: &[&str],
    booleans: &[&str],
) -> serde_json::Value {
    use rusqlite::types::ValueRef;
    use serde_json::{Map, Value, json};
    let mut stmt = c.prepare(sql).unwrap();
    let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let rows = stmt
        .query_map([], |r| {
            let mut row = Map::new();
            for (i, name) in names.iter().enumerate() {
                let v = match r.get_ref(i).unwrap() {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) if booleans.contains(&name.as_str()) => json!(n != 0),
                    ValueRef::Integer(n) => json!(n),
                    ValueRef::Real(n) => json!(n),
                    ValueRef::Text(t) => {
                        let t = String::from_utf8_lossy(t);
                        if json_columns.contains(&name.as_str()) {
                            serde_json::from_str(&t).unwrap()
                        } else {
                            json!(t)
                        }
                    }
                    ValueRef::Blob(_) => panic!("no blobs in {sql}"),
                };
                row.insert(name.clone(), v);
            }
            Ok(Value::Object(row))
        })
        .unwrap();
    Value::Array(rows.map(|r| r.unwrap()).collect())
}

/// A file of the work folder: this run loaded a file of `fixture/sql/`.
pub const SQL_LOADED: &str = "sql-loaded";

/// Loads `fixture/sql/<name>.sql` into the database of a run. The TypeScript run
/// loads the same file into Postgres.
fn load_sql(work: &Path, name: &str) {
    let file = golden_dir().join(format!("fixture/sql/{name}.sql"));
    let sql = std::fs::read_to_string(&file).unwrap();
    let c = rusqlite::Connection::open(work.join("root/agentsi.db")).unwrap();
    c.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
    c.execute_batch(&sql).unwrap();
    std::fs::write(work.join(SQL_LOADED), "").unwrap();
}

/// The id of the last reflection that is not a revert: `{LAST_REFLECTION}` in a step.
pub fn last_reflection(work: &Path) -> String {
    read_db(work)
        .query_row(
            "select id from reflections where product = 'alpha' and entry->>'reverts' is null
             order by position desc limit 1",
            [],
            |r| r.get(0),
        )
        .unwrap()
}

/// The rows of `agentsi.db` in the shape of `reflect-rows.sql`.
pub fn reflect_sqlite_rows(work: &Path) -> serde_json::Value {
    use serde_json::{Map, json};
    let c = read_db(work);
    let mut counts = Map::new();
    for table in ["minutes", "priorities", "usage", "meeting_log", "snapshots"] {
        let n: i64 = c
            .query_row(&format!("select count(*) from {table}"), [], |r| r.get(0))
            .unwrap();
        counts.insert(table.into(), json!(n));
    }
    json!({
        "counts": counts,
        "memory": query_rows(&c,
            "select product, agent, id, position, text, kind, helpful, harmful, source, created_at, retired_at, retired_why from memory order by product, agent, position",
            &[], &[]),
        "reflections": query_rows(&c,
            "select product, id, position, entry from reflections order by product, position",
            &["entry"], &[]),
        "decisions": query_rows(&c,
            "select id, product, minutes_id, text, why, status, decided_at, kind, reflection_id, action from decisions order by id",
            &[], &[]),
        "overlays": query_rows(&c,
            "select product, agent, body from agent_overlays order by product, agent",
            &[], &[]),
    })
}

/// The text of `@reflect-rows` and of a `.rows` file of a reflection. Beyond `rows_text`,
/// `normalize` replaces what a run makes: the id of a reflection, the id of a new item
/// and a time after 2026-10-31.
pub fn reflect_rows_text(work: &Path, rows: serde_json::Value) -> String {
    normalize(&rows_text(rows), work)
}

/// The tables that a meeting writes. `@meeting-rows` prints the count of each one.
const MEETING_TABLES: [&str; 8] = [
    "minutes",
    "priorities",
    "decisions",
    "usage",
    "memory",
    "meeting_log",
    "snapshots",
    "documents",
];

/// The one normalization. It makes the fields equal that differ between runs:
/// 1. the work folder is `<WORK>`;
/// 2. the hash and the date of a commit line of the snapshot;
/// 3. the seconds of the index line of `desk` and `index`.
/// 4. `generated_at` of the audit.
/// 5. the date of the last commit in `status`: a commit of the run has the date of the run.
/// 6. the minutes since the last index in `status`: the TypeScript CLI is slower than the Rust CLI.
/// 7. in the MCP text, the hash of a commit and the time of `updated=`, which come from the run.
/// 8. in the output of `meet`, the date part of a new minutes id and the seconds of a run.
/// 9. the id of a reflection, the id of a new memory item and a time after 2026-10-31.
/// 10. the checksum in the name of a call of a reflection. The order of the names stays.
pub fn normalize(text: &str, work: &Path) -> String {
    let text = text.replace(&*work.to_string_lossy(), "<WORK>");
    let reflection = Regex::new(r"\br-20\d\d-\d\d-\d\d-\d{4}(-\d+)?\b").unwrap();
    let text = reflection.replace_all(&text, "r-<STAMP>");
    // A new id is random, so its text cannot tell it from a fixture id such as `m-0001`.
    // After `@sql-*` the only ids of hexadecimal digits are new ones.
    let text = if work.join(SQL_LOADED).exists() {
        let item = Regex::new(r"\bm-[0-9a-f]{4,6}\b").unwrap();
        item.replace_all(&text, "m-<NEW>").into_owned()
    } else {
        text.into_owned()
    };
    let time = Regex::new(r"20\d\d-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z").unwrap();
    let text = time.replace_all(&text, |c: &regex::Captures| {
        if &c[0] > "2026-10-31" && &c[0] < "2099" {
            "<NOW>".to_string()
        } else {
            c[0].to_string()
        }
    });
    let sum = Regex::new(r"(?m)^=== ([a-z.-]+)-\d+$").unwrap();
    let text = sum.replace_all(&text, "=== $1");
    let stamp = Regex::new(r"20\d\d-\d\d-\d\d-\d{4}-").unwrap();
    let run_seconds = Regex::new(r"(finished in|FAILED after) \d+\.\ds").unwrap();
    let total = Regex::new(r"(?m)^(Cost: \$\d+\.\d{4}) in \d+s$").unwrap();
    let text = stamp.replace_all(&text, "<STAMP>-");
    let text = run_seconds.replace_all(&text, "$1 <SECONDS>s");
    let text = total.replace_all(&text, "$1 in <SECONDS>s");
    let commit = Regex::new(r"(?m)^- [0-9a-f]{7,40} \d{4}-\d{2}-\d{2} ").unwrap();
    let last_commit = Regex::new(r"Last commit: \d{4}-\d{2}-\d{2} ").unwrap();
    let indexed = Regex::new(r"indexed \d+ min ago").unwrap();
    let seconds = Regex::new(r#"(?m), \d+\.\ds("?,?)$"#).unwrap();
    let generated = Regex::new(r#""generated_at": "[^"]*""#).unwrap();
    let text = commit.replace_all(&text, "- <HASH> <DATE> ");
    let text = last_commit.replace_all(&text, "Last commit: <DATE> ");
    let text = indexed.replace_all(&text, "indexed <N> min ago");
    let text = seconds.replace_all(&text, ", <SECONDS>s$1");
    generated
        .replace_all(&text, "\"generated_at\": \"<NOW>\"")
        .into_owned()
}

/// The text of a golden file.
pub fn golden_text(work: &Path, exit: i32, stdout: &str, stderr: &str) -> String {
    format!(
        "exit: {exit}\n--- stdout\n{}\n--- stderr\n{}\n",
        normalize(stdout, work),
        normalize(stderr, work)
    )
}

/// One step of a case with more than one step: the step, then its output.
pub fn step_text(work: &Path, step: &str, text: &str) -> String {
    format!("## {}\n{text}", normalize(step, work))
}

/// One MCP case: the folder to run in and the file of requests.
pub struct McpCase {
    pub name: String,
    pub cwd: String,
    /// One JSON-RPC message on each line.
    pub requests: PathBuf,
}

/// The cases of `mcp-cases.tsv`.
pub fn mcp_cases() -> Vec<McpCase> {
    let dir = golden_dir();
    let text = std::fs::read_to_string(dir.join("mcp-cases.tsv")).unwrap();
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|line| {
            let mut cols = line.split('\t');
            let (name, cwd, file) = (
                cols.next().unwrap(),
                cols.next().unwrap(),
                cols.next().unwrap(),
            );
            McpCase {
                name: name.into(),
                cwd: cwd.into(),
                requests: dir.join("mcp").join(file),
            }
        })
        .collect()
}

/// The object with its keys in byte order, at each depth.
fn canonical(v: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Object(map) => {
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort();
            let mut out = serde_json::Map::new();
            let mut map = map;
            for k in keys {
                let value = map.remove(&k).unwrap();
                out.insert(k, canonical(value));
            }
            Value::Object(out)
        }
        Value::Array(a) => Value::Array(a.into_iter().map(canonical).collect()),
        other => other,
    }
}

/// The text of an MCP answer with the parts removed that depend on the run or on
/// the engine of the search:
/// 1. the snippet of each hit, because Postgres `ts_headline` and FTS5 `snippet` cut other words,
/// 2. the 12-character hash of a commit ref, and the time after `updated=`;
/// 3. the rules of `normalize`.
fn mcp_text(text: &str, work: &Path) -> String {
    let hit = Regex::new(r"^\d+\. \[").unwrap();
    let ref12 = Regex::new(r"ref=[0-9a-f]{12}\b").unwrap();
    let updated = Regex::new(r"updated=\d{4}-\d{2}-\d{2} \d{2}:\d{2}").unwrap();
    let mut out: Vec<String> = Vec::new();
    // After a hit line comes the title, then the snippet if there is one.
    let mut since_hit = usize::MAX;
    for line in text.lines() {
        if hit.is_match(line) {
            since_hit = 0;
        } else {
            since_hit = since_hit.saturating_add(1);
            if since_hit == 2 && line.starts_with("   ") {
                continue;
            }
        }
        out.push(line.to_string());
    }
    let text = out.join("\n");
    let text = ref12.replace_all(&text, "ref=<HASH>");
    let text = updated.replace_all(&text, "updated=<TIME>");
    normalize(&text, work)
}

/// The golden text of the stdout of one MCP session: the answers sorted by id, with
/// sorted keys, one JSON value after another.
pub fn mcp_golden(work: &Path, stdout: &str) -> String {
    use serde_json::Value;
    let mut answers: Vec<Value> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("MCP stdout line {l:?}: {e}")))
        .collect();
    answers.sort_by_key(|a| a["id"].as_i64().unwrap_or(i64::MAX));
    for a in &mut answers {
        if let Some(items) = a
            .pointer_mut("/result/content")
            .and_then(Value::as_array_mut)
        {
            for item in items {
                if let Some(t) = item["text"].as_str() {
                    item["text"] = Value::String(mcp_text(t, work));
                }
            }
        }
    }
    answers
        .into_iter()
        .map(|a| serde_json::to_string_pretty(&canonical(a)).unwrap() + "\n")
        .collect::<Vec<_>>()
        .join("\n")
}

/// One row of `documents`, as both engines give it.
pub struct DocRow {
    pub product: String,
    pub scope: Option<String>,
    pub kind: String,
    pub reference: String,
    pub title: Option<String>,
    pub body: Option<String>,
    pub updated_at: Option<String>,
}

/// The golden text of the `documents` table, one JSON object on each line, sorted.
/// The time of a doc comes from the mtime of a file, and a commit has a hash and a date
/// from the run, so those are replaced.
pub fn documents_golden(work: &Path, rows: Vec<DocRow>) -> String {
    let mut lines: Vec<(String, String, String, String, String)> = rows
        .into_iter()
        .map(|r| {
            let commit = r.kind == "commit";
            let from_run = commit || r.kind == "doc" || r.kind == "journal";
            let value = canonical(serde_json::json!({
                "product": r.product,
                "kind": r.kind,
                "ref": if commit { "<HASH>".to_string() } else { r.reference.clone() },
                "scope": r.scope,
                "title": r.title,
                "body": r.body.map(|b| normalize(&b, work)),
                "updated_at": if from_run { Some("<RUN>".to_string()) } else { r.updated_at },
            }));
            (
                r.product,
                r.kind,
                r.title.unwrap_or_default(),
                r.reference,
                value.to_string(),
            )
        })
        .collect();
    lines.sort();
    let mut out: String = lines.into_iter().map(|l| l.4 + "\n").collect();
    if out.is_empty() {
        out.push('\n');
    }
    out
}
