import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { detectProduct, getDoc, getPool, loadRegistry, memoryForPrompt, search, type Product } from "@agentsi/core";
import { z } from "zod";
import { listMinutesText } from "./list-minutes.ts";
import { pendingDecisionsText } from "./pending-decisions.ts";

const MAX_DOC_CHARS = 12_000;
const KINDS = ["doc", "journal", "commit", "issue", "minutes", "memory"] as const;

type ToolResult = { content: { type: "text"; text: string }[]; isError?: boolean };

const text = (t: string): ToolResult => ({ content: [{ type: "text", text: t }] });
const fail = (t: string): ToolResult => ({ content: [{ type: "text", text: t }], isError: true });

function resolveProduct(id: string | undefined, cwd: string): { product: Product; scope: string | null } {
  const registry = loadRegistry();
  const ids = registry.products.map((p) => p.id).join(", ");
  if (id) {
    const product = registry.products.find((p) => p.id === id);
    if (!product) throw new Error(`Unknown product "${id}". Products: ${ids}`);
    const detected = detectProduct(cwd, registry);
    return { product, scope: detected?.product.id === id ? detected.scope : null };
  }
  const detected = detectProduct(cwd, registry);
  if (!detected) throw new Error(`No product matches ${cwd}. Pass "product" explicitly. Products: ${ids}`);
  return detected;
}

const productParam = z.string().optional().describe("Product id. Default: product of the cwd.");

const pad = (n: number) => String(n).padStart(2, "0");
const stamp = (d: Date) =>
  `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;

interface Heading {
  level: number;
  text: string;
  start: number;
}

function headings(body: string): Heading[] {
  const out: Heading[] = [];
  let fenced = false;
  let start = 0;
  for (const line of body.split("\n")) {
    if (/^\s*(```|~~~)/.test(line)) fenced = !fenced;
    const m = fenced ? null : /^(#{1,6})\s+(.+?)\s*#*\s*$/.exec(line);
    if (m) out.push({ level: m[1]!.length, text: m[2]!, start });
    start += line.length + 1;
  }
  return out;
}

