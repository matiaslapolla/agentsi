//! The five tools of `agentsi mcp`: names, descriptions and input schemas.
//! They equal the tools of `packages/mcp/src/server.ts`, which the SDK turns
//! into JSON Schema (draft 7) with `zod`. The table makes both the answer to
//! `tools/list` and the check of the arguments of `tools/call`, with the
//! messages that `zod` gives.

use regex::Regex;
use serde_json::{Map, Value, json};

const KINDS: [&str; 6] = ["doc", "journal", "commit", "issue", "minutes", "memory"];
pub const MAX_DOC_CHARS: usize = 12_000;

enum Ty {
    Text {
        min_len: Option<usize>,
        pattern: Option<&'static str>,
    },
    Choice(&'static [&'static str]),
    Int {
        min: i64,
        max: Option<i64>,
        default: Option<i64>,
    },
    Bool {
        default: bool,
    },
}

struct Prop {
    name: &'static str,
    ty: Ty,
    description: &'static str,
    required: bool,
}

pub struct Tool {
    pub name: &'static str,
    description: &'static str,
    props: &'static [Prop],
}

const PRODUCT: Prop = Prop {
    name: "product",
    ty: Ty::Text {
        min_len: None,
        pattern: None,
    },
    description: "Product id. Default: product of the cwd.",
    required: false,
};

const fn text(name: &'static str, description: &'static str, required: bool) -> Prop {
    Prop {
        name,
        ty: Ty::Text {
            min_len: Some(1),
            pattern: None,
        },
        description,
        required,
    }
}

pub const TOOLS: [Tool; 5] = [
    Tool {
        name: "search",
        description: "Full-text search over the product's docs, journal, commits, issues and minutes. Returns ranked hits (kind, ref, title, snippet); read one with get_doc. Query in the language of the documents.",
        props: &[
            PRODUCT,
            text(
                "query",
                "Terms. Quotes for phrases, 'or' for alternatives, '-' to exclude.",
                true,
            ),
            Prop {
                name: "kind",
                ty: Ty::Choice(&KINDS),
                description: "Only this kind. memory is searched only when named.",
                required: false,
            },
            Prop {
                name: "limit",
                ty: Ty::Int {
                    min: 1,
                    max: Some(50),
                    default: Some(5),
                },
                description: "Max hits (default 5).",
                required: false,
            },
        ],
    },
    Tool {
        name: "get_doc",
        description: "Read one indexed item by kind and ref from a search hit. Bodies over 12000 chars are cut; use section or offset for the rest.",
        props: &[
            PRODUCT,
            Prop {
                name: "kind",
                ty: Ty::Choice(&KINDS),
                description: "Kind, as in the search hit.",
                required: true,
            },
            text(
                "ref",
                "Ref, as in the search hit. A commit hash prefix works.",
                true,
            ),
            text(
                "section",
                "Markdown heading text (case-insensitive): return only that section.",
                false,
            ),
            Prop {
                name: "offset",
                ty: Ty::Int {
                    min: 0,
                    max: None,
                    default: None,
                },
                description: "Character offset to start reading from.",
                required: false,
            },
        ],
    },
    Tool {
        name: "list_minutes",
        description: "List recent meeting minutes: id, status, date, question and the owner's marks on the priorities. Read one with get_doc kind=minutes.",
        props: &[
            PRODUCT,
            Prop {
                name: "limit",
                ty: Ty::Int {
                    min: 1,
                    max: Some(50),
                    default: Some(5),
                },
                description: "Max minutes (default 5).",
                required: false,
            },
            Prop {
                name: "include_archived",
                ty: Ty::Bool { default: false },
                description: "Also list archived minutes, marked as archived (default false).",
                required: false,
            },
        ],
    },
    Tool {
        name: "pending_decisions",
        description: "List decisions waiting for the owner's approval, grouped by minutes. Never act on a pending decision.",
        props: &[
            PRODUCT,
            Prop {
                name: "limit",
                ty: Ty::Int {
                    min: 1,
                    max: Some(100),
                    default: Some(10),
                },
                description: "Max decisions (default 10).",
                required: false,
            },
        ],
    },
    Tool {
        name: "role_memory",
        description: "Read what one role learned about this product (active memory items, newest first, with ids). Read it before answering in that role.",
        props: &[
            PRODUCT,
            Prop {
                name: "role",
                ty: Ty::Text {
                    min_len: None,
                    pattern: Some("^[a-z0-9_.-]+$"),
                },
                description: "Agent id: pm, social, dev, qa, scout or a specialist such as dev.backend.",
                required: true,
            },
        ],
    },
];

