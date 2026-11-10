//! The arguments of the desk process, its system prompt and its stdin lines.
//!
//! The desk process runs the owner's installed `claude`, so it never takes `--bare`: bare mode
//! does not read the login of the owner. It never takes `--no-session-persistence` either,
//! because a restart of the core resumes the session.

use agentsi_proto::desk_session_api::view_tools;
use serde_json::json;

/// The built-in tools of the desk session. They only read.
pub const READ_TOOLS: [&str; 3] = ["Read", "Grep", "Glob"];

/// The tools of `agentsi mcp` that the desk session may use, apart from the show tools.
/// They only read. `add_ticket` and `update_ticket` are not here, because they write.
pub const MCP_READ_TOOLS: [&str; 7] = [
    "search",
    "get_doc",
    "list_minutes",
    "pending_decisions",
    "role_memory",
    "list_tickets",
    "get_ticket",
];

/// The name of the AgentSI server in `--mcp-config`.
pub const MCP_SERVER: &str = "agentsi";

/// The environment name that tells `agentsi mcp` the uuid of its desk session.
pub const SESSION_ENV: &str = "AGENTSI_DESK_SESSION";

/// One start of the desk process of a product.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeskCall {
    pub model: String,
    pub effort: Option<String>,
    pub system: String,
    /// The `agentsi` program, which serves `agentsi mcp`.
    pub agentsi: String,
    /// The socket of the core, for `agentsi mcp`.
    pub socket: String,
    /// The uuid of the desk session.
    pub session: String,
    /// True: `--resume` the stored session. False: `--session-id` starts it.
    pub resume: bool,
}

/// The tools of `--allowedTools`: the read tools, the read tools of `agentsi mcp` and one
/// show tool for each view.
pub fn allowed_tools() -> Vec<String> {
    READ_TOOLS
        .iter()
        .map(|t| t.to_string())
        .chain(
            MCP_READ_TOOLS
                .iter()
                .map(|t| format!("mcp__{MCP_SERVER}__{t}")),
        )
        .chain(
            view_tools()
                .into_iter()
                .map(|t| format!("mcp__{MCP_SERVER}__{}", t.name)),
        )
        .collect()
}

/// The JSON of `--mcp-config`: `agentsi mcp`, with the desk session and the socket.
pub fn mcp_config(c: &DeskCall) -> String {
    json!({"mcpServers": {MCP_SERVER: {
        "type": "stdio",
        "command": c.agentsi,
        "args": ["mcp"],
        "env": {SESSION_ENV: c.session, agentsi_proto::socket::SOCKET_ENV: c.socket},
    }}})
    .to_string()
}

pub fn desk_args(c: &DeskCall) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--model",
        &c.model,
        "--system-prompt",
        &c.system,
        "--mcp-config",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push(mcp_config(c));
    args.extend(
        [
            "--strict-mcp-config",
            "--restricted",
            "--disable-slash-commands",
            "--permission-mode",
            "dontAsk",
            "--permission-prompts",
            "none",
        ]
        .map(String::from),
    );
    args.push(format!("--tools={}", READ_TOOLS.join(",")));
    args.extend(["--allowedTools".into(), allowed_tools().join(",")]);
    if let Some(e) = &c.effort {
        args.extend(["--effort".into(), e.clone()]);
    }
    let session = if c.resume { "--resume" } else { "--session-id" };
    args.extend([session.into(), c.session.clone()]);
    args
}

/// One owner message as a stdin line of `--input-format stream-json`, with its newline.
pub fn user_line(text: &str) -> String {
    let line = json!({
        "type": "user",
        "message": {"role": "user", "content": text},
        "parent_tool_use_id": null,
    });
    format!("{line}\n")
}

/// What the system prompt names.
pub struct DeskContext<'a> {
    pub product: &'a str,
    pub owner: &'a str,
    pub language: &'a str,
    pub cwd: &'a str,
}

/// The persona of the `pm` agent, then the desk rules and the catalog of views.
pub fn system_prompt(persona: &str, ctx: &DeskContext) -> String {
    let views: Vec<String> = view_tools()
        .iter()
        .map(|t| format!("- {}: {}", t.name, t.description))
        .collect();
    let desk = include_str!("system.md")
        .replace("{product}", ctx.product)
        .replace("{owner}", ctx.owner)
        .replace("{language}", ctx.language)
        .replace("{cwd}", ctx.cwd)
        .replace("{views}", &views.join("\n"));
    format!("{}\n\n{}", persona.trim_end(), desk.trim_end())
}

