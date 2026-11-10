//! Reads one stdout line of the desk process: a text delta, the items of an assistant
//! message, or the end of a reply.

use super::args::MCP_SERVER;
use agentsi_proto::desk_session_api::DeskItem;
use serde_json::Value;

/// What one stream line gives.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// The next text of the reply that streams.
    Delta(String),
    /// An item to store: `reply`, `tool` or `end`.
    Item(DeskItem),
    /// Claude Code could not authenticate. An `end` item follows.
    AuthError,
}

/// The items of one stream-json line. A line that is not JSON, or of another type, gives none.
pub fn read_line(line: &str) -> Vec<Out> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    match v["type"].as_str() {
        Some("stream_event") => delta(&v["event"]).into_iter().collect(),
        Some("assistant") => assistant(&v),
        Some("result") => result(&v),
        _ => Vec::new(),
    }
}

fn delta(event: &Value) -> Option<Out> {
    if event["type"] != "content_block_delta" || event["delta"]["type"] != "text_delta" {
        return None;
    }
    let text = event["delta"]["text"].as_str()?;
    (!text.is_empty()).then(|| Out::Delta(text.to_string()))
}

fn assistant(v: &Value) -> Vec<Out> {
    let mut out = Vec::new();
    if v["error"].as_str() == Some("authentication_failed") {
        out.push(Out::AuthError);
    }
    let blocks = v["message"]["content"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    for b in blocks {
        match b["type"].as_str() {
            Some("text") => {
                let text = b["text"].as_str().unwrap_or_default();
                if !text.trim().is_empty() {
                    out.push(Out::Item(DeskItem::Reply {
                        text: text.to_string(),
                    }));
                }
            }
            Some("tool_use") => {
                let name = b["name"].as_str().unwrap_or("tool");
                if !is_show_tool(name) {
                    out.push(Out::Item(DeskItem::Tool {
                        line: tool_line(name, &b["input"]),
                    }));
                }
            }
            _ => {}
        }
    }
    out
}

fn result(v: &Value) -> Vec<Out> {
    let failed = v["is_error"].as_bool() == Some(true);
    let error = failed.then(|| crate::story::claude::error_text(v));
    let mut out = Vec::new();
    if error.as_deref().is_some_and(is_auth_error) {
        out.push(Out::AuthError);
    }
    out.push(Out::Item(DeskItem::End {
        cost_usd: v["total_cost_usd"].as_f64().unwrap_or(0.0).max(0.0),
        duration_ms: v["duration_ms"].as_f64().unwrap_or(0.0).max(0.0).round() as u64,
        error,
    }));
    out
}

/// A `show_*` tool of the AgentSI server. Its view item comes from `desk.show`, so its
/// tool use makes no item.
pub fn is_show_tool(name: &str) -> bool {
    name.strip_prefix(&format!("mcp__{MCP_SERVER}__"))
        .is_some_and(|t| t.starts_with("show_"))
}

/// One line for a tool use, as the meeting screen writes it: `Read: docs/a.md`. A tool of the
/// AgentSI server shows its words: `mcp__agentsi__pending_decisions` gives `Pending decisions`.
pub fn tool_line(name: &str, input: &Value) -> String {
    let name = match name.strip_prefix(&format!("mcp__{MCP_SERVER}__")) {
        Some(tool) => words(tool),
        None => name.to_string(),
    };
    let arg = ["file_path", "pattern", "query", "path"]
        .iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str));
    match arg {
        Some(arg) => format!("{name}: {arg}"),
        None => name.to_string(),
    }
}

/// `pending_decisions` gives `Pending decisions`.
fn words(tool: &str) -> String {
    let text = tool.replace('_', " ");
    let mut chars = text.chars();
    chars.next().map_or(String::new(), |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// True for the error text of a reply that Claude Code could not authenticate.
pub fn is_auth_error(text: &str) -> bool {
    let t = text.to_lowercase();
    [
        "/login",
        "invalid api key",
        "authentication_error",
        "authentication failed",
        "oauth token",
        "not logged in",
    ]
    .iter()
    .any(|k| t.contains(k))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_text_delta_streams_and_other_events_do_not() {
        let line = json!({"type": "stream_event", "event": {"type": "content_block_delta",
            "index": 0, "delta": {"type": "text_delta", "text": "Hel"}}});
        assert_eq!(read_line(&line.to_string()), [Out::Delta("Hel".into())]);
        let tool = json!({"type": "stream_event", "event": {"type": "content_block_delta",
            "delta": {"type": "input_json_delta", "partial_json": "{"}}});
        assert!(read_line(&tool.to_string()).is_empty());
        assert!(read_line(r#"{"type":"system","subtype":"init"}"#).is_empty());
        assert!(read_line("not json").is_empty());
    }

    #[test]
    fn an_assistant_message_gives_a_reply_and_tool_lines_but_no_show_tool() {
        let line = json!({"type": "assistant", "message": {"content": [
            {"type": "text", "text": "Two decisions wait."},
            {"type": "tool_use", "name": "Read", "input": {"file_path": "docs/a.md"}},
            {"type": "tool_use", "name": "mcp__agentsi__pending_decisions", "input": {}},
            {"type": "tool_use", "name": "mcp__agentsi__search", "input": {"query": "pricing"}},
            {"type": "tool_use", "name": "mcp__agentsi__show_decisions", "input": {"ids": [1]}},
            {"type": "text", "text": "  "},
        ]}});
        assert_eq!(
            read_line(&line.to_string()),
            [
                Out::Item(DeskItem::Reply {
                    text: "Two decisions wait.".into()
                }),
                Out::Item(DeskItem::Tool {
                    line: "Read: docs/a.md".into()
                }),
                Out::Item(DeskItem::Tool {
                    line: "Pending decisions".into()
                }),
                Out::Item(DeskItem::Tool {
                    line: "Search: pricing".into()
                }),
            ]
        );
    }

    #[test]
    fn a_result_ends_the_reply_with_its_cost_and_error() {
        let ok = json!({"type": "result", "subtype": "success", "is_error": false,
            "total_cost_usd": 0.0123, "duration_ms": 4200.4, "result": "x"});
        assert_eq!(
            read_line(&ok.to_string()),
            [Out::Item(DeskItem::End {
                cost_usd: 0.0123,
                duration_ms: 4200,
                error: None
            })]
        );
        let failed = json!({"type": "result", "subtype": "error_during_execution",
            "is_error": true, "errors": ["boom"]});
        assert_eq!(
            read_line(&failed.to_string()),
            [Out::Item(DeskItem::End {
                cost_usd: 0.0,
                duration_ms: 0,
                error: Some("claude error (error_during_execution): boom".into())
            })]
        );
    }

    #[test]
    fn an_auth_failure_is_marked() {
        let result = json!({"type": "result", "subtype": "success", "is_error": true,
            "result": "Invalid API key · Please run /login"});
        let out = read_line(&result.to_string());
        assert_eq!(out[0], Out::AuthError);
        assert!(matches!(out[1], Out::Item(DeskItem::End { .. })));
        let message = json!({"type": "assistant", "error": "authentication_failed",
            "message": {"content": [{"type": "text", "text": "Not logged in"}]}});
        assert_eq!(read_line(&message.to_string())[0], Out::AuthError);
        assert!(!is_auth_error(
            "claude error (error_max_turns): too many turns"
        ));
    }
}
