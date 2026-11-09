//! `import-postgres` and `backup` against a real Postgres. The test runs
//! only when `AGENTSI_TEST_PG_URL` names a throwaway server, for example
//! `docker run --rm -d -e POSTGRES_PASSWORD=x -p 55432:5432 postgres:17` and
//! `AGENTSI_TEST_PG_URL=postgres://postgres:x@127.0.0.1:55432/postgres`.
//! It makes its own database on that server and drops it at the end.

use postgres::{Client, NoTls};
use rusqlite::Connection;
use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_agentsi-core");

fn sql_dir() -> std::path::PathBuf {
    // Read at run time: the shared CARGO_TARGET_DIR reuses this binary across worktrees.
    Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../packages/core/sql")
}

const FIXTURE: &str = r#"
insert into products values ('alpha', 'Alpha', '["/tmp/a"]', 'active'), ('beta', 'Beta', null, 'paused');
insert into documents (product, scope, kind, ref, title, body, updated_at) values
  ('alpha', null, 'doc', 'docs/voice.md', 'Voice mode', 'The pill speaks each reply', '2026-10-01 10:00:00.123+02'),
  ('alpha', null, 'commit', 'abc1234', 'fix: pill', 'Fix the pill voice', '2026-10-02 10:00:00+00'),
  ('alpha', null, 'memory', 'm-1', 'Voice memory', 'lesson', '2026-10-03 10:00:00+00');
insert into index_state values ('alpha', 'docs', 'abc1234', '2026-10-04 00:00:00.000456+00');
insert into minutes values ('2026-10-20-0900-a', 'alpha', null, 'What next?', 'valid', '2026-10-20 09:00:00+00', null,
  '{"id": "2026-10-20-0900-a", "product": "alpha"}', '[{"role": "pm", "cost_usd": 0.1}]');
insert into priorities values ('2026-10-20-0900-a', 0, 'Ship', 'why', '["a.md"]', 'pm', 'accepted', true, 'o1'),
  ('2026-10-20-0900-a', 1, 'Test', 'why', '[]', 'qa', null, false, null);
insert into decisions (id, product, minutes_id, text, why, status, decided_at, kind, reflection_id, action) values
  (3, 'alpha', '2026-10-20-0900-a', 'Use SQLite', 'w', 'approved', '2026-10-21 08:00:00+00', 'minutes', null, null),
  (9, 'alpha', null, 'Apply r-1', 'w', 'pending', null, 'reflection', 'r-1', 'apply');
insert into usage values ('2026-10-20-0900-a', 'pm', 'sonnet', 10, 20, 0, 0, 0.123456789, 1000),
  ('2026-10-20-0900-a', 'qa', 'haiku', 1, 2, 3, 4, 0.000000001, 2);
insert into stories values ('alpha', 'S001', '{"id": "S001", "status": "approved"}');
insert into story_logs values ('alpha', 'S001', 'line 1');
insert into memory values ('alpha', 'dev', 'm-aaaa', 0, 'Uses Postgres 17', 'fact', 2, 0, 'min-1', '2026-10-01T00:00:00.000Z', null, null);
insert into agent_overlays values ('alpha', 'dev', '---\nextends: dev\n---\n');
insert into briefs values ('alpha', '{"version": 1}');
insert into meeting_log (product, minutes_id, entry) values ('alpha', '2026-10-20-0900-a', '{"minutes_id": "2026-10-20-0900-a"}');
insert into snapshots values ('alpha', '2026-10-20-0900-a', 'snap');
insert into listens values ('alpha', '2026-10-20-0900-a', '2026-10-22T10:10:10.000Z');
insert into reflections values ('alpha', 'r-1', 0, '{"id": "r-1"}');
insert into archive_records values ('alpha', 'minutes', '2026-10-20-0900-a'), ('alpha', 'product', '');
"#;

struct Db {
    admin: Client,
    name: String,
    url: String,
}

impl Drop for Db {
    fn drop(&mut self) {
        let _ = self.admin.batch_execute(&format!(
            "drop database if exists {} with (force)",
            self.name
        ));
    }
}

