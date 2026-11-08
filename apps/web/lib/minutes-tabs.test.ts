import assert from "node:assert/strict";
import { test } from "node:test";
import { MINUTES_TABS, tabForHash } from "./minutes-tabs";

test("tabForHash selects the tab each hash names", () => {
  for (const tab of MINUTES_TABS) assert.equal(tabForHash(`#${tab}`, MINUTES_TABS), tab);
  assert.equal(tabForHash("priorities", MINUTES_TABS), "priorities");
});

test("tabForHash maps item anchors to the tab that holds them", () => {
  assert.equal(tabForHash("#p-0", MINUTES_TABS), "priorities");
  assert.equal(tabForHash("#p-12", MINUTES_TABS), "priorities");
  assert.equal(tabForHash("#d-3", MINUTES_TABS), "decisiones");
});

test("tabForHash ignores unknown hashes and hidden tabs", () => {
  for (const h of ["", "#", "#minutes", "#x-1", "#p-", "#p-a", "#Priorities", "#%E0"]) assert.equal(tabForHash(h, MINUTES_TABS), null, h);
  const shown = ["priorities", "decisiones", "uso"] as const;
  assert.equal(tabForHash("#disagreements", shown), null);
  assert.equal(tabForHash("#roles", shown), null);
  assert.equal(tabForHash("#decisiones", shown), "decisiones");
});
