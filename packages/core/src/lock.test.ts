import assert from "node:assert/strict";
import { existsSync, mkdtempSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { setTimeout as sleep } from "node:timers/promises";
import { withLock } from "./lock.ts";

test("withLock runs critical sections one at a time and releases the file", async () => {
  const file = join(mkdtempSync(join(tmpdir(), "agentsi-lock-")), "sub", ".lock");
  const trace: string[] = [];
  const section = (name: string) => async () => {
    trace.push(`${name} in`);
    await sleep(20);
    trace.push(`${name} out`);
  };
  await Promise.all([withLock(file, section("a")), withLock(file, section("b"))]);
  assert.deepEqual(trace, ["a in", "a out", "b in", "b out"]);
  assert.equal(existsSync(file), false);
  await assert.rejects(withLock(file, () => Promise.reject(new Error("boom"))), /boom/);
  assert.equal(existsSync(file), false);
});

test("withLock takes over a stale lock and times out on a live one", async () => {
  const file = join(mkdtempSync(join(tmpdir(), "agentsi-lock-")), ".lock");
  writeFileSync(file, "1");
  await assert.rejects(withLock(file, () => 1, { waitMs: 60 }), /Timed out/);
  const old = new Date(Date.now() - 120_000);
  utimesSync(file, old, old);
  assert.equal(await withLock(file, () => 2), 2);
});
