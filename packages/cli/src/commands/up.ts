import { spawnSync } from "node:child_process";
import { migrate, codeRoot } from "@agentsi/core";
import type { Command } from "../command.ts";

export const up: Command = {
  name: "up",
  summary: "Start Postgres and apply migrations",
  usage: "agentsi up",
  async run() {
    const res = spawnSync("docker", ["compose", "up", "-d", "--wait", "postgres"], { cwd: codeRoot, stdio: "inherit" });
    if (res.status !== 0) return res.status ?? 1;
    const applied = await migrate();
    console.log(applied.length ? `Applied: ${applied.join(", ")}` : "Database up to date.");
  },
};
