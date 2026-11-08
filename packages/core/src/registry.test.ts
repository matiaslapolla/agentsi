import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { DEFAULT_OWNER, detectProduct, loadOwner, ownerOf } from "./registry.ts";

const REG = {
  products: [
    { id: "miapp", name: "MiApp", paths: ["/p/miapp"], status: "active" as const, roles: ["pm"] },
    { id: "nested", name: "Nested", paths: ["/p/miapp/packages/x"], status: "active" as const, roles: ["pm"] },
  ],
};

test("detects a product from a nested cwd", () => {
  const d = detectProduct("/p/miapp/app/deep", REG);
  assert.equal(d?.product.id, "miapp");
  assert.equal(d?.scope, null);
});

test("longest path prefix wins and respects segment boundaries", () => {
  assert.equal(detectProduct("/p/miapp/packages/x/src", REG)?.product.id, "nested");
  assert.equal(detectProduct("/p/miapp-other", REG), null);
});

test("returns null when nothing matches", () => {
  assert.equal(detectProduct("/tmp/elsewhere", REG), null);
});

test("ownerOf reads a valid owner block and trims it", () => {
  assert.deepEqual(ownerOf({ owner: { name: " Ana ", language: "Rioplatense Spanish" } }), { name: "Ana", language: "Rioplatense Spanish" });
});

test("ownerOf gives the defaults with no block, and per missing key", () => {
  assert.deepEqual(ownerOf({}), { name: "the owner", language: "English" });
  assert.deepEqual(ownerOf({ owner: { name: "Ana" } }), { name: "Ana", language: "English" });
});

test("ownerOf rejects a block that is not an object and a key that is not a non-empty string", () => {
  assert.throws(() => ownerOf({ owner: "Ana" as never }), /owner block .* must be an object/);
  assert.throws(() => ownerOf({ owner: [] as never }), /owner block .* must be an object/);
  assert.throws(() => ownerOf({ owner: { name: "" } }), /owner\.name .* non-empty string/);
  assert.throws(() => ownerOf({ owner: { language: 3 as never } }), /owner\.language .* non-empty string/);
});

test("loadOwner reads the registry of a root, and gives the defaults when the root has none", () => {
  const root = mkdtempSync(join(tmpdir(), "agentsi-owner-"));
  try {
    assert.deepEqual(loadOwner(root), DEFAULT_OWNER);
    writeFileSync(join(root, "agentsi.config.json"), JSON.stringify({ owner: { name: "Ana", language: "Portuguese" }, products: [] }));
    assert.deepEqual(loadOwner(root), { name: "Ana", language: "Portuguese" });
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
