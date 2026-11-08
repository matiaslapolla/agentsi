import assert from "node:assert/strict";
import fs from "node:fs";
import { createRequire } from "node:module";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { after, beforeEach, test } from "node:test";
import type { ReactNode } from "react";

const web = path.resolve(import.meta.dirname, "..");
const PRODUCT = "agentsi";
const MEETINGS = [
  { id: "20261020-1000-a", question: "Qué hacemos con el login" },
  { id: "20261021-1000-b", question: "Cómo medimos la retención" },
  { id: "20261022-1000-c", question: "Qué priorizamos esta semana" },
];
const [A, B] = MEETINGS.map((m) => m.id);
/** The label of a heard call, the same word the call page uses. */
const LISTENED = /escuchad/i;

// env.ts reads the data root when it is first imported, so the fixture home is set before any app module loads.
const home = fs.mkdtempSync(path.join(os.tmpdir(), "agentsi-web-listened-"));
process.env.AGENTSI_HOME = home;
// No database here: the lists come from the files alone.
process.env.AGENTSI_DATABASE_URL = "postgres://agentsi@127.0.0.1:1/agentsi";
fs.writeFileSync(
  path.join(home, "agentsi.config.json"),
  JSON.stringify({ products: [{ id: PRODUCT, name: "AgentSI", paths: [], status: "active", roles: ["pm", "qa"] }] }),
);

after(() => fs.rmSync(home, { recursive: true, force: true }));

const actionsModule = await import("../app/actions");
const finishCall = actionsModule.finishCall;
const removeListen = (actionsModule as unknown as Record<string, unknown>).removeListen;

