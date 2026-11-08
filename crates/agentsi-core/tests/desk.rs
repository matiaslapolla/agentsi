//! `product.list`, `focus.report`, the terminal ids of a core run and the
//! `PATH` of a child, on a real core.

mod common;

use common::framed::{Framed, Pane};
use common::{BIN, CoreProc, TempDir};
use serde_json::{Value, json};
use std::path::Path;

fn create(c: &mut Framed, argv: Value, env: Value) -> String {
    c.ok(
        "terminal.create",
        // Wide, so a long path is one line of the screen.
        json!({"cwd": std::env::temp_dir(), "argv": argv, "env": env, "cols": 1000}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The screen text of a terminal after its program stopped.
fn output(c: &mut Framed, id: &str) -> String {
    let r = c.ok("terminal.attach", json!({"id": id}));
    let mut pane = Pane::new(r["channel"].as_u64().unwrap() as u32);
    pane.read_until(c, "the close", |p| p.close.is_some());
    pane.text().trim().to_string()
}

#[test]
fn product_list_is_empty_and_makes_no_file_without_a_database() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", &home)]);
    let mut c = Framed::connect(&core.socket.path);
    assert_eq!(c.ok("product.list", json!({})), json!({"products": []}));
    assert!(!home.join("agentsi.db").exists());
}

#[test]
fn product_list_gives_the_id_the_name_and_the_first_path() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let conn = agentsi_store::db::open_writer(&home.join("agentsi.db")).unwrap();
    conn.execute_batch(
        "insert into products (id, name, paths, status) values
           ('zeta', 'Zeta', '[\"/src/zeta\", \"/src/zeta2\"]', 'active'),
           ('alpha', null, '[]', 'active');",
    )
    .unwrap();
    drop(conn);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", &home)]);
    let mut c = Framed::connect(&core.socket.path);
    assert_eq!(
        c.ok("product.list", json!({})),
        json!({"products": [
            {"id": "alpha", "name": "alpha", "path": null},
            {"id": "zeta", "name": "Zeta", "path": "/src/zeta"},
        ]})
    );
}

#[test]
fn product_list_marks_an_archived_product_and_only_that_one() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let conn = agentsi_store::db::open_writer(&home.join("agentsi.db")).unwrap();
    conn.execute_batch(
        "insert into products (id, name, paths, status) values
           ('old', 'Old', '[]', 'active'), ('live', 'Live', '[]', 'active'), ('other', 'Other', '[]', 'active');
         insert into archive_records (product, kind, ref) values
           ('old', 'product', ''), ('live', 'minutes', 'm1'), ('live', 'decision', '3');",
    )
    .unwrap();
    drop(conn);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", &home)]);
    let mut c = Framed::connect(&core.socket.path);
    assert_eq!(
        c.ok("product.list", json!({})),
        json!({"products": [
            {"id": "live", "name": "Live", "path": null},
            {"id": "old", "name": "Old", "path": null, "archived": true},
            {"id": "other", "name": "Other", "path": null},
        ]}),
        "an archived minutes or decision does not archive the product"
    );
}

#[test]
fn product_list_without_a_data_root_is_unavailable() {
    let dir = TempDir::new();
    let config = dir.0.join("config");
    let core = CoreProc::start_with(
        &dir,
        &[
            ("AGENTSI_HOME", Path::new("")),
            ("XDG_CONFIG_HOME", &config),
        ],
    );
    let mut c = Framed::connect(&core.socket.path);
    let r = c.call("product.list", json!({}));
    assert_eq!(r["error"]["data"]["kind"], "unavailable", "{r}");
}

#[test]
fn focus_report_moves_the_dictation_target_once_for_each_change() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["dictation"]}));
    let a = create(&mut c, json!(["sleep", "30"]), json!({}));
    let b = create(&mut c, json!(["sleep", "30"]), json!({}));
    c.ok("focus.report", json!({"terminal": a}));
    assert_eq!(c.next_event("dictation.target"), json!({"terminal": a}));
    // The same pane again is no change: the next event is the one of `b`.
    c.ok("focus.report", json!({"terminal": a}));
    c.ok("focus.report", json!({"terminal": b}));
    assert_eq!(c.next_event("dictation.target"), json!({"terminal": b}));
    c.ok("focus.report", json!({}));
    assert_eq!(c.next_event("dictation.target"), json!({"terminal": null}));
    let r = c.call("focus.report", json!({"terminal": "t99-zzzzzz"}));
    assert_eq!(r["error"]["data"]["kind"], "not_found", "{r}");
}

