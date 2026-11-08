// Opens the stage 1 eval routes in light and dark, at a desktop and a phone viewport, and fails on a bad status,
// a console or page error, a theme that does not apply, or a failed page check.
// Usage:
//   pnpm --filter @agentsi/web smoke --base http://127.0.0.1:3380 [--chromium /usr/bin/chromium] [--report <file>]
//   pnpm --filter @agentsi/web smoke:demo   (builds and starts the app on the demo data of scripts/guide-demo.ts)
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { listStories } from "@agentsi/core/story";
import { chromium, type Page } from "playwright-core";
import { makeDemo, type Demo } from "../../../scripts/guide-demo.ts";

const VIEWPORTS = [
  { name: "1280×800", width: 1280, height: 800 },
  { name: "390×844", width: 390, height: 844 },
] as const;
type Viewport = (typeof VIEWPORTS)[number];

interface Route {
  row: string;
  path: string;
  check?: (page: Page, vp: Viewport) => Promise<{ ok: boolean; note: string }>;
}

const WEB = path.resolve(import.meta.dirname, "..");
const CODE = path.resolve(WEB, "../..");
const DIST_DIR = ".next-smoke";
// The owner's web app runs on this port from the main checkout.
const OWNER_PORT = 3361;

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i > -1 ? process.argv[i + 1] : undefined;
}

/** AC8: the first "Te toca" card is above the fold at 1280×800. */
async function firstCardAboveFold(page: Page, vp: Viewport) {
  if (vp.width !== 1280) return { ok: true, note: "-" };
  const box = await page.locator("#te-toca li a").first().boundingBox();
  if (!box) return { ok: false, note: "no Te toca card" };
  return { ok: box.y + Math.min(box.height, 40) <= vp.height, note: `first card top ${Math.round(box.y)}px` };
}

async function decisionsTabOpen(page: Page) {
  const active = (await page.locator('[role="tab"][data-state="active"]').first().textContent()) ?? "";
  return { ok: active.startsWith("Decisiones"), note: `active tab "${active.replace(/\d+$/, "").trim()}"` };
}

async function criteriaShown(page: Page) {
  const cards = await page.locator("section").filter({ hasText: "Criterios de aceptación" }).locator(":scope > ol > li").count();
  return { ok: cards > 0, note: `${cards} criteria` };
}

function routes(pid: string, minutesId: string, callId: string, story?: { pid: string; id: string }): Route[] {
  return [
    { row: "1", path: "/", check: firstCardAboveFold },
    { row: "2", path: `/p/${pid}` },
    { row: "8", path: `/p/${pid}/m/${minutesId}#decisiones`, check: decisionsTabOpen },
    { row: "18", path: `/p/${pid}/roles` },
    { row: "19", path: `/p/${pid}/memory` },
    { row: "20", path: `/p/${pid}/m/${callId}/call` },
    { row: "21", path: "/guia" },
    ...(story
      ? [
          { row: "stories", path: `/p/${story.pid}/stories` },
          { row: "story", path: `/p/${story.pid}/stories/${story.id}`, check: criteriaShown },
        ]
      : []),
  ];
}

/** The owner's data: the eval rows of docs/ideas/agentsi-web-ui-eval.md. */
const OWNER_ROUTES = routes(
  "agentsi",
  "2026-10-11-0402-que-le-falta-a-agentsi-v1-para-ser-usabl",
  "2026-10-18-0146-leyendo-las-4-minutas-de-agentsi-que-que",
);

/** Routes on the fixture ids of the demo: the first product with minutes and the first product with stories. */
async function demoRoutes(demo: Demo): Promise<Route[]> {
  // The demo seeds the fixture minutes of `miapp` into the database; the fixture folder lists their ids.
  const minutesOf = (pid: string) => {
    const dir = path.join(WEB, "fixtures/demo/minutes");
    if (pid !== "miapp" || !fs.existsSync(dir)) return [];
    return fs
      .readdirSync(dir)
      .filter((f) => f.endsWith(".json") && !f.endsWith(".usage.json"))
      .map((f) => (JSON.parse(fs.readFileSync(path.join(dir, f), "utf8")) as { id: string }).id);
  };
  const withMinutes = demo.products.find((p) => minutesOf(p.id).length);
  if (!withMinutes) throw new Error("The demo has no minutes.");
  const storied = (await Promise.all(demo.products.map(async (p) => ({ pid: p.id, stories: await listStories(p.id, demo.home) })))).find((p) => p.stories.length);
  if (!storied) throw new Error("The demo has no stories.");
  const minutesId = minutesOf(withMinutes.id)[0]!;
  return routes(withMinutes.id, minutesId, minutesId, { pid: storied.pid, id: storied.stories[0]!.id });
}

function freePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.once("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const port = (srv.address() as net.AddressInfo).port;
      srv.close(() => (port === OWNER_PORT ? freePort().then(resolve, reject) : resolve(port)));
    });
  });
}

