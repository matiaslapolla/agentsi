//! The `files` methods on a real core: list, create, rename and trash in a temporary repo, the
//! refusals of a path that leaves the product, the system trash, and `files.changed` for a
//! folder that a list read, but not for an ignored one. The core runs with a throwaway data
//! root, config folder and trash.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

fn kind(reply: &Value) -> &str {
    reply["error"]["data"]["kind"].as_str().unwrap_or("none")
}

struct Setup {
    base: PathBuf,
    alpha: PathBuf,
    trash: PathBuf,
    core: CoreProc,
}

fn start(dir: &TempDir) -> Setup {
    let base = dir.0.clone();
    let (root, config, home, data) = (
        base.join("root"),
        base.join("config"),
        base.join("home"),
        base.join("data"),
    );
    let alpha = base.join("alpha");
    let outside = base.join("outside");
    for d in [&root, &config, &home, &data, &alpha, &outside] {
        std::fs::create_dir_all(d).unwrap();
    }
    git(&alpha, &["init", "-q", "-b", "main"]);
    std::fs::write(alpha.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(alpha.join("README.md"), "a").unwrap();
    std::fs::create_dir_all(alpha.join("target")).unwrap();
    std::os::unix::fs::symlink(&outside, alpha.join("out")).unwrap();
    let registry = json!({"products": [
        {"id": "alpha", "name": "Alpha", "paths": [alpha], "status": "active", "roles": ["pm"]}]});
    std::fs::write(root.join("agentsi.config.json"), registry.to_string()).unwrap();
    let core = CoreProc::start_with(
        dir,
        &[
            ("AGENTSI_HOME", &root),
            ("XDG_CONFIG_HOME", &config),
            ("XDG_DATA_HOME", &data),
            ("HOME", &home),
            ("GIT_CONFIG_GLOBAL", Path::new("/dev/null")),
            ("GIT_CONFIG_SYSTEM", Path::new("/dev/null")),
        ],
    );
    Setup {
        base,
        alpha,
        trash: data.join("Trash"),
        core,
    }
}

fn names(list: &Value) -> Vec<(String, String, Value)> {
    list["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["name"].as_str().unwrap().to_string(),
                e["kind"].as_str().unwrap().to_string(),
                e["git"].clone(),
            )
        })
        .collect()
}

/// True when no `files.changed` comes within `quiet`.
fn quiet_for(c: &mut Framed, quiet: Duration) -> bool {
    use agentsi_proto::frame::{Kind, read_frame};
    c.stream.set_read_timeout(Some(quiet)).unwrap();
    let deadline = Instant::now() + quiet;
    let mut none = true;
    while Instant::now() < deadline {
        match read_frame(&mut c.stream) {
            Ok(Some(f)) if f.channel == 0 && f.kind == Kind::DATA => {
                let v: Value = serde_json::from_slice(&f.payload).unwrap();
                if v["method"] == "files.changed" {
                    none = false;
                }
            }
            Ok(Some(_)) => {}
            _ => break,
        }
    }
    c.stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    none
}

