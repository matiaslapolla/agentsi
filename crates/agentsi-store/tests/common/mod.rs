#![allow(dead_code)]

use agentsi_store::Result;
use agentsi_store::import::Source;
use agentsi_store::schema::{Kind, Table, table};
use rusqlite::types::Value;
use std::collections::HashMap;

pub fn t(s: &str) -> Value {
    Value::Text(s.to_string())
}

pub fn i(n: i64) -> Value {
    Value::Integer(n)
}

/// One valid row of `table`: each column gets a value of its kind, and
/// `with` sets the named ones.
pub fn row(table_name: &str, with: &[(&str, Value)]) -> Vec<Value> {
    let table = table(table_name).expect("a table of the schema");
    table
        .columns
        .iter()
        .map(|c| {
            if let Some((_, v)) = with.iter().find(|(n, _)| *n == c.name) {
                return v.clone();
            }
            match (table.name, c.name) {
                ("documents", "kind") => t("doc"),
                ("minutes", "status") => t("valid"),
                ("priorities", "mark") => t("accepted"),
                ("decisions", "status") => t("pending"),
                ("decisions", "kind") => t("minutes"),
                ("archive_records", "kind") => t("minutes"),
                ("tickets", "id") => t("C001"),
                ("tickets", "kind") => t("note"),
                ("tickets", "state") => t("open"),
                _ => match c.kind {
                    Kind::Text => t(&format!("{}-{}", table.name, c.name)),
                    Kind::Json => t(r#"{"k":[1,"x"]}"#),
                    Kind::Time => t("2026-11-02T10:00:00.000Z"),
                    Kind::Bool => i(1),
                    Kind::Int => i(7),
                    Kind::Real => Value::Real(0.5),
                },
            }
        })
        .collect()
}

/// A source in memory. `extra` adds to the count of a table, to test the
/// `FAIL` line.
#[derive(Default, Clone)]
pub struct MemSource {
    pub rows: HashMap<&'static str, Vec<Vec<Value>>>,
    pub extra: HashMap<&'static str, i64>,
}

impl MemSource {
    pub fn add(&mut self, table_name: &'static str, with: &[(&str, Value)]) -> &mut Self {
        self.rows
            .entry(table_name)
            .or_default()
            .push(row(table_name, with));
        self
    }
}

impl Source for MemSource {
    fn rows(&mut self, table: &'static Table) -> Result<Vec<Vec<Value>>> {
        Ok(self.rows.get(table.name).cloned().unwrap_or_default())
    }

    fn count(&mut self, table: &'static Table) -> Result<i64> {
        Ok(self.rows.get(table.name).map_or(0, |r| r.len() as i64)
            + self.extra.get(table.name).copied().unwrap_or(0))
    }
}

pub fn select_all(c: &rusqlite::Connection, table: &Table) -> Vec<Vec<Value>> {
    let cols: Vec<String> = table
        .columns
        .iter()
        .map(|c| format!("\"{}\"", c.name))
        .collect();
    let mut stmt = c
        .prepare(&format!(
            "select {} from \"{}\" order by rowid",
            cols.join(", "),
            table.name
        ))
        .unwrap();
    let n = table.columns.len();
    stmt.query_map([], |r| {
        (0..n)
            .map(|k| r.get::<_, Value>(k))
            .collect::<rusqlite::Result<Vec<_>>>()
    })
    .unwrap()
    .collect::<rusqlite::Result<_>>()
    .unwrap()
}
