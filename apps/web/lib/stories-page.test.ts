import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, test } from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { getPool } from "@agentsi/core";
import { testDb } from "@agentsi/core/test-db";

// S009: the owner pages the stories list of the web app, 5, 10 or 20 per page, like the decisions list (S008).
// The tests render the real stories page. Its data comes from a temp AGENTSI_HOME with 23 story files
// (no database, no network). The page's own params are its URL.
// PAGE_PARAM and SIZE_PARAM are assumed names, following the decisions list (`decisiones`, `decisiones_size`):
// the implementation must use them or these tests must change with them.
// The size choice is a link whose text is the size; the chosen one carries aria-current="true", as on the product page.
// The pages are links with the page number in PAGE_PARAM, and the range reads "1–10 de 23".

const PRODUCT = "s009-demo";
const PAGE_PARAM = "stories";
const SIZE_PARAM = "stories_size";
const TOTAL = 23;
const SIZES = [5, 10, 20] as const;
const STATUSES = ["not_implemented", "implemented", "discarded"] as const;
type Status = (typeof STATUSES)[number];

const pad = (n: number) => String(n).padStart(3, "0");
const idOf = (n: number) => `S${pad(n)}`;
const range = (from: number, to: number) => Array.from({ length: to - from + 1 }, (_, i) => from + i);
const idsFrom = (from: number, to: number) => range(from, to).map(idOf);

// Stories 1 to 14 are not implemented, 15 to 19 implemented, 20 to 23 discarded. Oldest id first is also group order,
// so the first 10 stories are S001 to S010, all under "Sin implementar".
const statusOf = (n: number): Status => (n <= 14 ? "not_implemented" : n <= 19 ? "implemented" : "discarded");

const criterion = (n: number, k: number) => ({
  given: `un producto con stories ${k}`,
  when: `abro la historia ${pad(n)}`,
  then: `veo el criterio ${k} de la historia ${pad(n)}`,
  test: "apps/web/lib/stories-page.test.ts",
  checks: `el criterio ${k} aparece`,
});

const storyFile = (n: number) => ({
  id: idOf(n),
  type: "story",
  slug: `historia-${pad(n)}`,
  title: `Historia ${pad(n)}`,
  as_a: "dueño de la agencia",
  i_want: `ver la historia ${pad(n)} en la lista`,
  so_that: "elegir cuántas veo por página",
  criteria: [criterion(n, 1), criterion(n, 2)],
  owner_role: "dev",
  objective: null,
  sources: [],
  status: statusOf(n),
  approval: "pending",
  changes_note: null,
  proposed_by: "pm",
  minutes: null,
  session: null,
  results: [],
  created_at: "2026-10-21T12:00:00.000Z",
  updated_at: "2026-10-21T12:00:00.000Z",
});

const home = fs.mkdtempSync(path.join(os.tmpdir(), "s009-stories-"));
fs.writeFileSync(
  path.join(home, "agentsi.config.json"),
  JSON.stringify({
    products: [
      {
        id: PRODUCT,
        name: "S009 demo",
        paths: [],
        status: "active",
        roles: ["pm", "dev", "qa"],
        stories: { repo: home, test: "node --test {files}", gate: ["npm run typecheck"] },
      },
    ],
  }),
);
// The stories are rows of an in-process database, not files.
const db = await testDb();
for (const n of range(1, TOTAL)) {
  await getPool().query("insert into stories (product, id, data) values ($1, $2, $3::jsonb)", [PRODUCT, idOf(n), JSON.stringify(storyFile(n))]);
}
process.env.AGENTSI_HOME = home;

const { default: StoriesPage } = await import("../app/p/[product]/stories/page");

after(async () => {
  await db.close();
  fs.rmSync(home, { recursive: true, force: true });
});

/** Opens the stories page with these URL params and returns its HTML, as the browser gets it. */
async function open(query: Record<string, string> = {}): Promise<string> {
  const element = await StoriesPage({
    params: Promise.resolve({ product: PRODUCT }),
    searchParams: Promise.resolve(query),
  });
  return renderToStaticMarkup(element);
}

const decode = (s: string) => s.replace(/&amp;/g, "&");
const queryOf = (href: string): Record<string, string> => Object.fromEntries(new URL(href, "http://local").searchParams);

/** Every anchor of the page: its attributes, its text and its decoded href. */
function anchors(html: string) {
  return [...html.matchAll(/<a\b([^>]*)>([\s\S]*?)<\/a>/g)].map((m) => ({
    attrs: m[1],
    text: m[2].replace(/<[^>]+>/g, "").trim(),
    href: decode(/href="([^"]*)"/.exec(m[1])?.[1] ?? ""),
  }));
}