#[test]
fn a_killed_terminal_is_no_longer_the_dictation_target() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["dictation"]}));
    let a = create(&mut c, json!(["sleep", "30"]), json!({}));
    c.ok("focus.report", json!({"terminal": a}));
    assert_eq!(c.next_event("dictation.target"), json!({"terminal": a}));
    c.ok("terminal.kill", json!({"id": a}));
    assert_eq!(c.next_event("dictation.target"), json!({"terminal": null}));
}

#[test]
fn a_terminal_id_names_its_core_run_and_an_id_of_another_run_is_not_found() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    let id = create(&mut c, json!(["sleep", "30"]), json!({}));
    let (n, run) = id
        .strip_prefix('t')
        .and_then(|r| r.split_once('-'))
        .unwrap_or_else(|| panic!("{id}"));
    assert_eq!(n, "1");
    assert_eq!(run.len(), 6, "{id}");
    assert!(run.chars().all(|ch| ch.is_ascii_alphanumeric()), "{id}");
    assert_eq!(c.ok("voice.mode", json!({"terminal": id}))["on"], false);
    for other in [
        "t1",
        &format!("t1-{run}x"),
        "t1-aaaaaa",
        &format!("t2-{run}"),
    ] {
        let r = c.call("voice.mode", json!({"terminal": other}));
        assert_eq!(r["error"]["data"]["kind"], "not_found", "{other}: {r}");
    }
    drop(c);
    drop(core);

    // A new core counts from 1 again, with another run mark: the old id names nothing.
    let dir2 = TempDir::new();
    let core2 = CoreProc::start(&dir2);
    let mut c2 = Framed::connect(&core2.socket.path);
    let id2 = create(&mut c2, json!(["sleep", "30"]), json!({}));
    assert!(id2.starts_with("t1-"), "{id2}");
    assert_ne!(id2, id);
    let r = c2.call("terminal.attach", json!({"id": id}));
    assert_eq!(r["error"]["data"]["kind"], "not_found", "{r}");
}

#[test]
fn the_child_path_finds_the_core_program_first() {
    let dir = TempDir::new();
    let core = CoreProc::start(&dir);
    let mut c = Framed::connect(&core.socket.path);
    let folder = Path::new(BIN).parent().unwrap().display().to_string();

    let id = create(&mut c, json!(["sh", "-c", "printenv PATH"]), json!({}));
    let path = output(&mut c, &id);
    let first = path.split(':').next().unwrap();
    assert_eq!(first, folder, "{path}");
    assert_eq!(
        path.split(':').filter(|d| *d == folder).count(),
        1,
        "{path}"
    );

    // A PATH in the params keeps its folders, after the folder of the core.
    let id = create(
        &mut c,
        json!(["/bin/sh", "-c", "printenv PATH; command -v agentsi-core"]),
        json!({"PATH": "/usr/bin:/bin"}),
    );
    let text = output(&mut c, &id);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], format!("{folder}:/usr/bin:/bin"), "{text}");
    assert_eq!(lines[1], BIN, "{text}");

    // A PATH that already finds this core first keeps its order.
    let kept = format!("/usr/bin:{folder}:/bin");
    let id = create(
        &mut c,
        json!(["/bin/sh", "-c", "printenv PATH"]),
        json!({"PATH": kept}),
    );
    assert_eq!(output(&mut c, &id), kept);
}

const DESK_ROWS: &str = "
    insert into products (id, name, paths, status) values ('demo', 'Demo', '[\"/src/demo\"]', 'active');
    insert into minutes (id, product, question, status, created_at, data, usage) values
      ('m1', 'demo', 'What now?', 'valid', '2026-11-02T10:00:00.000Z',
       '{\"id\":\"m1\",\"question\":\"What now?\",\"status\":\"valid\",\"priorities\":[{\"title\":\"P0\",\"why\":\"w\",\"sources\":[]}],\"decisions\":[{\"text\":\"Ship it\"}]}',
       '[{\"role\":\"pm\",\"model\":\"opus\",\"input_tokens\":4,\"output_tokens\":2,\"cache_read\":0,\"cache_write\":0,\"cost_usd\":0.5}]'),
      ('old', 'demo', 'No body', 'valid', '2026-10-01T10:00:00.000Z', null, null);
    insert into priorities (minutes_id, idx, title, mark) values ('m1', 0, 'P0', 'accepted');
    insert into decisions (id, product, minutes_id, text, status, kind) values
      (1, 'demo', 'm1', 'Ship it', 'pending', 'minutes');";

