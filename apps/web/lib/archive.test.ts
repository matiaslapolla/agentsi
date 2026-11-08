import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";
import type { Product } from "@agentsi/core";

// The data root is read when the modules load, so it is set before they are imported.
const root = mkdtempSync(join(tmpdir(), "agentsi-web-archive-"));
writeFileSync(join(root, "agentsi.config.json"), JSON.stringify({ products: [{ id: "agentsi", name: "AgentSI", paths: [root], status: "active", roles: ["pm", "dev"] }] }, null, 2) + "\n");
process.env.AGENTSI_HOME = root;
after(() => rmSync(root, { recursive: true, force: true }));

const { archiveMinutes: coreArchive, restoreMinutes: coreRestore, getPool } = await import("@agentsi/core");
const { testDb } = await import("@agentsi/core/test-db");
const { archiveMinutes, restoreMinutes } = await import("./archive");

const product: Product = { id: "agentsi", name: "AgentSI", paths: [], status: "active", roles: ["pm", "dev"] };
const ID = "2026-10-21-0900-roadmap";

/** A test database with one saved meeting of the product, shared by the web app and the CLI. */
async function given(t: { after(fn: () => unknown): void }) {
  const db = await testDb();
  t.after(() => db.close());
  const m = { id: ID, product: "agentsi", question: "What do we build next?", status: "valid", created_at: "2026-10-21T09:00:00Z", roles: {}, priorities: [], decisions: [] };
  await getPool().query("insert into minutes (id, product, data) values ($1, 'agentsi', $2::jsonb)", [ID, JSON.stringify(m)]);
  return m;
}

const archivedRows = async () => (await getPool().query("select ref from archive_records where kind = 'minutes'")).rows;

test("an archive from the web app is the record the CLI restores from", async (t) => {
  const m = await given(t);

  await archiveMinutes(product, ID, root);
  assert.equal((await archivedRows()).length, 1, "the web archive is a row of the database");
  await coreRestore("agentsi", ID);

  assert.deepEqual(await archivedRows(), [], "the CLI restore cleared the record the web app wrote");
  assert.deepEqual((await getPool().query("select data from minutes where id = $1", [ID])).rows, [{ data: m }], "the minutes keep their content");
});

test("an archive from the CLI is the record the web app restores from, and the web restore clears it", async (t) => {
  await given(t);
  await coreArchive("agentsi", ID);
  assert.deepEqual(await archivedRows(), [{ ref: ID }], "the CLI archive names the minutes in the database");

  await restoreMinutes(product, ID, root);

  assert.deepEqual(await archivedRows(), [], "the web restore cleared the record the CLI wrote");
});