/** The story ids that the page lists, in the order shown. */
const shown = (html: string): string[] => [...html.matchAll(/\/stories\/([ST]\d{3,})"/g)].map((m) => m[1]);

/** The range the page shows, e.g. "1–10 de 23". */
function rangeOf(html: string): { from: number; to: number; total: number } | null {
  const m = /(\d+)\s*[–-]\s*(\d+) de (\d+)/.exec(html);
  return m ? { from: Number(m[1]), to: Number(m[2]), total: Number(m[3]) } : null;
}

/** The page numbers the page links to. */
const linkedPages = (html: string): string[] =>
  anchors(html)
    .map((a) => queryOf(a.href)[PAGE_PARAM])
    .filter((p): p is string => p !== undefined);

/** The size choices on the page: a link whose text is the size, whose href carries the size param and no page, since a size choice goes back to page 1. */
function sizeChoices(html: string): { size: number; href: string; chosen: boolean }[] {
  return anchors(html)
    .filter((a) => SIZES.map(String).includes(a.text) && queryOf(a.href)[SIZE_PARAM] === a.text && queryOf(a.href)[PAGE_PARAM] === undefined)
    .map((a) => ({
      size: Number(a.text),
      href: a.href,
      chosen: /aria-(current|pressed|selected|checked)="(true|page)"/.test(a.attrs),
    }));
}

const chosenSizes = (html: string): number[] => sizeChoices(html).filter((c) => c.chosen).map((c) => c.size);

/** The owner picks `size` on the page that `html` shows: follow the choice and return the page it leads to. */
async function choose(html: string, size: number): Promise<string> {
  const choice = sizeChoices(html).find((c) => c.size === size);
  assert.ok(choice, `the stories list offers no way to choose ${size} per page`);
  return open(queryOf(choice.href));
}

/** The story ids under each status heading of the page, in the order shown. */
function groupsOf(html: string): Record<Status, string[]> {
  const out = {} as Record<Status, string[]>;
  for (const status of STATUSES) {
    const start = html.indexOf(`id="${status}"`);
    const end = start === -1 ? -1 : html.indexOf("</section>", start);
    out[status] = start === -1 || end === -1 ? [] : shown(html.slice(start, end));
  }
  return out;
}

/** The grouping that the stories of this list must show: each status with its own stories, oldest first. */
const groupsFor = (numbers: number[]): Record<Status, string[]> =>
  Object.fromEntries(STATUSES.map((s) => [s, numbers.filter((n) => statusOf(n) === s).map(idOf)])) as Record<Status, string[]>;

const hasError = (html: string) => /role="alert"/.test(html);

test("with no page size in the URL the list shows stories 1 to 10 of 23 and links to pages 2 and 3", async () => {
  const html = await open({});
  assert.deepEqual(shown(html), idsFrom(1, 10));
  assert.deepEqual(rangeOf(html), { from: 1, to: 10, total: 23 });
  const pages = linkedPages(html);
  assert.ok(pages.includes("2"), "the list links to page 2");
  assert.ok(pages.includes("3"), "the list links to page 3");
});

test("page 2 at 5 per page shows stories 6 to 10 and marks 5 as the chosen size", async () => {
  const html = await open({ [PAGE_PARAM]: "2", [SIZE_PARAM]: "5" });
  assert.deepEqual(shown(html), idsFrom(6, 10));
  assert.deepEqual(rangeOf(html), { from: 6, to: 10, total: 23 });
  assert.deepEqual(sizeChoices(html).map((c) => c.size), [5, 10, 20], "the list offers 5, 10 and 20");
  assert.deepEqual(chosenSizes(html), [5], "only 5 is marked as the chosen size");
});

for (const raw of ["7", "15", "abc", "0"]) {
  test(`a page size of ${raw} in the URL falls back to 10 per page without an error`, async () => {
    const html = await open({ [SIZE_PARAM]: raw });
    assert.equal(hasError(html), false, "the page shows no error");
    assert.deepEqual(shown(html), idsFrom(1, 10));
    assert.deepEqual(rangeOf(html), { from: 1, to: 10, total: 23 });
    assert.deepEqual(chosenSizes(html), [10], "10 is marked as the chosen size");
  });
}

for (const raw of ["abc", "0", "-1"]) {
  test(`a malformed page ${raw} in the URL shows page 1 without an error`, async () => {
    const html = await open({ [PAGE_PARAM]: raw });
    assert.equal(hasError(html), false, "the page shows no error");
    assert.deepEqual(shown(html), idsFrom(1, 10));
    assert.deepEqual(rangeOf(html), { from: 1, to: 10, total: 23 });
  });
}

for (const size of [10, 5] as const) {
  test(`a page past the end at ${size} per page clamps to the last page, which shows stories 21 to 23`, async () => {
    const html = await open({ [PAGE_PARAM]: "99", [SIZE_PARAM]: String(size) });
    assert.equal(hasError(html), false, "the page shows no error");
    assert.deepEqual(shown(html), idsFrom(21, 23));
    assert.deepEqual(rangeOf(html), { from: 21, to: 23, total: 23 });
  });
}

for (const size of [5, 20] as const) {
  test(`on page 3, choosing ${size} per page goes back to page 1 with ${size} stories grouped by status`, async () => {
    const onPage3 = await open({ [PAGE_PARAM]: "3", [SIZE_PARAM]: "10" });
    assert.deepEqual(shown(onPage3), idsFrom(21, 23), "the list starts on page 3");

    const chosen = await choose(onPage3, size);
    assert.deepEqual(shown(chosen), idsFrom(1, size), "the list goes back to page 1");
    assert.deepEqual(rangeOf(chosen), { from: 1, to: size, total: 23 });
    assert.deepEqual(chosenSizes(chosen), [size], `${size} is marked as the chosen size`);
    assert.deepEqual(groupsOf(chosen), groupsFor(range(1, size)), "each status shows its own stories");
  });
}

for (const size of SIZES) {
  test(`at ${size} per page every page keeps its stories under their status headings`, async () => {
    const pages = Math.ceil(TOTAL / size);
    for (let page = 1; page <= pages; page++) {
      const from = (page - 1) * size + 1;
      const to = Math.min(TOTAL, page * size);
      const html = await open({ [PAGE_PARAM]: String(page), [SIZE_PARAM]: String(size) });
      assert.deepEqual(shown(html), idsFrom(from, to), `page ${page} of ${pages} shows its stories`);
      assert.deepEqual(groupsOf(html), groupsFor(range(from, to)), `page ${page} of ${pages} groups its stories by status`);
    }
  });
}
