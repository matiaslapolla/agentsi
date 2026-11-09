//! The show tools of a desk session. They exist only when `AGENTSI_DESK_SESSION` names the
//! session: then each variant of `View` gives one `show_<kind>` tool, which calls `desk.show`.

use agentsi_proto::desk_session_api::{View, view_tools};
use serde_json::{Map, Value, json};

/// The `tools/list` entries of the show tools, in the order of `View`.
pub fn tool_schemas() -> Vec<Value> {
    view_tools()
        .into_iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": t.input_schema,
            })
        })
        .collect()
}

/// The kind of a show tool, or `None` for another name.
pub fn kind(name: &str) -> Option<String> {
    view_tools()
        .into_iter()
        .find(|t| t.name == name)
        .map(|t| t.kind)
}

/// The view of a call of a show tool: its arguments, with the kind.
pub fn view(kind: &str, args: &Map<String, Value>) -> Result<View, String> {
    let mut value = args.clone();
    value.insert("kind".into(), Value::String(kind.to_string()));
    serde_json::from_value(Value::Object(value))
        .map_err(|e| format!("Invalid arguments of show_{kind}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_show_tool_maps_its_arguments_to_a_view() {
        assert_eq!(kind("show_decisions").as_deref(), Some("decisions"));
        assert_eq!(kind("search"), None);
        let args = json!({"ids": [1, 2]}).as_object().unwrap().clone();
        assert_eq!(
            view("decisions", &args).unwrap(),
            View::Decisions { ids: vec![1, 2] }
        );
        let bad = json!({"ids": "x"}).as_object().unwrap().clone();
        let e = view("decisions", &bad).unwrap_err();
        assert!(
            e.starts_with("Invalid arguments of show_decisions: "),
            "{e}"
        );
        let schemas = tool_schemas();
        assert_eq!(schemas[0]["name"], "show_decisions");
        assert_eq!(schemas[0]["inputSchema"]["required"], json!(["ids"]));
    }
}
