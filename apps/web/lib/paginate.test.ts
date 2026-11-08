import assert from "node:assert/strict";
import test from "node:test";
import { paginate } from "./paginate";

const list = Array.from({ length: 23 }, (_, i) => i + 1);

test("slices the requested page and reports its range", () => {
  const p = paginate(list, "2", 10);
  assert.deepEqual(p.items, [11, 12, 13, 14, 15, 16, 17, 18, 19, 20]);
  assert.deepEqual([p.page, p.pages, p.from, p.to, p.total], [2, 3, 11, 20, 23]);
  assert.deepEqual(paginate(list, "3", 10).items, [21, 22, 23]);
});

test("clamps malformed and out-of-range pages", () => {
  assert.equal(paginate(list, undefined, 10).page, 1);
  assert.equal(paginate(list, "abc", 10).page, 1);
  assert.equal(paginate(list, "-4", 10).page, 1);
  assert.equal(paginate(list, "99", 10).page, 3);
  assert.equal(paginate(list, ["2", "3"], 10).page, 2);
});

test("an empty list is one empty page", () => {
  assert.deepEqual(paginate([], "5", 10), { items: [], page: 1, pages: 1, total: 0, from: 0, to: 0 });
});
