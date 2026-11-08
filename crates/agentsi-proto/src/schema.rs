//! The OpenRPC document of the protocol, made from the Rust types.
//!
//! `schema/core.openrpc.json` holds a copy. A test fails when the copy is
//! not current.

use crate::method::catalog;
use crate::rpc::ErrorKind;
use serde_json::{Map, Value, json};

pub const PROTOCOL: u32 = 1;
pub const STABILITY: &str = "unstable until M2";

fn kind_name(kind: ErrorKind) -> Value {
    serde_json::to_value(kind).unwrap_or_default()
}

fn def_name(schema: &Value) -> Option<String> {
    Some(
        schema
            .get("$ref")?
            .as_str()?
            .strip_prefix("#/$defs/")?
            .to_string(),
    )
}

/// The by-name params of an object schema: one entry for each property.
fn params_of(schema: &Value, defs: &Map<String, Value>) -> Vec<Value> {
    let object = def_name(schema)
        .and_then(|n| defs.get(&n))
        .unwrap_or(schema);
    let required: Vec<&str> = object["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let mut properties: Vec<(&String, &Value)> = object["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .collect();
    properties.sort_by_key(|(name, _)| *name);
    properties
        .into_iter()
        .map(|(name, s)| {
            let mut p =
                json!({"name": name, "required": required.contains(&name.as_str()), "schema": s});
            if let Some(d) = s.get("description") {
                p["description"] = d.clone();
            }
            p
        })
        .collect()
}

pub fn openrpc() -> Value {
    let mut c = catalog();
    let mut defs = c.take_definitions();
    let methods: Vec<Value> = c
        .methods
        .iter()
        .map(|m| {
            let mut kinds = vec![ErrorKind::Invalid, ErrorKind::Internal];
            for k in m.errors {
                if !kinds.contains(k) {
                    kinds.push(*k);
                }
            }
            let errors: Vec<Value> = kinds
                .into_iter()
                .map(|k| json!({"code": k.code(), "message": kind_name(k), "data": {"kind": kind_name(k)}}))
                .collect();
            json!({
                "name": m.name,
                "description": m.description,
                "paramStructure": "by-name",
                "params": params_of(&m.params, &defs),
                "result": {"name": "result", "schema": m.result},
                "errors": errors,
            })
        })
        .collect();
    let events: Vec<Value> = c
        .events
        .iter()
        .map(|e| json!({"name": e.name, "description": e.description, "params": params_of(&e.params, &defs)}))
        .collect();

    // A params type is inlined above. Its definition stays only if another
    // schema uses it too.
    let inlined: Vec<String> = c
        .methods
        .iter()
        .map(|m| &m.params)
        .chain(c.events.iter().map(|e| &e.params))
        .filter_map(def_name)
        .collect();
    let rest = json!([methods, events]).to_string();
    for name in inlined {
        let needle = format!("\"#/$defs/{name}\"");
        let used = rest.contains(&needle)
            || defs
                .iter()
                .any(|(k, v)| *k != name && v.to_string().contains(&needle));
        if !used {
            defs.remove(&name);
        }
    }

    let doc = json!({
        "openrpc": "1.3.2",
        "info": {
            "title": "agentsi-core",
            "version": format!("{PROTOCOL}.0.0-unstable"),
            "description": format!(
                "The protocol of the AgentSI core: JSON-RPC 2.0 on a Unix socket, one message on each line, \
                or in frames after core.hello with framed: true. Protocol {PROTOCOL}, {STABILITY}. \
                Each error has a stable data.kind and a data.hint. x-notifications lists the events."
            ),
            "license": {"name": "Apache-2.0"},
            "x-protocol": PROTOCOL,
            "x-stability": STABILITY,
        },
        "methods": methods,
        "x-notifications": events,
        "components": {"schemas": defs},
    });
    let text = doc
        .to_string()
        .replace("\"#/$defs/", "\"#/components/schemas/");
    sorted(serde_json::from_str(&text).unwrap_or(doc))
}

/// The same value with the keys of each object in order. Another crate of
/// the workspace can turn on `serde_json/preserve_order`, and the text of the
/// schema must not change with the build.
fn sorted(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            Value::Object(entries.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        Value::Array(items) => Value::Array(items.into_iter().map(sorted).collect()),
        other => other,
    }
}

/// The text of `schema/core.openrpc.json`.
pub fn render() -> String {
    let mut text = serde_json::to_string_pretty(&openrpc()).unwrap_or_default();
    text.push('\n');
    text
}
