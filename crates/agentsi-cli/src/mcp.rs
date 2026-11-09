//! `agentsi mcp`: the MCP server on stdio, for Claude Code. It serves the 5
//! tools of `packages/mcp` with the same names and input schemas, then the 4
//! ticket tools. In a desk session (`AGENTSI_DESK_SESSION`), the `show_*` tools
//! follow, and the tools that write are refused. The server is a small JSON-RPC loop, because the protocol of a
//! server with tools needs four methods. Each tool asks the core, which owns
//! the database. A ticket tool writes as the session `{by: "session", id}`.
//!
//! The core is joined on the first call that needs data, so `initialize` and
//! `tools/list` work when no core runs. Stdout holds the protocol only.

mod desk;
mod format;
mod tools;

use crate::core::Core;
use agentsi_proto::agency_api::{ProductWhoami, WhoamiParams};
use agentsi_proto::desk_session_api::{DeskShow, DeskShowParams};
use agentsi_proto::knowledge_api::{
    KbGetDocParams, KbPendingParams, KbRecentMinutesParams, KbRoleMemoryParams, KbSearchParams,
    KnowledgeGetDoc, KnowledgePendingDecisions, KnowledgeRecentMinutes, KnowledgeRoleMemory,
    KnowledgeSearch,
};
use agentsi_proto::ticket_api::{
    Author, AuthorBy, Link, TicketAdd, TicketAddParams, TicketGet, TicketGetParams, TicketKind,
    TicketList, TicketListParams, TicketMove, TicketMoveParams, TicketState, TicketUpdate,
    TicketUpdateParams,
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
    /// The id of the session that writes tickets.
    session: String,
    /// The current tool sent a write request to the core.
    sent: bool,
    /// The uuid of the desk session, from `AGENTSI_DESK_SESSION`.
    desk: Option<String>,
}

