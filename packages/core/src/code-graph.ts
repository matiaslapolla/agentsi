import { run } from "./exec.ts";

export interface McpAccess {
  /** JSON for `--mcp-config`. */
  config: string;
  /** Tool names for `--allowedTools`, as mcp__<server>__<tool>. */
  tools: string[];
}

const BINARY = "codebase-memory-mcp";
export const CODE_GRAPH_SERVER = "codegraph";
const REINDEX_TIMEOUT_MS = 5 * 60_000;

// The read-only tools of `--tool-profile=analysis` in 0.12.1. The full profile also exposes index_repository,
// delete_project, manage_adr and ingest_traces, which write.
const ANALYSIS_TOOLS = [
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

export function codeGraphMcp(): McpAccess {
  const config = { mcpServers: { [CODE_GRAPH_SERVER]: { type: "stdio", command: BINARY, args: ["--tool-profile=analysis"] } } };
  return { config: JSON.stringify(config), tools: ANALYSIS_TOOLS.map((t) => `mcp__${CODE_GRAPH_SERVER}__${t}`) };
}

function projectName(stdout: string): string | null {
  try {
    const { project } = JSON.parse(stdout) as { project?: unknown };
    return typeof project === "string" && project ? project : null;
  } catch {
    return null;
  }
}

/** Reindexes one repo. Returns its code graph project name, or null after one log line when the binary is missing or fails. */
export async function reindexCodeGraph(repo: string, log: (line: string) => void = console.error): Promise<string | null> {
  const res = await run(BINARY, ["cli", "--quiet", "index_repository", "--repo-path", repo], repo, REINDEX_TIMEOUT_MS);
  const project = res.ok ? projectName(res.stdout) : null;
  if (project) return project;
  const reason = (res.stderr || res.stdout).trim().split("\n")[0] || `${BINARY} is not on PATH`;
  log(`code graph: no reindex of ${repo}: ${reason.slice(0, 200)}`);
  return null;
}
