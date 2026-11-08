//! `agentsi clear` against a real core on a throwaway data root. The TypeScript CLI has no
//! such command, so the expected text is in this file.

use agentsi_proto::client::Client;
use agentsi_proto::core_api::{Empty, Shutdown};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Work {
    dir: tempfile::TempDir,
    socket: PathBuf,
    core: PathBuf,
}

impl Drop for Work {
    fn drop(&mut self) {
        if let Ok(mut c) = Client::connect(&self.socket) {
            let _ = c.call::<Shutdown>(&Empty {});
        }
    }
}

fn core_program() -> PathBuf {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("run with cargo"));
    // The build folder may hold a core of another checkout: mark the sources as changed.
    let main = manifest.join("../agentsi-core/src/main.rs");
    let file = std::fs::OpenOptions::new().write(true).open(&main).unwrap();
    file.set_modified(std::time::SystemTime::now()).unwrap();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let status = Command::new(cargo)
        .args(["build", "-q", "-p", "agentsi-core", "--bins"])
        .status()
        .expect("cargo runs");
    assert!(status.success(), "cargo build -p agentsi-core");
    Path::new(env!("CARGO_BIN_EXE_agentsi"))
        .parent()
        .unwrap()
        .join("agentsi-core")
}

fn work() -> Work {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    let conn = agentsi_store::db::open_writer(&agentsi_store::paths::db_path(&root)).unwrap();
    for p in ["x", "y"] {
        conn.execute_batch(&format!(
            "insert into products values ('{p}', '{p}', '[]', 'active');
             insert into minutes (id, product, created_at, data) values ('m-{p}', '{p}', '2026-01-01T00:00:00.000Z', '{{}}');"
        ))
        .unwrap();
        let audio = root.join(format!("workspace/{p}/audio/m-{p}"));
        std::fs::create_dir_all(&audio).unwrap();
        std::fs::write(audio.join("call.opus"), "audio").unwrap();
    }
    std::fs::write(
        root.join("agentsi.config.json"),
        "{\n  \"products\": [\n    { \"id\": \"x\", \"paths\": [\"/nowhere/x\"] },\n    { \"id\": \"y\", \"paths\": [\"/nowhere/y\"] }\n  ]\n}\n",
    )
    .unwrap();
    Work {
        socket: dir.path().join("run/core.sock"),
        core: core_program(),
        dir,
    }
}

fn agentsi(w: &Work, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_agentsi"))
        .args(args)
        .current_dir(w.dir.path())
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", w.dir.path())
        .env("AGENTSI_HOME", w.dir.path().join("root"))
        .env("XDG_DATA_HOME", w.dir.path().join("data"))
        .env("AGENTSI_CORE_SOCKET", &w.socket)
        .env("AGENTSI_CORE_BIN", &w.core)
        .output()
        .expect("agentsi runs");
    let work = w.dir.path().to_str().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).replace(work, "<WORK>"),
        String::from_utf8_lossy(&out.stderr).replace(work, "<WORK>"),
    )
}

fn minutes(w: &Work) -> Vec<String> {
    let c = rusqlite::Connection::open(w.dir.path().join("root/agentsi.db")).unwrap();
    let mut stmt = c.prepare("select id from minutes order by id").unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn clear_without_yes_prints_the_plan_and_deletes_nothing() {
    let w = work();
    let (code, out, err) = agentsi(&w, &["clear", "meetings", "--product", "x"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        out,
        "meetings x\n  rows   minutes          1\n  folder workspace/x/audio/m-x  (5 B)\nTotal: 1 rows, 1 files, 5 B.\nNothing was deleted. Repeat with --yes to delete this plan.\n"
    );
    assert_eq!(minutes(&w), ["m-x", "m-y"]);
    assert!(
        w.dir
            .path()
            .join("root/workspace/x/audio/m-x/call.opus")
            .exists()
    );
}

#[test]
fn clear_with_yes_backs_up_then_deletes_one_product_only() {
    let w = work();
    let (code, out, err) = agentsi(&w, &["clear", "product", "--product", "x", "--yes"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Deleted "), "{out}");
    assert!(
        out.contains("Backup: <WORK>/root/backups/agentsi-"),
        "{out}"
    );
    assert!(out.contains("Removed from the registry: x"), "{out}");
    assert_eq!(minutes(&w), ["m-y"]);
    assert!(!w.dir.path().join("root/workspace/x").exists());
    assert!(
        w.dir
            .path()
            .join("root/workspace/y/audio/m-y/call.opus")
            .exists()
    );
    let registry = std::fs::read_to_string(w.dir.path().join("root/agentsi.config.json")).unwrap();
    assert!(
        !registry.contains("\"x\"") && registry.contains("\"y\""),
        "{registry}"
    );
}

#[test]
fn clear_refuses_a_bad_item_a_missing_product_and_a_bad_date() {
    let w = work();
    let (code, _, err) = agentsi(&w, &["clear", "nothing"]);
    assert_eq!(code, 1);
    assert!(
        err.contains("Unknown item \"nothing\"") && err.contains("agentsi clear <item>"),
        "{err}"
    );
    let (code, _, err) = agentsi(&w, &["clear", "meetings"]);
    assert_eq!(code, 1);
    assert!(err.contains("needs a product"), "{err}");
    let (code, _, err) = agentsi(
        &w,
        &[
            "clear",
            "meetings",
            "--product",
            "x",
            "--before",
            "soon",
            "--yes",
        ],
    );
    assert_eq!(code, 1);
    assert!(err.contains("Bad date"), "{err}");
    assert_eq!(minutes(&w), ["m-x", "m-y"]);
}

#[test]
fn clear_is_not_in_the_help_that_keeps_the_typescript_text() {
    let w = work();
    let (_, out, _) = agentsi(&w, &["help"]);
    assert!(!out.contains("clear"), "{out}");
}
