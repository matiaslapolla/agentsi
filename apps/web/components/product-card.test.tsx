import assert from "node:assert/strict";
import { test } from "node:test";
import { renderToStaticMarkup } from "react-dom/server";
import { ProductCard } from "./product-card";
import { computeInbox, type InboxMinutes, type ProductQuest } from "../lib/inbox";

// S011: the product card on the home page groups its indicators into a "pending" group and a "done" group.
// The tests render the real ProductCard with quests that the real computeInbox builds from minutes, then read
// the HTML the way the owner sees it: which labelled group holds which numbers.
// The story does not fix the group wording, so a label may be the English word or the Spanish UI word.
// Each indicator gets its own number so a count can be told apart from the others.

const NOW = new Date("2026-10-24T12:00:00Z");
const PRODUCT = { id: "a", name: "Producto A", status: "active" as const };
const PENDING_WORDS = ["pending", "pendiente", "pendientes"];
const DONE_WORDS = ["done", "hecho", "hechos", "hecha", "hechas"];

function meeting(id: string, over: Partial<InboxMinutes> = {}): InboxMinutes {
  return {
    id,
    product: PRODUCT.id,
    createdAt: "2026-10-23T10:00:00Z",
    valid: true,
    priorities: [],
    decisions: [],
    hasCall: false,
    heard: false,
    ...over,
  };
}

function questFrom(minutes: InboxMinutes[]): ProductQuest {
  const [quest] = computeInbox({ products: [PRODUCT], minutes, now: NOW }).products;
  return quest;
}

// A minimal reader for the HTML that the card renders. No DOM library is installed in apps/web.
interface El {
  tag: string;
  attrs: Record<string, string>;
  kids: (El | string)[];
  parent: El | null;
}

const VOID = new Set(["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"]);

