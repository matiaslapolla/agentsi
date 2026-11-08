import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { Client } from "pg";
import { DEFAULT_DATABASE_URL, resolveDataRoot } from "@agentsi/core";

const WEB = path.resolve(import.meta.dirname, "..");
const ROOT = resolveDataRoot(path.resolve(WEB, "../..")).root;
const TARGET = path.join(ROOT, "workspace/_demo");
const FIXTURE = path.join(WEB, "fixtures/demo");
const DB = process.env.AGENTSI_DATABASE_URL ?? DEFAULT_DATABASE_URL;
const PRODUCT = "_demo";

async function withDb(fn: (c: Client) => Promise<void>) {
  const c = new Client({ connectionString: DB, connectionTimeoutMillis: 2000 });
  try {
    await c.connect();
  } catch (e) {
    console.log(`db: unreachable (${(e as Error).message}), skipped`);
    return;
  }
  try {
    await fn(c);
  } finally {
    await c.end();
  }
}

async function tables(c: Client, names: string[]) {
  const missing: string[] = [];
  for (const n of names) {
    const r = await c.query("select to_regclass($1) as t", [`public.${n}`]);
    if (!r.rows[0].t) missing.push(n);
  }
  return missing;
}

async function clean(c: Client) {
  const have = new Set<string>();
  for (const n of ["usage", "priorities", "decisions", "minutes"]) {
    if (!(await tables(c, [n])).length) have.add(n);
  }
  const ids = have.has("minutes") ? (await c.query("select id from minutes where product = $1", [PRODUCT])).rows.map((r) => r.id) : [];
  if (have.has("usage")) await c.query("delete from usage where minutes_id = any($1)", [ids]);
  if (have.has("priorities")) await c.query("delete from priorities where minutes_id = any($1)", [ids]);
  if (have.has("decisions")) await c.query("delete from decisions where product = $1", [PRODUCT]);
  if (have.has("minutes")) await c.query("delete from minutes where product = $1", [PRODUCT]);
}

async function seedDb(c: Client) {
  const missing = await tables(c, ["minutes", "priorities", "decisions", "usage"]);
  if (missing.length) {
    console.log(`db: tables missing (${missing.join(", ")}), skipped`);
    return;
  }
  await clean(c);
  const dir = path.join(TARGET, "minutes");
  for (const f of fs.readdirSync(dir).filter((x) => x.endsWith(".json") && !x.endsWith(".usage.json"))) {
    const m = JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
    await c.query(
      "insert into minutes(id, product, scope, question, status, created_at, file) values ($1,$2,$3,$4,$5,$6,$7)",
      [m.id, PRODUCT, m.scope, m.question, m.status, m.created_at, `workspace/${PRODUCT}/minutes/${f}`],
    );
    for (const [i, p] of m.priorities.entries()) {
      await c.query(
        "insert into priorities(minutes_id, idx, title, why, sources, owner_role, mark, is_new) values ($1,$2,$3,$4,$5,$6,$7,$8)",
        [m.id, i, p.title, p.why, JSON.stringify(p.sources), p.owner_role, p.mark, p.is_new],
      );
    }
    for (const d of m.decisions) {
      await c.query("insert into decisions(product, minutes_id, text, why) values ($1,$2,$3,$4)", [PRODUCT, m.id, d.text, d.why]);
    }
    const usage = JSON.parse(fs.readFileSync(path.join(dir, f.replace(/\.json$/, ".usage.json")), "utf8"));
    for (const u of usage) {
      await c.query(
        "insert into usage(minutes_id, role, model, input_tokens, output_tokens, cache_read, cache_write, cost_usd, duration_ms) values ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        [m.id, u.role, u.model, u.input_tokens, u.output_tokens, u.cache_read, u.cache_write, u.cost_usd, u.duration_ms],
      );
    }
    console.log(`db: seeded ${m.id}`);
  }
}

function silentAudio() {
  for (const id of fs.readdirSync(path.join(TARGET, "audio"))) {
    const timings = JSON.parse(fs.readFileSync(path.join(TARGET, "audio", id, "timings.json"), "utf8")) as { end: number }[];
    const secs = Math.ceil(Math.max(...timings.map((t) => t.end)) + 1);
    const out = path.join(TARGET, "audio", id, "call.webm");
    const r = spawnSync(
      "ffmpeg",
      ["-y", "-loglevel", "error", "-f", "lavfi", "-i", "anullsrc=r=24000:cl=mono", "-t", String(secs), "-c:a", "libopus", "-b:a", "32k", out],
      { stdio: "inherit" },
    );
    console.log(r.status === 0 ? `audio: silent placeholder ${secs}s` : "audio: ffmpeg unavailable, player shows the no-audio state");
  }
}

const cmd = process.argv[2];
if (cmd === "seed") {
  fs.rmSync(TARGET, { recursive: true, force: true });
  fs.cpSync(FIXTURE, TARGET, { recursive: true });
  console.log(`files: copied to ${TARGET}`);
  if (process.argv.includes("--audio")) silentAudio();
  await withDb(seedDb);
} else if (cmd === "clean") {
  await withDb(clean);
  fs.rmSync(TARGET, { recursive: true, force: true });
  console.log(`files: removed ${TARGET}`);
} else {
  console.error("usage: demo.ts seed [--audio] | clean");
  process.exit(1);
}
