//! Search on FTS5, and the backup with its restore drill.

mod common;

use agentsi_store::backup::{backup, backup_dates, backup_due};
use agentsi_store::search::{SearchOptions, get_doc, search};
use agentsi_store::{Store, read};
use common::{MemSource, i, t};
use rusqlite::Connection;
use std::time::{Duration, SystemTime};

fn doc(
    c: &Connection,
    id: i64,
    kind: &str,
    reference: &str,
    title: &str,
    body: &str,
    updated_at: &str,
) {
    c.execute(
        "insert into documents (id, product, kind, ref, title, body, updated_at) values (?1, 'alpha', ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, kind, reference, title, body, updated_at],
    )
    .unwrap();
}

fn docs() -> Connection {
    let mut c = Connection::open_in_memory().unwrap();
    agentsi_store::db::migrate(&mut c).unwrap();
    doc(
        &c,
        1,
        "doc",
        "docs/voice.md",
        "Voice mode",
        "The pill speaks each reply.",
        "2026-10-01T00:00:00.000Z",
    );
    doc(
        &c,
        2,
        "doc",
        "docs/pill.md",
        "Pill",
        "Voice mode starts the pill. Voice and voice.",
        "2026-10-02T00:00:00.000Z",
    );
    doc(
        &c,
        3,
        "memory",
        "m-1",
        "Voice memory",
        "Voice mode lesson",
        "2026-10-03T00:00:00.000Z",
    );
    doc(
        &c,
        4,
        "commit",
        "abcdef1234567890",
        "fix: voice",
        "Fix the pill",
        "2026-10-04T00:00:00.000Z",
    );
    doc(
        &c,
        5,
        "doc",
        "docs/canción.md",
        "Canción",
        "Música y acción",
        "2026-10-05T00:00:00.000Z",
    );
    c
}

fn refs(c: &Connection, query: &str, opts: &SearchOptions) -> Vec<String> {
    search(c, "alpha", query, opts)
        .unwrap()
        .into_iter()
        .map(|h| h.reference)
        .collect()
}

#[test]
fn search_matches_each_word_ranks_the_title_first_and_skips_memory() {
    let c = docs();
    let all = SearchOptions::default();
    assert_eq!(
        refs(&c, "voice mode", &all),
        vec!["docs/voice.md", "docs/pill.md"]
    );
    assert_eq!(refs(&c, "\"pill speaks\"", &all), vec!["docs/voice.md"]);
    assert_eq!(refs(&c, "speaks or fix", &all).len(), 2);
    let mut without = refs(&c, "voice -speaks", &all);
    without.sort();
    assert_eq!(without, vec!["abcdef1234567890", "docs/pill.md"]);
    assert_eq!(
        refs(
            &c,
            "voice",
            &SearchOptions {
                kind: Some("memory"),
                ..Default::default()
            }
        ),
        vec!["m-1"]
    );
    assert_eq!(
        refs(
            &c,
            "voice",
            &SearchOptions {
                limit: Some(1),
                ..Default::default()
            }
        )
        .len(),
        1
    );
    assert_eq!(
        refs(
            &c,
            "VOICE speaks zzz",
            &SearchOptions {
                any_term: true,
                ..Default::default()
            }
        )
        .len(),
        3
    );
    assert_eq!(
        refs(&c, "cancion musica", &all),
        vec!["docs/canción.md"],
        "diacritics do not count"
    );
    assert!(refs(&c, "\"", &all).is_empty());
    let hit = &search(&c, "alpha", "speaks", &all).unwrap()[0];
    assert!(hit.rank > 0.0);
    assert!(hit.snippet.contains("speaks"), "{}", hit.snippet);
}

#[test]
fn the_fts_triggers_follow_updates_and_deletes() {
    let c = docs();
    c.execute(
        "update documents set body = 'Nothing here' where id = 1",
        [],
    )
    .unwrap();
    assert!(!refs(&c, "speaks", &SearchOptions::default()).contains(&"docs/voice.md".to_string()));
    c.execute("delete from documents where id = 2", []).unwrap();
    assert!(refs(&c, "starts", &SearchOptions::default()).is_empty());
    c.execute(
        "insert into documents_fts (documents_fts) values ('integrity-check')",
        [],
    )
    .unwrap();
}

