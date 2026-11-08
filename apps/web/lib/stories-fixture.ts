import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { getPool } from "@agentsi/core";
import { testDb } from "@agentsi/core/test-db";

const fixtures = path.resolve(import.meta.dirname, "..", "fixtures", "stories");

/**
 * For tests: an in-process database with the fixture stories of product `agentsi` in its stories table,
 * and a temp data root (for the archive record). Stories are not files, so the root starts empty.
 * The database closes when the test ends.
 */
export async function fixtureRoot(t: { after: (fn: () => void | Promise<void>) => void }): Promise<string> {
  const db = await testDb();
  t.after(() => db.close());
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "agentsi-web-stories-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  for (const f of fs.readdirSync(fixtures)) {
    const story = JSON.parse(fs.readFileSync(path.join(fixtures, f), "utf8")) as { id: string };
    await getPool().query("insert into stories (product, id, data) values ($1, $2, $3::jsonb)", ["agentsi", story.id, JSON.stringify(story)]);
  }
  return root;
}