function section(body: string, name: string): string | null {
  const hs = headings(body);
  const want = name.replace(/^#+\s*/, "").trim().toLowerCase();
  const exact = hs.findIndex((h) => h.text.toLowerCase() === want);
  const at = exact >= 0 ? exact : hs.findIndex((h) => h.text.toLowerCase().includes(want));
  if (at < 0) return null;
  const h = hs[at]!;
  const end = hs.slice(at + 1).find((n) => n.level <= h.level)?.start ?? body.length;
  return body.slice(h.start, end).trimEnd();
}

const h2List = (body: string) =>
  headings(body)
    .filter((h) => h.level === 2)
    .map((h) => h.text)
    .join(" | ");

export function createServer(cwd = process.cwd()): McpServer {
  const server = new McpServer({ name: "agentsi", version: "0.2.0" });

  const tool = <S extends z.ZodRawShape>(
    name: string,
    description: string,
    shape: S,
    run: (args: z.infer<z.ZodObject<S>>, ctx: { product: Product; scope: string | null }) => Promise<string>,
  ) =>
    server.registerTool(
      name,
      { description, inputSchema: { product: productParam, ...shape } },
      (async (args: z.infer<z.ZodObject<S>> & { product?: string }) => {
        try {
          return text(await run(args, resolveProduct(args.product, cwd)));
        } catch (err) {
          return fail((err as Error).message);
        }
      }) as never,
    );

  tool(
    "search",
    "Full-text search over the product's docs, journal, commits, issues and minutes. Returns ranked hits (kind, ref, title, snippet); read one with get_doc. Query in the language of the documents.",
    {
      query: z.string().min(1).describe("Terms. Quotes for phrases, 'or' for alternatives, '-' to exclude."),
      kind: z.enum(KINDS).optional().describe("Only this kind. memory is searched only when named."),
      limit: z.number().int().min(1).max(50).default(5).describe("Max hits (default 5)."),
    },
    async ({ query, kind, limit }, { product }) => {
      const opts = { kind, limit: limit ?? 5 };
      let hits = await search(product.id, query, opts);
      let note = "";
      if (!hits.length) {
        hits = await search(product.id, query, { ...opts, anyTerm: true });
        note = `No hits for all terms; showing hits for any term.\n`;
      }
      if (!hits.length) return `No hits in ${product.id} for "${query}".`;
      return (
        note +
        hits
          .map((h, i) => {
            const ref = h.kind === "commit" ? h.ref.slice(0, 12) : h.ref;
            let snippet = h.snippet.replace(/\s+/g, " ").trim();
            if (snippet.startsWith(h.title)) snippet = snippet.slice(h.title.length).trim();
            return `${i + 1}. [${h.kind}] ref=${ref}${h.scope ? ` scope=${h.scope}` : ""}\n   ${h.title}${snippet ? `\n   ${snippet}` : ""}`;
          })
          .join("\n")
      );
    },
  );

  tool(
    "get_doc",
    `Read one indexed item by kind and ref from a search hit. Bodies over ${MAX_DOC_CHARS} chars are cut; use section or offset for the rest.`,
    {
      kind: z.enum(KINDS).describe("Kind, as in the search hit."),
      ref: z.string().min(1).describe("Ref, as in the search hit. A commit hash prefix works."),
      section: z.string().min(1).optional().describe("Markdown heading text (case-insensitive): return only that section."),
      offset: z.number().int().min(0).optional().describe("Character offset to start reading from."),
    },
    async ({ kind, ref, section: name, offset }, { product }) => {
      const doc = await getDoc(product.id, kind, ref);
      if (!doc) throw new Error(`No ${kind} "${ref}" in ${product.id}. Use search to find valid refs.`);
      let body = doc.body;
      if (name) {
        const found = section(body, name);
        if (found === null) throw new Error(`No section "${name}" in ${doc.ref}. Sections: ${h2List(body) || "none"}`);
        body = found;
      }
      const from = offset ?? 0;
      const to = Math.min(body.length, from + MAX_DOC_CHARS);
      const head = `# ${doc.title}\nkind=${doc.kind} ref=${doc.ref} updated=${stamp(doc.updated_at)}\n\n`;
      if (from === 0 && to === body.length) return head + body;
      const sections = h2List(body);
      const tail =
        to < body.length
          ? `[truncated: chars ${from}-${to} of ${body.length}. Request offset=${to}${sections ? ` or section=<heading>. Sections: ${sections}` : ""}]`
          : `[chars ${from}-${to} of ${body.length}]`;
      return `${head}${body.slice(from, to)}\n\n${tail}`;
    },
  );

  tool(
    "list_minutes",
    "List recent meeting minutes: id, status, date, question and the owner's marks on the priorities. Read one with get_doc kind=minutes.",
    {
      limit: z.number().int().min(1).max(50).default(5).describe("Max minutes (default 5)."),
      include_archived: z.boolean().default(false).describe("Also list archived minutes, marked as archived (default false)."),
    },
    async ({ limit, include_archived }, { product }) =>
      listMinutesText(getPool(), product.id, limit ?? 5, { includeArchived: include_archived ?? false }),
  );

  tool(
    "pending_decisions",
    "List decisions waiting for the owner's approval, grouped by minutes. Never act on a pending decision.",
    { limit: z.number().int().min(1).max(100).default(10).describe("Max decisions (default 10).") },
    async ({ limit }, { product }) => {
      return pendingDecisionsText(getPool(), product.id, limit ?? 10);    },
  );

  tool(
    "role_memory",
    "Read what one role learned about this product (active memory items, newest first, with ids). Read it before answering in that role.",
    { role: z.string().regex(/^[a-z0-9_.-]+$/i).describe("Agent id: pm, social, dev, qa, scout or a specialist such as dev.backend.") },
    async ({ role }, { product }) => memoryForPrompt(product.id, role) || "sin memoria todavía",
  );

  return server;
}

/** Serves MCP on stdio until the client closes the pipe. Stdout is protocol-only; log to stderr. */
export async function runStdio(cwd = process.cwd()): Promise<void> {
  const server = createServer(cwd);
  const closed = new Promise<void>((resolve) => {
    server.server.onclose = resolve;
  });
  await server.connect(new StdioServerTransport());
  console.error(`agentsi mcp: ready (cwd ${cwd})`);
  await closed;
}
