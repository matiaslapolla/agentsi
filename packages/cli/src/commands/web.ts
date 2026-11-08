import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, openSync } from "node:fs";
import { createConnection } from "node:net";
import { homedir } from "node:os";
import { join } from "node:path";
import { DEFAULT_DATABASE_URL, codeRoot, dataRoot } from "@agentsi/core";
import type { Command } from "../command.ts";

const WEB_HOST = "127.0.0.1";
const WEB_PORT = 3361;
const WEB_URL = `http://${WEB_HOST}:${WEB_PORT}`;

function listening(host: string, port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const s = createConnection({ host, port });
    s.once("connect", () => (s.destroy(), resolve(true)));
    s.once("error", () => resolve(false));
    s.setTimeout(1000, () => (s.destroy(), resolve(false)));
  });
}

async function waitFor(host: string, port: number, timeoutMs: number): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await listening(host, port)) return true;
    await new Promise((r) => setTimeout(r, 500));
  }
  return false;
}

export const web: Command = {
  name: "web",
  summary: "Start the web app",
  usage: "agentsi web",
  async run() {
    if (!existsSync(join(codeRoot, "apps/web/package.json"))) {
      throw new Error(`apps/web is not there yet (${join(codeRoot, "apps/web")}); the web app has not been built.`);
    }
    const db = new URL(process.env.AGENTSI_DATABASE_URL ?? DEFAULT_DATABASE_URL);
    const dbPort = Number(db.port || 5432);
    if (!(await listening(db.hostname, dbPort))) {
      const up = spawnSync("docker", ["compose", "up", "-d", "--wait", "postgres"], { cwd: codeRoot, stdio: "inherit" });
      if (up.status !== 0) throw new Error("Postgres is not running and `docker compose up -d --wait postgres` failed");
    }
    if (!(await listening(WEB_HOST, WEB_PORT))) {
      const logDir = join(homedir(), ".cache/agentsi");
      mkdirSync(logDir, { recursive: true });
      const log = openSync(join(logDir, "web.log"), "a");
      const child = spawn("pnpm", ["--filter", "@agentsi/web", "dev"], {
        cwd: codeRoot,
        env: { ...process.env, AGENTSI_HOME: dataRoot },
        detached: true,
        stdio: ["ignore", log, log],
      });
      child.unref();
      if (!(await waitFor(WEB_HOST, WEB_PORT, 60_000))) {
        throw new Error(`The web app did not answer on port ${WEB_PORT} within 60s. See ${join(logDir, "web.log")}`);
      }
    }
    console.log(WEB_URL);
  },
};