#[test]
fn the_methods_list_create_rename_and_trash_and_refuse_a_path_outside() {
    let dir = TempDir::new();
    let s = start(&dir);
    let mut c = Framed::connect(&s.core.socket.path);

    let list = c.ok("files.list", json!({"product": "alpha", "path": ""}));
    assert_eq!(
        names(&list),
        [
            ("target".into(), "folder".into(), json!("ignored")),
            (".gitignore".into(), "file".into(), json!("new")),
            ("out".into(), "link".into(), json!("new")),
            ("README.md".into(), "file".into(), json!("new")),
        ]
    );

    c.ok(
        "files.create",
        json!({"product": "alpha", "path": "docs/tmp", "kind": "folder"}),
    );
    assert!(s.alpha.join("docs/tmp").is_dir());
    let docs = c.ok("files.list", json!({"product": "alpha", "path": "docs"}));
    assert_eq!(names(&docs)[0].0, "tmp");
    let again = c.call(
        "files.create",
        json!({"product": "alpha", "path": "docs/tmp", "kind": "folder"}),
    );
    assert_eq!(kind(&again), "conflict", "{again}");

    for bad in ["../x", "docs/../../x", "/tmp/x", "out/x", ".git/x"] {
        let r = c.call(
            "files.create",
            json!({"product": "alpha", "path": bad, "kind": "file"}),
        );
        assert_eq!(kind(&r), "invalid", "{bad}: {r}");
    }
    assert!(!s.base.join("x").exists());
    assert_eq!(
        std::fs::read_dir(s.base.join("outside")).unwrap().count(),
        0
    );
    let r = c.call("files.list", json!({"product": "alpha", "path": "out"}));
    assert_eq!(kind(&r), "invalid", "{r}");

    c.ok(
        "files.create",
        json!({"product": "alpha", "path": "docs/a.md", "kind": "file"}),
    );
    c.ok(
        "files.rename",
        json!({"product": "alpha", "from": "docs/a.md", "to": "b.md"}),
    );
    assert!(s.alpha.join("b.md").is_file());
    let r = c.call(
        "files.rename",
        json!({"product": "alpha", "from": "b.md", "to": "README.md"}),
    );
    assert_eq!(kind(&r), "conflict", "{r}");
    let r = c.call(
        "files.rename",
        json!({"product": "alpha", "from": "b.md", "to": "../b.md"}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");

    c.ok("files.trash", json!({"product": "alpha", "path": "b.md"}));
    assert!(!s.alpha.join("b.md").exists());
    assert!(s.trash.join("files/b.md").is_file());
    assert!(s.trash.join("info/b.md.trashinfo").is_file());
    let r = c.call("files.trash", json!({"product": "alpha", "path": ""}));
    assert_eq!(kind(&r), "invalid", "{r}");
    let r = c.call(
        "files.trash",
        json!({"product": "alpha", "path": "gone.md"}),
    );
    assert_eq!(kind(&r), "not_found", "{r}");
    // A link goes to the trash, and what it points to stays.
    std::fs::write(s.base.join("outside/keep.txt"), "k").unwrap();
    c.ok("files.trash", json!({"product": "alpha", "path": "out"}));
    assert!(s.base.join("outside/keep.txt").is_file());

    let r = c.call("files.list", json!({"product": "nope", "path": ""}));
    assert_eq!(kind(&r), "not_found", "{r}");
}

#[test]
fn a_new_file_in_a_listed_folder_sends_files_changed_within_1_s_and_an_ignored_one_sends_nothing() {
    let dir = TempDir::new();
    let s = start(&dir);
    let mut c = Framed::connect(&s.core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["files"]}));
    c.ok("files.list", json!({"product": "alpha", "path": ""}));
    c.ok("files.list", json!({"product": "alpha", "path": "target"}));
    std::thread::sleep(Duration::from_millis(200));

    let at = Instant::now();
    std::fs::write(s.alpha.join("new.txt"), "x").unwrap();
    assert_eq!(
        c.next_event("files.changed"),
        json!({"product": "alpha", "path": ""})
    );
    assert!(at.elapsed() < Duration::from_secs(1), "{:?}", at.elapsed());
    // The burst of the write ends in that one event.
    assert!(quiet_for(&mut c, Duration::from_millis(1500)));

    for i in 0..20 {
        std::fs::write(s.alpha.join(format!("target/{i}.o")), "x").unwrap();
    }
    assert!(quiet_for(&mut c, Duration::from_millis(1500)));

    // A folder that is not listed sends nothing until a list reads it.
    std::fs::create_dir_all(s.alpha.join("src")).unwrap();
    assert_eq!(
        c.next_event("files.changed"),
        json!({"product": "alpha", "path": ""})
    );
    std::fs::write(s.alpha.join("src/a.rs"), "x").unwrap();
    assert!(quiet_for(&mut c, Duration::from_millis(800)));
    c.ok("files.list", json!({"product": "alpha", "path": "src"}));
    std::thread::sleep(Duration::from_millis(100));
    std::fs::write(s.alpha.join("src/b.rs"), "x").unwrap();
    assert_eq!(
        c.next_event("files.changed"),
        json!({"product": "alpha", "path": "src"})
    );
}
