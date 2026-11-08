//! The compatibility rules between a released schema and the current one.
//!
//! Adapted from Herdr's `herdr-core/tests/compat.rs` (Apache-2.0,
//! github.com/herdrdev/herdr). Changed: the rules also cover the events in
//! `x-notifications`, and they live in the library so that a tool can run them.
//!
//! A change breaks a client when:
//! - a method, a parameter, a result field or an event goes away,
//! - a parameter becomes required,
//! - a parameter accepts less,
//! - a result field changes its type, or can be missing now.

use serde_json::Value;
use std::collections::BTreeSet;

fn resolve<'a>(doc: &'a Value, s: &'a Value) -> &'a Value {
    match s["$ref"]
        .as_str()
        .and_then(|r| r.strip_prefix("#/components/schemas/"))
    {
        Some(name) => resolve(doc, &doc["components"]["schemas"][name]),
        None => s,
    }
}

/// The kinds of value (`string`, `null`, ...) and the structured schemas among them.
fn shape<'a>(doc: &'a Value, s: &'a Value) -> (BTreeSet<String>, Vec<&'a Value>) {
    let s = resolve(doc, s);
    let mut kinds = BTreeSet::new();
    let mut structured = Vec::new();
    match &s["type"] {
        Value::String(t) => {
            kinds.insert(t.clone());
        }
        Value::Array(ts) => kinds.extend(ts.iter().filter_map(|t| t.as_str().map(String::from))),
        _ => {}
    }
    if s.get("properties").is_some() || s.get("items").is_some() || s.get("enum").is_some() {
        structured.push(s);
    }
    for member in s["anyOf"]
        .as_array()
        .into_iter()
        .chain(s["oneOf"].as_array())
        .flatten()
    {
        let (k, o) = shape(doc, member);
        kinds.extend(k);
        structured.extend(o);
    }
    (kinds, structured)
}

fn names(v: &Value) -> BTreeSet<String> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| x.as_str().map(String::from))
        .collect()
}

fn list<'a>(doc: &'a Value, key: &str) -> Vec<&'a Value> {
    doc[key].as_array().into_iter().flatten().collect()
}

struct Side<'a> {
    doc: &'a Value,
    schema: &'a Value,
}

/// For an input, the new side must accept each value that the old side
/// accepted. For an output, the new side must give each field that the old
/// side gave, and nothing that the old side could not give.
fn compare(old: Side, new: Side, input: bool, at: &str, depth: u8, out: &mut Vec<String>) {
    if depth > 8 {
        return;
    }
    let (ok, oo) = shape(old.doc, old.schema);
    let (nk, no) = shape(new.doc, new.schema);
    let (wider, narrower) = if input { (&nk, &ok) } else { (&ok, &nk) };
    if !wider.is_empty() && !narrower.is_subset(wider) {
        out.push(format!("{at}: type {ok:?} became {nk:?}"));
    }
    if !input && ((!ok.is_empty() && nk.is_empty()) || (!oo.is_empty() && no.is_empty())) {
        out.push(format!("{at}: lost its type"));
    }
    let (Some(o), Some(n)) = (oo.first(), no.first()) else {
        return;
    };
    let (oe, ne) = (names(&o["enum"]), names(&n["enum"]));
    let enum_ok = if input {
        oe.is_subset(&ne)
    } else {
        ne.is_subset(&oe)
    };
    if !oe.is_empty() && !enum_ok {
        out.push(format!("{at}: values {oe:?} became {ne:?}"));
    }
    if o.get("items").is_some() {
        let (os, ns) = (
            Side {
                doc: old.doc,
                schema: &o["items"],
            },
            Side {
                doc: new.doc,
                schema: &n["items"],
            },
        );
        compare(os, ns, input, &format!("{at}[]"), depth + 1, out);
    }
    let (oreq, nreq) = (names(&o["required"]), names(&n["required"]));
    for (k, os) in o["properties"].as_object().into_iter().flatten() {
        let path = format!("{at}.{k}");
        match n["properties"].get(k) {
            None => out.push(format!("{path}: removed")),
            Some(ns) => compare(
                Side {
                    doc: old.doc,
                    schema: os,
                },
                Side {
                    doc: new.doc,
                    schema: ns,
                },
                input,
                &path,
                depth + 1,
                out,
            ),
        }
        if !input && oreq.contains(k) && !nreq.contains(k) {
            out.push(format!("{path}: may be missing now"));
        }
    }
    if input {
        for k in nreq.difference(&oreq) {
            out.push(format!("{at}.{k}: newly required"));
        }
    }
}