function decode(s: string): string {
  return s.replace(/&quot;/g, '"').replace(/&#x27;/g, "'").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");
}

function parse(html: string): El {
  const root: El = { tag: "#root", attrs: {}, kids: [], parent: null };
  let cur = root;
  const token = /<(\/?)([a-zA-Z][\w-]*)((?:\s+[^\s=/>]+(?:="[^"]*")?)*)\s*(\/?)>|([^<]+)/g;
  for (const m of html.matchAll(token)) {
    if (m[5] !== undefined) {
      cur.kids.push(decode(m[5]));
      continue;
    }
    const tag = m[2].toLowerCase();
    if (m[1]) {
      let n: El | null = cur;
      while (n && n.tag !== tag) n = n.parent;
      if (n?.parent) cur = n.parent;
      continue;
    }
    const attrs: Record<string, string> = {};
    for (const a of m[3].matchAll(/([^\s=]+)(?:="([^"]*)")?/g)) attrs[a[1]] = decode(a[2] ?? "");
    const el: El = { tag, attrs, kids: [], parent: cur };
    cur.kids.push(el);
    if (!m[4] && !VOID.has(tag)) cur = el;
  }
  return root;
}

function norm(s: string): string {
  return s.replace(/\s+/g, " ").trim();
}

function text(el: El): string {
  return norm(el.kids.map((k) => (typeof k === "string" ? k : text(k))).join(""));
}

function all(el: El): El[] {
  return el.kids.flatMap((k) => (typeof k === "string" ? [] : [k, ...all(k)]));
}

// A count is a number the owner reads on the card, e.g. the 3 next to the unmarked priorities.
function hasCount(el: El, n: number): boolean {
  return [el, ...all(el)].some((x) => text(x) === String(n));
}

// A group label is a word the owner reads, or a name given to the group (aria-label, title).
function isLabel(el: El, words: string[]): boolean {
  const named = [el.attrs["aria-label"], el.attrs.title].some((v) => v !== undefined && words.includes(norm(v).toLowerCase()));
  if (named) return true;
  const t = text(el).toLowerCase();
  return words.includes(t) && !el.kids.some((k) => typeof k !== "string" && text(k).toLowerCase() === t);
}

// The group labelled with `words` is the nearest container of the label that holds at least one of `counts`.
function groupFor(card: El, words: string[], counts: number[]): El | null {
  for (const label of all(card).filter((el) => isLabel(el, words))) {
    for (let a: El | null = label; a && a !== card.parent; a = a.parent) {
      if (counts.some((n) => hasCount(a, n))) return a;
    }
  }
  return null;
}

function hasLabel(el: El, words: string[]): boolean {
  return [el, ...all(el)].some((x) => isLabel(x, words));
}

function renderCard(quest?: ProductQuest): El {
  const html = renderToStaticMarkup(<ProductCard product={PRODUCT} note={null} quest={quest} />);
  return parse(html).kids[0] as El;
}

test("S011 1: unmarked priorities, pending decisions and unheard calls sit together in a pending group, each with its count", () => {
  const quest = questFrom([
    meeting("m1", {
      priorities: [
        { mark: null, isNew: false },
        { mark: null, isNew: false },
        { mark: null, isNew: false },
      ],
      decisions: [{ status: "pending" }, { status: "pending" }],
    }),
    meeting("m2", { hasCall: true, heard: false }),
  ]);
  // Fixture check: the inbox sees 3 unmarked, 2 pending decisions and 1 unheard call.
  assert.deepEqual([quest.unmarked, quest.pendingDecisions, quest.unheard], [3, 2, 1]);

  const card = renderCard(quest);
  const pending = groupFor(card, PENDING_WORDS, [3, 2, 1]);
  assert.ok(pending, "the card has no group labelled pending");
  assert.ok(hasLabel(pending, PENDING_WORDS), "the group does not carry the label pending");
  for (const n of [3, 2, 1]) {
    assert.ok(hasCount(pending, n), `the pending group does not show the count ${n}`);
  }
});

test("S011 2: marked priorities, resolved decisions and heard calls sit together in a done group, apart from the pending group", () => {
  const quest = questFrom([
    meeting("m1", {
      priorities: [
        { mark: "accepted", isNew: false },
        { mark: "accepted", isNew: false },
        { mark: "rejected", isNew: false },
        { mark: "changed", isNew: false },
      ],
      decisions: [{ status: "approved" }, { status: "rejected" }],
    }),
    meeting("m2", { hasCall: true, heard: true }),
  ]);
  // Fixture check: nothing is pending, so the card has no pending counts to show.
  assert.deepEqual([quest.unmarked, quest.pendingDecisions, quest.unheard], [0, 0, 0]);

  const card = renderCard(quest);
  const done = groupFor(card, DONE_WORDS, [4, 2, 1]);
  assert.ok(done, "the card has no group labelled done");
  assert.ok(hasLabel(done, DONE_WORDS), "the group does not carry the label done");
  for (const n of [4, 2, 1]) {
    assert.ok(hasCount(done, n), `the done group does not show the count ${n}`);
  }

  const pending = groupFor(card, PENDING_WORDS, [4, 2, 1]);
  if (pending) {
    assert.notEqual(pending, done, "the done counts are inside the pending group");
    for (const n of [4, 2, 1]) {
      assert.ok(!hasCount(pending, n), `the count ${n} of the done items is inside the pending group`);
    }
  }
});

test("S011 3: with nothing pending, the card shows no pending group and does not look like something waits", () => {
  const quest = questFrom([
    meeting("m1", {
      priorities: [{ mark: "accepted", isNew: false }],
      decisions: [{ status: "approved" }],
      hasCall: true,
      heard: true,
    }),
  ]);
  // Fixture check: every pending count is zero.
  assert.deepEqual([quest.unmarked, quest.pendingDecisions, quest.unheard], [0, 0, 0]);

  const card = renderCard(quest);
  assert.equal(hasLabel(card, PENDING_WORDS), false, "the card shows a pending group");

  const waiting = /pendient|pending|sin marcar|sin escuchar/i;
  assert.doesNotMatch(text(card), waiting, "the card reads as if something waits for the owner");
  assert.equal(
    all(card).some((el) => Object.values(el.attrs).some((v) => waiting.test(v))),
    false,
    "an attribute of the card names something that waits for the owner",
  );
  assert.match(text(card), /Producto A/, "the card does not name the product");
});

test("S011 4: a product without meetings shows neither group and still links to the product page", () => {
  const card = renderCard(undefined);

  assert.equal(card.tag, "a", "the card is not a link");
  assert.equal(card.attrs.href, `/p/${PRODUCT.id}`, "the card does not link to the product page");
  assert.equal(hasLabel(card, PENDING_WORDS), false, "the card shows a pending group");
  assert.equal(hasLabel(card, DONE_WORDS), false, "the card shows a done group");
  assert.match(text(card), /Producto A/, "the card does not name the product");
});
