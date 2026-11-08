import { existsSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import pg from "pg";
import { newestBackupAge } from "./backup.ts";
import { DEFAULT_DATABASE_URL } from "./db.ts";
import { run } from "./exec.ts";
import { resolveDataRoot, type DataRootSource } from "./paths.ts";
import { loadRegistry } from "./registry.ts";
import { BWRAP, sandboxWorks } from "./sandbox.ts";

/** `required` checks decide the exit code; the others only name what they block. */
export type DoctorLevel = "required" | "stories" | "optional";

export type DoctorStatus = "ok" | "missing" | "fail";

export interface DoctorResult {
  name: string;
  level: DoctorLevel;
  status: DoctorStatus;
  /** The version or the found path when ok; why it failed otherwise, empty when missing. */
  detail: string;
  /** The features that a failed check blocks. */
  blocks: string;
}

/** What the checks read from the machine; tests pass fakes. */
export interface DoctorProbes {
  /** The trimmed output of `cmd args`, or null when the command is missing or fails. */
  output(cmd: string, args: string[]): Promise<string | null>;
  /** The server version at `url`, or a throw when the database is not reachable. */
  database(url: string): Promise<string>;
  /** True when bwrap can start a sandbox with no network. */
  sandbox(): Promise<boolean>;
  dataRoot(): DataRootSource;
  /** The number of products in the registry of `root`; throws when it is missing or not valid JSON. */
  products(root: string): number;
  isDir(path: string): boolean;
  /** Hours since the newest dump in `root`, or null with none. */
  backupAge(root: string): number | null;
  exists(path: string): boolean;
  env: NodeJS.ProcessEnv;
}

/** Claude Code flags that every model call of AgentSI passes; an older CLI rejects them. */
export const CLAUDE_FLAGS = ["--safe-mode", "--restricted", "--permission-prompts", "--max-budget-usd", "--strict-mcp-config", "--disable-slash-commands"];

const firstLine = (s: string) => s.split("\n")[0]!.trim();

export const realProbes: DoctorProbes = {
  async output(cmd, args) {
    const r = await run(cmd, args, homedir(), 15_000);
    return r.ok ? `${r.stdout}${r.stderr}`.trim() : null;
  },
  async database(url) {
    const client = new pg.Client({ connectionString: url, connectionTimeoutMillis: 5000, query_timeout: 5000 });
    try {
      await client.connect();
      const { rows } = await client.query<{ version: string }>("select current_setting('server_version') as version");
      return rows[0]!.version;
    } finally {
      await client.end().catch(() => {});
    }
  },
  sandbox: sandboxWorks,
  dataRoot: () => resolveDataRoot(),
  products: (root) => loadRegistry(join(root, "agentsi.config.json")).products.length,
  isDir: (path) => existsSync(path) && statSync(path).isDirectory(),
  backupAge: (root) => newestBackupAge(root),
  exists: existsSync,
  env: process.env,
};

type Outcome = { status: DoctorStatus; detail: string };
type Check = Omit<DoctorResult, "status" | "detail"> & { probe: (p: DoctorProbes) => Promise<Outcome> };

const ok = (detail: string): Outcome => ({ status: "ok", detail });
const failed = (detail: string): Outcome => ({ status: "fail", detail });
const MISSING: Outcome = { status: "missing", detail: "" };

/** The URL without its password, so the output can go into an issue. */
const redact = (url: string): string => {
  try {
    const u = new URL(url);
    if (u.password) u.password = "***";
    return u.toString();
  } catch {
    return "(a URL that does not parse)";
  }
};

/** A check that passes when `cmd args` runs; its first output line is the detail. */
const tool = (name: string, level: DoctorLevel, blocks: string, args = ["--version"], cmd = name): Check => ({
  name,
  level,
  blocks,
  probe: async (p) => {
    const out = await p.output(cmd, args);
    return out === null ? MISSING : ok(firstLine(out));
  },
});

/** The daily timer runs `agentsi backup`; two missed days are a warning. */
const BACKUP_MAX_HOURS = 48;

const CHECKS: Check[] = [
  tool("node", "required", "every command"),
  tool("pnpm", "required", "the install and the web app"),
  tool("docker", "required", "the database (agentsi up)"),
  {
    name: "database",
    level: "required",
    blocks: "meetings, the knowledge base, status and the web app",
    probe: async (p) => {
      const url = p.env.AGENTSI_DATABASE_URL ?? DEFAULT_DATABASE_URL;
      try {
        return ok(`PostgreSQL ${await p.database(url)}`);
      } catch (err) {
        return failed(`not reachable at ${redact(url)}: ${(err as Error).message}. Start it with agentsi up`);
      }
    },
  },
  {
    name: "claude",
    level: "required",
    blocks: "meetings, ask, reflect and stories",
    probe: async (p) => {
      const version = await p.output("claude", ["--version"]);
      if (version === null) return MISSING;
      const help = (await p.output("claude", ["--help"])) ?? "";
      const absent = CLAUDE_FLAGS.filter((f) => !help.includes(f));
      if (absent.length) return failed(`${firstLine(version)} lacks ${absent.join(", ")}. Update Claude Code`);
      return ok(firstLine(version));
    },
  },
  {
    name: "data root",
    level: "required",
    blocks: "every command that reads the registry or the workspace",
    probe: async (p) => {
      const { root, from } = p.dataRoot();
      if (!p.isDir(root)) return failed(`${root} (${from}) does not exist`);
      try {
        return ok(`${root} (${from}), ${p.products(root)} products`);
      } catch (err) {
        return failed(`${root} (${from}) has no valid agentsi.config.json: ${(err as Error).message}`);
      }
    },
  },
  {
    name: "backup",
    level: "optional",
    blocks: "recovery of the data after a lost database (agentsi backup)",
    probe: async (p) => {
      const hours = p.backupAge(p.dataRoot().root);
      if (hours === null) return MISSING;
      const age = hours < 48 ? `${Math.round(hours)} h old` : `${Math.round(hours / 24)} days old`;
      return hours > BACKUP_MAX_HOURS ? failed(`the newest dump is ${age}. Run agentsi backup, and check the timer`) : ok(`newest dump ${age}`);
    },
  },
  {
    name: BWRAP,
    level: "stories",
    blocks: "story sessions (agentsi story run)",
    probe: async (p) => {
      const version = await p.output(BWRAP, ["--version"]);
      if (version === null) return MISSING;
      return (await p.sandbox()) ? ok(firstLine(version)) : failed(`${firstLine(version)} cannot start a sandbox`);
    },
  },
  tool("gh", "optional", "open issues in the knowledge base and the issue check of the minutes"),
  tool("codebase-memory-mcp", "optional", "the code graph of meetings and story sessions"),
  tool("uv", "optional", "meeting audio (agentsi render)"),
  tool("ffmpeg", "optional", "meeting audio (agentsi render)", ["-version"]),
  {
    name: "TTS models",
    level: "optional",
    blocks: "meeting audio (agentsi render)",
    probe: async (p) => {
      const dir = p.env.TTS_MODELS_DIR?.trim() || join(p.env.XDG_DATA_HOME?.trim() || join(homedir(), ".local/share"), "agentsi/models");
      return p.isDir(dir) ? ok(dir) : failed(`${dir} does not exist. Set TTS_MODELS_DIR`);
    },
  },
];

/** Runs every check in order; nothing is written. */
export async function doctor(probes: DoctorProbes = realProbes): Promise<DoctorResult[]> {
  const results: DoctorResult[] = [];
  for (const { probe, ...check } of CHECKS) results.push({ ...check, ...(await probe(probes)) });
  return results;
}

export const doctorPassed = (results: DoctorResult[]): boolean => results.every((r) => r.status === "ok" || r.level !== "required");
