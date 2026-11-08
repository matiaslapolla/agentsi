//! The code graph: the MCP access that a role gets, and the reindex of a repo.
//! The rules copy `packages/core/src/code-graph.ts`. The graph itself comes from
//! the program `codebase-memory-mcp`, which is not part of AgentSI.

use super::exec;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

const BINARY: &str = "codebase-memory-mcp";
pub const CODE_GRAPH_SERVER: &str = "codegraph";
const REINDEX_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The read-only tools of `--tool-profile=analysis` in 0.12.1. The full
/// profile also exposes `index_repository`, `delete_project`, `manage_adr`
/// and `ingest_traces`, which write.
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

pub struct McpAccess {
    /// JSON for `--mcp-config`.
    pub config: String,
    /// Tool names for `--allowedTools`, as `mcp__<server>__<tool>`.
    pub tools: Vec<String>,
}

pub fn mcp_access() -> McpAccess {
    let config: Value = json!({
        "mcpServers": {
            CODE_GRAPH_SERVER: {"type": "stdio", "command": BINARY, "args": ["--tool-profile=analysis"]}
        }
    });
    McpAccess {
        config: config.to_string(),
        tools: ANALYSIS_TOOLS
            .iter()
            .map(|t| format!("mcp__{CODE_GRAPH_SERVER}__{t}"))
            .collect(),
    }
}

fn project_name(stdout: &str) -> Option<String> {
    let value: Value = serde_json::from_str(stdout).ok()?;
    value
        .get("project")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .map(String::from)
}

/// Reindexes one repo. Returns its code graph project name, or `None` after one
/// log line when the binary is missing or fails.
pub fn reindex(repo: &Path, log: &mut dyn FnMut(&str)) -> Option<String> {
    reindex_with(BINARY, repo, log)
}

fn reindex_with(program: &str, repo: &Path, log: &mut dyn FnMut(&str)) -> Option<String> {
    let repo_path = repo.to_string_lossy();
    let res = exec::run(
        program,
        &[
            "cli",
            "--quiet",
            "index_repository",
            "--repo-path",
            &repo_path,
        ],
        repo,
        REINDEX_TIMEOUT,
    );
    if res.ok
        && let Some(project) = project_name(&res.stdout)
    {
        return Some(project);
    }
    let text = if res.stderr.trim().is_empty() {
        &res.stdout
    } else {
        &res.stderr
    };
    let first = text.trim().lines().next().unwrap_or("");
    let reason = if first.is_empty() {
        format!("{program} is not on PATH")
    } else {
        first.chars().take(200).collect()
    };
    log(&format!(
        "code graph: no reindex of {}: {reason}",
        repo.display()
    ));
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_access_names_the_read_only_tools_of_the_server() {
        let access = mcp_access();
        assert_eq!(access.tools.len(), 13);
        assert_eq!(access.tools[0], "mcp__codegraph__search_graph");
        assert!(!access.tools.iter().any(|t| t.contains("index_repository")));
        let config: Value = serde_json::from_str(&access.config).unwrap();
        assert_eq!(
            config["mcpServers"]["codegraph"]["args"][0],
            "--tool-profile=analysis"
        );
    }

    #[test]
    fn a_missing_binary_logs_one_line_and_gives_no_project() {
        let dir = tempfile::tempdir().unwrap();
        let mut lines = Vec::new();
        let project = reindex_with("agentsi-no-such-program", dir.path(), &mut |l| {
            lines.push(l.to_string())
        });
        assert_eq!(project, None);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with("code graph: no reindex of "));
        assert!(
            lines[0].contains("No such file") || lines[0].contains("not found"),
            "{}",
            lines[0]
        );
    }

    #[test]
    fn a_project_name_must_be_a_non_empty_string() {
        assert_eq!(
            project_name("{\"project\":\"app\"}").as_deref(),
            Some("app")
        );
        assert_eq!(project_name("{\"project\":\"\"}"), None);
        assert_eq!(project_name("{\"project\":3}"), None);
        assert_eq!(project_name("not json"), None);
    }
}
