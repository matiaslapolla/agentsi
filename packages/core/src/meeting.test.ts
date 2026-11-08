import assert from "node:assert/strict";
import { test } from "node:test";
import { claudeArgs, mapUsage } from "./claude.ts";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { codeGraphMcp } from "./code-graph.ts";
import { compactSnapshot, prepareAgents, roleCall, synthesisPrompt, type RoleContext } from "./meeting.ts";
import { makeItem, writeMemoryItems } from "./memory.ts";
import { DEFAULT_OWNER } from "./registry.ts";
import { loadRole, parseAgentFile, type RoleDef } from "./roles.ts";
import { writeOverlay } from "./roster.ts";
import { testDb } from "./test-db.ts";

test("parseAgentFile strips the frontmatter and reads the model", () => {
  const { frontmatter, body } = parseAgentFile("---\nname: dev\nmodel: sonnet\ntools: Read, Grep\n---\n\nYou are dev.\n");
  assert.equal(frontmatter.model, "sonnet");
  assert.equal(frontmatter.tools, "Read, Grep");
  assert.equal(body, "You are dev.");
});

test("loadRole reads the plugin personas", async () => {
  assert.equal((await loadRole("qa")).model, "haiku");
  assert.ok(!(await loadRole("pm")).body.startsWith("---"));
  await assert.rejects(loadRole("../x"));
});

test("mapUsage maps the claude -p envelope to the usage contract", () => {
  const u = mapUsage(
    {
      duration_ms: 2322.4,
      total_cost_usd: 0.5,
      usage: { input_tokens: 6, output_tokens: 97, cache_read_input_tokens: 8704, cache_creation_input_tokens: 83 },
      modelUsage: { "claude-haiku-5-5": { costUSD: 0.4 }, "claude-sonnet-5-5": { costUSD: 0.1 } },
    },
    "haiku",
  );
  assert.deepEqual(u, {
    model: "claude-haiku-5-5",
    input_tokens: 6,
    output_tokens: 97,
    cache_read: 8704,
    cache_write: 83,
    cost_usd: 0.5,
    duration_ms: 2322,
  });
});

test("claudeArgs keeps calls read-only and non-interactive", () => {
  const args = claudeArgs({ prompt: "p", model: "haiku", system: "s", cwd: "/", tools: [] });
  assert.ok(args.includes("--restricted") && args.includes("--no-session-persistence"));
  assert.ok(args.includes("--tools="));
  assert.ok(!args.includes("--allowedTools"));
});

test("compactSnapshot drops the journal and keeps the north star", () => {
  const full = "# Snapshot: X\n\n## North star and guidelines\n\nabc\n\n## Journal tail\n\nsecret\n\n## Docs index\n\n- docs/a.md";
  const c = compactSnapshot(full);
  assert.ok(c.includes("North star") && c.includes("docs/a.md") && !c.includes("secret"));
});

