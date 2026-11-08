//! The rules of the import, with a source in memory: insert only the rows
//! that the database lacks, never replace a row, rename a minutes id that
//! another product holds, and count each table.

mod common;

use agentsi_store::import::{Report, import, import_file};
use agentsi_store::schema::table;
use agentsi_store::{Store, read};
use common::{MemSource, i, select_all, t};
use rusqlite::Connection;
use rusqlite::types::Value;
use std::path::Path;

const ID: &str = "2026-10-20-0900-first";

fn run(path: &Path, source: &mut MemSource) -> (Report, Vec<String>) {
    let mut lines = Vec::new();
    let report = import_file(path, source, &mut |l| lines.push(l.to_string())).unwrap();
    (report, lines)
}

/// A meeting of `product` with its rows in each table that names a minutes.
fn meeting(src: &mut MemSource, product: &str, id: &str, question: &str) {
    let data = format!(r#"{{"id":"{id}","product":"{product}","question":"{question}"}}"#);
    let entry = format!(r#"{{"minutes_id":"{id}","cost_usd":0.1}}"#);
    src.add(
        "minutes",
        &[
            ("id", t(id)),
            ("product", t(product)),
            ("question", t(question)),
            ("data", t(&data)),
        ],
    )
    .add(
        "priorities",
        &[
            ("minutes_id", t(id)),
            ("idx", i(0)),
            ("title", t(&format!("priority of {product}"))),
        ],
    )
    .add(
        "usage",
        &[
            ("minutes_id", t(id)),
            ("role", t("pm")),
            ("cost_usd", Value::Real(0.25)),
        ],
    )
    .add(
        "decisions",
        &[
            ("id", i(if product == "alpha" { 1 } else { 2 })),
            ("product", t(product)),
            ("minutes_id", t(id)),
        ],
    )
    .add(
        "meeting_log",
        &[
            ("product", t(product)),
            ("minutes_id", t(id)),
            ("position", i(1)),
            ("entry", t(&entry)),
        ],
    )
    .add(
        "snapshots",
        &[
            ("product", t(product)),
            ("minutes_id", t(id)),
            ("body", t(&format!("snapshot of {product}"))),
        ],
    )
    .add(
        "listens",
        &[
            ("product", t(product)),
            ("minutes_id", t(id)),
            ("at", t("2026-10-22T10:10:10.000Z")),
        ],
    )
    .add(
        "archive_records",
        &[
            ("product", t(product)),
            ("kind", t("minutes")),
            ("ref", t(id)),
        ],
    )
    .add(
        "documents",
        &[
            ("id", i(if product == "alpha" { 100 } else { 200 })),
            ("product", t(product)),
            ("kind", t("minutes")),
            ("ref", t(id)),
        ],
    );
}

fn full_source() -> MemSource {
    let mut src = MemSource::default();
    for name in agentsi_store::schema::TABLES.iter().map(|t| t.name) {
        if ![
            "minutes",
            "priorities",
            "usage",
            "decisions",
            "meeting_log",
            "snapshots",
            "listens",
            "archive_records",
            "documents",
        ]
        .contains(&name)
        {
            src.add(name, &[]);
        }
    }
    meeting(&mut src, "alpha", ID, "first question");
    src
}

fn dump(path: &Path) -> String {
    let c = Connection::open(path).unwrap();
    agentsi_store::schema::TABLES
        .iter()
        .map(|t| format!("{}={:?}\n", t.name, select_all(&c, t)))
        .collect()
}

#[test]
fn the_import_inserts_each_row_and_a_second_run_inserts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agentsi.db");
    let mut src = full_source();
    let (first, _) = run(&db, &mut src);
    assert!(first.ok(), "{:?}", first.tables);
    for t in &first.tables {
        assert_eq!((t.source, t.sqlite, t.inserted), (1, 1, 1), "{}", t.table);
        assert_eq!(t.line(), format!("{} pg=1 sqlite=1 ok", t.table));
    }
    let before = dump(&db);
    let inode = std::fs::metadata(&db)
        .map(|m| std::os::unix::fs::MetadataExt::ino(&m))
        .unwrap();
    let (second, _) = run(&db, &mut src);
    assert!(second.ok());
    assert_eq!(second.inserted(), 0);
    assert_eq!(dump(&db), before, "the second run changes no row");
    let after = std::fs::metadata(&db)
        .map(|m| std::os::unix::fs::MetadataExt::ino(&m))
        .unwrap();
    assert_eq!(
        inode, after,
        "with nothing to insert, the file stays in place"
    );
    assert!(!agentsi_store::import::work_file(&db).exists());
}

#[test]
fn the_import_leaves_a_newer_row_in_sqlite_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agentsi.db");
    {
        let store = Store::open(&db).unwrap();
        store
            .write(|tx| {
                tx.execute(
                    "insert into stories (product, id, data) values ('alpha', 'S001', '{\"status\":\"implemented\"}')",
                    [],
                )?;
                tx.execute("insert into decisions (id, product, text, status) values (1, 'alpha', 'newer', 'approved')", [])?;
                tx.execute("insert into usage (minutes_id, role, cost_usd) values (?1, 'pm', 9.0)", [ID])?;
                Ok(())
            })
            .unwrap();
    }
    let mut src = full_source();
    src.add(
        "stories",
        &[
            ("product", t("alpha")),
            ("id", t("S001")),
            ("data", t("{\"status\":\"not_implemented\"}")),
        ],
    );
    let (report, _) = run(&db, &mut src);
    assert!(report.ok(), "{:?}", report.tables);
    let c = Connection::open(&db).unwrap();
    let story = read::story(&c, "alpha", "S001").unwrap().unwrap();
    assert_eq!(story["status"], "implemented", "the newer story stays");
    let decisions = read::decisions(&c, "alpha", None).unwrap();
    assert_eq!(decisions.len(), 1);
    assert_eq!(
        (decisions[0].text.as_deref(), decisions[0].status.as_deref()),
        (Some("newer"), Some("approved"))
    );
    let usage = read::usage(&c, ID).unwrap();
    assert_eq!(
        usage.len(),
        1,
        "a minutes with usage rows gets none from the source"
    );
    assert_eq!(usage[0].cost_usd, Some(9.0));
}