impl Prop {
    fn schema(&self) -> Value {
        let mut m = Map::new();
        match &self.ty {
            Ty::Text { min_len, pattern } => {
                m.insert("type".into(), json!("string"));
                if let Some(n) = min_len {
                    m.insert("minLength".into(), json!(n));
                }
                if let Some(p) = pattern {
                    m.insert("pattern".into(), json!(p));
                }
            }
            Ty::Choice(values) => {
                m.insert("type".into(), json!("string"));
                m.insert("enum".into(), json!(values));
            }
            Ty::Int { min, max, default } => {
                m.insert("type".into(), json!("integer"));
                m.insert("minimum".into(), json!(min));
                if let Some(max) = max {
                    m.insert("maximum".into(), json!(max));
                }
                if let Some(d) = default {
                    m.insert("default".into(), json!(d));
                }
            }
            Ty::Bool { default } => {
                m.insert("type".into(), json!("boolean"));
                m.insert("default".into(), json!(default));
            }
        }
        m.insert("description".into(), json!(self.description));
        Value::Object(m)
    }

    /// The value after the defaults, or the messages of `zod` for a bad value.
    fn check(&self, given: Option<&Value>, issues: &mut Vec<String>) -> Option<Value> {
        let at = self.name;
        let mut bad = |message: String| {
            issues.push(format!("{message} at {at}"));
            None
        };
        // `null` is a value of the wrong type for `zod`, not a missing value.
        let Some(value) = given else {
            return match (&self.ty, self.required) {
                (_, true) => bad("Required".into()),
                (
                    Ty::Int {
                        default: Some(d), ..
                    },
                    _,
                ) => Some(json!(d)),
                (Ty::Bool { default }, _) => Some(json!(default)),
                _ => None,
            };
        };
        match (&self.ty, value) {
            (Ty::Text { min_len, pattern }, Value::String(s)) => {
                if min_len.is_some_and(|n| s.chars().count() < n) {
                    return bad("String must contain at least 1 character(s)".into());
                }
                if let Some(p) = pattern
                    && !Regex::new(&format!("(?i){p}")).is_ok_and(|r| r.is_match(s))
                {
                    return bad("Invalid".into());
                }
                Some(value.clone())
            }
            (Ty::Choice(values), Value::String(s)) => {
                if values.contains(&s.as_str()) {
                    return Some(value.clone());
                }
                let all: Vec<String> = values.iter().map(|v| format!("'{v}'")).collect();
                bad(format!(
                    "Invalid enum value. Expected {}, received '{s}'",
                    all.join(" | ")
                ))
            }
            (Ty::Int { min, max, .. }, Value::Number(n)) => {
                let Some(whole) = n
                    .as_i64()
                    .or_else(|| n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
                else {
                    return bad("Expected integer, received float".into());
                };
                if whole < *min {
                    return bad(format!("Number must be greater than or equal to {min}"));
                }
                if let Some(max) = max.filter(|m| whole > *m) {
                    return bad(format!("Number must be less than or equal to {max}"));
                }
                Some(json!(whole))
            }
            (Ty::Bool { .. }, Value::Bool(_)) => Some(value.clone()),
            (ty, other) => bad(format!(
                "Expected {}, received {}",
                match ty {
                    Ty::Text { .. } | Ty::Choice(_) => "string",
                    Ty::Int { .. } => "number",
                    Ty::Bool { .. } => "boolean",
                },
                type_name(other)
            )),
        }
    }
}

fn type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

impl Tool {
    pub fn schema(&self) -> Value {
        let properties: Map<String, Value> = self
            .props
            .iter()
            .map(|p| (p.name.to_string(), p.schema()))
            .collect();
        let required: Vec<&str> = self
            .props
            .iter()
            .filter(|p| p.required)
            .map(|p| p.name)
            .collect();
        let mut input = Map::new();
        input.insert("type".into(), json!("object"));
        input.insert("properties".into(), Value::Object(properties));
        if !required.is_empty() {
            input.insert("required".into(), json!(required));
        }
        input.insert("additionalProperties".into(), json!(false));
        input.insert(
            "$schema".into(),
            json!("http://json-schema.org/draft-07/schema#"),
        );
        json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": input,
            "execution": {"taskSupport": "forbidden"},
        })
    }

    /// The arguments with the defaults of the schema. An unknown key is dropped,
    /// as `zod` drops it. The error is the text that the SDK sends.
    pub fn validate(&self, given: &Value) -> Result<Map<String, Value>, String> {
        let given = given.as_object();
        let mut issues = Vec::new();
        let mut out = Map::new();
        for p in self.props {
            if let Some(v) = p.check(given.and_then(|g| g.get(p.name)), &mut issues) {
                out.insert(p.name.to_string(), v);
            }
        }
        if issues.is_empty() {
            Ok(out)
        } else {
            Err(format!(
                "MCP error -32602: Input validation error: Invalid arguments for tool {}: {}",
                self.name,
                issues.join("\n")
            ))
        }
    }
}

