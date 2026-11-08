import assert from "node:assert/strict";
import { test } from "node:test";
import { navState } from "./nav";

const products = [{ id: "agentsi" }, { id: "beta" }];

test("navState marks the item for each route", () => {
  const cases: [string, ReturnType<typeof navState>][] = [
    ["/", { item: "agency", product: null }],
    ["/p/agentsi", { item: "product", product: "agentsi" }],
    ["/p/agentsi/roles", { item: "roles", product: "agentsi" }],
    ["/p/agentsi/memory", { item: "memory", product: "agentsi" }],
    ["/p/agentsi/stories", { item: "stories", product: "agentsi" }],
    ["/p/agentsi/stories/S001", { item: "stories", product: "agentsi" }],
    ["/p/agentsi/m/2026-10-18-0146-x", { item: "minutes", product: "agentsi" }],
    ["/p/beta/m/2026-10-18-0146-x/call", { item: "minutes", product: "beta" }],
    ["/guia", { item: "guide", product: null }],
    ["/como-funciona", { item: "how", product: null }],
  ];
  for (const [path, want] of cases) assert.deepEqual(navState(path, products), want, path);
});

test("navState marks nothing for unknown routes and products", () => {
  assert.deepEqual(navState("/p/nope", products), { item: null, product: null });
  assert.deepEqual(navState("/otra", products), { item: null, product: null });
});
