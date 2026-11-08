import assert from "node:assert/strict";
import { test } from "node:test";
import { computeInbox, healthStyle, questBoard, questHref, toDoQuests, waitingItems, type Health, type InboxInput, type InboxMinutes } from "./inbox";

const NOW = new Date("2026-10-12T15:00:00Z");

function minutes(over: Partial<InboxMinutes> = {}): InboxMinutes {
  return {
    id: "m1",
    product: "a",
    createdAt: "2026-10-11T10:00:00Z",
    valid: true,
    priorities: [],
    decisions: [],
    hasCall: false,
    heard: false,
    ...over,
  };
}

function input(ms: InboxMinutes[], over: Partial<InboxInput> = {}): InboxInput {
  return {
    products: [
      { id: "a", name: "A", status: "active" },
      { id: "b", name: "B", status: "active" },
      { id: "c", name: "C", status: "planned" },
    ],
    minutes: ms,
    now: NOW,
    ...over,
  };
}

test("quest board puts open meetings before calls and meetings to hold, and flags neglected products", () => {
  const g = computeInbox(
    input([
      minutes({ priorities: [{ mark: null, isNew: false }, { mark: "accepted", isNew: false }], decisions: [{ status: "pending" }], hasCall: true }),
    ]),
  );
  const board = questBoard(g.products);
  assert.deepEqual(board.map((q) => `${q.kind}:${q.product}`), [
    "close:a",
    "listen:a",
    "meet:b",
  ]);
  assert.equal(g.products.find((p) => p.id === "a")?.health, "waiting");
  assert.equal(g.products.find((p) => p.id === "b")?.health, "never");
  assert.equal(g.products.find((p) => p.id === "c")?.health, "planned");
});

test("healthStyle gives a label, a text class and an icon for every Health value", () => {
  const all: Record<Health, string> = {
    waiting: "te espera",
    neglected: "20 días sin reunión",
    fresh: "al día",
    idle: "20 días sin reunión",
    never: "sin reuniones",
    planned: "planeado",
  };
  for (const [health, label] of Object.entries(all) as [Health, string][]) {
    const s = healthStyle(health, 20);
    assert.equal(s.label, label);
    assert.match(s.textClass, /^text-status-(waiting|neglected|fresh|idle)$/);
    assert.ok(s.icon);
  }
});

test("toDoQuests drops the meet quests and waitingItems sums the open items", () => {
  const g = computeInbox(
    input([minutes({ priorities: [{ mark: null, isNew: false }], decisions: [{ status: "pending" }], hasCall: true })]),
  );
  assert.ok(questBoard(g.products).some((q) => q.kind === "meet"));
  assert.deepEqual(toDoQuests(g.products).map((q) => q.kind), ["close", "listen"]);
  assert.equal(waitingItems(g.products[0]), 3);
});

test("questHref points each quest kind to its page", () => {
  assert.equal(questHref({ kind: "close", product: "a", minutesId: "m1" }), "/p/a/m/m1");
  assert.equal(questHref({ kind: "listen", product: "a", minutesId: "m1" }), "/p/a/m/m1/call");
  assert.equal(questHref({ kind: "meet", product: "a" }), "/p/a");
});
