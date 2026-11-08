import { spawn } from "node:child_process";
import { copyFileSync, createWriteStream, existsSync, mkdirSync, readdirSync, renameSync, rmSync, statSync } from "node:fs";
import { join } from "node:path";
import { pipeline } from "node:stream/promises";
import { createGzip } from "node:zlib";
import { codeRoot, dataRoot } from "./paths.ts";

/** `pg_dump` in the Postgres container of `docker-compose.yml`; it writes the SQL dump on stdout. */
export const DUMP_COMMAND = ["docker", "compose", "exec", "-T", "postgres", "pg_dump", "-U", "agentsi", "agentsi"];
export const KEEP_BACKUPS = 14;

export const backupDir = (root = dataRoot) => join(root, "backups");

export interface BackupOptions {
  root?: string;
  keep?: number;
  /** The command whose stdout is the dump; tests pass a fake. */
  command?: string[];
  /** The directory the command runs in: the code checkout, which holds `docker-compose.yml`. */
  cwd?: string;
  now?: Date;
}

export interface BackupResult {
  dump: string;
  registry: string | null;
  removed: string[];
}

const stamp = (d: Date) => d.toISOString().slice(0, 16).replace("T", "-").replace(":", "");
const DUMP = /^agentsi-(\d{4}-\d{2}-\d{2}-\d{4})\.sql\.gz$/;

/** Gzips the stdout of `command` into `file`; the file appears only when the command exits 0. */
async function dumpTo(command: string[], cwd: string, file: string): Promise<void> {
  const [cmd, ...args] = command;
  const child = spawn(cmd!, args, { cwd, stdio: ["ignore", "pipe", "pipe"] });
  let stderr = "";
  child.stderr.on("data", (d) => (stderr += d));
  const exited = new Promise<number>((resolve, reject) => {
    child.on("error", reject);
    child.on("close", (code) => resolve(code ?? 1));
  });
  const tmp = `${file}.partial`;
  try {
    await pipeline(child.stdout, createGzip(), createWriteStream(tmp));
    const code = await exited;
    if (code !== 0) throw new Error(`${command.join(" ")} exited ${code}: ${stderr.trim()}`);
    renameSync(tmp, file);
  } finally {
    rmSync(tmp, { force: true });
  }
}

/**
 * Writes `backups/agentsi-<stamp>.sql.gz` and a copy of the registry in the data root, then keeps the newest
 * `keep` dumps and their registry copies. The database is the only copy of the data (ADR 0003).
 */
export async function backup(opts: BackupOptions = {}): Promise<BackupResult> {
  const root = opts.root ?? dataRoot;
  const keep = opts.keep ?? KEEP_BACKUPS;
  if (!Number.isInteger(keep) || keep < 1) throw new Error(`keep must be a whole number of 1 or more, not ${keep}`);
  const dir = backupDir(root);
  mkdirSync(dir, { recursive: true });
  const at = stamp(opts.now ?? new Date());
  const dump = join(dir, `agentsi-${at}.sql.gz`);
  await dumpTo(opts.command ?? DUMP_COMMAND, opts.cwd ?? codeRoot, dump);

  const config = join(root, "agentsi.config.json");
  const registry = existsSync(config) ? join(dir, `agentsi.config-${at}.json`) : null;
  if (registry) copyFileSync(config, registry);

  const stamps = readdirSync(dir)
    .map((f) => DUMP.exec(f)?.[1])
    .filter((s): s is string => !!s)
    .sort();
  const removed: string[] = [];
  for (const old of stamps.slice(0, Math.max(0, stamps.length - keep))) {
    for (const f of [`agentsi-${old}.sql.gz`, `agentsi.config-${old}.json`]) {
      if (!existsSync(join(dir, f))) continue;
      rmSync(join(dir, f));
      removed.push(f);
    }
  }
  return { dump, registry, removed };
}

/** Hours since the newest dump in the data root was written, or null when there is none. */
export function newestBackupAge(root = dataRoot, now = Date.now()): number | null {
  const dir = backupDir(root);
  if (!existsSync(dir)) return null;
  const times = readdirSync(dir)
    .filter((f) => DUMP.test(f))
    .map((f) => statSync(join(dir, f)).mtimeMs);
  return times.length ? (now - Math.max(...times)) / 3_600_000 : null;
}
