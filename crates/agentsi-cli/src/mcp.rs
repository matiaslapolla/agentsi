//! `agentsi mcp`: the MCP server on stdio, for Claude Code. It serves the 5
//! tools of `packages/mcp` with the same names and input schemas. The server
//! is a small JSON-RPC loop, because the protocol of a server with tools needs
//! four methods. Each tool asks the core, which owns the database.
//!
//! The core is joined on the first call that needs data, so `initialize` and
//! `tools/list` work when no core runs. Stdout holds the protocol only.

mod format;
mod tools;

use crate::core::Core;
use agentsi_proto::agency_api::{ProductWhoami, WhoamiParams};
use agentsi_proto::knowledge_api::{
    KbGetDocParams, KbPendingParams, KbRecentMinutesParams, KbRoleMemoryParams, KbSearchParams,
    KnowledgeGetDoc, KnowledgePendingDecisions, KnowledgeRecentMinutes, KnowledgeRoleMemory,
    KnowledgeSearch,
};
use serde_json::{Map, Value, json};
use std::io::{BufRead, Write};

pub const USAGE: &str = "agentsi mcp";

const SERVER_NAME: &str = "agentsi";
const SERVER_VERSION: &str = "0.2.0";
const LATEST_PROTOCOL: &str = "2025-11-25";
const PROTOCOLS: [&str; 5] = [
    LATEST_PROTOCOL,
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
    "2024-10-07",
];

struct Server {
    cwd: String,
    core: Option<Core>,
}

impl Server {
    fn core(&mut self) -> Result<&mut Core, String> {
        if self.core.is_none() {
            self.core = Some(Core::connect()?);
        }
        Ok(self.core.as_mut().expect("just connected"))
    }

    /// The product of the `product` argument, else of the folder of the process.
    fn product(&mut self, given: Option<&str>) -> Result<String, String> {
        let cwd = self.cwd.clone();
        let core = self.core()?;
        let listed = core.call::<ProductWhoami>(&WhoamiParams {
            list: true,
            ..WhoamiParams::default()
        })?;
        let ids = || {
            listed
                .products
                .iter()
                .flatten()
                .map(|p| p.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        if let Some(id) = given {
            return match listed.products.iter().flatten().any(|p| p.id == id) {
                true => Ok(id.to_string()),
                false => Err(format!("Unknown product \"{id}\". Products: {}", ids())),
            };
        }
        let found = core.call::<ProductWhoami>(&WhoamiParams {
            cwd: Some(cwd.clone()),
            ..WhoamiParams::default()
        })?;
        found.product.map(|p| p.product).ok_or_else(|| {
            format!(
                "No product matches {cwd}. Pass \"product\" explicitly. Products: {}",
                ids()
            )
        })
    }

    /// Runs the tool. A failure drops the connection and tries once more on a new one,
    /// because the core can restart while this server runs. The tools only read.
    fn run_tool(&mut self, name: &str, args: &Map<String, Value>) -> Result<String, String> {
        let first = self.ask_core(name, args);
        if first.is_err() && self.core.take().is_some() {
            return self.ask_core(name, args);
        }
        first
    }

    fn ask_core(&mut self, name: &str, args: &Map<String, Value>) -> Result<String, String> {
        let text = |k: &str| args.get(k).and_then(Value::as_str);
        let number = |k: &str| args.get(k).and_then(Value::as_u64);
        let product = self.product(text("product"))?;
        let core = self.core()?;
        match name {
            "search" => {
                let query = text("query").unwrap_or_default();
                let found = core.call::<KnowledgeSearch>(&KbSearchParams {
                    product: product.clone(),
                    query: query.to_string(),
                    kind: text("kind").map(String::from),
                    limit: number("limit").map(|n| n as u32),
                })?;
                Ok(format::search_text(
                    &product,
                    query,
                    &found.hits,
                    found.any_term,
                ))
            }
            "get_doc" => {
                let doc = core.call::<KnowledgeGetDoc>(&KbGetDocParams {
                    product,
                    kind: text("kind").unwrap_or_default().to_string(),
                    reference: text("ref").unwrap_or_default().to_string(),
                })?;
                format::doc_text(&doc, text("section"), number("offset").map(|n| n as usize))
            }
            "list_minutes" => {
                let found = core.call::<KnowledgeRecentMinutes>(&KbRecentMinutesParams {
                    product: product.clone(),
                    limit: number("limit").unwrap_or(5) as u32,
                    include_archived: args
                        .get("include_archived")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })?;
                Ok(format::minutes_text(&product, &found.minutes))
            }
            "pending_decisions" => {
                let found = core.call::<KnowledgePendingDecisions>(&KbPendingParams {
                    product: product.clone(),
                    limit: number("limit").unwrap_or(10) as u32,
                })?;
                Ok(format::pending_text(&product, &found))
            }
            "role_memory" => {
                let found = core.call::<KnowledgeRoleMemory>(&KbRoleMemoryParams {
                    product,
                    role: text("role").unwrap_or_default().to_string(),
                })?;
                // The TypeScript tool tries `|| "sin memoria todavía"` on a promise, which is
                // never falsy, so it answers with an empty text. The text stays empty here.
                Ok(found.text)
            }
            other => Err(format!("MCP error -32602: Tool {other} not found")),
        }
    }

    fn call_tool(&mut self, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let given = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let result = match tools::tool(name) {
            None => Err(format!("MCP error -32602: Tool {name} not found")),
            Some(tool) => tool
                .validate(&given)
                .and_then(|args| self.run_tool(name, &args)),
        };
        match result {
            Ok(text) => json!({"content": [{"type": "text", "text": text}]}),
            Err(text) => json!({"content": [{"type": "text", "text": text}], "isError": true}),
        }
    }

    /// The answer to one request, or `None` for a notification.
    fn handle(&mut self, message: &Value) -> Option<Value> {
        let id = message.get("id")?.clone();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        let result = match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str);
                let version = asked
                    .filter(|v| PROTOCOLS.contains(v))
                    .unwrap_or(LATEST_PROTOCOL);
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": true}},
                    "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
                })
            }
            "ping" => json!({}),
            "tools/list" => {
                json!({"tools": tools::TOOLS.iter().map(tools::Tool::schema).collect::<Vec<_>>()})
            }
            "tools/call" => self.call_tool(&params),
            _ => {
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": "Method not found"},
                }));
            }
        };
        Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }
}

