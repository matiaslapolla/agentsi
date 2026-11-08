// Inbox state is derived from what the owner already does (meetings, marks, decisions, calls); nothing here is stored.

export type MarkValue = "accepted" | "changed" | "rejected";
export type DecisionStatus = "pending" | "approved" | "rejected";

export interface InboxMinutes {
  id: string;
  product: string;
  question?: string;
  createdAt?: string;
  valid: boolean;
  priorities: { mark: MarkValue | null; isNew: boolean; owner?: string }[];
  decisions: { status: DecisionStatus; decidedAt?: string | null }[];
  hasCall: boolean;
  heard: boolean;
}

export interface InboxInput {
  products: { id: string; name: string; status: "active" | "planned" }[];
  minutes: InboxMinutes[];
  now: Date;
}

export const NEGLECT_DAYS = 14;
const FRESH_DAYS = 7;

export function meetingProgress(m: { priorities: { mark: MarkValue | null }[]; decisions: { status: DecisionStatus }[] }) {
  const marked = m.priorities.filter((p) => p.mark).length;
  const decided = m.decisions.filter((d) => d.status !== "pending").length;
  const total = m.priorities.length + m.decisions.length;
  return {
    marked,
    priorities: m.priorities.length,
    decided,
    decisions: m.decisions.length,
    cleared: total > 0 && marked + decided === total,
  };
}

export type Health = "planned" | "never" | "neglected" | "waiting" | "fresh" | "idle";

export type HealthIcon = "Clock" | "TriangleAlert" | "CircleCheck" | "Circle" | "CircleDashed" | "CalendarClock";

export interface HealthStyle {
  label: string;
  /** Literal class so Tailwind finds it. */
  textClass: string;
  icon: HealthIcon;
}

/** `days` is the age of the last meeting; only `neglected` and `idle` print it. */
const HEALTH_STYLE: Record<Health, { label: (days: number | null) => string; textClass: string; icon: HealthIcon }> = {
  waiting: { label: () => "te espera", textClass: "text-status-waiting", icon: "Clock" },
  neglected: { label: (d) => `${d ?? 0} días sin reunión`, textClass: "text-status-neglected", icon: "TriangleAlert" },
  fresh: { label: () => "al día", textClass: "text-status-fresh", icon: "CircleCheck" },
  idle: { label: (d) => `${d ?? 0} días sin reunión`, textClass: "text-status-idle", icon: "Circle" },
  never: { label: () => "sin reuniones", textClass: "text-status-idle", icon: "CircleDashed" },
  planned: { label: () => "planeado", textClass: "text-status-idle", icon: "CalendarClock" },
};

export function healthStyle(health: Health, days: number | null = null): HealthStyle {
  const { label, ...style } = HEALTH_STYLE[health];
  return { label: label(days), ...style };
}

export interface Quest {
  kind: "close" | "listen" | "meet";
  product: string;
  minutesId?: string;
  marks: number;
  decisions: number;
}

/** The page of a quest: its minutes, its call, or the product page for a new meeting. */
export function questHref(q: Pick<Quest, "kind" | "product" | "minutesId">): string {
  if (q.kind === "listen") return `/p/${q.product}/m/${q.minutesId}/call`;
  if (q.kind === "meet") return `/p/${q.product}`;
  return `/p/${q.product}/m/${q.minutesId}`;
}

export interface ProductQuest {
  id: string;
  name: string;
  health: Health;
  meetings: number;
  daysSinceMeeting: number | null;
  unmarked: number;
  pendingDecisions: number;
  unheard: number;
  marked: number;
  resolvedDecisions: number;
  heard: number;
  quests: Quest[];
  /** Stories of the product that are not archived; set by the data layer, not by `computeInbox`. */
  stories?: number;
}

export interface InboxState {
  products: ProductQuest[];
}

function ageInDays(iso: string | undefined, now: Date): number | null {
  if (!iso) return null;
  const t = Date.parse(iso);
  return Number.isNaN(t) ? null : Math.max(0, Math.floor((now.getTime() - t) / 86_400_000));
}

function productQuest(p: InboxInput["products"][number], ms: InboxMinutes[], now: Date): ProductQuest {
  const sorted = [...ms].sort((a, b) => (b.createdAt ?? b.id).localeCompare(a.createdAt ?? a.id));
  const daysSinceMeeting = sorted.length ? ageInDays(sorted[0].createdAt, now) : null;
  const quests: Quest[] = [];
  let unmarked = 0;
  let pendingDecisions = 0;
  let unheard = 0;
  let marked = 0;
  let resolvedDecisions = 0;
  let heard = 0;
  for (const m of sorted) {
    const prog = meetingProgress(m);
    marked += prog.marked;
    resolvedDecisions += prog.decided;
    if (m.hasCall && m.heard) heard += 1;
    const marks = prog.priorities - prog.marked;
    const decisions = prog.decisions - prog.decided;
    unmarked += marks;
    pendingDecisions += decisions;
    if (marks + decisions > 0) {
      quests.push({ kind: "close", product: p.id, minutesId: m.id, marks, decisions });
    }
    if (m.hasCall && !m.heard) {
      unheard += 1;
      quests.push({ kind: "listen", product: p.id, minutesId: m.id, marks: 0, decisions: 0 });
    }
  }
  const planned = p.status === "planned";
  const neglected = daysSinceMeeting === null || daysSinceMeeting >= NEGLECT_DAYS;
  if (!planned && neglected) quests.push({ kind: "meet", product: p.id, marks: 0, decisions: 0 });
  const health: Health = planned
    ? "planned"
    : daysSinceMeeting === null
      ? "never"
      : waitingItems({ unmarked, pendingDecisions, unheard }) > 0
        ? "waiting"
        : neglected
          ? "neglected"
          : daysSinceMeeting < FRESH_DAYS
            ? "fresh"
            : "idle";
  return { id: p.id, name: p.name, health, meetings: ms.length, daysSinceMeeting, unmarked, pendingDecisions, unheard, marked, resolvedDecisions, heard, quests };
}

const QUEST_ORDER: Record<Quest["kind"], number> = { close: 0, listen: 1, meet: 2 };

/** Next actions across products: open meetings first, then calls, then the most neglected products. */
export function questBoard(products: ProductQuest[]): Quest[] {
  const age = new Map(products.map((p) => [p.id, p.daysSinceMeeting ?? Infinity]));
  return products
    .flatMap((p) => p.quests)
    .sort((a, b) => QUEST_ORDER[a.kind] - QUEST_ORDER[b.kind] || (age.get(b.product) ?? 0) - (age.get(a.product) ?? 0));
}

/** What the home shows under "Te toca". A "meet" quest is a hint on the product card, not a task. */
export function toDoQuests(products: ProductQuest[]): Quest[] {
  return questBoard(products).filter((q) => q.kind !== "meet");
}

/** Items that wait for the owner in one product: priorities to mark, decisions to resolve and calls to hear. */
export function waitingItems(p: Pick<ProductQuest, "unmarked" | "pendingDecisions" | "unheard">): number {
  return p.unmarked + p.pendingDecisions + p.unheard;
}

export function computeInbox(input: InboxInput): InboxState {
  const { minutes, now } = input;
  return { products: input.products.map((p) => productQuest(p, minutes.filter((m) => m.product === p.id), now)) };
}