test("prepareAgents resolves every agent, its memory and the pm before any call, and fails on a bad agent", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const root = mkdtempSync(join(tmpdir(), "agentsi-prepare-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, "plugin/agents"), { recursive: true });
  for (const r of ["pm", "dev"]) writeFileSync(join(root, "plugin/agents", `${r}.md`), `---\nmodel: haiku\n---\n\nYou are ${r}.\n`);
  await writeOverlay("p", "dev.backend-api", "---\nextends: dev\n---\n\nAPI.\n");
  await writeMemoryItems("p", "dev.backend-api", [makeItem("Usa pnpm.", { source: "m", created_at: "2026-10-01T00:00:00.000Z", taken: [] })]);

  const prepared = await prepareAgents("p", ["dev", "dev.backend-api"], root);
  assert.deepEqual(prepared.agents.map((a) => a.def.id), ["dev", "dev.backend-api"]);
  assert.match(prepared.agents[1]!.memory, /Usa pnpm\./);
  assert.equal(prepared.agents[0]!.memory, "");
  assert.equal(prepared.pm.id, "pm");

  for (const bad of ["growth-", "dev--ops", "dev.mobile"]) await assert.rejects(prepareAgents("p", ["dev", bad], root), /agent id|no overlay|Unknown role/, bad);
});

test("claudeArgs with MCP drops --safe-mode, which drops --mcp-config servers, and allows the MCP tools", () => {
  const mcp = codeGraphMcp();
  const args = claudeArgs({ prompt: "p", model: "haiku", system: "s", cwd: "/", tools: ["Read"], mcp });
  assert.ok(!args.includes("--safe-mode"));
  assert.ok(args.includes("--restricted") && args.includes("--strict-mcp-config"));
  assert.equal(args[args.indexOf("--mcp-config") + 1], mcp.config);
  assert.equal(args[args.indexOf("--allowedTools") + 1], ["Read", ...mcp.tools].join(","));
  assert.ok(claudeArgs({ prompt: "p", model: "haiku", system: "s", cwd: "/", tools: ["Read"] }).includes("--safe-mode"));
});

test("the code graph exposes only read-only tools", () => {
  const { config, tools } = codeGraphMcp();
  assert.deepEqual(JSON.parse(config).mcpServers.codegraph.args, ["--tool-profile=analysis"]);
  for (const writer of ["index_repository", "delete_project", "manage_adr", "ingest_traces"]) {
    assert.ok(!tools.includes(`mcp__codegraph__${writer}`), writer);
  }
});

test("a role with tools gets the code graph and its prompt rule, a role without tools gets neither", () => {
  const def: RoleDef = { id: "dev", role: "dev", source: "global", model: "haiku", body: "You are dev." };
  const ctx: RoleContext = { question: "q", snap: "s", memory: "", scope: null, cwd: "/", brief: null, owner: DEFAULT_OWNER, codeGraph: ["proj-a"] };
  const call = roleCall(def, ctx, true);
  assert.deepEqual(call.mcp, codeGraphMcp());
  assert.match(call.prompt, /code graph tools \(mcp__codegraph__\*, projects: proj-a\).*Read the file to confirm/);
  assert.match(call.prompt, /story \(an id from the Stories section/);

  const bare = roleCall({ ...def, tools: [] }, ctx, true);
  assert.equal(bare.mcp, undefined);
  assert.doesNotMatch(bare.prompt, /code graph/);
  assert.equal(roleCall(def, { ...ctx, codeGraph: [] }, true).mcp, undefined);
});

test("compactSnapshot keeps the Stories section and the synthesis names stories by title", () => {
  const full = "# Snapshot: X\n\n## North star and guidelines\n\nabc\n\n## Stories\n\n- S001 Filtrar\n\n## Journal tail\n\nsecret";
  assert.match(compactSnapshot(full), /## Stories\n\n- S001 Filtrar/);
  assert.match(synthesisPrompt("q", {}, "", null, DEFAULT_OWNER), /Name a story by its title, never by its id\./);
});

test("the call script uses Simplified Technical English in the owner's language", () => {
  const synthesis = synthesisPrompt("q", {}, "", null, { name: "Ana", language: "Brazilian Portuguese" });
  assert.match(synthesis, /Simplified Technical English style applied to Brazilian Portuguese/);
  assert.match(synthesis, /at most 20 words in a sentence/);
});

test("the role and synthesis prompts name the owner and use the owner's language", () => {
  const owner = { name: "Ana", language: "Brazilian Portuguese" };
  const def: RoleDef = { id: "dev", role: "dev", source: "global", model: "haiku", body: "You are dev." };
  const role = roleCall(def, { question: "q", snap: "s", memory: "", scope: null, cwd: "/", brief: null, owner }, true).prompt;
  const synthesis = synthesisPrompt("q", {}, "", null, owner);
  for (const prompt of [role, synthesis]) {
    assert.match(prompt, /Ana must/);
    assert.match(prompt, /in Brazilian Portuguese/);
    assert.doesNotMatch(prompt, /Rioplatense|Spanish/i);
  }
  assert.match(role, /Write "\[no data\]"/);
  assert.match(synthesis, /approval of Ana/);
  assert.match(roleCall(def, { question: "q", snap: "s", memory: "", scope: null, cwd: "/", brief: null, owner: DEFAULT_OWNER }, true).prompt, /Answer in English.*\n.*\n.*the owner must approve/);
});
