import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** The code checkout: packages, web app, plugin and the global roles. */
export const codeRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");

export interface DataRootSource {
  root: string;
  from: "AGENTSI_HOME" | "home file" | "data folder" | "code checkout";
}

const homeFile = (env: NodeJS.ProcessEnv) => join(env.XDG_CONFIG_HOME?.trim() || join(homedir(), ".config"), "agentsi", "home");

/** Relative to the home dir, never the cwd: the CLI, the web app and the MCP server run from different dirs. */
const fromHome = (path: string) => resolve(homedir(), path.replace(/^~(?=\/|$)/, homedir()));

/**
 * `AGENTSI_HOME`, then the one-line home file, then the ignored `data/` folder of the code checkout.
 * A checkout that still keeps its registry at the top and has no `data/` folder stays where it is (ADR 0003).
 */
export function resolveDataRoot(code = codeRoot, env: NodeJS.ProcessEnv = process.env): DataRootSource {
  const fromEnv = env.AGENTSI_HOME?.trim();
  if (fromEnv) return { root: fromHome(fromEnv), from: "AGENTSI_HOME" };
  const file = homeFile(env);
  const fromFile = existsSync(file) ? readFileSync(file, "utf8").split("\n")[0]!.trim() : "";
  if (fromFile) return { root: fromHome(fromFile), from: "home file" };
  if (existsSync(join(code, "agentsi.config.json")) && !existsSync(join(code, "data"))) return { root: code, from: "code checkout" };
  return { root: join(code, "data"), from: "data folder" };
}

/** Holds `agentsi.config.json` and `workspace/`, and gets their commits. */
export const dataRoot = resolveDataRoot().root;

export const configPath = join(dataRoot, "agentsi.config.json");
export const sqlDir = join(codeRoot, "packages/core/sql");

/** Every path below lives in `root`, the data root by default; tests pass a temp repo. */
export const workspaceRoot = (root = dataRoot) => join(root, "workspace");
/** A root that carries `plugin/agents/` (a test repo) uses its own; the data root uses the code checkout's. */
export const pluginAgentsDir = (root = dataRoot) =>
  existsSync(join(root, "plugin", "agents")) ? join(root, "plugin", "agents") : join(codeRoot, "plugin", "agents");
export const workspaceDir = (product: string, root = dataRoot) => join(workspaceRoot(root), product);
export const minutesDir = (product: string, root = dataRoot) => join(workspaceDir(product, root), "minutes");
export const audioDir = (product: string, minutesId: string, root = dataRoot) => join(workspaceDir(product, root), "audio", minutesId);
/** Voice mode: one `<session id>.json` per Claude Code session that turned it on, plus transcripts and the log. */
export const voiceDir = (root = dataRoot) => join(root, "voice");
// Where minutes, the meeting log, snapshots, listens, reflections and the archive record lived before they moved to Postgres; only `agentsi data import` reads them.
export const meetingLogPath = (product: string, root = dataRoot) => join(workspaceDir(product, root), "meetings.jsonl");
export const snapshotPath = (product: string, minutesId: string, root = dataRoot) =>
  join(workspaceDir(product, root), "snapshots", `${minutesId}.md`);
export const reflectionsPath = (product: string, root = dataRoot) => join(workspaceDir(product, root), "reflections.jsonl");
export const listensPath = (product: string, root = dataRoot) => join(workspaceDir(product, root), "listens.jsonl");
export const archivePath = (product: string, root = dataRoot) => join(workspaceDir(product, root), "archive.json");
/** Held while a reflection proposal is applied or reverted; gitignored. */
export const workspaceLockPath = (product: string, root = dataRoot) => join(workspaceDir(product, root), ".lock");
/** Where stories lived before they moved to Postgres; only `agentsi data import` reads it. */
export const storiesDir = (product: string, root = dataRoot) => join(workspaceDir(product, root), "stories");