pub fn tool(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_schema_has_the_fields_of_the_typescript_server() {
        let schema = tool("search").unwrap().schema();
        assert_eq!(
            schema["inputSchema"]["properties"]["limit"],
            json!({
                "type": "integer", "minimum": 1, "maximum": 50, "default": 5,
                "description": "Max hits (default 5)."
            })
        );
        assert_eq!(schema["inputSchema"]["required"], json!(["query"]));
        assert_eq!(schema["execution"], json!({"taskSupport": "forbidden"}));
        assert!(tool("list_minutes").unwrap().schema()["inputSchema"]["required"].is_null());
    }

    #[test]
    fn a_bad_argument_gets_the_message_of_zod() {
        let search = tool("search").unwrap();
        let err = search.validate(&json!({"limit": 0})).unwrap_err();
        assert_eq!(
            err,
            "MCP error -32602: Input validation error: Invalid arguments for tool search: \
Required at query\nNumber must be greater than or equal to 1 at limit"
        );
        let err = search
            .validate(&json!({"query": "x", "kind": "nope", "limit": 2.5}))
            .unwrap_err();
        assert!(err.contains("Invalid enum value. Expected 'doc' | 'journal' | 'commit' | 'issue' | 'minutes' | 'memory', received 'nope' at kind"));
        assert!(err.contains("Expected integer, received float at limit"));
        let err = tool("role_memory")
            .unwrap()
            .validate(&json!({"role": "a b"}))
            .unwrap_err();
        assert!(err.ends_with("Invalid at role"));
        let err = search.validate(&json!({"query": 3})).unwrap_err();
        assert!(err.ends_with("Expected string, received number at query"));
    }

    #[test]
    fn a_good_call_gets_the_defaults_and_loses_unknown_keys() {
        let args = tool("search")
            .unwrap()
            .validate(&json!({"query": "voice", "extra": 1, "limit": 7.0}))
            .unwrap();
        assert_eq!(args["limit"], json!(7));
        assert!(!args.contains_key("extra"));
        let args = tool("list_minutes").unwrap().validate(&json!({})).unwrap();
        assert_eq!(
            (&args["limit"], &args["include_archived"]),
            (&json!(5), &json!(false))
        );
        assert!(
            tool("role_memory")
                .unwrap()
                .validate(&json!({"role": "Dev.Backend"}))
                .is_ok()
        );
    }
}
