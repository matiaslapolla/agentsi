import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { getPool } from "./db.ts";
import { git, run } from "./exec.ts";
import { memoryAgents, readMemoryItems, renderForPrompt } from "./memory.ts";
import { loadRegistry, getProduct, type Product } from "./registry.ts";
import { repoTargets, type RepoTarget } from "./repos.ts";
import { readMinutes } from "./minutes-store.ts";
import { renderMinutesMarkdown } from "./minutes-md.ts";

const ROOT_DOCS = ["CLAUDE.md", "CONTEXT.md", "PRODUCT.md", "DESIGN.md", "README.md", "I18N.md"];
const SKIP_DIRS = new Set(["node_modules", ".worktrees", ".git", ".next", "dist"]);
const MAX_FILE_BYTES = 200 * 1024;
const FRESH_MS = 10 * 60 * 1000;
const COMMIT_DAYS = 90;
const REPO_KINDS = ["doc", "journal"] as const;

type Kind = "doc" | "journal" | "commit" | "issue" | "minutes" | "memory";

interface DocRow {
  scope: string | null;
  kind: Kind;
  ref: string;
  title: string;
  body: string;
  updatedAt: Date;
}

export interface RepoIndexResult {
  path: string;
  skipped: boolean;
  docs: number;
  commits: number;
  issues: number | null;
}

export interface IndexResult {
  product: string;
  repos: RepoIndexResult[];
  minutes: number;
  memory: number;
  ms: number;
}

export interface IndexOptions {
  force?: boolean;
}

async function upsertDocs(product: string, rows: DocRow[]): Promise<void> {
  const unique = new Map(rows.map((r) => [`${r.kind}\0${r.ref}`, r]));
  const all = [...unique.values()];
  const pool = getPool();
  for (let i = 0; i < all.length; i += 400) {
    const chunk = all.slice(i, i + 400);
    await pool.query(
      `insert into documents (product, scope, kind, ref, title, body, updated_at)
       select $1, * from unnest($2::text[], $3::text[], $4::text[], $5::text[], $6::text[], $7::timestamptz[])
       on conflict (product, kind, ref) do update
         set scope = excluded.scope, title = excluded.title, body = excluded.body, updated_at = excluded.updated_at
         where (documents.scope, documents.title, documents.body) is distinct from (excluded.scope, excluded.title, excluded.body)`,
      [
        product,
        chunk.map((r) => r.scope),
        chunk.map((r) => r.kind),
        chunk.map((r) => r.ref),
        chunk.map((r) => r.title),
        chunk.map((r) => r.body),
        chunk.map((r) => r.updatedAt.toISOString()),
      ],
    );
  }
}

async function deleteMissing(
  product: string,
  kinds: readonly Kind[],
  scope: string | null,
  prefix: string,
  keep: string[],
): Promise<void> {
  await getPool().query(
    `delete from documents
     where product = $1 and kind = any($2::text[]) and scope is not distinct from $3
       and ($4 = '' or starts_with(ref, $4 || '/') or starts_with(ref, $4 || '#'))
       and ref <> all($5::text[])`,
    [product, kinds, scope, prefix, keep],
  );
}

const firstHeading = (text: string, fallback: string) =>
  /^#\s+(.+)$/m.exec(text)?.[1]?.trim() ?? fallback;

function readCapped(file: string): { text: string; mtime: Date } | null {
  try {
    const st = statSync(file);
    if (!st.isFile() || st.size > MAX_FILE_BYTES) return null;
    return { text: readFileSync(file, "utf8"), mtime: st.mtime };
  } catch {
    return null;
  }
}

function walkMarkdown(dir: string, out: string[] = []): string[] {
  if (!existsSync(dir)) return out;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (SKIP_DIRS.has(entry.name)) continue;
    const full = join(dir, entry.name);
    if (entry.isDirectory()) walkMarkdown(full, out);
    else if (entry.isFile() && entry.name.endsWith(".md")) out.push(full);
  }
  return out;
}

export function collectDocs(t: RepoTarget): DocRow[] {
  const refOf = (rel: string) => (t.prefix ? `${t.prefix}/${rel}` : rel);
  const files = [...ROOT_DOCS.map((f) => join(t.path, f)), ...walkMarkdown(join(t.path, "docs"))];
  const rows: DocRow[] = [];
  for (const file of files) {
    const f = readCapped(file);
    if (!f) continue;
    const rel = relative(t.path, file).split(sep).join("/");
    rows.push({ scope: t.scope, kind: "doc", ref: refOf(rel), title: firstHeading(f.text, rel), body: f.text, updatedAt: f.mtime });
  }
  const journal = readCapped(join(t.path, ".claude/JOURNAL.md"));
  if (journal) {
    rows.push({
      scope: t.scope,
      kind: "journal",
      ref: refOf(".claude/JOURNAL.md"),
      title: "Journal",
      body: journal.text,
      updatedAt: journal.mtime,
    });
  }
  return rows;
}

