import { existsSync, readFileSync } from "node:fs";
import { isAbsolute, join, relative, resolve } from "node:path";
import { getPool, type Queryable } from "./db.ts";
import { sha256 } from "./meeting-log.ts";
import type { Product } from "./registry.ts";

const STAGES = ["idea", "building", "launched", "growing", "maintenance"] as const;
const SOURCE_KINDS = ["issues", "notion", "analytics", "docs", "other"] as const;

export interface Objective {
  id: string;
  text: string;
  signal: string;
  due: string;
}

export interface Brief {
  version: 1;
  product: { what: string; audience: string; stage: (typeof STAGES)[number] };
  objectives: Objective[];
  north_stars: string[];
  sources_of_truth: { kind: (typeof SOURCE_KINDS)[number]; ref: string; note: string }[];
  out_of_scope: string[];
  roles: Record<string, string>;
  updated_at: string;
}

/** The brief of a product, or null when it has none. */
export async function readBrief(product: string): Promise<Brief | null> {
  const { rows } = await getPool().query<{ data: Brief | string }>("select data from briefs where product = $1", [product]);
  const data = rows[0]?.data;
  return data === undefined ? null : typeof data === "string" ? (JSON.parse(data) as Brief) : data;
}

/** Stores the brief of a product, replacing the one it had. */
export async function writeBrief(product: string, brief: Brief, db: Queryable = getPool()): Promise<void> {
  await db.query(
    `insert into briefs (product, data) values ($1, $2::json)
     on conflict (product) do update set data = excluded.data`,
    [product, JSON.stringify(brief)],
  );
}

/** Removes the brief of a product; true when it had one. */
export async function deleteBrief(product: string, db: Queryable = getPool()): Promise<boolean> {
  const { rows } = await db.query("delete from briefs where product = $1 returning product", [product]);
  return rows.length > 0;
}

export const briefHash = (brief: Brief): string => sha256(JSON.stringify(brief));

/** Resolves a path relative to a product root; null when it escapes the roots or does not exist. */
export function resolveUnderRoots(ref: string, roots: string[]): string | null {
  if (isAbsolute(ref)) return null;
  for (const root of roots) {
    const full = resolve(join(root, ref));
    if (!relative(root, full).startsWith("..") && existsSync(full)) return full;
  }
  return null;
}

const isDate = (s: unknown): boolean => {
  if (typeof s !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(s)) return false;
  const d = new Date(`${s}T00:00:00Z`);
  return !Number.isNaN(d.getTime()) && d.toISOString().startsWith(s);
};

const list = <T>(v: unknown): T[] => (Array.isArray(v) ? (v as T[]) : []);

/** Failures of a brief against its product and roster (agent ids); empty when valid. */
export function validateBrief(brief: Brief, product: Product, roster: string[]): string[] {
  const failures: string[] = [];

  if (!STAGES.includes(brief.product?.stage)) {
    failures.push(`product.stage "${brief.product?.stage}" is not one of ${STAGES.join(", ")}.`);
  }

  const objectives = list<Objective>(brief.objectives);
  if (objectives.length < 1 || objectives.length > 3) failures.push(`objectives must hold 1 to 3 items, found ${objectives.length}.`);
  const seen = new Set<string>();
  for (const o of objectives) {
    if (typeof o?.id !== "string" || !o.id.trim()) failures.push("An objective has no id.");
    else if (seen.has(o.id)) failures.push(`Objective id "${o.id}" is repeated.`);
    else seen.add(o.id);
    if (!isDate(o?.due)) failures.push(`Objective "${o?.id}" has an invalid due date "${o?.due}" (expected YYYY-MM-DD).`);
  }

  for (const ref of list<string>(brief.north_stars)) {
    if (typeof ref !== "string" || !resolveUnderRoots(ref, product.paths)) {
      failures.push(`north_stars path "${ref}" does not exist under a product path.`);
    }
  }

  const declared = Object.keys(brief.roles ?? {});
  for (const id of roster.filter((r) => !declared.includes(r))) failures.push(`roles has no entry for roster agent "${id}".`);
  for (const id of declared.filter((r) => !roster.includes(r))) failures.push(`roles entry "${id}" is not in the roster.`);

  return failures;
}

const esc = (s: string): string => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

export function renderBriefXml(brief: Brief): string {
  const lines = ["<brief>"];
  lines.push(
    `<product stage="${esc(brief.product.stage)}"><what>${esc(brief.product.what)}</what><audience>${esc(brief.product.audience)}</audience></product>`,
  );
  lines.push("<objectives>");
  for (const o of brief.objectives) {
    lines.push(`<objective id="${esc(o.id)}" due="${esc(o.due)}" signal="${esc(o.signal)}">${esc(o.text)}</objective>`);
  }
  lines.push("</objectives>");
  lines.push(`<north_stars>${brief.north_stars.map((p) => `<path>${esc(p)}</path>`).join("")}</north_stars>`);
  lines.push("<sources_of_truth>");
  for (const s of brief.sources_of_truth) lines.push(`<source kind="${esc(s.kind)}" ref="${esc(s.ref)}">${esc(s.note)}</source>`);
  lines.push("</sources_of_truth>");
  lines.push(`<out_of_scope>${brief.out_of_scope.map((x) => `<item>${esc(x)}</item>`).join("")}</out_of_scope>`);
  lines.push("<roles>");
  for (const [id, why] of Object.entries(brief.roles)) lines.push(`<agent id="${esc(id)}">${esc(why)}</agent>`);
  lines.push("</roles>", "</brief>");
  return lines.join("\n");
}
