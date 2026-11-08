import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { testHome } from "./test-home.ts";

const bin = fileURLToPath(new URL("../bin/agentsi.mjs", import.meta.url));
const desk = (env: NodeJS.ProcessEnv, cwd: string, ...args: string[]) => spawnSync(process.execPath, [bin, "desk", ...args], { cwd, encoding: "utf8", env });

test("desk exits with code 2 from a directory of no product", (t) => {
  const h = testHome([{ id: "demo", name: "Demo", roles: ["pm"] }]);
  const elsewhere = mkdtempSync(join(tmpdir(), "agentsi-desk-none-"));
  t.after(() => (h.cleanup(), rmSync(elsewhere, { recursive: true, force: true })));
  const res = desk(h.env, elsewhere);
  assert.equal(res.status, 2);
  assert.equal(res.stdout, "");
  assert.match(res.stderr, /No product matches/);
});

test("desk prints one JSON object with the product, the owner, index, status and pending decisions, and the database errors", (t) => {
  const h = testHome([{ id: "demo", name: "Demo", roles: ["pm"] }]);
  const elsewhere = mkdtempSync(join(tmpdir(), "agentsi-desk-flag-"));
  t.after(() => (h.cleanup(), rmSync(elsewhere, { recursive: true, force: true })));
  for (const res of [desk(h.env, h.repo), desk(h.env, elsewhere, "--product", "demo")]) {
    // The test database URL points at a closed port, so the index and the status fail and say why.
    assert.equal(res.status, 1, res.stderr);
    const out = JSON.parse(res.stdout);
    assert.deepEqual(Object.keys(out), ["product", "name", "scope", "roles", "paths", "owner", "index", "status", "pending_decisions", "decisions_error"]);
    assert.equal(out.product, "demo");
    assert.deepEqual(out.owner, { name: "the owner", language: "English" });
    assert.equal(out.index.ok, false);
    assert.match(out.index.error, /ECONNREFUSED/);
    assert.equal(out.status.ok, false);
    assert.match(out.status.error, /ECONNREFUSED/);
    assert.deepEqual(out.pending_decisions, []);
  }
});
