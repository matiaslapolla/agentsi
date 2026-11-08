import {
  listStories,
  loadStory,
  STORY_ID,
  type Story,
  type StoryApproval,
  type StoryResultKind,
  type StorySession,
  type StorySessionState,
  type StoryStatus,
} from "@agentsi/core/story";
import { ROOT } from "./env";

export type { Criterion, Story, StoryAction, StoryResult, StorySession, StoryStatus } from "@agentsi/core/story";

export type Tone = "neutral" | "ok" | "bad" | "warn" | "accent";

export interface StoryList {
  stories: Story[];
  /** Set when the stories cannot be read from the database; the page shows it instead of crashing. */
  error: string | null;
}

/** Archived stories are left out unless `includeArchived` is set; then they carry `archived: true`. */
export async function readStories(product: string, root = ROOT, opts: { includeArchived?: boolean } = {}): Promise<StoryList> {
  try {
    return { stories: await listStories(product, root, opts), error: null };
  } catch (e) {
    return { stories: [], error: (e as Error).message };
  }
}

export async function readStory(product: string, id: string, root = ROOT): Promise<Story | null> {
  if (!STORY_ID.test(id)) return null;
  try {
    return await loadStory(product, id, root);
  } catch {
    return null;
  }
}

export const STATUS_GROUPS: { status: StoryStatus; label: string }[] = [
  { status: "not_implemented", label: "Sin implementar" },
  { status: "implemented", label: "Implementadas" },
  { status: "discarded", label: "Descartadas" },
];

export function groupStories(stories: Story[]): { status: StoryStatus; label: string; stories: Story[] }[] {
  return STATUS_GROUPS.map((g) => ({ ...g, stories: stories.filter((s) => s.status === g.status) }));
}

export const STATUS_LABEL: Record<StoryStatus, { label: string; tone: Tone }> = {
  not_implemented: { label: "sin implementar", tone: "neutral" },
  implemented: { label: "implementada", tone: "ok" },
  discarded: { label: "descartada", tone: "bad" },
};

export const APPROVAL_LABEL: Record<StoryApproval, { label: string; tone: Tone }> = {
  pending: { label: "espera tu aprobación", tone: "warn" },
  approved: { label: "aprobada", tone: "ok" },
  changes_requested: { label: "con cambios pedidos", tone: "warn" },
};

export const SESSION_LABEL: Record<StorySessionState, { label: string; tone: Tone }> = {
  queued: { label: "en cola", tone: "neutral" },
  tests: { label: "QA escribe los tests", tone: "accent" },
  work: { label: "el owner implementa", tone: "accent" },
  checks: { label: "corren los checks", tone: "accent" },
  ready: { label: "lista para revisar", tone: "ok" },
  failed: { label: "falló", tone: "bad" },
};

export const RESULT_LABEL: Record<StoryResultKind, { label: string; tone: Tone }> = {
  merged_unchanged: { label: "mergeada sin cambios", tone: "ok" },
  merged_changed: { label: "mergeada con cambios tuyos", tone: "ok" },
  discarded: { label: "descartada", tone: "bad" },
};

export type StepState = "done" | "current" | "todo" | "failed";

export interface SessionStep {
  state: StorySessionState;
  label: string;
  status: StepState;
}

const STEPS: { state: StorySessionState; label: string }[] = [
  { state: "queued", label: "En cola" },
  { state: "tests", label: "Tests de QA" },
  { state: "work", label: "Trabajo del owner" },
  { state: "checks", label: "Checks" },
  { state: "ready", label: "Lista" },
];

/**
 * The runner moves through queued, tests, work and checks in order. `failed` does not say
 * at which step it stopped: a tests commit marks the tests done, an owner attempt marks the work done.
 */
export function sessionSteps(session: StorySession): SessionStep[] {
  if (session.state !== "failed") {
    const at = STEPS.findIndex((s) => s.state === session.state);
    return STEPS.map((s, i) => ({
      ...s,
      status: i < at || session.state === "ready" ? "done" : i === at ? "current" : "todo",
    }));
  }
  const reached = 1 + (session.tests_commit ? 1 : 0) + (session.attempts > 0 ? 1 : 0);
  return [
    ...STEPS.slice(0, 4).map((s, i): SessionStep => ({ ...s, status: i < reached ? "done" : i === reached ? "failed" : "todo" })),
    { state: "failed", label: "Falló", status: "failed" },
  ];
}

/** "Como <rol>, quiero …, para …"; a task has no `as_a`. */
export function storySentence(s: Pick<Story, "as_a" | "i_want" | "so_that">): { lead: string; text: string }[] {
  return [
    ...(s.as_a ? [{ lead: "Como", text: s.as_a }] : []),
    { lead: s.as_a ? "quiero" : "Quiero", text: s.i_want },
    { lead: "para", text: s.so_that },
  ];
}

export function shortDate(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime())
    ? iso
    : d.toLocaleString("es-AR", { day: "2-digit", month: "short", year: "numeric", hour: "2-digit", minute: "2-digit" });
}

/** The PM rewrite needs a model call, so the web records the note and names this command. */
export const changesCommand = (id: string, note: string) => `agentsi story changes ${id} "${note.replace(/(["\\$`])/g, "\\$1")}"`;