fn home_with_desk_rows(dir: &TempDir) -> std::path::PathBuf {
    let home = dir.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let conn = agentsi_store::db::open_writer(&home.join("agentsi.db")).unwrap();
    conn.execute_batch(DESK_ROWS).unwrap();
    home
}

#[test]
fn the_desk_methods_read_a_database_and_say_what_is_missing() {
    let dir = TempDir::new();
    let home = home_with_desk_rows(&dir);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", &home)]);
    let mut c = Framed::connect(&core.socket.path);

    let desk = c.ok("product.desk", json!({"product": "demo"}));
    assert_eq!(desk["product"]["name"], "Demo");
    assert_eq!(desk["pending_decisions"][0]["id"], 1);
    assert_eq!(desk["pending_decisions"][0]["text"], "Ship it");
    assert_eq!(desk["minutes_total"], 2);
    assert_eq!(desk["last_minutes"][0]["id"], "m1");
    assert_eq!(desk["last_minutes"][0]["marks"]["accepted"], 1);
    let r = c.call("product.desk", json!({"product": "nope"}));
    assert_eq!(r["error"]["data"]["kind"], "not_found", "{r}");

    let list = c.ok("minutes.list", json!({"product": "demo"}));
    assert_eq!(list["minutes"].as_array().unwrap().len(), 2);
    assert_eq!(
        c.ok("minutes.list", json!({"product": "nope"})),
        json!({"minutes": []})
    );

    let m = c.ok("minutes.get", json!({"id": "m1"}));
    assert_eq!(m["question"], "What now?");
    assert_eq!(m["priorities"][0]["mark"], "accepted");
    assert_eq!(m["decisions"][0]["id"], 1);
    assert_eq!(m["usage_totals"]["cost_usd"], 0.5);
    // A row with no stored body and an unknown id are both not found.
    for id in ["old", "nope"] {
        let r = c.call("minutes.get", json!({"id": id}));
        assert_eq!(r["error"]["data"]["kind"], "not_found", "{r}");
    }
}

#[test]
fn the_desk_methods_need_a_database_file_and_never_make_it() {
    let dir = TempDir::new();
    let home = dir.0.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", &home)]);
    let mut c = Framed::connect(&core.socket.path);
    let r = c.call("product.desk", json!({"product": "demo"}));
    assert_eq!(r["error"]["data"]["kind"], "unavailable", "{r}");
    let r = c.call("minutes.get", json!({"id": "m1"}));
    assert_eq!(r["error"]["data"]["kind"], "unavailable", "{r}");
    assert_eq!(
        c.ok("minutes.list", json!({"product": "demo"})),
        json!({"minutes": []})
    );
    assert!(!home.join("agentsi.db").exists());
}

fn backups(home: &Path) -> Vec<std::path::PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(home.join("backups"))
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    files
}

#[test]
fn a_core_that_starts_with_no_backup_today_writes_one_and_a_second_start_writes_none() {
    let dir = TempDir::new();
    let home = home_with_desk_rows(&dir);
    {
        let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", &home)]);
        common::wait_until("the backup of the first start", || {
            core.stderr().contains("core: backup written to")
        });
        let files = backups(&home);
        assert_eq!(files.len(), 1, "{files:?}");
        let conn = rusqlite::Connection::open(&files[0]).unwrap();
        let check: String = conn
            .query_row("pragma integrity_check", [], |r| r.get(0))
            .unwrap();
        assert_eq!(check, "ok");
        let rows: i64 = conn
            .query_row("select count(*) from minutes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2);
    }
    let before = std::fs::metadata(&backups(&home)[0])
        .unwrap()
        .modified()
        .unwrap();
    let dir2 = TempDir::new();
    let core = CoreProc::start_with(&dir2, &[("AGENTSI_HOME", &home)]);
    common::wait_until("the check of the second start", || {
        core.stderr().contains("core: the backup of today exists")
    });
    let files = backups(&home);
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(
        std::fs::metadata(&files[0]).unwrap().modified().unwrap(),
        before
    );
}
