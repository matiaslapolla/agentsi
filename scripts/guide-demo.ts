import { execFileSync, spawnSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** A throwaway data root, product repos and database, so the guide shows fictional products only. */
export interface Demo {
  home: string;
  env: NodeJS.ProcessEnv;
  products: DemoProduct[];
  /** Replaces the temporary paths and the code checkout with the paths a reader expects. */
  scrub: (text: string) => string;
  cleanup: () => void;
}

export interface DemoProduct {
  id: string;
  name: string;
  paths: string[];
  status: "active";
  roles: string[];
  stories?: { test: string; gate: string[] };
}

const CONTAINER = "agentsi-postgres";
const DB_NAME = "agentsi_guide";
const DB_URL = `postgres://agentsi:agentsi@127.0.0.1:5561/${DB_NAME}`;
const AUTHOR = { GIT_AUTHOR_NAME: "Demo", GIT_AUTHOR_EMAIL: "demo@example.com", GIT_COMMITTER_NAME: "Demo", GIT_COMMITTER_EMAIL: "demo@example.com" };
const ROLES = ["pm", "social", "dev", "qa", "scout"];

const REPOS: { id: string; name: string; what: string; commits: string[]; stories?: DemoProduct["stories"] }[] = [
  {
    id: "miapp",
    name: "MiApp",
    what: "Turnos online para consultorios chicos.",
    commits: ["feat: agenda del día", "feat: alta de paciente", "fix: horario de verano en la agenda", "feat: webhook de pagos"],
    stories: { test: "pnpm vitest run {files}", gate: ["pnpm typecheck", "pnpm lint"] },
  },
  { id: "tienda", name: "Tienda", what: "Tienda online de una marca de ropa.", commits: ["feat: carrito", "feat: checkout"] },
  { id: "blog", name: "Blog", what: "Blog personal con notas técnicas.", commits: ["feat: primera nota", "feat: feed RSS"] },
];

const psql = (sql: string) => execFileSync("docker", ["exec", CONTAINER, "psql", "-q", "-U", "agentsi", "-d", "agentsi", "-c", sql], { stdio: "pipe" });

function git(cwd: string, ...args: string[]) {
  execFileSync("git", args, { cwd, env: { ...process.env, ...AUTHOR }, stdio: "pipe" });
}

function makeRepo(dir: string, r: (typeof REPOS)[number]) {
  mkdirSync(join(dir, "docs"), { recursive: true });
  git(dir, "init", "-q", "-b", "main");
  writeFileSync(join(dir, "README.md"), `# ${r.name}\n\n${r.what}\n`);
  writeFileSync(join(dir, "PRODUCT.md"), `# ${r.name}\n\n${r.what}\n\n## North star\n\nEl primer cliente pagando este trimestre.\n`);
  writeFileSync(join(dir, ".gitignore"), ".worktrees/\nnode_modules/\n");
  r.commits.forEach((msg, i) => {
    writeFileSync(join(dir, "docs", "notes.md"), `# Notas\n\n${r.commits.slice(0, i + 1).map((c) => `- ${c}`).join("\n")}\n`);
    git(dir, "add", ".");
    git(dir, "commit", "-qm", msg);
  });
}

/** Builds the demo environment. The caller runs `cleanup` when it is done. */
export function makeDemo(codeRoot: string): Demo {
  const base = mkdtempSync(join(tmpdir(), "agentsi-guide-"));
  const home = join(base, "home");
  const products: DemoProduct[] = REPOS.map((r) => ({
    id: r.id,
    name: r.name,
    paths: [join(base, "dev", r.id)],
    status: "active",
    roles: ROLES,
    ...(r.stories ? { stories: r.stories } : {}),
  }));
  const env = { ...process.env, ...AUTHOR, AGENTSI_HOME: home, AGENTSI_DATABASE_URL: DB_URL, NO_COLOR: "1" };
  const cleanup = () => {
    try {
      psql(`drop database if exists ${DB_NAME} with (force)`);
    } catch {
      console.warn(`Could not drop the database ${DB_NAME}.`);
    }
    rmSync(base, { recursive: true, force: true });
  };
  try {
    REPOS.forEach((r, i) => makeRepo(products[i]!.paths[0]!, r));
    mkdirSync(home);
    writeFileSync(join(home, "agentsi.config.json"), `{\n  "products": [\n${products.map((p) => `    ${JSON.stringify(p)}`).join(",\n")}\n  ]\n}\n`);
    git(home, "init", "-q", "-b", "main");
    git(home, "add", ".");
    git(home, "commit", "-qm", "registry");

    psql(`drop database if exists ${DB_NAME} with (force)`);
    psql(`create database ${DB_NAME}`);
    const cli = join(codeRoot, "packages/cli/bin/agentsi.mjs");
    execFileSync("node", [cli, "migrate"], { cwd: codeRoot, env, stdio: "pipe" });
    execFileSync(process.execPath, ["--import", "tsx", fileURLToPath(import.meta.url), "seed"], { cwd: codeRoot, env, stdio: "inherit" });
    for (const p of products) execFileSync("node", [cli, "index", "--product", p.id], { cwd: codeRoot, env, stdio: "pipe" });
  } catch (e) {
    cleanup();
    throw e;
  }
  const scrub = (text: string) => text.replaceAll(join(base, "dev"), "~/dev").replaceAll(home, "~/.agentsi").replaceAll(codeRoot, "~/agentsi");
  return { home, env, products, scrub, cleanup };
}

/** Runs inside the demo environment: writes the fixture meeting, its audio and two stories through the real code paths. */
async function seed() {
  const core = await import("../packages/core/src/index.ts");
  const fixture = resolve(fileURLToPath(import.meta.url), "../../apps/web/fixtures/demo");
  const product = core.getProduct("miapp");
  const minutesDir = join(fixture, "minutes");
  for (const f of readdirSync(minutesDir).filter((x) => x.endsWith(".json") && !x.endsWith(".usage.json"))) {
    const m = { ...JSON.parse(readFileSync(join(minutesDir, f), "utf8")), product: product.id } as import("../packages/core/src/types.ts").Minutes;
    const usage = JSON.parse(readFileSync(join(minutesDir, f.replace(/\.json$/, ".usage.json")), "utf8"));
    const total = usage.reduce((a: number, u: { cost_usd: number }) => a + u.cost_usd, 0);
    const entry: import("../packages/core/src/meeting-log.ts").MeetingLogEntry = {
      minutes_id: m.id,
      created_at: m.created_at,
      question: m.question,
      scope: m.scope,
      brief_hash: null,
      agents: Object.fromEntries(
        usage.filter((u: { role: string }) => u.role !== "pm").map((u: { role: string; model: string; cost_usd: number; duration_ms: number }) => [
          u.role,
          { prompt_hash: "demo", memory_hash: "demo", model: u.model, cost_usd: u.cost_usd, duration_ms: u.duration_ms, error: null, priorities: 1, sources: 2, unresolved_sources: [], sin_datos: 0 },
        ]),
      ),
      synthesis: { first_check: { passed: true, failures: [] }, repaired: false, final_check: { passed: true, failures: [] }, invented_sources: [], cost_usd: 0 },
      total_cost_usd: total,
    };
    await core.writeMinutes(m, usage, { entry, snapshot: `# ${product.name}\n` });
    const audio = join(core.dataRoot, "workspace", product.id, "audio", m.id);
    cpSync(join(fixture, "audio", m.id), audio, { recursive: true });
    const timings = JSON.parse(readFileSync(join(audio, "timings.json"), "utf8")) as { end: number }[];
    const secs = String(Math.ceil(Math.max(...timings.map((t) => t.end)) + 1));
    spawnSync("ffmpeg", ["-y", "-loglevel", "error", "-f", "lavfi", "-i", "anullsrc=r=24000:cl=mono", "-t", secs, "-c:a", "libopus", "-b:a", "32k", join(audio, "call.webm")]);
  }
  const now = "2026-10-12T10:00:00.000Z";
  const stories: Pick<import("../packages/core/src/story.ts").Story, "title" | "as_a" | "i_want" | "so_that" | "owner_role" | "approval">[] = [
    { title: "Ver la agenda del día en el celular", as_a: "recepcionista", i_want: "ver los turnos del día en el celular", so_that: "confirme a cada paciente sin ir a la compu", owner_role: "dev", approval: "approved" },
    { title: "Cobrar la seña al reservar un turno", as_a: "profesional", i_want: "cobrar una seña al reservar", so_that: "baje el ausentismo", owner_role: "dev", approval: "pending" },
  ];
  for (const [i, s] of stories.entries()) {
    const id = `S00${i + 1}`;
    await core.saveStory(product, {
      ...s,
      id,
      type: "story",
      slug: core.storySlug(s.title),
      criteria: [1, 2].map((n) => ({ given: `un turno ${n}`, when: "abro la agenda", then: "veo el turno", test: `tests/${id.toLowerCase()}-c${n}.test.ts`, checks: `criterio ${n}` })),
      objective: null,
      sources: [{ type: "path", ref: "PRODUCT.md" }],
      status: "not_implemented",
      changes_note: null,
      proposed_by: "pm",
      minutes: null,
      session: null,
      results: [],
      created_at: now,
      updated_at: now,
    });
  }
  git(core.dataRoot, "add", ".");
  git(core.dataRoot, "commit", "-qm", "story(miapp): demo stories");
  await core.closePool();
}

if (process.argv[2] === "seed" && process.argv[1] === fileURLToPath(import.meta.url)) await seed();
