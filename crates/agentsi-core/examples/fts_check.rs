//! The FTS criterion of `docs/specs/agentsi-desktop-data.md`: for 10 fixed
//! queries, 3 of the first 5 refs of the SQLite search equal the refs of the
//! Postgres search of `packages/core/src/search.ts`.
//!
//! `cargo run -p agentsi-core --example fts_check -- <postgres url> <agentsi.db> [<queries file>]`
//! The queries file has one `<product><TAB><query>` on each line. Without it, the
//! check uses the owner's products of 2026-11-02. It only reads both databases.
//! The exit code is 1 when a query fails.

use agentsi_store::search::{SearchOptions, search};
use std::collections::BTreeSet;
use std::process::ExitCode;

/// Products and queries of the owner's data on 2026-11-02.
const QUERIES: &[(&str, &str)] = &[
    ("agentsi", "voice mode"),
    ("agentsi", "postgres backup"),
    ("agentsi", "story session"),
    ("agentsi", "meeting minutes"),
    ("agentsi", "reflection"),
    ("agentsi", "web ui"),
    ("harbor", "login"),
    ("lumen", "map"),
    ("orchard", "deploy"),
    ("cedar", "landing"),
];

/// The query of `search()` in `packages/core/src/search.ts`, with no kind and a limit of 5.
const PG_SEARCH: &str = "select d.ref from documents d, websearch_to_tsquery('simple', $2) q
    where d.product = $1 and d.tsv @@ q and d.kind <> 'memory'
    order by ts_rank(d.tsv, q) desc, d.updated_at desc limit 5";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (url, db, file) = match args.as_slice() {
        [url, db] => (url, db, None),
        [url, db, file] => (url, db, Some(file)),
        _ => {
            eprintln!("usage: fts_check <postgres url> <agentsi.db> [<queries file>]");
            return ExitCode::from(2);
        }
    };
    let from_file: Vec<(String, String)> = file
        .map(|f| {
            std::fs::read_to_string(f)
                .expect("the queries file")
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
                .filter_map(|l| l.split_once('\t'))
                .map(|(p, q)| (p.to_string(), q.to_string()))
                .collect()
        })
        .unwrap_or_else(|| {
            QUERIES
                .iter()
                .map(|(p, q)| (p.to_string(), q.to_string()))
                .collect()
        });
    let mut pg = postgres::Client::connect(url, postgres::NoTls).expect("the Postgres database");
    let lite =
        rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .expect("the database file");
    let mut passed = 0;
    for (product, query) in &from_file {
        let want: BTreeSet<String> = pg
            .query(PG_SEARCH, &[&product.as_str(), &query.as_str()])
            .expect("the Postgres search")
            .iter()
            .map(|r| r.get(0))
            .collect();
        let opts = SearchOptions {
            limit: Some(5),
            ..Default::default()
        };
        let got: BTreeSet<String> = search(&lite, product, query, &opts)
            .expect("the search")
            .into_iter()
            .map(|h| h.reference)
            .collect();
        let same = want.intersection(&got).count();
        // With fewer than 3 Postgres results, each one must be in the SQLite results.
        let ok = !want.is_empty() && same >= 3.min(want.len());
        passed += usize::from(ok);
        println!(
            "{product} \"{query}\": pg={} same={same} {}",
            want.len(),
            if ok { "ok" } else { "FAIL" }
        );
    }
    println!("passed {passed} of {}", from_file.len());
    if passed == from_file.len() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
