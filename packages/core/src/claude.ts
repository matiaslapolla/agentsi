import { spawn } from "node:child_process";
import type { McpAccess } from "./code-graph.ts";
import type { UsageRow } from "./types.ts";

export interface ClaudeCall {
  prompt: string;
  model: string;
  system: string;
  cwd: string;
  /** Built-in tools the call may use. Empty means none. */
  tools: string[];
  /** JSON Schema of the structured answer. Omit for plain text. */
  schema?: object;
  effort?: string;
  timeoutMs?: number;
  maxBudgetUsd?: number;
  /** MCP servers and the tools of theirs the call may use. */
  mcp?: McpAccess;
}

export interface ClaudeResult {
  /** Parsed structured output, or the plain text answer when no schema was given. */
  output: unknown;
  usage: Omit<UsageRow, "role">;
}

export type ClaudeCaller = (c: ClaudeCall) => Promise<ClaudeResult>;

export interface RawResult {
  is_error?: boolean;
  subtype?: string;
  result?: string;
  /** Set on some error results, such as a budget stop, which carry no `result`. */
  errors?: string[];
  structured_output?: unknown;
  duration_ms?: number;
  total_cost_usd?: number;
  usage?: {
    input_tokens?: number;
    output_tokens?: number;
    cache_read_input_tokens?: number;
    cache_creation_input_tokens?: number;
  };
  modelUsage?: Record<string, { costUSD?: number }>;
}

/** A `claude -p` run that ended with `is_error`; `raw` keeps its envelope. */
export class ClaudeError extends Error {
  readonly raw: RawResult;
  constructor(raw: RawResult) {
    super(`claude error (${raw.subtype ?? "unknown"}): ${String(raw.result ?? raw.errors?.join("; ") ?? "").slice(0, 300)}`);
    this.raw = raw;
  }
}

/** The `subtype` of a run that reached `--max-budget-usd`. */
export const BUDGET_STOP = "error_max_budget_usd";

/** A loop guard, not a cost target: about 3x the most expensive call measured per model. */
export const defaultBudgetUsd = (model: string) => (model.includes("haiku") ? 0.1 : 0.4);

export function claudeArgs(c: ClaudeCall): string[] {
  const args = [
    "-p",
    "--model", c.model,
    "--system-prompt", c.system,
    "--output-format", "json",
    "--no-session-persistence",
    // --safe-mode also drops the servers of --mcp-config. --restricted and --strict-mcp-config still apply.
    ...(c.mcp ? [] : ["--safe-mode"]),
    "--restricted",
    "--strict-mcp-config",
    "--disable-slash-commands",
    "--permission-mode", "dontAsk",
    "--permission-prompts", "none",
    "--max-budget-usd", String(c.maxBudgetUsd ?? defaultBudgetUsd(c.model)),
    `--tools=${c.tools.join(",")}`,
  ];
  if (c.mcp) args.push("--mcp-config", c.mcp.config);
  const allowed = [...c.tools, ...(c.mcp?.tools ?? [])];
  if (allowed.length) args.push("--allowedTools", allowed.join(","));
  if (c.effort) args.push("--effort", c.effort);
  if (c.schema) args.push("--json-schema", JSON.stringify(c.schema));
  return args;
}

/** Maps the `claude -p --output-format json` envelope to the usage contract. */
export function mapUsage(raw: RawResult, requestedModel: string): Omit<UsageRow, "role"> {
  const u = raw.usage ?? {};
  const models = Object.entries(raw.modelUsage ?? {}).sort((a, b) => (b[1].costUSD ?? 0) - (a[1].costUSD ?? 0));
  return {
    model: models[0]?.[0] ?? requestedModel,
    input_tokens: u.input_tokens ?? 0,
    output_tokens: u.output_tokens ?? 0,
    cache_read: u.cache_read_input_tokens ?? 0,
    cache_write: u.cache_creation_input_tokens ?? 0,
    cost_usd: raw.total_cost_usd ?? 0,
    duration_ms: Math.round(raw.duration_ms ?? 0),
  };
}

export const totalInputTokens = (u: Pick<UsageRow, "input_tokens" | "cache_read" | "cache_write">) =>
  u.input_tokens + u.cache_read + u.cache_write;

export interface ClaudeProcess {
  args: string[];
  cwd: string;
  prompt: string;
  timeoutMs: number;
  env?: NodeJS.ProcessEnv;
}

/** Runs `claude` once and parses its JSON envelope. The prompt goes through stdin to avoid argv limits. */
export function runClaude(p: ClaudeProcess): Promise<RawResult> {
  return new Promise((resolve, reject) => {
    const child = spawn("claude", p.args, { cwd: p.cwd, env: p.env ?? process.env, stdio: ["pipe", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      child.kill("SIGKILL");
    }, p.timeoutMs);
    child.stdout.on("data", (d) => (stdout += d));
    child.stderr.on("data", (d) => (stderr += d));
    child.on("error", (err) => {
      clearTimeout(timer);
      reject(err);
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      if (timedOut) return reject(new Error(`claude timed out after ${Math.round(p.timeoutMs / 1000)}s`));
      let raw: RawResult;
      try {
        raw = JSON.parse(stdout) as RawResult;
      } catch {
        return reject(new Error(`claude exited ${code} without JSON: ${(stderr || stdout).trim().slice(0, 300)}`));
      }
      if (raw.is_error) return reject(new ClaudeError(raw));
      resolve(raw);
    });
    child.stdin.on("error", () => {});
    child.stdin.end(p.prompt);
  });
}

/** One headless Claude Code call. */
export async function claude(c: ClaudeCall): Promise<ClaudeResult> {
  // A call without tools is a single turn: a cache write would cost 25% more and never be read.
  const env = c.tools.length ? process.env : { ...process.env, DISABLE_PROMPT_CACHING: "1" };
  const raw = await runClaude({ args: claudeArgs(c), cwd: c.cwd, prompt: c.prompt, timeoutMs: c.timeoutMs ?? 6 * 60_000, env });
  const output = c.schema ? raw.structured_output : raw.result;
  if (output === undefined || output === null || output === "") {
    throw new Error(`claude returned no ${c.schema ? "structured output" : "text"} (${raw.subtype ?? "unknown"})`);
  }
  return { output, usage: mapUsage(raw, c.model) };
}
