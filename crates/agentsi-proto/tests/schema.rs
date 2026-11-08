//! The two schema tests of the protocol spec.
//!
//! To write `schema/core.openrpc.json` again after a change of the types:
//! `AGENTSI_UPDATE_SCHEMA=1 cargo test -p agentsi-proto --test schema`.

use agentsi_proto::compat::breaks;
use agentsi_proto::core_api::Topic;
use agentsi_proto::schema::{openrpc, render};
use serde_json::Value;
use std::path::PathBuf;

fn schema_dir() -> PathBuf {
    // Read at run time: the shared CARGO_TARGET_DIR reuses this binary across worktrees.
    PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../schema")
}

#[test]
fn the_schema_file_is_current() {
    let path = schema_dir().join("core.openrpc.json");
    let now = render();
    if std::env::var_os("AGENTSI_UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, &now).unwrap();
    }
    let file = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        file == now,
        "{} is not current. Run: AGENTSI_UPDATE_SCHEMA=1 cargo test -p agentsi-proto --test schema",
        path.display()
    );
}

/// From M2, `schema/released.openrpc.json` holds the schema of the last
/// release. During M1 the file does not exist, and this test passes.
#[test]
fn the_protocol_only_grows_since_the_last_release() {
    let path = schema_dir().join("released.openrpc.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("no {}: M1 has no released schema", path.display());
        return;
    };
    let released: Value = serde_json::from_str(&text).unwrap();
    let broken = breaks(&released, &openrpc());
    assert!(
        broken.is_empty(),
        "these changes break clients of the released protocol:\n{}",
        broken.join("\n")
    );
}

#[test]
fn a_removed_result_field_of_a_released_method_breaks_the_compatibility() {
    let released = openrpc();
    let mut changed = released.clone();
    let hello = changed["components"]["schemas"]["HelloResult"]
        .as_object_mut()
        .unwrap();
    hello["properties"]
        .as_object_mut()
        .unwrap()
        .remove("version");
    let broken = breaks(&released, &changed);
    assert!(
        broken.iter().any(|b| b == "core.hello ->.version: removed"),
        "{broken:?}"
    );
}

#[test]
fn the_schema_is_marked_unstable_until_m2() {
    let doc = openrpc();
    assert_eq!(doc["info"]["x-protocol"], 1);
    assert_eq!(doc["info"]["x-stability"], "unstable until M2");
}

#[test]
fn each_event_has_a_topic_or_goes_to_each_subscriber() {
    for event in agentsi_proto::catalog().events {
        let namespace = event.name.split('.').next().unwrap();
        assert!(
            Topic::of_event(event.name).is_some() || namespace == "core",
            "event {} has no topic",
            event.name
        );
    }
}

#[test]
fn each_method_name_has_a_namespace() {
    for method in agentsi_proto::catalog().methods {
        let (namespace, name) = method.name.split_once('.').unwrap_or_default();
        assert!(!namespace.is_empty() && !name.is_empty(), "{}", method.name);
    }
}