async function collectCommits(t: RepoTarget): Promise<DocRow[] | null> {
  const res = await git(
    t.path,
    "log",
    "--branches",
    `--since=${COMMIT_DAYS}.days`,
    "--format=%H%x1f%aI%x1f%s%x1f%b%x1e",
  );
  if (!res.ok) return null;
  const rows: DocRow[] = [];
  for (const rec of res.stdout.split("\x1e")) {
    const [hash, date, subject, body = ""] = rec.trim().split("\x1f");
    if (!hash || !date || !subject) continue;
    rows.push({
      scope: t.scope,
      kind: "commit",
      ref: hash,
      title: subject,
      body: `${subject}\n${body.trim().slice(0, 600)}`.trim(),
      updatedAt: new Date(date),
    });
  }
  return rows;
}

async function collectIssues(t: RepoTarget): Promise<DocRow[] | null> {
  const remotes = await git(t.path, "remote");
  if (!remotes.ok || !remotes.stdout.trim()) return null;
  const res = await run(
    "gh",
    ["issue", "list", "--state", "open", "--json", "number,title,body,labels", "--limit", "50"],
    t.path,
    25_000,
  );
  if (!res.ok) return null;
  try {
    const issues = JSON.parse(res.stdout) as { number: number; title: string; body: string; labels: { name: string }[] }[];
    return issues.map((i) => ({
      scope: t.scope,
      kind: "issue" as const,
      ref: t.prefix ? `${t.prefix}#${i.number}` : String(i.number),
      title: i.title,
      body: `${i.title}\n${i.labels.map((l) => l.name).join(", ")}\n${i.body ?? ""}`.trim(),
      updatedAt: new Date(),
    }));
  } catch {
    return null;
  }
}

async function indexRepo(product: Product, t: RepoTarget, force: boolean): Promise<RepoIndexResult> {
  const pool = getPool();
  const head = (await git(t.path, "rev-parse", "HEAD")).stdout.trim() || "none";
  if (!force) {
    const { rows } = await pool.query<{ fresh: boolean }>(
      `select last_commit = $3 and indexed_at > now() - make_interval(secs => $4) as fresh
       from index_state where product = $1 and path = $2`,
      [product.id, t.path, head, FRESH_MS / 1000],
    );
    if (rows[0]?.fresh) return { path: t.path, skipped: true, docs: 0, commits: 0, issues: null };
  }

  const [commits, issues] = await Promise.all([collectCommits(t), collectIssues(t)]);
  const docs = collectDocs(t);
  await upsertDocs(product.id, [...docs, ...(commits ?? []), ...(issues ?? [])]);

  await deleteMissing(product.id, REPO_KINDS, t.scope, t.prefix, docs.map((d) => d.ref));
  if (commits && product.paths.length === 1) {
    await deleteMissing(product.id, ["commit"], t.scope, "", commits.map((c) => c.ref));
  }
  if (issues) await deleteMissing(product.id, ["issue"], t.scope, t.prefix, issues.map((i) => i.ref));

  await pool.query(
    `insert into index_state (product, path, last_commit, indexed_at) values ($1, $2, $3, now())
     on conflict (product, path) do update set last_commit = excluded.last_commit, indexed_at = excluded.indexed_at`,
    [product.id, t.path, head],
  );
  return { path: t.path, skipped: false, docs: docs.length, commits: commits?.length ?? 0, issues: issues?.length ?? null };
}

export async function indexWorkspace(productId: string): Promise<{ minutes: number; memory: number }> {
  const minutesRows: DocRow[] = [];
  for (const m of await readMinutes(productId)) {
    minutesRows.push({
      scope: m.scope,
      kind: "minutes",
      ref: m.id,
      title: m.question,
      body: renderMinutesMarkdown(m),
      updatedAt: new Date(m.created_at),
    });
  }
  const memoryRows: DocRow[] = [];
  for (const agent of await memoryAgents(productId)) {
    try {
      const items = await readMemoryItems(productId, agent);
      const body = renderForPrompt(items, Infinity);
      const last = items.flatMap((i) => [i.created_at, i.retired_at ?? ""]).sort().at(-1);
      if (body) memoryRows.push({ scope: null, kind: "memory", ref: agent, title: `Memory: ${agent}`, body, updatedAt: last ? new Date(last) : new Date() });
    } catch {
      continue;
    }
  }
  await upsertDocs(productId, [...minutesRows, ...memoryRows]);
  const pool = getPool();
  for (const [kind, rows] of [["minutes", minutesRows], ["memory", memoryRows]] as const) {
    await pool.query(`delete from documents where product = $1 and kind = $2 and ref <> all($3::text[])`, [
      productId,
      kind,
      rows.map((r) => r.ref),
    ]);
  }
  return { minutes: minutesRows.length, memory: memoryRows.length };
}

export async function indexProduct(productId: string, opts: IndexOptions = {}): Promise<IndexResult> {
  const started = Date.now();
  const product = getProduct(productId);
  const targets = repoTargets(product);
  const repos: RepoIndexResult[] = [];
  let next = 0;
  const worker = async () => {
    while (next < targets.length) {
      const t = targets[next++]!;
      repos.push(await indexRepo(product, t, opts.force ?? false));
    }
  };
  await Promise.all(Array.from({ length: Math.min(4, targets.length) }, worker));

  const ws = await indexWorkspace(product.id);
  return { product: product.id, repos, ...ws, ms: Date.now() - started };
}

export async function indexAll(opts: IndexOptions = {}): Promise<IndexResult[]> {
  const results: IndexResult[] = [];
  for (const p of loadRegistry().products.filter((p) => p.status === "active")) {
    results.push(await indexProduct(p.id, opts));
  }
  return results;
}
