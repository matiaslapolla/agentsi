//! The schema: one write and read for each table (the 17 tables of Postgres
//! and the local tables), and the checks of the type map.

mod common;

use agentsi_store::Store;
use agentsi_store::schema::{LOCAL_TABLES, TABLES, all, table};
use common::{row, select_all, t};
use rusqlite::types::Value;
use std::collections::BTreeSet;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("agentsi.db")).unwrap();
    (dir, store)
}

fn write_and_read(name: &'static str) {
    let (_dir, store) = store();
    let table = table(name).unwrap();
    let written = row(name, &[]);
    let values = written.clone();
    store
        .write(move |tx| {
            let marks: Vec<String> = (1..=values.len()).map(|k| format!("?{k}")).collect();
            let cols: Vec<&str> = table.columns.iter().map(|c| c.name).collect();
            tx.execute(
                &format!(
                    "insert into \"{name}\" ({}) values ({})",
                    cols.join(", "),
                    marks.join(", ")
                ),
                rusqlite::params_from_iter(values.iter()),
            )?;
            Ok(())
        })
        .unwrap();
    let read = store.read(|c| Ok(select_all(c, table))).unwrap();
    assert_eq!(read, vec![written], "{name} gives back the row it took");
}

macro_rules! table_tests {
    ($($name:ident),* $(,)?) => {
        $(#[test] fn $name() { write_and_read(stringify!($name)); })*

        #[test]
        fn the_tests_cover_each_table() {
            let tested: BTreeSet<&str> = [$(stringify!($name)),*].into_iter().collect();
            let tables: BTreeSet<&str> = all().map(|t| t.name).collect();
            assert_eq!(tested, tables);
            assert_eq!(TABLES.len(), 17);
            assert_eq!(
                LOCAL_TABLES.iter().map(|t| t.name).collect::<Vec<_>>(),
                ["tickets", "desk_sessions", "desk_items"]
            );
        }
    };
}

table_tests!(
    products,
    documents,
    index_state,
    minutes,
    priorities,
    decisions,
    usage,
    stories,
    story_logs,
    memory,
    agent_overlays,
    briefs,
    meeting_log,
    snapshots,
    listens,
    reflections,
    archive_records,
    tickets,
    desk_sessions,
    desk_items,
);

#[test]
fn the_sql_schema_has_the_columns_of_the_table_list() {
    let (_dir, store) = store();
    store
        .read(|c| {
            for table in all() {
                let mut stmt = c.prepare(&format!(
                    "select name from pragma_table_info('{}') order by cid",
                    table.name
                ))?;
                let cols: Vec<String> = stmt
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                let want: Vec<&str> = table.columns.iter().map(|c| c.name).collect();
                assert_eq!(cols, want, "{}", table.name);
                let mut stmt = c.prepare(&format!(
                    "select name from pragma_table_info('{}') where pk > 0 order by pk",
                    table.name
                ))?;
                let key: Vec<String> = stmt
                    .query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?;
                assert_eq!(key, table.key, "{} key", table.name);
            }
            Ok(())
        })
        .unwrap();
}

#[test]
fn the_schema_lists_the_tables_the_fts_table_and_each_btree_index() {
    let (_dir, store) = store();
    let names = |kind: &'static str| {
        store
            .read(move |c| {
                let mut stmt = c.prepare(
                    "select name from sqlite_schema where type = ?1 and name not like 'sqlite_%'
                     and name not like 'documents_fts_%' order by name",
                )?;
                Ok(stmt
                    .query_map([kind], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<BTreeSet<_>>>()?)
            })
            .unwrap()
    };
    let mut tables: BTreeSet<String> = all().map(|t| t.name.to_string()).collect();
    tables.insert("documents_fts".into());
    assert_eq!(names("table"), tables);
    let indexes: BTreeSet<String> = [
        "documents_product_kind_idx",
        "documents_product_scope_idx",
        "minutes_product_idx",
        "decisions_product_status_idx",
        "usage_minutes_idx",
        "memory_product_agent_idx",
        "meeting_log_product_idx",
        "reflections_product_idx",
        "tickets_product_state_idx",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(names("index"), indexes);
    let triggers = store
        .read(|c| {
            Ok(c.query_row(
                "select count(*) from sqlite_schema where type = 'trigger'",
                [],
                |r| r.get::<_, i64>(0),
            )?)
        })
        .unwrap();
    assert_eq!(triggers, 3);
}

#[test]
fn the_checks_of_the_type_map_reject_bad_values() {
    let (_dir, store) = store();
    let bad: Vec<(&'static str, &'static str, Value)> = vec![
        ("stories", "data", t("{not json")),
        ("priorities", "is_new", Value::Integer(2)),
        ("decisions", "status", t("maybe")),
        ("decisions", "kind", t("other")),
        ("minutes", "status", t("unknown")),
        ("documents", "kind", t("note")),
        ("archive_records", "kind", t("other")),
        ("tickets", "kind", t("task")),
        ("tickets", "state", t("closed")),
        ("tickets", "id", t("S001")),
        ("tickets", "author", t("{not json")),
        ("tickets", "links", t("{not json")),
        ("desk_items", "kind", t("other")),
        ("desk_items", "body", t("{not json")),
        ("desk_items", "sent", Value::Integer(2)),
    ];
    for (name, col, value) in bad {
        let table = table(name).unwrap();
        let values = row(name, &[(col, value)]);
        let result = store.write(move |tx| {
            let marks: Vec<String> = (1..=values.len()).map(|k| format!("?{k}")).collect();
            let cols: Vec<&str> = table.columns.iter().map(|c| c.name).collect();
            tx.execute(
                &format!(
                    "insert into \"{name}\" ({}) values ({})",
                    cols.join(", "),
                    marks.join(", ")
                ),
                rusqlite::params_from_iter(values.iter()),
            )?;
            Ok(())
        });
        assert!(result.is_err(), "{name}.{col} takes a bad value");
    }
}

#[test]
fn the_store_sets_wal_foreign_keys_and_the_migration_number() {
    let (dir, store) = store();
    let (mode, version, fk): (String, i64, i64) = store
        .read(|c| {
            Ok((
                c.pragma_query_value(None, "journal_mode", |r| r.get(0))?,
                c.pragma_query_value(None, "user_version", |r| r.get(0))?,
                c.pragma_query_value(None, "foreign_keys", |r| r.get(0))?,
            ))
        })
        .unwrap();
    assert_eq!((mode.as_str(), version, fk), ("wal", 3, 1));
    drop(store);
    // A second open applies nothing and keeps the rows.
    let again = Store::open(&dir.path().join("agentsi.db")).unwrap();
    assert_eq!(
        again
            .read(|c| Ok(c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?))
            .unwrap(),
        3
    );
}

#[test]
fn a_database_of_the_first_migration_opens_with_the_later_migrations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("agentsi.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(agentsi_store::db::MIGRATIONS[0].1)
            .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
        conn.execute("insert into products (id) values ('p')", [])
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    let (version, tickets, desk, products): (i64, i64, i64, i64) = store
        .read(|c| {
            Ok((
                c.pragma_query_value(None, "user_version", |r| r.get(0))?,
                c.query_row("select count(*) from tickets", [], |r| r.get(0))?,
                c.query_row("select count(*) from desk_items", [], |r| r.get(0))?,
                c.query_row("select count(*) from products", [], |r| r.get(0))?,
            ))
        })
        .unwrap();
    assert_eq!((version, tickets, desk, products), (3, 0, 0, 1));
}

#[test]
fn a_reader_cannot_write() {
    let (_dir, store) = store();
    let result = store.read(|c| Ok(c.execute("insert into products (id) values ('x')", [])?));
    assert!(result.is_err());
}
