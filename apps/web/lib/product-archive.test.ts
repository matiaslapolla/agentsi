import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { after, before, test } from "node:test";
import { renderToStaticMarkup } from "react-dom/server";

// S015: the home page hides the card of an archived product by default. With the option to show archived products,
// the archived card shows too, with a restore action. The tests render the real home page (no database, no network).
// The product is archived with the real CLI, so the page reads the archive record that the CLI wrote.
// The option is found as a link on the page, whatever its URL parameter is, and followed as the owner would follow it.

const ACTIVE = { id: "helix", name: "Helix" };
const ARCHIVED = { id: "orbit", name: "Orbit" };

const root = fs.mkdtempSync(path.join(os.tmpdir(), "s015-web-archive-"));
process.env.AGENTSI_HOME = root;
process.env.AGENTSI_DATABASE_URL = "postgres://x:x@127.0.0.1:1/x";

let testDatabase: { close(): Promise<void> } | undefined;

before(async () => {
  fs.writeFileSync(
    path.join(root, "agentsi.config.json"),
    JSON.stringify(
      {
        products: [
          { id: ACTIVE.id, name: ACTIVE.name, paths: [root], status: "active", roles: ["pm", "dev"] },
          { id: ARCHIVED.id, name: ARCHIVED.name, paths: [root], status: "active", roles: ["pm", "dev"] },
        ],
      },
      null,
      2,
    ) + "\n",
  );
  // The owner archives the product in the terminal, so the home page reads the record that the archive wrote to the database.
  const { testDb } = await import("@agentsi/core/test-db");
  const { archiveProduct } = await import("@agentsi/core");
  testDatabase = await testDb();
  await archiveProduct(ARCHIVED.id);
});

after(async () => {
  await testDatabase?.close();
  fs.rmSync(root, { recursive: true, force: true });
});

/** Opens the home page with these URL params and returns its HTML, as the browser gets it. */
async function open(query: Record<string, string> = {}): Promise<string> {
  const { default: Home } = await import("../app/page");
  const element = await Home({ searchParams: Promise.resolve(query) });
  return renderToStaticMarkup(element);
}

const decode = (s: string) => s.replace(/&amp;/g, "&");
const textOf = (html: string) => html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");

/** Every product card of the page: each list item that links to the product. */
const cardOf = (html: string, id: string): string | undefined => (html.match(/<li\b[\s\S]*?<\/li>/g) ?? []).find((li) => li.includes(`href="/p/${id}"`));

/** The page's own link to show the archived products, read from its href. */
function archivedOption(html: string): Record<string, string> {
  const anchor = [...html.matchAll(/<a\b([^>]*)>([\s\S]*?)<\/a>/g)].find((m) => /archiv/i.test(textOf(m[2])));
  assert.ok(anchor, "the home page has an option to show the archived products");
  const href = decode(/href="([^"]*)"/.exec(anchor[1])?.[1] ?? "");
  return Object.fromEntries(new URL(href, "http://local").searchParams);
}

test("the home page shows the active product's card and hides the archived product's card by default", async () => {
  const html = await open();

  assert.ok(cardOf(html, ACTIVE.id), "the active product has its card");
  assert.equal(html.includes(`href="/p/${ARCHIVED.id}"`), false, "the archived product has no card by default");
  assert.equal(textOf(html).includes(ARCHIVED.name), false, "the archived product's name is not shown by default");
});

test("with the option to show archived products, the archived card shows too, with a restore action", async () => {
  const query = archivedOption(await open());

  const html = await open(query);

  assert.ok(cardOf(html, ACTIVE.id), "the active product still has its card");
  const archived = cardOf(html, ARCHIVED.id);
  assert.ok(archived, "the archived product's card shows with the option");
  assert.match(textOf(archived), /restaur/i, "the archived card has a restore action");
  assert.doesNotMatch(textOf(cardOf(html, ACTIVE.id) ?? ""), /restaur/i, "the active card has no restore action");
});
