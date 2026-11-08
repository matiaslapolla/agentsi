import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, test } from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

// S008: the owner picks how many pending decisions show per page (5, 10 or 20) on the product page.
// The tests render the real product page. Its data comes from a temp AGENTSI_HOME and a fake query
// layer (no database, no network). The page's own params are its URL: `decisiones` is the page
// number that the page already reads, and SIZE_PARAM is the size the choice writes into the URL.
// SIZE_PARAM is an assumed name; the implementation must use it or these tests must change with it.

const PRODUCT = "s008-demo";
const PAGE_PARAM = "decisiones";
const SIZE_PARAM = "decisiones_size";
const TOTAL = 23;
const SIZES = [5, 10, 20] as const;

const pad = (n: number) => String(n).padStart(2, "0");
const range = (from: number, to: number) => Array.from({ length: to - from + 1 }, (_, i) => from + i);

const home = fs.mkdtempSync(path.join(os.tmpdir(), "s008-decisions-"));
fs.writeFileSync(
  path.join(home, "agentsi.config.json"),
  JSON.stringify({ products: [{ id: PRODUCT, name: "S008 demo", paths: [], status: "active", roles: ["pm"] }] }),
);
process.env.AGENTSI_HOME = home;

const pending = Array.from({ length: TOTAL }, (_, i) => ({
  id: i + 1,
  kind: "product",
  minutes_id: null,
  reflection_id: null,
  text: `Decisión ${pad(i + 1)}`,
  why: null,
  action: null,
}));

(globalThis as { __agentsiPool?: unknown }).__agentsiPool = {
  on() {},
  async query(sql: string) {
    return { rows: /from decisions where product/.test(sql) ? pending : [] };
  },
};

const { default: ProductPage } = await import("../app/p/[product]/page");

after(() => {
  delete (globalThis as { __agentsiPool?: unknown }).__agentsiPool;
  fs.rmSync(home, { recursive: true, force: true });
});

/** Opens the product page with these URL params and returns its HTML, as the browser gets it. */
async function open(query: Record<string, string> = {}): Promise<string> {
  const element = await ProductPage({
    params: Promise.resolve({ product: PRODUCT }),
    searchParams: Promise.resolve(query),
  });
  return renderToStaticMarkup(element);
}

const decode = (s: string) => s.replace(/&amp;/g, "&");

/** The decisions section, from its anchor to its end. */
function decisionsSection(html: string): string {
  const start = html.indexOf(`id="${PAGE_PARAM}"`);
  assert.notEqual(start, -1, "the product page has a decisions section");
  const end = html.indexOf("</section>", start);
  assert.notEqual(end, -1, "the decisions section closes");
  return html.slice(start, end);
}

/** The decision numbers in the list, in the order shown. */
function shownDecisions(html: string): number[] {
  const list = /<ul\b[\s\S]*?<\/ul>/.exec(decisionsSection(html))?.[0] ?? "";
  return [...list.matchAll(/Decisión (\d{2})/g)].map((m) => Number(m[1]));
}

/** The pager of the decisions list; empty when the list fits on one page. */
function pager(html: string): string {
  const start = html.indexOf(`aria-label="Páginas de decisiones pendientes"`);
  if (start === -1) return "";
  return html.slice(start, html.indexOf("</nav>", start));
}

/** Range and page as the pager shows them, e.g. "1–10 de 23" and "1/3". */
function pageState(html: string) {
  const nav = pager(html);
  const range = /(\d+)–(\d+) de (\d+)/.exec(nav);
  const current = /aria-current="page">(\d+)\/(\d+)</.exec(nav);
  if (!range || !current) return null;
  return { from: Number(range[1]), to: Number(range[2]), total: Number(range[3]), page: Number(current[1]), pages: Number(current[2]) };
}