#[test]
fn get_doc_finds_a_commit_by_a_prefix_and_prefers_the_exact_ref() {
    let c = docs();
    assert_eq!(
        get_doc(&c, "alpha", "commit", "ABCDEF1")
            .unwrap()
            .unwrap()
            .reference,
        "abcdef1234567890"
    );
    assert_eq!(
        get_doc(&c, "alpha", "doc", "docs/pill.md")
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("Pill")
    );
    assert!(get_doc(&c, "alpha", "doc", "docs/pi").unwrap().is_none());
}

fn at(s: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(s)
}

const DAY: u64 = 86_400;
const NOV_2: u64 = 1_793_620_799 - 43_199;

#[test]
fn backup_writes_a_sound_file_and_the_restore_drill_gives_the_same_counts() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("agentsi.config.json"), "{\"products\":[]}").unwrap();
    let db = root.join("agentsi.db");
    let mut src = MemSource::default();
    for n in 0..5 {
        src.add("decisions", &[("id", i(n + 1)), ("product", t("alpha"))]);
        src.add(
            "documents",
            &[
                ("id", i(n + 1)),
                ("product", t("alpha")),
                ("ref", t(&format!("r{n}"))),
            ],
        );
    }
    src.add("products", &[("id", t("alpha"))]);
    assert!(
        agentsi_store::import::import_file(&db, &mut src, &mut |_| {})
            .unwrap()
            .ok()
    );

    let store = Store::open(&db).unwrap();
    assert!(backup_due(root, at(NOV_2)).unwrap());
    let to = root.to_path_buf();
    let result = store
        .with_writer(move |c| backup(c, &to, 14, at(NOV_2)))
        .unwrap();
    assert_eq!(result.file, root.join("backups/agentsi-2026-11-02.db"));
    assert_eq!(
        std::fs::read_to_string(result.registry.unwrap()).unwrap(),
        "{\"products\":[]}"
    );
    assert!(!backup_due(root, at(NOV_2)).unwrap());

    let copy = Connection::open(&result.file).unwrap();
    let check: String = copy
        .query_row("pragma integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
    drop(copy);

    // The drill: restore the backup into an empty database, then compare.
    let restored = root.join("restored.db");
    {
        let mut empty = Connection::open(&restored).unwrap();
        let from = Connection::open(&result.file).unwrap();
        let backup = rusqlite::backup::Backup::new(&from, &mut empty).unwrap();
        backup.run_to_completion(64, Duration::ZERO, None).unwrap();
    }
    let source = store.read(read::counts).unwrap();
    let drilled = read::counts(&Connection::open(&restored).unwrap()).unwrap();
    assert_eq!(drilled, source);
    assert_eq!(source.iter().find(|(t, _)| *t == "decisions").unwrap().1, 5);
}

#[test]
fn backup_keeps_the_newest_backups_and_their_registry_copies() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::write(root.join("agentsi.config.json"), "{}").unwrap();
    std::fs::create_dir_all(root.join("backups")).unwrap();
    std::fs::write(
        root.join("backups/agentsi-2026-10-01-0300.sql.gz"),
        "old dump",
    )
    .unwrap();
    let store = Store::open(&root.join("agentsi.db")).unwrap();
    for day in 0..4 {
        let root = root.clone();
        store
            .with_writer(move |c| backup(c, &root, 2, at(NOV_2 + day * DAY)))
            .unwrap();
    }
    assert_eq!(
        backup_dates(&root).unwrap(),
        vec!["2026-11-04", "2026-11-05"]
    );
    let mut names: Vec<String> = std::fs::read_dir(root.join("backups"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "agentsi-2026-10-01-0300.sql.gz",
            "agentsi-2026-11-04.db",
            "agentsi-2026-11-05.db",
            "agentsi.config-2026-11-04.json",
            "agentsi.config-2026-11-05.json",
        ],
        "the Postgres dumps of the TypeScript CLI stay"
    );
    assert!(
        store
            .with_writer(move |c| backup(c, &root, 0, at(NOV_2)))
            .is_err()
    );
}