/// `AGENTSI_TERMINAL`, else `CLAUDE_CODE_SESSION_ID`, else `unknown`.
fn session_id(var: impl Fn(&str) -> Option<String>) -> String {
    ["AGENTSI_TERMINAL", "CLAUDE_CODE_SESSION_ID"]
        .into_iter()
        .filter_map(|k| var(k).map(|v| v.trim().to_string()))
        .find(|v| !v.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// `AGENTSI_DESK_SESSION`, when it holds more than spaces.
fn desk_session(var: impl Fn(&str) -> Option<String>) -> Option<String> {
    var("AGENTSI_DESK_SESSION")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
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
    /// because the core can restart while this server runs. A write tries again only
    /// when it failed before its request went out.
    fn run_tool(&mut self, name: &str, args: &Map<String, Value>) -> Result<String, String> {
        self.sent = false;
        let first = self.ask_core(name, args);
        if first.is_err() && !self.sent && self.core.take().is_some() {
            return self.ask_core(name, args);
        }
        first
    }

    fn ask_core(&mut self, name: &str, args: &Map<String, Value>) -> Result<String, String> {
        let text = |k: &str| args.get(k).and_then(Value::as_str);
        let number = |k: &str| args.get(k).and_then(Value::as_u64);
        let product = self.product(text("product"))?;
        let by = Author {
            by: AuthorBy::Session,
            id: self.session.clone(),
        };
        if tools::tool(name).is_some_and(|t| t.writes) {
            self.sent = true;
        }
        let core = self.core()?;
        let links = || -> Result<Option<Vec<Link>>, String> {
            args.get("links")
                .map(|v| serde_json::from_value(v.clone()).map_err(|e| e.to_string()))
                .transpose()
        };
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
            "list_tickets" => {
                let states = match text("state").and_then(TicketState::parse) {
                    Some(s) => vec![s],
                    None => vec![TicketState::Open, TicketState::Waiting],
                };
                let found = core.call::<TicketList>(&TicketListParams {
                    product: product.clone(),
                    states: Some(states),
                    kind: text("kind").and_then(TicketKind::parse),
                    assignee: text("assignee").map(String::from),
                    limit: Some(number("limit").unwrap_or(20) as u32),
                    ..TicketListParams::default()
                })?;
                Ok(format::tickets_text(&product, &found.tickets))
            }
            "get_ticket" => Ok(core
                .call::<TicketGet>(&TicketGetParams {
                    product,
                    id: text("id").unwrap_or_default().to_string(),
                })?
                .markdown),
            "add_ticket" => {
                let added = core.call::<TicketAdd>(&TicketAddParams {
                    product,
                    kind: text("kind")
                        .and_then(TicketKind::parse)
                        .unwrap_or(TicketKind::Note),
                    title: text("title").unwrap_or_default().to_string(),
                    body: text("body").map(String::from),
                    assignee: text("assignee").map(String::from),
                    links: links()?,
                    scope: None,
                    objective: None,
                    author: by,
                })?;
                Ok(format!("Added {}", added.ticket.line()))
            }
            "update_ticket" => {
                let id = text("id").unwrap_or_default().to_string();
                let update = TicketUpdateParams {
                    product: product.clone(),
                    id: id.clone(),
                    title: text("title").map(String::from),
                    body: text("body").map(String::from),
                    kind: text("kind").and_then(TicketKind::parse),
                    assignee: text("assignee").map(String::from),
                    links: links()?,
                    scope: None,
                    objective: None,
                    by: by.clone(),
                };
                let waiting_on = text("waiting_on").map(String::from);
                let state = text("state")
                    .and_then(TicketState::parse)
                    .or(waiting_on.as_ref().map(|_| TicketState::Waiting));
                let fields = update.title.is_some()
                    || update.body.is_some()
                    || update.kind.is_some()
                    || update.assignee.is_some()
                    || update.links.is_some();
                if !fields && state.is_none() {
                    return Err(format!("Nothing to change in ticket {id}."));
                }
                let mut ticket = None;
                if fields {
                    ticket = Some(core.call::<TicketUpdate>(&update)?.ticket);
                }
                if let Some(state) = state {
                    let moved = core.call::<TicketMove>(&TicketMoveParams {
                        product,
                        id,
                        state,
                        waiting_on,
                        by,
                    })?;
                    ticket = Some(moved.ticket);
                }
                Ok(ticket.map(|t| t.line()).unwrap_or_default())
            }
            other => Err(format!("MCP error -32602: Tool {other} not found")),
        }
    }

    /// A `show_*` tool: sends `desk.show` for the desk session.
    fn show(&mut self, session: &str, kind: &str, given: &Value) -> Result<String, String> {
        let args = given.as_object().cloned().unwrap_or_default();
        let view = desk::view(kind, &args)?;
        let params = DeskShowParams {
            session: session.to_string(),
            view,
        };
        self.sent = true;
        self.core()?.call::<DeskShow>(&params)?;
        Ok("The view is in the desk conversation.".into())
    }

    fn call_tool(&mut self, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let given = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let desk = self.desk.clone();
        let show = desk.as_ref().and_then(|_| desk::kind(name));
        let result = match (tools::tool(name), desk, show) {
            (_, Some(session), Some(kind)) => self.show(&session, &kind, &given),
            (Some(tool), Some(_), _) if tool.writes => Err(format!(
                "The desk session only reads. It cannot run {name}."
            )),
            (tool, _, _) => self.base_tool(tool, name, &given),
        };
        match result {
            Ok(text) => json!({"content": [{"type": "text", "text": text}]}),
            Err(text) => json!({"content": [{"type": "text", "text": text}], "isError": true}),
        }
    }

    fn base_tool(
        &mut self,
        tool: Option<&tools::Tool>,
        name: &str,
        given: &Value,
    ) -> Result<String, String> {
        match tool {
            None => Err(format!("MCP error -32602: Tool {name} not found")),
            Some(tool) => tool
                .validate(given)
                .and_then(|args| self.run_tool(name, &args)),
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
                let mut list: Vec<Value> = tools::TOOLS.iter().map(tools::Tool::schema).collect();
                if self.desk.is_some() {
                    list.extend(desk::tool_schemas());
                }
                json!({"tools": list})
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
        Server {
            cwd,
            core: None,
            session: session_id(|k| std::env::var(k).ok()),
            sent: false,
            desk: desk_session(|k| std::env::var(k).ok()),
        },
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
            session: "unknown".into(),
            sent: false,
            desk: None,
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
    fn tools_list_has_the_nine_tools_and_a_bad_call_needs_no_core() {
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
                "role_memory",
                "list_tickets",
                "get_ticket",
                "add_ticket",
                "update_ticket"
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

    fn desk_server() -> Server {
        Server {
            cwd: "/nowhere".into(),
            core: None,
            session: "unknown".into(),
            sent: false,
            desk: Some("u1".into()),
        }
    }

    #[test]
    fn a_desk_session_lists_the_show_tools_after_the_base_tools_and_refuses_writes() {
        let mut server = desk_server();
        let r = server
            .handle(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
            .unwrap();
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names.len(), 13);
        assert_eq!(
            names[8..],
            [
                "update_ticket",
                "show_decisions",
                "show_priorities",
                "show_story",
                "show_diff"
            ]
        );
        for (name, args) in [
            ("add_ticket", json!({"kind": "note", "title": "x"})),
            ("update_ticket", json!({"id": "C001", "state": "done"})),
        ] {
            let r = server
                .handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                                "params": {"name": name, "arguments": args}}))
                .unwrap();
            assert_eq!(r["result"]["isError"], true);
            assert_eq!(
                r["result"]["content"][0]["text"],
                format!("The desk session only reads. It cannot run {name}.")
            );
        }
        let r = server
            .handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                            "params": {"name": "show_decisions", "arguments": {"ids": "x"}}}))
            .unwrap();
        assert_eq!(r["result"]["isError"], true);
        let r = ask(
            "tools/call",
            json!({"name": "show_decisions", "arguments": {"ids": [1]}}),
        );
        assert_eq!(
            r["result"]["content"][0]["text"],
            "MCP error -32602: Tool show_decisions not found"
        );
    }

    #[test]
    fn the_desk_session_comes_from_its_variable_when_it_is_not_blank() {
        let env =
            |v: &'static str| move |k: &str| (k == "AGENTSI_DESK_SESSION").then(|| v.to_string());
        assert_eq!(desk_session(env(" u1 ")).as_deref(), Some("u1"));
        assert_eq!(desk_session(env("  ")), None);
        assert_eq!(desk_session(|_| None), None);
    }

    #[test]
    fn the_session_id_is_the_terminal_then_the_claude_session_then_unknown() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| pairs.iter().find(|p| p.0 == k).map(|p| p.1.to_string())
        };
        assert_eq!(
            session_id(env(&[
                ("AGENTSI_TERMINAL", "t1"),
                ("CLAUDE_CODE_SESSION_ID", "c1")
            ])),
            "t1"
        );
        assert_eq!(
            session_id(env(&[
                ("AGENTSI_TERMINAL", " "),
                ("CLAUDE_CODE_SESSION_ID", "c1")
            ])),
            "c1"
        );
        assert_eq!(session_id(env(&[])), "unknown");
    }

    #[test]
    fn a_bad_ticket_call_is_refused_before_the_core() {
        let r = ask(
            "tools/call",
            json!({"name": "add_ticket", "arguments": {"kind": "task", "title": "x"}}),
        );
        assert_eq!(r["result"]["isError"], true);
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("received 'task' at kind"), "{text}");
        let r = ask(
            "tools/call",
            json!({"name": "update_ticket", "arguments": {"id": "S001",
                   "links": [{"type": "note", "ref": ""}]}}),
        );
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Invalid at id"), "{text}");
        assert!(text.contains("Invalid link type at links.0.type"), "{text}");
        assert!(text.contains("Required at links.0.ref"), "{text}");
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
                session: "unknown".into(),
                sent: false,
                desk: None,
            },
        )
        .unwrap();
        let lines: Vec<&str> = std::str::from_utf8(&out).unwrap().lines().collect();
        assert_eq!(lines.len(), 1);
        assert_eq!(serde_json::from_str::<Value>(lines[0]).unwrap()["id"], 7);
    }
}