async function waitUp(url: string, server: ChildProcess) {
  for (let i = 0; i < 120; i++) {
    if (server.exitCode !== null) throw new Error(`next start exited with code ${server.exitCode}.`);
    try {
      const res = await fetch(url);
      if (res.ok) return;
    } catch {
      // not listening yet
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`${url} did not answer in 60 s.`);
}

async function run(base: string, list: Route[]) {
  const executablePath = arg("chromium") ?? (fs.existsSync("/usr/bin/chromium") ? "/usr/bin/chromium" : undefined);
  const browser = await chromium.launch({ executablePath });
  const lines: string[] = [];
  let failed = false;
  try {
    for (const vp of VIEWPORTS) {
      for (const theme of ["light", "dark"] as const) {
        const context = await browser.newContext({ viewport: { width: vp.width, height: vp.height } });
        await context.addInitScript((t) => localStorage.setItem("agentsi-theme", t), theme);
        for (const r of list) {
          const page = await context.newPage();
          const errors: string[] = [];
          page.on("console", (m) => {
            // The app ships no favicon yet, so the browser's own request for it logs a 404.
            if (m.type() === "error" && !m.location().url.endsWith("/favicon.ico")) errors.push(`${m.text()} (${m.location().url})`);
          });
          page.on("pageerror", (e) => errors.push(e.message));
          const res = await page.goto(base + r.path, { waitUntil: "networkidle" });
          const status = res?.status() ?? 0;
          const applied = await page.evaluate(() => document.documentElement.dataset.theme);
          const check = r.check ? await r.check(page, vp) : { ok: true, note: "-" };
          const ok = status === 200 && !errors.length && applied === theme && check.ok;
          if (!ok) failed = true;
          const detail = errors.length ? ` ${errors.join(" / ").replace(/\|/g, "\\|")}` : "";
          lines.push(`| ${r.row} | ${r.path} | ${vp.name} | ${theme} | ${status} | ${errors.length} | ${applied ?? "none"} | ${check.note} | ${ok ? "PASS" : "FAIL"} |${detail}`);
          await page.close();
        }
        await context.close();
      }
    }
  } finally {
    await browser.close();
  }
  return { failed, table: ["| Eval row | Route | Viewport | Theme | Status | Errors | Theme applied | Check | Result |", "|---|---|---|---|---|---|---|---|---|", ...lines].join("\n") };
}

function report(table: string, failed: boolean, demo: boolean) {
  const commit = execFileSync("git", ["rev-parse", "--short", "HEAD"], { cwd: CODE, encoding: "utf8" }).trim();
  const date = new Date().toISOString().slice(0, 10);
  const how = demo
    ? `\`pnpm --filter @agentsi/web smoke:demo\`: a production build from an empty \`${DIST_DIR}\` on the demo data of \`scripts/guide-demo.ts\`.`
    : `\`pnpm --filter @agentsi/web smoke --base ${arg("base")}\`.`;
  return `# Web smoke${demo ? " on the demo data" : ""}\n\n- Date: ${date}\n- Commit: \`${commit}\`\n- Run: ${how}\n- Result: **${failed ? "FAIL" : "PASS"}**\n\n${table}\n`;
}

async function main(): Promise<number> {
  const demoMode = process.argv.includes("--demo");
  let base = arg("base");
  let list = OWNER_ROUTES;
  let demo: Demo | undefined;
  let server: ChildProcess | undefined;
  const stop = () => {
    if (server?.pid && server.exitCode === null) {
      try {
        process.kill(-server.pid, "SIGTERM");
      } catch {
        // already gone
      }
    }
    demo?.cleanup();
    demo = undefined;
  };
  process.on("exit", stop);
  for (const sig of ["SIGINT", "SIGTERM"] as const) process.on(sig, () => process.exit(130));

  try {
    if (demoMode) {
      demo = makeDemo(CODE);
      list = await demoRoutes(demo);
      const env = { ...demo.env, AGENTSI_WEB_DIST_DIR: DIST_DIR, NEXT_TELEMETRY_DISABLED: "1" };
      fs.rmSync(path.join(WEB, DIST_DIR), { recursive: true, force: true });
      execFileSync("pnpm", ["exec", "next", "build"], { cwd: WEB, env, stdio: ["ignore", 2, 2] });
      const port = await freePort();
      base = `http://127.0.0.1:${port}`;
      server = spawn("pnpm", ["exec", "next", "start", "-p", String(port), "-H", "127.0.0.1"], { cwd: WEB, env, stdio: ["ignore", 2, 2], detached: true });
      await waitUp(`${base}/guia`, server);
    }
    if (!base) {
      console.error("Usage: smoke --base http://127.0.0.1:<port> | --demo [--chromium <path>] [--report <file>]");
      return 2;
    }
    const { failed, table } = await run(base, list);
    const text = report(table, failed, demoMode);
    const out = arg("report");
    if (out) fs.writeFileSync(path.resolve(out), text);
    console.log(text);
    return failed ? 1 : 0;
  } finally {
    stop();
  }
}

process.exitCode = await main();