/// The by-name params of a method or an event, compared as inputs or as outputs.
fn compare_params(
    old: &Value,
    om: &Value,
    new: &Value,
    nm: &Value,
    input: bool,
    name: &str,
    out: &mut Vec<String>,
) {
    let (op, np) = (list(om, "params"), list(nm, "params"));
    for p in &op {
        let at = format!("{name}({})", p["name"].as_str().unwrap_or_default());
        match np.iter().find(|q| q["name"] == p["name"]) {
            None => out.push(format!("{at}: removed")),
            Some(q) => {
                if input && q["required"] == true && p["required"] != true {
                    out.push(format!("{at}: newly required"));
                }
                if !input && p["required"] == true && q["required"] != true {
                    out.push(format!("{at}: may be missing now"));
                }
                compare(
                    Side {
                        doc: old,
                        schema: &p["schema"],
                    },
                    Side {
                        doc: new,
                        schema: &q["schema"],
                    },
                    input,
                    &at,
                    0,
                    out,
                );
            }
        }
    }
    if input {
        for q in np
            .iter()
            .filter(|q| q["required"] == true && !op.iter().any(|p| p["name"] == q["name"]))
        {
            out.push(format!(
                "{name}({}): new and required",
                q["name"].as_str().unwrap_or_default()
            ));
        }
    }
}

/// Each change from `old` to `new` that breaks a client of `old`.
pub fn breaks(old: &Value, new: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for om in list(old, "methods") {
        let name = om["name"].as_str().unwrap_or_default();
        let Some(nm) = list(new, "methods").into_iter().find(|m| m["name"] == name) else {
            out.push(format!("{name}: removed"));
            continue;
        };
        compare_params(old, om, new, nm, true, name, &mut out);
        let (os, ns) = (
            Side {
                doc: old,
                schema: &om["result"]["schema"],
            },
            Side {
                doc: new,
                schema: &nm["result"]["schema"],
            },
        );
        compare(os, ns, false, &format!("{name} ->"), 0, &mut out);
    }
    for oe in list(old, "x-notifications") {
        let name = oe["name"].as_str().unwrap_or_default();
        match list(new, "x-notifications")
            .into_iter()
            .find(|e| e["name"] == name)
        {
            None => out.push(format!("event {name}: removed")),
            Some(ne) => compare_params(old, oe, new, ne, false, &format!("event {name}"), &mut out),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::breaks;
    use serde_json::{Value, json};

    fn doc(params: Value, result: Value, event: Value) -> Value {
        json!({
            "methods": [{"name": "m", "params": params, "result": {"schema": result}}],
            "x-notifications": [{"name": "e", "params": event}],
            "components": {"schemas": {}},
        })
    }

    fn obj(props: Value, required: Value) -> Value {
        json!({"type": "object", "properties": props, "required": required})
    }

    fn base() -> Value {
        doc(
            json!([
                {"name": "t", "required": true, "schema": {"type": "string"}},
                {"name": "o", "required": false, "schema": {"type": "string", "enum": ["a", "b"]}},
            ]),
            obj(
                json!({"x": {"type": "string"}, "y": {"type": ["integer", "null"]}}),
                json!(["x"]),
            ),
            json!([{"name": "id", "required": true, "schema": {"type": "string"}}]),
        )
    }

    #[test]
    fn the_same_schema_breaks_nothing() {
        assert_eq!(breaks(&base(), &base()), Vec::<String>::new());
    }

    #[test]
    fn new_inputs_values_and_fields_break_nothing() {
        let grown = doc(
            json!([
                {"name": "t", "required": true, "schema": {"type": "string"}},
                {"name": "o", "required": false, "schema": {"type": "string", "enum": ["a", "b", "c"]}},
                {"name": "n", "required": false, "schema": {"type": "string"}},
            ]),
            obj(
                json!({"x": {"type": "string"}, "y": {"type": "integer"}, "z": {"type": "string"}}),
                json!(["x"]),
            ),
            json!([
                {"name": "id", "required": true, "schema": {"type": "string"}},
                {"name": "more", "required": false, "schema": {"type": "string"}},
            ]),
        );
        assert_eq!(breaks(&base(), &grown), Vec::<String>::new());
    }

    #[test]
    fn each_breaking_change_is_found() {
        let broken = doc(
            json!([
                {"name": "t", "required": true, "schema": {"type": "integer"}},
                {"name": "o", "required": true, "schema": {"type": "string", "enum": ["a"]}},
                {"name": "n", "required": true, "schema": {"type": "string"}},
            ]),
            obj(
                json!({"y": {"type": ["integer", "string", "null"]}}),
                json!([]),
            ),
            json!([]),
        );
        let found = breaks(&base(), &broken).join("\n");
        for expected in [
            "m(t): type",
            "m(o): newly required",
            "m(o): values",
            "m(n): new and required",
            "m ->.x: removed",
            "m ->.y: type",
            "event e(id): removed",
        ] {
            assert!(found.contains(expected), "missed {expected:?} in:\n{found}");
        }
        let gone = json!({"methods": [], "x-notifications": [], "components": {"schemas": {}}});
        assert_eq!(breaks(&base(), &gone), ["m: removed", "event e: removed"]);
    }
}