#[test]
fn a_minutes_id_that_another_product_holds_becomes_id_dash_product_with_its_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agentsi.db");
    let mut alpha = MemSource::default();
    meeting(&mut alpha, "alpha", ID, "question of alpha");
    assert!(run(&db, &mut alpha).0.ok());

    let mut beta = MemSource::default();
    meeting(&mut beta, "beta", ID, "question of beta");
    let (report, lines) = run(&db, &mut beta);
    assert!(report.ok(), "{:?}", report.tables);
    let new = format!("{ID}-beta");
    assert_eq!(report.renames.len(), 1);
    assert_eq!(
        (
            report.renames[0].from.as_str(),
            report.renames[0].to.as_str()
        ),
        (ID, new.as_str())
    );
    assert_eq!(
        lines,
        vec![format!(
            "minutes {ID} of beta: alpha holds this id, so the import uses {new}"
        )]
    );

    let c = Connection::open(&db).unwrap();
    let ids: Vec<(String, String)> = c
        .prepare("select id, product from minutes order by id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(
        ids,
        vec![
            (ID.to_string(), "alpha".to_string()),
            (new.clone(), "beta".to_string())
        ]
    );
    assert_eq!(
        read::minutes(&c, &new).unwrap().unwrap()["id"],
        new.as_str(),
        "the JSON takes the new id"
    );
    assert_eq!(
        read::minutes(&c, ID).unwrap().unwrap()["question"],
        "question of alpha"
    );
    assert_eq!(
        read::snapshot(&c, "beta", &new).unwrap().as_deref(),
        Some("snapshot of beta")
    );
    assert_eq!(read::snapshot(&c, "beta", ID).unwrap(), None);
    assert_eq!(read::listens(&c, "beta").unwrap()[0].minutes_id, new);
    let log = read::meeting_log(&c, "beta").unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(
        log[0]["minutes_id"],
        new.as_str(),
        "the log line takes the new id"
    );
    assert_eq!(
        read::priorities(&c, &new).unwrap()[0].title.as_deref(),
        Some("priority of beta")
    );
    assert_eq!(
        read::priorities(&c, ID).unwrap()[0].title.as_deref(),
        Some("priority of alpha")
    );
    assert_eq!(read::usage(&c, &new).unwrap().len(), 1);
    assert_eq!(
        read::decisions(&c, "beta", None).unwrap()[0]
            .minutes_id
            .as_deref(),
        Some(new.as_str())
    );
    assert_eq!(read::archived(&c, "beta").unwrap()[0].reference, new);
    let doc = agentsi_store::search::get_doc(&c, "beta", "minutes", &new).unwrap();
    assert!(
        doc.is_some(),
        "the minutes document of beta takes the new id"
    );
    assert!(
        agentsi_store::search::get_doc(&c, "alpha", "minutes", ID)
            .unwrap()
            .is_some()
    );
    drop(c);

    let before = dump(&db);
    let (again, lines) = run(&db, &mut beta);
    assert_eq!(again.inserted(), 0, "the second run finds the renamed rows");
    assert_eq!(lines.len(), 1);
    assert_eq!(dump(&db), before);
}