/// The text that goes to the process for one owner message: the unsent desk notes, then the
/// owner text.
pub fn owner_text(notes: &[String], text: &str) -> String {
    if notes.is_empty() {
        return text.to_string();
    }
    let lines: Vec<String> = notes.iter().map(|n| format!("- {n}")).collect();
    format!("Desk notes:\n{}\n\n{text}", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn call(resume: bool) -> DeskCall {
        DeskCall {
            model: "opus".into(),
            effort: Some("high".into()),
            system: "the system".into(),
            agentsi: "/opt/agentsi/agentsi".into(),
            socket: "/run/core.sock".into(),
            session: "1b4e28ba-2fa1-41d2-883f-0016d3cca427".into(),
            resume,
        }
    }

    #[test]
    fn the_arguments_are_those_of_the_spec() {
        let args = desk_args(&call(false));
        let at = |name: &str| {
            let i = args.iter().position(|a| a == name).unwrap();
            args[i + 1].as_str()
        };
        assert_eq!(
            &args[..7],
            [
                "-p",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages"
            ]
        );
        assert_eq!(at("--model"), "opus");
        assert_eq!(at("--system-prompt"), "the system");
        assert_eq!(at("--permission-mode"), "dontAsk");
        assert_eq!(at("--permission-prompts"), "none");
        assert_eq!(at("--effort"), "high");
        assert_eq!(at("--session-id"), "1b4e28ba-2fa1-41d2-883f-0016d3cca427");
        assert!(args.contains(&"--tools=Read,Grep,Glob".to_string()));
        for flag in [
            "--strict-mcp-config",
            "--restricted",
            "--disable-slash-commands",
        ] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
        assert_eq!(
            at("--allowedTools"),
            "Read,Grep,Glob,mcp__agentsi__search,mcp__agentsi__get_doc,\
             mcp__agentsi__list_minutes,mcp__agentsi__pending_decisions,\
             mcp__agentsi__role_memory,mcp__agentsi__list_tickets,mcp__agentsi__get_ticket,\
             mcp__agentsi__show_decisions,\
             mcp__agentsi__show_priorities,mcp__agentsi__show_story,mcp__agentsi__show_diff,\
             mcp__agentsi__show_table,mcp__agentsi__show_chart,mcp__agentsi__show_checklist,\
             mcp__agentsi__show_page,mcp__agentsi__show_tickets"
        );
        let mcp: Value = serde_json::from_str(at("--mcp-config")).unwrap();
        assert_eq!(
            mcp["mcpServers"]["agentsi"],
            serde_json::json!({"type": "stdio", "command": "/opt/agentsi/agentsi", "args": ["mcp"],
                "env": {"AGENTSI_DESK_SESSION": "1b4e28ba-2fa1-41d2-883f-0016d3cca427",
                        "AGENTSI_CORE_SOCKET": "/run/core.sock"}})
        );
        let resumed = desk_args(&call(true));
        assert!(!resumed.iter().any(|a| a == "--session-id"));
        let i = resumed.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(resumed[i + 1], "1b4e28ba-2fa1-41d2-883f-0016d3cca427");
    }

    #[test]
    fn the_desk_session_never_gets_bare_mode_or_a_tool_that_writes() {
        for resume in [false, true] {
            let text = desk_args(&call(resume)).join(" ");
            for banned in [
                "--bare",
                "--no-session-persistence",
                "--permission-prompt-tool",
                "--dangerously-skip-permissions",
                "bypassPermissions",
                "Bash",
                "Edit",
                "Write",
                "NotebookEdit",
                "WebFetch",
                "add_ticket",
                "update_ticket",
            ] {
                assert!(!text.contains(banned), "{banned} in {text}");
            }
        }
    }

    #[test]
    fn the_desk_session_reads_tickets_and_names_no_tool_that_writes_them() {
        let allowed = allowed_tools();
        for read in ["mcp__agentsi__list_tickets", "mcp__agentsi__get_ticket"] {
            assert!(allowed.iter().any(|t| t == read), "{read}");
        }
        assert!(allowed.iter().any(|t| t == "mcp__agentsi__show_tickets"));
        let args = desk_args(&call(false)).join(" ");
        for write in [
            "add_ticket",
            "update_ticket",
            "ticket.move",
            "ticket.promote",
        ] {
            assert!(!args.contains(write), "{write} in {args}");
        }
    }

    #[test]
    fn an_owner_message_is_one_stream_json_line() {
        let line = user_line("What waits\nfor me?");
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let v: Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(v["type"], "user");
        assert_eq!(v["message"]["content"], "What waits\nfor me?");
        assert_eq!(v["parent_tool_use_id"], Value::Null);
    }

    #[test]
    fn the_notes_go_before_the_owner_text() {
        assert_eq!(owner_text(&[], "Hi"), "Hi");
        assert_eq!(
            owner_text(&["The owner approved decision 14.".into()], "Hi"),
            "Desk notes:\n- The owner approved decision 14.\n\nHi"
        );
    }

    #[test]
    fn the_system_prompt_holds_the_persona_the_rules_and_the_catalog() {
        let text = system_prompt(
            "You are the PM.\n",
            &DeskContext {
                product: "alpha",
                owner: "Ada",
                language: "English",
                cwd: "/work/alpha",
            },
        );
        assert!(text.starts_with("You are the PM.\n\n# The front desk"));
        assert!(text.contains("Answer in English"));
        assert!(text.contains("- show_decisions: Shows pending decisions"));
        assert!(!text.contains('{'), "{text}");
    }
}