/** Every link in a fragment, with its href decoded and its rel if it has one. */
function links(fragment: string): { href: string; rel: string | null }[] {
  return [...fragment.matchAll(/<a\b([^>]*)>/g)].map((m) => ({
    href: decode(/href="([^"]*)"/.exec(m[1])?.[1] ?? ""),
    rel: /rel="([^"]*)"/.exec(m[1])?.[1] ?? null,
  }));
}

function queryOf(href: string): Record<string, string> {
  return Object.fromEntries(new URL(href, "http://local").searchParams);
}

/** The link the page offers for choosing `size` per page, from the decisions section. */
function sizeChoice(html: string, size: number): string | undefined {
  return links(decisionsSection(html)).find(
    (l) => l.rel === null && new URL(l.href, "http://local").searchParams.get(SIZE_PARAM) === String(size),
  )?.href;
}

/** The owner picks `size` on the page that `html` shows: follow the choice and return the page it leads to. */
async function choose(html: string, size: number): Promise<string> {
  const href = sizeChoice(html, size);
  assert.ok(href, `the decisions list offers no way to choose ${size} per page`);
  return open(queryOf(href));
}

test("with no page size in the URL the list shows 10 of 23 decisions on 3 pages", async () => {
  const html = await open({});
  assert.deepEqual(shownDecisions(html), range(1, 10));
  assert.deepEqual(pageState(html), { from: 1, to: 10, total: 23, page: 1, pages: 3 });
});

for (const size of SIZES) {
  test(`choosing ${size} per page shows ${size} decisions with the page count and range to match`, async () => {
    const html = await choose(await open({}), size);
    assert.deepEqual(shownDecisions(html), range(1, size));
    assert.deepEqual(pageState(html), { from: 1, to: size, total: 23, page: 1, pages: Math.ceil(23 / size) });
  });
}

test("changing the size from page 3 of 5 to 20 goes back to page 1 and shows the first 20 decisions", async () => {
  const onPage3 = await open({ [PAGE_PARAM]: "3", [SIZE_PARAM]: "5" });
  assert.deepEqual(pageState(onPage3), { from: 11, to: 15, total: 23, page: 3, pages: 5 }, "the list starts on page 3 of 5");

  const after20 = await choose(onPage3, 20);
  assert.deepEqual(shownDecisions(after20), range(1, 20));
  assert.deepEqual(pageState(after20), { from: 1, to: 20, total: 23, page: 1, pages: 2 });
});

for (const raw of ["7", "abc", undefined] as const) {
  test(`a page size of ${raw ?? "nothing"} in the URL falls back to 10 per page without error`, async () => {
    const html = await open(raw === undefined ? {} : { [SIZE_PARAM]: raw });
    assert.deepEqual(shownDecisions(html), range(1, 10));
    assert.deepEqual(pageState(html), { from: 1, to: 10, total: 23, page: 1, pages: 3 });
  });
}

test("the pager links keep the chosen size, so the next page still shows 5 per page", async () => {
  const chosen = await choose(await open({}), 5);

  const next = links(pager(chosen)).find((l) => l.rel === "next");
  assert.ok(next, "page 1 of 5 has a next link");
  assert.equal(new URL(next.href, "http://local").searchParams.get(SIZE_PARAM), "5", "the next link keeps the size");

  const second = await open(queryOf(next.href));
  assert.deepEqual(shownDecisions(second), range(6, 10));
  assert.deepEqual(pageState(second), { from: 6, to: 10, total: 23, page: 2, pages: 5 });

  const middle = pager(await open({ [PAGE_PARAM]: "3", [SIZE_PARAM]: "5" }));
  const pagerLinks = links(middle).filter((l) => l.rel !== null);
  assert.deepEqual(pagerLinks.map((l) => l.rel).sort(), ["next", "prev"], "a middle page has both prev and next links");
  for (const l of pagerLinks) {
    assert.equal(new URL(l.href, "http://local").searchParams.get(SIZE_PARAM), "5", `the ${l.rel} link keeps the size`);
  }
});
