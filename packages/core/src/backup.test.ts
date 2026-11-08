import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { gunzipSync } from "node:zlib";
import { backup, backupDir, newestBackupAge } from "./backup.ts";

const SQL = "create table products (id text);\n";
const fakeDump = ["sh", "-c", `printf '${SQL.trim()}\\n'`];

function root(t: { after: (fn: () => void) => void }): string {
  const dir = mkdtempSync(join(tmpdir(), "agentsi-backup-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  writeFileSync(join(dir, "agentsi.config.json"), '{"products":[]}');
  return dir;
}

test("backup gzips the dump and copies the registry into backups/", async (t) => {
  const dir = root(t);
  const r = await backup({ root: dir, command: fakeDump, cwd: dir, now: new Date("2026-11-02T15:30:00Z") });
  assert.equal(r.dump, join(backupDir(dir), "agentsi-2026-11-02-1530.sql.gz"));
  assert.equal(gunzipSync(readFileSync(r.dump)).toString(), SQL);
  assert.equal(readFileSync(r.registry!, "utf8"), '{"products":[]}');
  assert.deepEqual(r.removed, []);
});

test("backup keeps the newest dumps and their registry copies", async (t) => {
  const dir = root(t);
  for (const day of [1, 2, 3, 4]) await backup({ root: dir, command: fakeDump, cwd: dir, keep: 2, now: new Date(`2026-11-0${day}T10:00:00Z`) });
  assert.deepEqual(readdirSync(backupDir(dir)).sort(), [
    "agentsi-2026-11-03-1000.sql.gz",
    "agentsi-2026-11-04-1000.sql.gz",
    "agentsi.config-2026-11-03-1000.json",
    "agentsi.config-2026-11-04-1000.json",
  ]);
});

test("a failed dump leaves no file and keeps the old dumps", async (t) => {
  const dir = root(t);
  await backup({ root: dir, command: fakeDump, cwd: dir, now: new Date("2026-11-01T10:00:00Z") });
  const failing = ["sh", "-c", "echo 'connection refused' >&2; exit 2"];
  await assert.rejects(backup({ root: dir, command: failing, cwd: dir, keep: 1, now: new Date("2026-11-02T10:00:00Z") }), /exited 2: connection refused/);
  assert.deepEqual(readdirSync(backupDir(dir)).sort(), ["agentsi-2026-11-01-1000.sql.gz", "agentsi.config-2026-11-01-1000.json"]);
});

test("newestBackupAge gives the hours since the newest dump, or null with none", async (t) => {
  const dir = root(t);
  assert.equal(newestBackupAge(dir), null);
  const { dump } = await backup({ root: dir, command: fakeDump, cwd: dir });
  const threeDaysAgo = (Date.now() - 72 * 3_600_000) / 1000;
  utimesSync(dump, threeDaysAgo, threeDaysAgo);
  assert.ok(Math.abs(newestBackupAge(dir)! - 72) < 0.1);
  assert.ok(existsSync(dump));
});
