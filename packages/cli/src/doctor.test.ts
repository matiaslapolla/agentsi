import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import type { DoctorProbes } from "@agentsi/core";
import { runDoctor } from "./commands/doctor.ts";

const bin = fileURLToPath(new URL("../bin/agentsi.mjs", import.meta.url));

/** Every tool answers `<cmd> 1.0`, except the ones in `missing`; the database answers unless `dbDown`. */
const probes = (missing: string[], dbDown = false): DoctorProbes => ({
  output: async (cmd, args) => (missing.includes(cmd) ? null : args[0] === "--help" ? "--safe-mode --restricted --permission-prompts --max-budget-usd --strict-mcp-config --disable-slash-commands" : `${cmd} 1.0`),
  database: async () => (dbDown ? Promise.reject(new Error("ECONNREFUSED")) : "17.2"),
  sandbox: async () => true,
  dataRoot: () => ({ root: "/data", from: "home file" }),
  products: () => 1,
  isDir: () => true,
  exists: () => true,
  backupAge: () => 3,
  env: { TTS_MODELS_DIR: "/models" },
});

async function report(p: DoctorProbes, t: { mock: { method: typeof test.mock.method } }) {
  const out: string[] = [];
  t.mock.method(console, "log", (line: string) => out.push(line));
  const code = await runDoctor(p);
  return { code, out: out.join("\n") };
}

test("doctor prints one line for each check and exits 0 when only optional tools are missing", async (t) => {
  const { code, out } = await report(probes(["gh", "bwrap"]), t);
  assert.equal(code, 0);
  const lines = out.split("\n").filter((l) => /^(ok|missing|fail) /.test(l));
  assert.equal(lines.length, 13);
  assert.match(out, /^ok +node +node 1\.0$/m);
  assert.match(out, /^missing +bwrap \(stories\) +Not found\. It blocks story sessions \(agentsi story run\)\.$/m);
  assert.match(out, /^missing +gh \(optional\) +Not found\. It blocks open issues in the knowledge base/m);
  assert.match(out, /Every required check passes\./);
});

test("doctor exits 1 and names the blocked features when a required check fails", async (t) => {
  const { code, out } = await report(probes(["claude"], true), t);
  assert.equal(code, 1);
  assert.match(out, /^missing +claude +Not found\. It blocks meetings, ask, reflect and stories\.$/m);
  assert.match(out, /^fail +database +not reachable at .*ECONNREFUSED\. Start it with agentsi up\. It blocks meetings, the knowledge base, status and the web app\.$/m);
  assert.match(out, /A required check failed\./);
});

test("agentsi help lists doctor", () => {
  const r = spawnSync(process.execPath, [bin, "help"], { encoding: "utf8", env: { ...process.env, AGENTSI_HOME: tmpdir() } });
  assert.equal(r.status, 0);
  assert.match(r.stdout, /^ {2}doctor +Check the tools that AgentSI needs$/m);
  for (const name of ["backup", "data", "voice"]) assert.match(r.stdout, new RegExp(`^ {2}${name} +`, "m"), `help lists ${name}`);
});