#[test]
fn a_table_with_fewer_rows_than_the_source_fails_and_the_file_stays_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agentsi.db");
    let mut src = full_source();
    assert!(run(&db, &mut src).0.ok());
    let before = dump(&db);

    src.add("products", &[("id", t("second"))]);
    src.extra.insert("decisions", 1);
    let (report, _) = run(&db, &mut src);
    assert!(!report.ok());
    let failed: Vec<String> = report
        .tables
        .iter()
        .filter(|t| !t.ok())
        .map(|t| t.line())
        .collect();
    assert_eq!(failed, vec!["decisions pg=2 sqlite=1 FAIL"]);
    assert_eq!(
        dump(&db),
        before,
        "a failed import moves nothing into place"
    );
    assert!(!agentsi_store::import::work_file(&db).exists());
}

#[test]
fn a_source_row_that_breaks_a_check_stops_the_import_with_no_change() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agentsi.db");
    let mut src = full_source();
    assert!(run(&db, &mut src).0.ok());
    let before = dump(&db);
    src.add("briefs", &[("product", t("beta")), ("data", t("{broken"))]);
    let result = import_file(&db, &mut src, &mut |_| {});
    assert!(result.is_err());
    assert_eq!(dump(&db), before);
}

#[test]
fn the_import_keeps_each_decision_id_and_the_next_id_follows_the_largest() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agentsi.db");
    let mut src = MemSource::default();
    for id in [3, 17, 42] {
        src.add("decisions", &[("id", i(id)), ("product", t("alpha"))]);
    }
    assert!(run(&db, &mut src).0.ok());
    let c = Connection::open(&db).unwrap();
    let ids: Vec<i64> = read::decisions(&c, "alpha", None)
        .unwrap()
        .iter()
        .map(|d| d.id)
        .collect();
    assert_eq!(ids, vec![3, 17, 42]);
    c.execute("insert into decisions (product) values ('alpha')", [])
        .unwrap();
    assert_eq!(c.last_insert_rowid(), 43);
}

#[test]
fn import_runs_inside_a_transaction_of_the_caller() {
    let mut c = Connection::open_in_memory().unwrap();
    agentsi_store::db::migrate(&mut c).unwrap();
    let mut src = full_source();
    let tx = c.transaction().unwrap();
    let report = import(&tx, &mut src, &mut |_| {}).unwrap();
    assert!(report.ok());
    drop(tx);
    let rows = select_all(&c, table("products").unwrap());
    assert!(rows.is_empty(), "a dropped transaction keeps nothing");
}

#[test]
fn a_fresh_import_writes_a_new_file_and_refuses_when_the_file_exists() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agentsi.db");
    let mut src = full_source();
    let report = agentsi_store::import::import_fresh(&db, &mut src, &mut |_| {}).unwrap();
    assert!(report.ok(), "{:?}", report.tables);
    assert_eq!(
        report.inserted(),
        report.tables.iter().map(|t| t.source as u64).sum::<u64>()
    );
    let before = dump(&db);

    let again = agentsi_store::import::import_fresh(&db, &mut src, &mut |_| {});
    let error = again.unwrap_err().to_string();
    assert!(error.contains("exists"), "{error}");
    assert_eq!(dump(&db), before, "a refused fresh import changes nothing");
    assert!(!agentsi_store::import::work_file(&db).exists());
}

#[test]
fn a_failed_fresh_import_leaves_no_database_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("agentsi.db");
    let mut src = full_source();
    src.extra.insert("memory", 1);
    let report = agentsi_store::import::import_fresh(&db, &mut src, &mut |_| {}).unwrap();
    assert!(!report.ok());
    assert!(!db.exists());
    assert!(!agentsi_store::import::work_file(&db).exists());
}