/// Serves MCP until the client closes stdin.
pub fn run(_argv: &[String]) -> Result<i32, crate::commands::Fail> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let cwd = cwd.to_string_lossy().into_owned();
    eprintln!("agentsi mcp: ready (cwd {cwd})");
    serve(
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        Server { cwd, core: None },
    )
    .map_err(|e| e.to_string())?;
    Ok(0)
}

fn serve(input: impl BufRead, mut output: impl Write, mut server: Server) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            eprintln!("agentsi mcp: a line is not JSON");
            continue;
        };
        if let Some(reply) = server.handle(&message) {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(method: &str, params: Value) -> Value {
        let mut server = Server {
            cwd: "/nowhere".into(),
            core: None,
        };
        server
            .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
            .unwrap()
    }

    #[test]
    fn initialize_answers_with_a_version_that_both_sides_know() {
        let r = ask("initialize", json!({"protocolVersion": "2025-06-18"}));
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(
            r["result"]["serverInfo"],
            json!({"name": "agentsi", "version": "0.2.0"})
        );
        let r = ask("initialize", json!({"protocolVersion": "1999-01-01"}));
        assert_eq!(r["result"]["protocolVersion"], "2025-11-25");
    }

    #[test]
    fn tools_list_has_the_five_tools_and_a_bad_call_needs_no_core() {
        let r = ask("tools/list", Value::Null);
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "search",
                "get_doc",
                "list_minutes",
                "pending_decisions",
                "role_memory"
            ]
        );
        let r = ask("tools/call", json!({"name": "nope", "arguments": {}}));
        assert_eq!(r["result"]["isError"], true);
        assert_eq!(
            r["result"]["content"][0]["text"],
            "MCP error -32602: Tool nope not found"
        );
        let r = ask("tools/call", json!({"name": "search", "arguments": {}}));
        assert!(
            r["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .ends_with("Required at query")
        );
        assert_eq!(ask("resources/list", Value::Null)["error"]["code"], -32601);
    }

    #[test]
    fn a_notification_gets_no_answer_and_the_loop_ends_at_eof() {
        let input = b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\nnot json\n{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n";
        let mut out = Vec::new();
        serve(
            &input[..],
            &mut out,
            Server {
                cwd: "/".into(),
                core: None,
            },
        )
        .unwrap();
        let lines: Vec<&str> = std::str::from_utf8(&out).unwrap().lines().collect();
        assert_eq!(lines.len(), 1);
        assert_eq!(serde_json::from_str::<Value>(lines[0]).unwrap()["id"], 7);
    }
}