type CallPageFn = (props: { params: Promise<{ product: string; id: string }> }) => Promise<ReactNode>;
type ProductPageFn = (props: {
  params: Promise<{ product: string }>;
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) => Promise<ReactNode>;

const callPage = (await import(pathToFileURL(path.join(web, "app/p/[product]/m/[id]/call/page.tsx")).href)).default as CallPageFn;
const productPage = (await import(pathToFileURL(path.join(web, "app/p/[product]/page.tsx")).href)).default as ProductPageFn;

const webRequire = createRequire(path.join(web, "package.json"));
const reactDomServer = pathToFileURL(webRequire.resolve("react-dom/server")).href;
const { renderToStaticMarkup } = (await import(reactDomServer)) as typeof import("react-dom/server");

const ws = path.join(home, "workspace", PRODUCT);

const { getPool } = await import("@agentsi/core");
const { testDb } = await import("@agentsi/core/test-db");
let testDatabase: { close(): Promise<void> } | undefined;

/** The product as the owner has it: three meetings in the test database, each with a call (an audio file) that can be played. */
async function seed(): Promise<void> {
  await testDatabase?.close();
  testDatabase = await testDb();
  fs.rmSync(ws, { recursive: true, force: true });
  for (const [i, { id, question }] of MEETINGS.entries()) {
    const m = {
      id,
      product: PRODUCT,
      question,
      status: "valid",
      created_at: `2026-10-${20 + i}T10:00:00Z`,
      priorities: [],
      decisions: [],
      script: [{ role: "pm", text: "Vamos a lo que importa." }],
    };
    await getPool().query("insert into minutes (id, product, question, status, created_at, data) values ($1, $2, $3, 'valid', $4, $5::jsonb)", [id, PRODUCT, question, m.created_at, JSON.stringify(m)]);
    fs.mkdirSync(path.join(ws, "audio", id), { recursive: true });
    fs.writeFileSync(path.join(ws, "audio", id, "call.webm"), "audio");
    fs.writeFileSync(
      path.join(ws, "audio", id, "timings.json"),
      JSON.stringify([{ idx: 0, role: "pm", text: "Vamos a lo que importa.", start: 0, end: 3 }]),
    );
  }
}
beforeEach(seed);
after(() => testDatabase?.close());

/** The stored listens of the product, oldest first: one row per meeting. */
async function listenRows(): Promise<{ minutes_id: string; at: string }[]> {
  return (await getPool().query("select minutes_id, at from listens where product = $1 order by at, minutes_id", [PRODUCT])).rows as { minutes_id: string; at: string }[];
}

async function listenedIds(): Promise<string[]> {
  return (await listenRows()).map((r) => r.minutes_id);
}

/** Every file of the product's workspace, so a store on disk would show up as a new file. */
function workspaceFiles(): string[] {
  return fs
    .readdirSync(ws, { recursive: true })
    .map(String)
    .filter((f) => fs.statSync(path.join(ws, f)).isFile())
    .sort();
}

async function minutesSnapshot(): Promise<unknown[]> {
  return (await getPool().query("select id, data from minutes order by id")).rows;
}

async function callPageHtml(id: string): Promise<string> {
  return renderToStaticMarkup(await callPage({ params: Promise.resolve({ product: PRODUCT, id }) }));
}

async function productPageHtml(): Promise<string> {
  return renderToStaticMarkup(await productPage({ params: Promise.resolve({ product: PRODUCT }), searchParams: Promise.resolve({}) }));
}

/** The buttons of the call page whose text or label says listened. */
function listenedButtons(html: string): string[] {
  return (html.match(/<button\b[\s\S]*?<\/button>/g) ?? []).filter((b) => LISTENED.test(b));
}

/** The call page shows the meeting as listened: its listened button is pressed. */
const shownAsListened = (html: string): boolean => listenedButtons(html).some((b) => /aria-pressed="true"/.test(b));

/** The list item of one meeting on the product page. */
function itemFor(html: string, id: string): string {
  const item = (html.match(/<li\b[\s\S]*?<\/li>/g) ?? []).find((li) => li.includes(`href="/p/${PRODUCT}/m/${id}"`));
  if (!item) throw new Error(`no list item for meeting ${id} on the product page`);
  return item;
}

test("the listened button writes the listen through recordListen to the listens table, and the call page reads it back as listened", async () => {
  const before = workspaceFiles();
  const html = await callPageHtml(A);
  assert.equal(shownAsListened(html), false, "a call with no listen record must not show as listened");
  assert.equal(listenedButtons(html).length, 1, "the call page has a listened button");

  // Pressing the button runs finishCall, the same action the player runs when a call ends.
  await finishCall(PRODUCT, A);

  assert.deepEqual(await listenedIds(), [A]);
  assert.deepEqual(
    workspaceFiles().filter((f) => !before.includes(f)),
    [],
    "the listen goes to the database and to no file",
  );
  assert.equal(shownAsListened(await callPageHtml(A)), true);
});

test("the meetings list labels the meetings with a listen record and no others", async () => {
  await finishCall(PRODUCT, A); // the listened button
  await finishCall(PRODUCT, B); // a call that played to the end

  const html = await productPageHtml();
  assert.match(itemFor(html, A), LISTENED);
  assert.match(itemFor(html, B), LISTENED);
  assert.doesNotMatch(itemFor(html, MEETINGS[2].id), LISTENED);
});

test("pressing the button again removes that meeting's listen, its list label, and keeps the other listens", async () => {
  await finishCall(PRODUCT, A);
  await finishCall(PRODUCT, B);
  const bRow = (await listenRows()).find((r) => r.minutes_id === B);

  assert.equal(typeof removeListen, "function", "app/actions.ts exports removeListen, the action of the unmark press");
  await (removeListen as (product: string, minutesId: string) => Promise<unknown>)(PRODUCT, A);

  assert.deepEqual(await listenRows(), [bRow], "only the listen of the unmarked meeting is removed");
  assert.equal(shownAsListened(await callPageHtml(A)), false);
  const html = await productPageHtml();
  assert.doesNotMatch(itemFor(html, A), LISTENED);
  assert.match(itemFor(html, B), LISTENED);
});

test("the product page reads the listened meeting back from the database, and the minutes are unchanged", async () => {
  const minutesBefore = await minutesSnapshot();
  await finishCall(PRODUCT, A);

  // The listen is a row, so a page that loads later reads it from there, with nothing of the press in memory.
  const later = await productPageHtml();
  assert.match(itemFor(later, A), LISTENED);
  assert.doesNotMatch(itemFor(later, B), LISTENED);
  assert.deepEqual(await minutesSnapshot(), minutesBefore, "marking a meeting does not edit its minutes");
});
