//! `code-graph.ts`: the code graph server that a story session may use, and
//! its reindex before the session.

use super::claude::McpAccess;
use crate::sandbox::run_command;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

const BINARY: &str = "codebase-memory-mcp";
pub const SERVER: &str = "codegraph";
const REINDEX_TIMEOUT: Duration = Duration::from_secs(5 * 60);

// The read-only tools of `--tool-profile=analysis` in 0.12.1. The full profile also exposes tools that write.
const ANALYSIS_TOOLS: [&str; 13] = [
    "search_graph",
    "query_graph",
    "trace_path",
    "get_code_snippet",
    "get_file_outline",
    "get_graph_schema",
    "compare_graphs",
    "get_architecture",
    "search_code",
    "list_projects",
    "index_status",
    "check_index_coverage",
    "detect_changes",
];

pub fn mcp() -> McpAccess {
    let config = json!({ "mcpServers": { SERVER: { "type": "stdio", "command": BINARY, "args": ["--tool-profile=analysis"] } } });
    McpAccess {
        config: config.to_string(),
        tools: ANALYSIS_TOOLS
            .iter()
            .map(|t| format!("mcp__{SERVER}__{t}"))
            .collect(),
    }
}

/// Reindexes one repo. The code graph project name, or `None` after one log
/// line when the binary is missing or fails.
pub fn reindex(repo: &Path, log: &dyn Fn(&str), on_start: &dyn Fn(Option<u32>)) -> Option<String> {
    let args: Vec<String> = ["cli", "--quiet", "index_repository", "--repo-path"]
        .iter()
        .map(|s| s.to_string())
        .chain(std::iter::once(repo.to_string_lossy().into_owned()))
        .collect();
    let res = run_command(BINARY, &args, repo, REINDEX_TIMEOUT, on_start);
    let project = if res.ok {
        serde_json::from_str::<Value>(&res.stdout)
            .ok()
            .and_then(|v| {
                v["project"]
                    .as_str()
                    .filter(|p| !p.is_empty())
                    .map(String::from)
            })
    } else {
        None
    };
    if project.is_some() {
        return project;
    }
    let text = if res.stderr.is_empty() {
        &res.stdout
    } else {
        &res.stderr
    };
    let first = text.trim().lines().next().unwrap_or_default();
    let reason = if first.is_empty() {
        format!("{BINARY} is not on PATH")
    } else {
        first.chars().take(200).collect()
    };
    log(&format!(
        "code graph: no reindex of {}: {reason}",
        repo.display()
    ));
    None
}
