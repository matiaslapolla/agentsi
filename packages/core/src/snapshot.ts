import { existsSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { readBrief, resolveUnderRoots, type Brief } from "./brief.ts";
import { getPool, type Queryable } from "./db.ts";
import { git } from "./exec.ts";
import { collectDocs } from "./indexer.ts";
import { markBox } from "./minutes-md.ts";
import { dataRoot } from "./paths.ts";
import { getProduct, type Product } from "./registry.ts";
import { repoTargets, type RepoTarget } from "./repos.ts";
import { listStories } from "./story.ts";

const MAX_BYTES = 60 * 1024;
const MAX_COMMITS = 40;
/** Commits AgentSI makes on its own workspace: they repeat what the minutes and memory sections already say. */
const BOOKKEEPING = /^\w+ \S+ (minutes|memory|reflect|roles)\([\w.-]+\): /;
const NORTH_STAR = ["PRODUCT.md", "CLAUDE.md", "CONTEXT.md", "README.md"];

const head = (file: string, lines: number): string | null =>
  existsSync(file) ? readFileSync(file, "utf8").split("\n").slice(0, lines).join("\n").trimEnd() : null;

const tail = (file: string, lines: number): string | null =>
  existsSync(file) ? readFileSync(file, "utf8").trimEnd().split("\n").slice(-lines).join("\n") : null;

const label = (t: RepoTarget) => (t.scope ? `${t.scope}: ` : "");

export function northStar(product: Product, targets: RepoTarget[], brief: Brief | null): string {
  const lines = targets.length > 1 ? 25 : 80;
  const out: string[] = ["## North star and guidelines"];
  const shown = new Set<string>();
  for (const ref of brief?.north_stars ?? []) {
    const full = resolveUnderRoots(ref, product.paths);
    const text = full && statSync(full).isFile() ? head(full, lines) : null;
    if (full && text) {
      shown.add(full);
      out.push("", `### ${ref} (declared in the brief)`, "", text);
    }
  }
  for (const t of targets) {
    for (const f of NORTH_STAR) {
      if (shown.has(join(t.path, f))) continue;
      const text = head(join(t.path, f), lines);
      if (text) out.push("", `### ${label(t)}${f}`, "", text);
    }
  }
  return out.join("\n");
}

function docsIndex(targets: RepoTarget[]): string {
  const out: string[] = ["## Docs index"];
  const groupOrder = ["docs/ideas", "docs/specs", "docs/plans", "docs/adr"];
  for (const t of targets) {
    const docs = collectDocs(t)
      .filter((d) => d.kind === "doc")
      .map((d) => ({ ref: t.prefix ? d.ref.slice(t.prefix.length + 1) : d.ref, title: d.title }))
      .sort((a, b) => {
        const ga = groupOrder.findIndex((g) => a.ref.startsWith(g + "/"));
        const gb = groupOrder.findIndex((g) => b.ref.startsWith(g + "/"));
        return (ga < 0 ? 99 : ga) - (gb < 0 ? 99 : gb) || a.ref.localeCompare(b.ref);
      });
    out.push("", ...(targets.length > 1 ? [`### ${t.scope ?? t.path}`, ""] : []));
    for (const d of docs) out.push(`- ${d.ref} — ${d.title.slice(0, 120)}`);
  }
  return out.join("\n");
}

function journal(targets: RepoTarget[]): string {
  const out: string[] = ["## Journal tail"];
  for (const t of targets) {
    const text = tail(join(t.path, ".claude/JOURNAL.md"), 80);
    if (text) out.push("", ...(targets.length > 1 ? [`### ${t.scope ?? t.path}`, ""] : []), text);
  }
  return out.length > 1 ? out.join("\n") : "## Journal tail\n\n_No journal._";
}

async function commits(targets: RepoTarget[]): Promise<string> {
  const out: string[] = ["## Commits, last 14 days"];
  for (const t of targets) {
    const res = await git(t.path, "log", "--branches", "--no-merges", "--since=14.days", "--format=%h %as %s", "-n", "80");
    const lines = (res.ok ? res.stdout.trim().split("\n").filter(Boolean) : []).filter((l) => !BOOKKEEPING.test(l)).slice(0, MAX_COMMITS);
    if (targets.length > 1) out.push("", `### ${t.scope ?? t.path} (${lines.length})`, "");
    else out.push("", `${lines.length} commits`, "");
    for (const l of lines) out.push(`- ${l.slice(0, 120)}`);
  }
  return out.join("\n");
}

/** The open stories and a count of the closed ones; null when the product has no stories. */
export async function storiesSection(productId: string, root = dataRoot): Promise<string | null> {
  const stories = await listStories(productId, root);
  if (!stories.length) return null;
  const open = stories.filter((s) => s.status === "not_implemented");
  const count = (status: string) => stories.filter((s) => s.status === status).length;
  return [
    "## Stories",
    "",
    ...(open.length
      ? open.map((s) => `- ${s.id} ${s.title} — ${s.status}, approval ${s.approval}, session ${s.session?.state ?? "none"}`)
      : ["_No open stories._"]),
    "",
    `Implemented: ${count("implemented")}. Discarded: ${count("discarded")}.`,
  ].join("\n");
}

export async function dbSections(productId: string, scope: string | null, pool: Queryable = getPool()): Promise<string[]> {
  const sections: string[] = [];
  try {
    const issues = await pool.query<{ ref: string; title: string }>(
      `select ref, title from documents where product = $1 and kind = 'issue' and ($2::text is null or scope = $2)
       order by updated_at desc, ref limit 50`,
      [productId, scope],
    );
    sections.push(
      ["## Open issues", "", ...(issues.rows.length ? issues.rows.map((i) => `- #${i.ref} ${i.title}`) : ["_None indexed._"])].join("\n"),
    );

    const recent = await pool.query<{ id: string; question: string; created_at: Date }>(
      "select id, question, created_at from minutes where product = $1 order by created_at desc limit 3",
      [productId],
    );
    const out = ["## Last minutes, priorities and marks"];
    for (const m of recent.rows) {
      out.push("", `### ${m.id} — ${m.question}`, "");
      const ps = await pool.query<{ idx: number; title: string; mark: "accepted" | "changed" | "rejected" | null; is_new: boolean }>(
        "select idx, title, mark, is_new from priorities where minutes_id = $1 order by idx",
        [m.id],
      );
      for (const p of ps.rows) out.push(`- #${p.idx} ${markBox(p.mark)} ${p.title}${p.is_new ? " (new)" : ""}`);
    }
    if (!recent.rows.length) out.push("", "_No meetings yet._");
    sections.push(out.join("\n"));

    const pending = await pool.query<{ id: string; text: string; action: string | null; minutes_id: string | null }>(
      "select id, text, action, minutes_id from decisions where product = $1 and status = 'pending' order by id",
      [productId],
    );
    sections.push(
      ["## Pending decisions", "", ...(pending.rows.length ? pending.rows.map((d) => `- [${d.id}] ${d.text}${d.action ? ` → ${d.action}` : ""}`) : ["_None._"])].join("\n"),
    );
  } catch (err) {
    sections.push(`## Database sections unavailable\n\n${(err as Error).message}`);
  }
  return sections;
}

export async function snapshot(productId: string, scope: string | null = null): Promise<string> {
  const product = getProduct(productId);
  const targets = repoTargets(product, scope);
  const header = [
    `# Snapshot: ${product.name}${scope ? ` / ${scope}` : ""}`,
    "",
    `- Product: ${product.id} (${product.status})`,
    `- Active roles: ${product.roles.join(", ")}`,
    `- Paths: ${(targets.length ? targets.map((t) => t.path) : product.paths).join(", ")}`,
  ].join("\n");

  const sections = [
    header,
    northStar(product, targets, await readBrief(product.id)),
    ...[await storiesSection(productId).catch(() => null)].filter((s) => s !== null),
    ...(await dbSections(productId, scope)),
    await commits(targets),
    journal(targets),
    docsIndex(targets),
  ];

  let budget = MAX_BYTES;
  const kept: string[] = [];
  for (const s of sections) {
    const size = Buffer.byteLength(s) + 2;
    if (size <= budget) {
      kept.push(s);
      budget -= size;
    } else {
      const cut = Buffer.from(s).subarray(0, Math.max(0, budget - 40)).toString("utf8");
      kept.push(`${cut}\n\n_[truncated to fit ${MAX_BYTES / 1024} KB]_`);
      break;
    }
  }
  return kept.join("\n\n") + "\n";
}
