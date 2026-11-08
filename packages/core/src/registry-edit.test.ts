import assert from "node:assert/strict";
import { test } from "node:test";
import { addProduct, updateProduct } from "./registry-edit.ts";

const TEXT = `{
  "products": [
    { "id": "a", "name": "A", "paths": ["/p/a"], "status": "active", "roles": ["pm", "dev"] },
    { "id": "b", "name": "B", "paths": ["/p/b"], "status": "active", "roles": ["pm"] }
  ]
}
`;

test("updateProduct changes only the roles of one line", () => {
  const out = updateProduct(TEXT, "a", { roles: ["pm", "dev.backend", "dev.mobile"] });
  assert.equal(out, TEXT.replace('["pm", "dev"]', '["pm", "dev.backend", "dev.mobile"]'));
  assert.throws(() => updateProduct(TEXT, "zzz", { roles: ["pm"] }), /Unknown product/);
});

test("addProduct appends one line in the registry format", () => {
  const out = addProduct(TEXT, { id: "c", name: "C", paths: ["/p/c"], status: "active", roles: ["pm", "qa"] });
  const lines = out.split("\n");
  assert.equal(lines[3], '    { "id": "b", "name": "B", "paths": ["/p/b"], "status": "active", "roles": ["pm"] },');
  assert.equal(lines[4], '    { "id": "c", "name": "C", "paths": ["/p/c"], "status": "active", "roles": ["pm", "qa"] }');
  assert.deepEqual(lines.slice(0, 3), TEXT.split("\n").slice(0, 3));
  assert.throws(() => addProduct(TEXT, { id: "a", name: "A", paths: [], status: "active", roles: [] }), /already registered/);
});
