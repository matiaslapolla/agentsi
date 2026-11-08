import assert from "node:assert/strict";
import { test } from "node:test";
import { CLAUDE_FLAGS, doctor, doctorPassed, type DoctorProbes } from "./doctor.ts";

const VERSIONS: Record<string, string> = {
  node: "v26.7.0",
  pnpm: "11.23.0",
  docker: "Docker version 29.7.2, build a7dcaa6fdb",
  claude: "2.1.306 (Claude Code)",
  bwrap: "bubblewrap 0.12.0",
  gh: "gh version 2.102.0 (2026-09-30)\nhttps://github.com/cli/cli/releases/tag/v2.102.0",
  "codebase-memory-mcp": "codebase-memory-mcp 0.12.1",
  uv: "uv 0.13.1",
  ffmpeg: "ffmpeg version n9.0.1 Copyright (c) 2000-2026 the FFmpeg developers\nbuilt with gcc",
};

/** A machine with every tool; `missing` removes commands, `patch` changes other probes. */
function probes(missing: string[] = [], patch: Partial<DoctorProbes> = {}): DoctorProbes {
  return {
    output: async (cmd, args) => {
      if (missing.includes(cmd)) return null;
      if (cmd === "claude" && args[0] === "--help") return `Usage: claude [options]\n${CLAUDE_FLAGS.join("\n")}`;
      return VERSIONS[cmd] ?? null;
    },
    database: async () => "17.2",
    sandbox: async () => true,
    dataRoot: () => ({ root: "/data", from: "AGENTSI_HOME" }),
    products: () => 3,
    isDir: () => true,
    exists: () => true,
    backupAge: () => 5,
    env: { TTS_MODELS_DIR: "/models" },
    ...patch,
  };
}

const byName = async (p: DoctorProbes) => Object.fromEntries((await doctor(p)).map((r) => [r.name, r]));

test("on a full machine every check passes and shows its version or path", async () => {
  const results = await doctor(probes());
  assert.deepEqual(
    results.map((r) => [r.name, r.level, r.status === "ok", r.detail]),
    [
      ["node", "required", true, "v26.7.0"],
      ["pnpm", "required", true, "11.23.0"],
      ["docker", "required", true, "Docker version 29.7.2, build a7dcaa6fdb"],
      ["database", "required", true, "PostgreSQL 17.2"],
      ["claude", "required", true, "2.1.306 (Claude Code)"],
      ["data root", "required", true, "/data (AGENTSI_HOME), 3 products"],
      ["backup", "optional", true, "newest dump 5 h old"],
      ["bwrap", "stories", true, "bubblewrap 0.12.0"],
      ["gh", "optional", true, "gh version 2.102.0 (2026-09-30)"],
      ["codebase-memory-mcp", "optional", true, "codebase-memory-mcp 0.12.1"],
      ["uv", "optional", true, "uv 0.13.1"],
      ["ffmpeg", "optional", true, "ffmpeg version n9.0.1 Copyright (c) 2000-2026 the FFmpeg developers"],
      ["TTS models", "optional", true, "/models"],
    ],
  );
  assert.equal(doctorPassed(results), true);
});

test("a missing optional or stories tool fails its check and keeps the required ones passing", async () => {
  const results = await doctor(probes(["gh", "bwrap", "ffmpeg"], { isDir: (p) => p !== "/models" }));
  const failed = results.filter((r) => r.status !== "ok");
  assert.deepEqual(failed.map((r) => [r.name, r.status, r.detail]), [
    ["bwrap", "missing", ""],
    ["gh", "missing", ""],
    ["ffmpeg", "missing", ""],
    ["TTS models", "fail", "/models does not exist. Set TTS_MODELS_DIR"],
  ]);
  assert.match(failed[0]!.blocks, /story sessions/);
  assert.equal(doctorPassed(results), true);
});

test("each required check fails on its own cause and fails the run", async () => {
  for (const tool of ["node", "pnpm", "docker", "claude"]) {
    const results = await doctor(probes([tool]));
    assert.deepEqual(results.filter((r) => r.status !== "ok").map((r) => [r.name, r.status]), [[tool, "missing"]], tool);
    assert.equal(doctorPassed(results), false, tool);
  }
  const urls: string[] = [];
  const down: Partial<DoctorProbes> = {
    database: async (url) => (urls.push(url), Promise.reject(new Error("connect ECONNREFUSED 10.0.0.9:5432"))),
    env: { AGENTSI_DATABASE_URL: "postgres://user:s3cret@10.0.0.9:5432/agentsi", TTS_MODELS_DIR: "/models" },
  };
  const db = await byName(probes([], down));
  assert.deepEqual(urls, ["postgres://user:s3cret@10.0.0.9:5432/agentsi"]);
  assert.equal(db.database!.status, "fail");
  assert.equal(db.database!.detail, "not reachable at postgres://user:***@10.0.0.9:5432/agentsi: connect ECONNREFUSED 10.0.0.9:5432. Start it with agentsi up");
});

test("the claude check fails on a CLI that lacks a flag AgentSI passes", async () => {
  const old = probes([], { output: async (cmd, args) => (cmd === "claude" && args[0] === "--help" ? "--safe-mode --max-budget-usd" : VERSIONS[cmd] ?? null) });
  const { claude } = await byName(old);
  assert.equal(claude!.status, "fail");
  assert.match(claude!.detail, /^2\.1\.306 \(Claude Code\) lacks --restricted, --permission-prompts, --strict-mcp-config, --disable-slash-commands\. Update Claude Code$/);
});

test("the data root check names a missing dir and a broken registry; the data root needs no git repo (ADR 0003)", async () => {
  assert.equal((await byName(probes([], { exists: () => false })))["data root"]!.status, "ok");
  const cases: [Partial<DoctorProbes>, RegExp][] = [
    [{ isDir: () => false }, /^\/data \(AGENTSI_HOME\) does not exist$/],
    [{ products: () => { throw new Error("ENOENT"); } }, /^\/data \(AGENTSI_HOME\) has no valid agentsi\.config\.json: ENOENT$/],
  ];
  for (const [patch, re] of cases) {
    const results = await doctor(probes([], patch));
    const root = results.find((r) => r.name === "data root")!;
    assert.equal(root.status, "fail");
    assert.match(root.detail, re);
    assert.equal(doctorPassed(results), false);
  }
});

test("the backup check warns past two days and is missing with no dump, and never fails the doctor", async () => {
  const stale = await doctor(probes([], { backupAge: () => 72 }));
  const check = stale.find((r) => r.name === "backup")!;
  assert.equal(check.status, "fail");
  assert.match(check.detail, /3 days old\. Run agentsi backup/);
  assert.equal(doctorPassed(stale), true);
  assert.equal((await byName(probes([], { backupAge: () => null })))["backup"]!.status, "missing");
});

test("bwrap that cannot start a sandbox fails the stories check only", async () => {
  const results = await doctor(probes([], { sandbox: async () => false }));
  assert.deepEqual(results.filter((r) => r.status !== "ok").map((r) => [r.name, r.detail]), [["bwrap", "bubblewrap 0.12.0 cannot start a sandbox"]]);
  assert.equal(doctorPassed(results), true);
});
