import assert from "node:assert/strict";
import { test } from "node:test";
import { roleColorVar, roleMeta, sortRoles } from "./roles";

test("a specialist uses its role's color and a focus label", () => {
  assert.equal(roleMeta("dev.backend").label, "Dev · backend");
  assert.equal(roleMeta("dev.backend").initials, "DV");
  assert.equal(roleColorVar("dev.backend"), "var(--role-dev)");
  assert.equal(roleMeta("dev").label, "Dev");
});

test("a custom role keeps the fallback", () => {
  assert.equal(roleMeta("seo").label, "SEO");
  assert.equal(roleColorVar("seo"), "var(--muted-foreground)");
  assert.equal(roleColorVar("seo.links"), "var(--muted-foreground)");
});

test("specialists sort next to their role", () => {
  assert.deepEqual(sortRoles(["qa", "dev.mobile", "pm", "seo", "dev.backend"]), ["pm", "dev.mobile", "dev.backend", "qa", "seo"]);
});
