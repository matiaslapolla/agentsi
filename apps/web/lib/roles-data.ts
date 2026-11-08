import { execFile } from "node:child_process";
import { promisify } from "node:util";
import type { AuditReport, Brief, ReflectionEntry, RosterEntry } from "@agentsi/core";
import { readReflections } from "@agentsi/core/reflections";
import { rows } from "./db";
import { CLI_BIN, ROOT } from "./env";
import { productDir, safeSegment } from "./workspace";

export type { AuditReport, Brief, Proposal, ReflectionEntry, ReplayReport } from "@agentsi/core";

const run = promisify(execFile);

export type Loaded<T> = { ok: true; data: T } | { ok: false; error: string };

async function cliJson<T>(args: string[], product: string): Promise<Loaded<T>> {
  try {
    const { stdout } = await run(process.execPath, [CLI_BIN, ...args, "--json", "--product", product], {
      cwd: ROOT,
      timeout: 20_000,
    });
    return { ok: true, data: JSON.parse(stdout) as T };
  } catch (e) {
    const err = e as { stderr?: string; message?: string };
    return { ok: false, error: (err.stderr || err.message || "falló el comando").toString().trim().slice(0, 300) };
  }
}

/** The brief and the overlays are in the `briefs` and `agent_overlays` tables. */
export async function briefFor(product: string): Promise<Brief | null> {
  if (!productDir(product)) return null;
  const [found] = await rows<{ data: Brief | string }>("select data from briefs where product = $1", [product]);
  if (!found) return null;
  return typeof found.data === "string" ? (JSON.parse(found.data) as Brief) : found.data;
}

async function overlayFor(product: string, agent: string): Promise<string | null> {
  const [found] = await rows<{ body: string }>("select body from agent_overlays where product = $1 and agent = $2", [product, agent]);
  return found?.body ?? null;
}

export interface RosterRow extends RosterEntry {
  why: string | null;
  overlay: string | null;
  prompt: string | null;
}

export async function rosterFor(product: string, brief: Brief | null): Promise<Loaded<RosterRow[]>> {
  const dir = productDir(product);
  const list = await cliJson<{ roster: RosterEntry[] }>(["roles"], product);
  if (!list.ok || !dir) return list.ok ? { ok: true, data: [] } : list;
  const rows = await Promise.all(
    list.data.roster.map(async (e): Promise<RosterRow> => {
      const def = e.error || !safeSegment(e.id) ? null : await cliJson<{ body: string }>(["roles", e.id], product);
      return {
        ...e,
        why: brief?.roles[e.id] ?? null,
        overlay: safeSegment(e.id) ? await overlayFor(product, e.id) : null,
        prompt: def?.ok ? def.data.body : null,
      };
    }),
  );
  return { ok: true, data: rows };
}

export const auditFor = (product: string): Promise<Loaded<AuditReport>> => cliJson<AuditReport>(["audit"], product);

/** Newest first. */
export async function reflectionsFor(product: string): Promise<ReflectionEntry[]> {
  if (!productDir(product)) return [];
  return (await readReflections(product, ROOT, () => {})).reverse();
}
