//! `role.general.*` and `roles.roster` over the socket: a general role is a file of the data
//! root that every product can add to its roster. Each write commits its file and sends
//! `data.changed` of kind `roles`.

mod common;

use common::framed::Framed;
use common::{CoreProc, TempDir};
use serde_json::{Value, json};
use std::path::Path;
use std::process::Command;

const REGISTRY: &str = r#"{
  "owner": { "name": "Ada", "language": "English" },
  "products": [
    { "id": "alpha", "name": "Alpha", "paths": ["/work/alpha"], "status": "active", "roles": ["pm", "dev"] },
    { "id": "beta", "name": "Beta", "paths": ["/work/beta"], "status": "active", "roles": ["pm"] }
  ]
}
"#;

fn kind(reply: &Value) -> &str {
    reply["error"]["data"]["kind"].as_str().unwrap_or("none")
}

fn message(reply: &Value) -> &str {
    reply["error"]["message"].as_str().unwrap_or_default()
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn data_root(dir: &TempDir) -> std::path::PathBuf {
    let root = dir.0.join("root");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("agentsi.config.json"), REGISTRY).unwrap();
    agentsi_store::db::open_writer(&root.join("agentsi.db")).unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.name", "t"]);
    git(&root, &["config", "user.email", "t@example.com"]);
    git(&root, &["add", "agentsi.config.json"]);
    git(&root, &["commit", "-q", "-m", "registry"]);
    root
}

fn designer() -> Value {
    json!({
        "id": "designer",
        "name": "Designer",
        "description": "Reads the screens.",
        "body": "You are the designer."
    })
}

#[test]
fn a_general_role_is_a_file_that_two_rosters_add_and_that_cannot_be_deleted_while_used() {
    let dir = TempDir::new();
    let root = data_root(&dir);
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", root.as_path())]);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["data"]}));

    let made = c.ok("role.general.create", designer());
    assert_eq!(made["role"], designer());
    assert_eq!(made["commit"]["committed"], true, "{made}");
    assert_eq!(
        c.next_event("data.changed"),
        json!({"product": "", "kind": "roles"})
    );
    let file = root.join("plugin/agents/designer.md");
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .ends_with("You are the designer.\n")
    );
    assert!(
        git(&root, &["status", "--porcelain", "plugin"])
            .trim()
            .is_empty()
    );

    let r = c.call(
        "role.general.create",
        json!({"id": "dev", "name": "", "description": "", "body": "x"}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");

    for product in ["alpha", "beta"] {
        let set = c.ok(
            "roles.roster",
            json!({"product": product, "roles": ["pm", "designer"]}),
        );
        assert_eq!(set["roster"], json!(["pm", "designer"]));
        assert_eq!(
            c.next_event("data.changed"),
            json!({"product": product, "kind": "roles"})
        );
        let listed = c.ok("roles.list", json!({"product": product}));
        assert_eq!(listed["roster"][1]["id"], "designer");
        assert_eq!(listed["roster"][1]["source"], "global");
    }
    let r = c.call(
        "roles.roster",
        json!({"product": "alpha", "roles": ["pm", "ghost"]}),
    );
    assert_eq!(kind(&r), "invalid", "{r}");

    let list = c.ok("role.general.list", json!({}));
    assert_eq!(list["roles"][0]["id"], "designer");
    assert_eq!(list["roles"][0]["used_by"], json!(["alpha", "beta"]));
    assert_eq!(
        list["builtin"],
        json!(["pm", "social", "dev", "qa", "scout"])
    );

    let r = c.call("role.general.delete", json!({"id": "designer"}));
    assert_eq!(kind(&r), "invalid", "{r}");
    assert!(message(&r).contains("alpha, beta"), "{r}");
    assert!(file.exists());

    let mut edited = designer();
    edited["body"] = json!("You review the screens.");
    c.ok("role.general.update", edited);
    assert_eq!(c.next_event("data.changed")["kind"], "roles");
    let r = c.call(
        "role.general.update",
        json!({"id": "ghost", "name": "", "description": "", "body": "x"}),
    );
    assert_eq!(kind(&r), "not_found", "{r}");

    for product in ["alpha", "beta"] {
        c.ok("roles.roster", json!({"product": product, "roles": ["pm"]}));
    }
    let gone = c.ok("role.general.delete", json!({"id": "designer"}));
    assert_eq!(gone["commit"]["committed"], true, "{gone}");
    assert!(!file.exists());
    assert!(
        git(&root, &["status", "--porcelain", "plugin"])
            .trim()
            .is_empty()
    );
}

#[test]
fn make_general_moves_a_custom_role_and_the_roster_keeps_it() {
    let dir = TempDir::new();
    let root = data_root(&dir);
    let conn = agentsi_store::db::open_writer(&root.join("agentsi.db")).unwrap();
    conn.execute(
        "insert into agent_overlays (product, agent, body) values ('alpha', 'seo', '---\ndescription: Search\n---\nYou rank pages.')",
        [],
    )
    .unwrap();
    drop(conn);
    std::fs::write(
        root.join("agentsi.config.json"),
        REGISTRY.replace(r#""roles": ["pm", "dev"]"#, r#""roles": ["pm", "seo"]"#),
    )
    .unwrap();
    let core = CoreProc::start_with(&dir, &[("AGENTSI_HOME", root.as_path())]);
    let mut c = Framed::connect(&core.socket.path);
    c.ok("core.subscribe", json!({"topics": ["data"]}));

    let before = c.ok("roles.list", json!({"product": "alpha"}));
    assert_eq!(before["roster"][1]["source"], "custom");

    let made = c.ok(
        "role.general.promote",
        json!({"product": "alpha", "agent": "seo"}),
    );
    assert_eq!(made["role"]["id"], "seo");
    assert_eq!(made["role"]["description"], "Search");
    assert_eq!(
        c.next_event("data.changed"),
        json!({"product": "alpha", "kind": "roles"})
    );
    assert!(root.join("plugin/agents/seo.md").exists());

    let after = c.ok("roles.list", json!({"product": "alpha"}));
    assert_eq!(after["roster"][1]["id"], "seo");
    assert_eq!(after["roster"][1]["source"], "global");
    assert_eq!(after["overlays"], json!([]));
    c.ok(
        "roles.roster",
        json!({"product": "beta", "roles": ["pm", "seo"]}),
    );

    let r = c.call(
        "role.general.promote",
        json!({"product": "alpha", "agent": "seo"}),
    );
    assert_eq!(kind(&r), "not_found", "{r}");
}