fn fresh_db(base: &str) -> Db {
    let mut admin = Client::connect(base, NoTls).expect("the test Postgres");
    let name = format!("agentsi_import_{}", std::process::id());
    admin
        .batch_execute(&format!("drop database if exists {name} with (force)"))
        .unwrap();
    admin
        .batch_execute(&format!("create database {name}"))
        .unwrap();
    let (head, _) = base.rsplit_once('/').unwrap();
    let url = format!("{head}/{name}");
    let mut client = Client::connect(&url, NoTls).unwrap();
    let mut files: Vec<_> = std::fs::read_dir(sql_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    for f in files {
        client
            .batch_execute(&std::fs::read_to_string(f).unwrap())
            .unwrap();
    }
    client.batch_execute(FIXTURE).unwrap();
    Db { admin, name, url }
}

fn core(root: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(BIN)
        .args(args)
        .arg("--data-root")
        .arg(root)
        .env("AGENTSI_CORE_SOCKET", root.join("run/core.sock"))
        .output()
        .unwrap();
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    (out.status.code().unwrap_or(-1), text)
}

/// Each row of each table as text, to show that Postgres does not change.
fn pg_dump(c: &mut Client) -> String {
    agentsi_store::schema::TABLES
        .iter()
        .map(|t| {
            let rows = c
                .query(
                    &format!("select t::text from \"{}\" t order by 1", t.name),
                    &[],
                )
                .unwrap();
            let rows: Vec<String> = rows.iter().map(|r| r.get(0)).collect();
            format!("{}={rows:?}\n", t.name)
        })
        .collect()
}

#[test]
fn import_postgres_copies_each_table_and_a_second_run_inserts_nothing() {
    let Ok(base) = std::env::var("AGENTSI_TEST_PG_URL") else {
        eprintln!("skipped: AGENTSI_TEST_PG_URL is not set");
        return;
    };
    let db = fresh_db(&base);
    let mut pg = Client::connect(&db.url, NoTls).unwrap();
    let before = pg_dump(&mut pg);
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let (code, out) = core(root, &["import-postgres", &db.url]);
    assert_eq!(code, 0, "{out}");
    for t in agentsi_store::schema::TABLES {
        let n: i64 = pg
            .query_one(&format!("select count(*) from \"{}\"", t.name), &[])
            .unwrap()
            .get(0);
        assert!(
            out.contains(&format!("{} pg={n} sqlite={n} ok\n", t.name)),
            "{}: {out}",
            t.name
        );
    }
    assert!(out.contains("rows inserted into"), "{out}");

    let lite = Connection::open(root.join("agentsi.db")).unwrap();
    let ids: Vec<i64> = lite
        .prepare("select id from decisions order by id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let pg_ids: Vec<i64> = pg
        .query("select id from decisions order by id", &[])
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(ids, pg_ids);
    let lite_cost: f64 = lite
        .query_row("select sum(cost_usd) from usage", [], |r| r.get(0))
        .unwrap();
    let pg_cost: f64 = pg
        .query_one("select sum(cost_usd)::float8 from usage", &[])
        .unwrap()
        .get(0);
    assert!((lite_cost - pg_cost).abs() < 1e-6);
    let times: Vec<String> = lite
        .prepare("select updated_at from documents order by id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        times,
        vec![
            "2026-10-01T08:00:00.123Z",
            "2026-10-02T10:00:00.000Z",
            "2026-10-03T10:00:00.000Z"
        ]
    );
    let indexed: String = lite
        .query_row("select indexed_at from index_state", [], |r| r.get(0))
        .unwrap();
    assert_eq!(indexed, "2026-10-04T00:00:00.000456Z", "microseconds stay");
    let flags: Vec<i64> = lite
        .prepare("select is_new from priorities order by idx")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(flags, vec![1, 0]);
    let hits = agentsi_store::search::search(&lite, "alpha", "pill", &Default::default()).unwrap();
    assert_eq!(
        hits.iter()
            .map(|h| h.reference.as_str())
            .collect::<Vec<_>>(),
        vec!["abc1234", "docs/voice.md"]
    );
    drop(lite);

    let (code, out) = core(root, &["import-postgres", &db.url]);
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("import: 0 rows inserted"), "{out}");
    let (code, out) = core(root, &["import-postgres", &db.url, "--fresh"]);
    assert_eq!(code, 1, "{out}");
    assert!(
        out.contains("exists. --fresh writes only a new database file"),
        "{out}"
    );

    let fresh_dir = tempfile::tempdir().unwrap();
    let (code, out) = core(fresh_dir.path(), &["import-postgres", "--fresh", &db.url]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(out.matches(" ok\n").count(), 17, "{out}");
    assert_eq!(pg_dump(&mut pg), before, "Postgres does not change");

    // The restore drill: back up, restore into an empty database, compare with Postgres.
    std::fs::write(root.join("agentsi.config.json"), "{\"products\":[]}").unwrap();
    let (code, out) = core(root, &["backup"]);
    assert_eq!(code, 0, "{out}");
    let file = out
        .lines()
        .find_map(|l| l.strip_prefix("backup: "))
        .unwrap()
        .to_string();
    let copy = Connection::open(&file).unwrap();
    assert_eq!(
        copy.query_row("pragma integrity_check", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "ok"
    );
    let mut restored = Connection::open(root.join("restored.db")).unwrap();
    rusqlite::backup::Backup::new(&copy, &mut restored)
        .unwrap()
        .run_to_completion(64, std::time::Duration::ZERO, None)
        .unwrap();
    for (table, n) in agentsi_store::read::counts(&restored).unwrap() {
        if agentsi_store::schema::LOCAL_TABLES.iter().any(|t| t.name == table) {
            assert_eq!(n, 0, "{table}");
            continue;
        }
        let pg_n: i64 = pg
            .query_one(&format!("select count(*) from \"{table}\""), &[])
            .unwrap()
            .get(0);
        assert_eq!(n, pg_n, "{table}");
    }
}

#[test]
fn import_postgres_refuses_to_run_while_a_core_holds_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("run")).unwrap();
    let lock = std::fs::File::create(root.join("run/core.lock")).unwrap();
    lock.try_lock().unwrap();
    let (code, out) = core(
        root,
        &["import-postgres", "postgres://nobody@127.0.0.1:1/none"],
    );
    assert_eq!(code, 1);
    assert!(out.contains("a core runs on"), "{out}");
    assert!(!root.join("agentsi.db").exists());
}

#[test]
fn backup_without_a_database_file_fails() {
    let dir = tempfile::tempdir().unwrap();
    let (code, out) = core(dir.path(), &["backup"]);
    assert_eq!(code, 1);
    assert!(out.contains("does not exist"), "{out}");
}

#[test]
fn import_postgres_fresh_refuses_an_existing_file_before_it_connects() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("agentsi.db"), "keep").unwrap();
    let (code, out) = core(
        root,
        &[
            "import-postgres",
            "--fresh",
            "postgres://nobody@127.0.0.1:1/none",
        ],
    );
    assert_eq!(code, 1);
    assert!(
        out.contains("exists. --fresh writes only a new database file"),
        "{out}"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("agentsi.db")).unwrap(),
        "keep"
    );
    let (code, _) = core(root, &["import-postgres", "--fresh"]);
    assert_eq!(code, 2, "a url is necessary");
}
