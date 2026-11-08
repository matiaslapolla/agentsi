import { archivedStoryIds, changeStoryArchive } from "./archive.ts";
import type { CommitResult } from "./exec.ts";
import { getPool } from "./db.ts";
import { dataRoot } from "./paths.ts";
import type { Product } from "./registry.ts";
import type { Source } from "./types.ts";

export type StoryType = "story" | "task";
export type StoryStatus = "not_implemented" | "implemented" | "discarded";
export type StoryApproval = "pending" | "approved" | "changes_requested";
export type StorySessionState = "queued" | "tests" | "work" | "checks" | "ready" | "failed";
export type StoryResultKind = "merged_unchanged" | "merged_changed" | "discarded";

export interface Criterion {
  given: string;
  when: string;
  then: string;
  /** Repo-relative path of the test file; null only in a task. */
  test: string | null;
  /** One plain sentence of what the test proves, for the web page. */
  checks: string;
}

export interface StorySession {
  state: StorySessionState;
  /** `story/<id>-<slug>` */
  branch: string;
  /** `<repo>/.worktrees/<id>-<slug>` */
  worktree: string;
  base: string;
  tests_commit: string | null;
  final_commit: string | null;
  attempts: number;
  failure: string | null;
  log: string;
  started_at: string;
  /** The process that runs the session; absent until it starts. */
  pid?: number | null;
  /** Set when Claude Code stopped a session at its cap; absent otherwise. */
  stop?: StoryStop | null;
}

export interface StoryStop {
  reason: "budget";
  cap_usd: number;
}

export const stopText = (stop: StoryStop): string => `Stopped: ${stop.reason}, cap USD ${stop.cap_usd.toFixed(2)} for each session.`;

export interface StoryResult {
  result: StoryResultKind;
  at: string;
  commit: string | null;
}

export interface Story {
  /** `S014` for a story, `T015` for a task; one counter per product. */
  id: string;
  type: StoryType;
  slug: string;
  /** Short, in the owner's language; the call script names the story by it, never by the id. */
  title: string;
  as_a: string | null;
  i_want: string;
  so_that: string;
  criteria: Criterion[];
  owner_role: string;
  objective: string | null;
  sources: Source[];
  status: StoryStatus;
  approval: StoryApproval;
  changes_note: string | null;
  proposed_by: "owner" | "pm";
  minutes: string | null;
  session: StorySession | null;
  /** Newest last. */
  results: StoryResult[];
  created_at: string;
  updated_at: string;
  /** Set by `listStories` on a story in the archive record; never written to the story file. */
  archived?: boolean;
}

const TYPES: StoryType[] = ["story", "task"];
const STATUSES: StoryStatus[] = ["not_implemented", "implemented", "discarded"];
const APPROVALS: StoryApproval[] = ["pending", "approved", "changes_requested"];
const SESSION_STATES: StorySessionState[] = ["queued", "tests", "work", "checks", "ready", "failed"];
const RESULT_KINDS: StoryResultKind[] = ["merged_unchanged", "merged_changed", "discarded"];
const SOURCE_TYPES: Record<Source["type"], true> = { commit: true, issue: true, path: true, url: true, story: true };
const PREFIX: Record<StoryType, string> = { story: "S", task: "T" };

export const STORY_ID = /^[ST]\d{3,}$/;
const SLUG = /^[a-z0-9]+(-[a-z0-9]+)*$/;

const text = (v: unknown): boolean => typeof v === "string" && v.trim().length > 0;
const textOrNull = (v: unknown): boolean => v === null || typeof v === "string";
const isTime = (v: unknown): boolean => typeof v === "string" && !Number.isNaN(Date.parse(v));

/** A repo-relative path: no leading slash, no backslash, no `.` or `..` segment. */
export function isRepoRelative(path: string): boolean {
  if (!path.trim() || path.startsWith("/") || path.includes("\\") || /^[A-Za-z]:/.test(path)) return false;
  return path.split("/").every((seg) => seg !== "" && seg !== "." && seg !== "..");
}

function sessionFailures(s: StorySession): string[] {
  const failures: string[] = [];
  if (!SESSION_STATES.includes(s.state)) failures.push(`session.state "${s.state}" is not one of ${SESSION_STATES.join(", ")}.`);
  for (const key of ["branch", "worktree", "base", "log"] as const) {
    if (!text(s[key])) failures.push(`session.${key} is empty.`);
  }
  for (const key of ["tests_commit", "final_commit", "failure"] as const) {
    if (!textOrNull(s[key])) failures.push(`session.${key} must be a string or null.`);
  }
  if (!Number.isInteger(s.attempts) || s.attempts < 0) failures.push(`session.attempts must be a whole number of 0 or more.`);
  if (s.pid !== undefined && s.pid !== null && !(Number.isInteger(s.pid) && s.pid > 0)) failures.push("session.pid must be a process id or null.");
  if (!isTime(s.started_at)) failures.push(`session.started_at "${s.started_at}" is not a date.`);
  if (s.stop !== undefined && s.stop !== null && !(s.stop.reason === "budget" && typeof s.stop.cap_usd === "number" && s.stop.cap_usd > 0)) {
    failures.push('session.stop must be null or { reason: "budget", cap_usd } with a positive cap.');
  }
  return failures;
}

/** Failures of a story against the roster (agent ids); empty when valid. */
export function validateStory(s: Story, ctx: { roster: string[] }): string[] {
  const failures: string[] = [];
  const isType = TYPES.includes(s.type);
  if (!isType) failures.push(`type "${s.type}" is not one of ${TYPES.join(", ")}.`);
  if (typeof s.id !== "string" || !STORY_ID.test(s.id)) failures.push(`id "${s.id}" is not S or T followed by 3 or more digits.`);
  else if (isType && s.id[0] !== PREFIX[s.type]) failures.push(`id "${s.id}" must start with ${PREFIX[s.type]} for type "${s.type}".`);
  if (typeof s.slug !== "string" || !SLUG.test(s.slug)) failures.push(`slug "${s.slug}" is not kebab-case.`);
  if (!text(s.title)) failures.push("title is empty.");

  if (s.type === "story" && !text(s.as_a)) failures.push(`as_a is empty; a story of type "story" needs a user.`);
  else if (s.as_a !== null && !text(s.as_a)) failures.push("as_a must be a user or null.");
  if (!text(s.i_want)) failures.push("i_want is empty.");
  if (!text(s.so_that)) failures.push("so_that is empty.");

  const criteria = Array.isArray(s.criteria) ? s.criteria : [];
  if (criteria.length < 2 || criteria.length > 5) failures.push(`criteria must hold 2 to 5 items, found ${criteria.length}.`);
  criteria.forEach((c, i) => {
    const at = `criteria[${i + 1}]`;
    for (const key of ["given", "when", "then", "checks"] as const) {
      if (!text(c?.[key])) failures.push(`${at}.${key} is empty.`);
    }
    if (c?.test === null || c?.test === undefined) {
      if (s.type === "story") failures.push(`${at}.test is null; a story of type "story" needs a test path.`);
    } else if (typeof c.test !== "string" || !isRepoRelative(c.test)) {
      failures.push(`${at}.test "${c.test}" is not a repo-relative path.`);
    }
  });

  if (!ctx.roster.includes(s.owner_role)) failures.push(`owner_role "${s.owner_role}" is not in the roster (${ctx.roster.join(", ")}).`);
  if (!textOrNull(s.objective)) failures.push("objective must be an objective id or null.");

  if (!Array.isArray(s.sources)) failures.push("sources must be a list.");
  else {
    for (const src of s.sources) {
      if (!Object.hasOwn(SOURCE_TYPES, src?.type)) failures.push(`Source type "${src?.type}" is not one of ${Object.keys(SOURCE_TYPES).join(", ")}.`);
      if (!text(src?.ref)) failures.push(`A source of type "${src?.type}" has an empty ref.`);
      else if (src.type === "story" && !STORY_ID.test(src.ref)) failures.push(`A story source "${src.ref}" is not a story id.`);
    }
  }

  if (!STATUSES.includes(s.status)) failures.push(`status "${s.status}" is not one of ${STATUSES.join(", ")}.`);
  if (!APPROVALS.includes(s.approval)) failures.push(`approval "${s.approval}" is not one of ${APPROVALS.join(", ")}.`);
  if (s.approval === "changes_requested" && !text(s.changes_note)) failures.push("changes_note is empty; approval changes_requested needs a note.");
  else if (!textOrNull(s.changes_note)) failures.push("changes_note must be a string or null.");
  if (s.proposed_by !== "owner" && s.proposed_by !== "pm") failures.push(`proposed_by "${s.proposed_by}" is not owner or pm.`);
  if (!textOrNull(s.minutes)) failures.push("minutes must be a minutes id or null.");

  if (s.session !== null) {
    if (typeof s.session !== "object" || s.session === undefined) failures.push("session must be an object or null.");
    else failures.push(...sessionFailures(s.session));
  }

  if (!Array.isArray(s.results)) failures.push("results must be a list.");
  else {
    for (const r of s.results) {
      if (!RESULT_KINDS.includes(r?.result)) failures.push(`result "${r?.result}" is not one of ${RESULT_KINDS.join(", ")}.`);
      if (!isTime(r?.at)) failures.push(`A result has an invalid date "${r?.at}".`);
      if (!textOrNull(r?.commit)) failures.push("A result commit must be a string or null.");
    }
  }

  if (!isTime(s.created_at)) failures.push(`created_at "${s.created_at}" is not a date.`);
  if (!isTime(s.updated_at)) failures.push(`updated_at "${s.updated_at}" is not a date.`);
  return failures;
}

/** Kebab-case of the title, without accents, at most 48 characters. */
export function storySlug(title: string): string {
  const slug = title
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  if (slug.length <= 48) return slug || "story";
  const cut = slug.slice(0, 48);
  const dash = cut.lastIndexOf("-");
  return dash > 0 ? cut.slice(0, dash) : cut;
}

/** `<id>-<slug>`: the name of the story branch and worktree. */
export const storyFileBase = (story: Pick<Story, "id" | "slug">): string => `${story.id}-${story.slug}`;

const storyNumber = (id: string): number => Number(id.slice(1));

function readStoryRow(data: unknown): Story {
  const story = (typeof data === "string" ? JSON.parse(data) : data) as Story;
  // Stories written before the owner config name the first owner; the row keeps it until the next save.
  if ((story.proposed_by as string) === "matias") story.proposed_by = "owner";
  return story;
}

/** Every story of the product, oldest id first. Archived stories are left out unless `includeArchived` is set; then they carry `archived: true`. */
export async function listStories(product: string, root = dataRoot, opts: { includeArchived?: boolean } = {}): Promise<Story[]> {
  const archived = await archivedStoryIds(product, root);
  const { rows } = await getPool().query("select data from stories where product = $1", [product]);
  return rows
    .map((r) => readStoryRow(r.data))
    .filter((s) => opts.includeArchived || !archived.has(s.id))
    .map((s) => (archived.has(s.id) ? { ...s, archived: true } : s))
    .sort((a, b) => storyNumber(a.id) - storyNumber(b.id));
}

/** The story by product and id. The data root is only for the archive record; the story itself is in the database. */
export async function loadStory(product: string, id: string, _root = dataRoot): Promise<Story> {
  if (!STORY_ID.test(id)) throw new Error(`Invalid story id "${id}"`);
  const { rows } = await getPool().query("select data from stories where product = $1 and id = $2", [product, id]);
  if (!rows[0]) throw new Error(`Unknown story "${id}" in ${product}`);
  return readStoryRow(rows[0].data);
}

/** True when the story is in the archive record of the product. */
export const isStoryArchived = async (product: string, id: string, root = dataRoot): Promise<boolean> => (await archivedStoryIds(product, root)).has(id);

/** The refusal text of `story run` on an archived story. */
export const archivedStoryError = (id: string): string => `Story ${id} is archived. Run agentsi restore ${id} first.`;

function storyArchive(product: string, id: string, archive: boolean, root: string): Promise<CommitResult> {
  return changeStoryArchive(product, id, archive, root, async () => {
    const s = await loadStory(product, id, root);
    if (archive && isRunningSession(s)) throw new Error(`Story ${id} has a story session running; wait for it to finish before archiving.`);
  });
}

/** Records the story in the archive record. Its files stay on disk. Refused while a session is running. */
export const archiveStory = (product: string, id: string, root = dataRoot) => storyArchive(product, id, true, root);

/** Takes the story out of the archive record; it comes back as it was. */
export const restoreStory = (product: string, id: string, root = dataRoot) => storyArchive(product, id, false, root);

/** The highest number of either type plus 1, with the prefix of `type`. */
export async function nextStoryId(product: string, type: StoryType, _root = dataRoot): Promise<string> {
  const { rows } = await getPool().query("select id from stories where product = $1", [product]);
  const highest = Math.max(0, ...rows.map((r) => storyNumber(String(r.id))));
  return `${PREFIX[type]}${String(highest + 1).padStart(3, "0")}`;
}

/**
 * Validates the story against the product roster, sets `updated_at`, and upserts the story JSON
 * in the stories table by product and id. Writes no file and makes no commit. Returns the saved story.
 */
export async function saveStory(product: Product, story: Story, _root = dataRoot): Promise<Story> {
  const { archived: _archived, ...plain } = story;
  const saved: Story = { ...plain, updated_at: new Date().toISOString() };
  const failures = validateStory(saved, { roster: product.roles });
  if (failures.length) throw new Error(`Story ${story.id} is invalid:\n${failures.map((f) => `- ${f}`).join("\n")}`);
  await getPool().query(
    `insert into stories (product, id, data) values ($1, $2, $3::jsonb)
     on conflict (product, id) do update set data = excluded.data`,
    [product.id, saved.id, JSON.stringify(saved)],
  );
  return saved;
}

/** The Markdown view of a story. It is rendered from the story JSON on every read; no `.md` file exists. */
export function storyMarkdown(s: Story): string {
  const out: string[] = [];
  out.push(`# ${s.id} ${s.title}`, "");
  out.push(`<!-- Rendered from the story JSON in the database. -->`, "");
  out.push(`- Type: ${s.type}`);
  out.push(`- Owner: ${s.owner_role}`);
  out.push(`- Status: ${s.status}`);
  out.push(`- Approval: ${s.approval}`);
  if (s.approval === "changes_requested" && s.changes_note) out.push(`- Changes requested: ${s.changes_note}`);
  out.push(`- Proposed by: ${s.proposed_by}${s.minutes ? ` in minutes ${s.minutes}` : ""}`);
  if (s.objective) out.push(`- Objective: ${s.objective}`);
  out.push("");

  if (s.as_a) out.push(`**As a** ${s.as_a}`, "");
  out.push(`**I want** ${s.i_want}`, "", `**So that** ${s.so_that}`, "");

  out.push("## Criteria", "");
  s.criteria.forEach((c, i) => {
    out.push(`### ${i + 1}`, "");
    out.push(`- **Given** ${c.given}`, `- **When** ${c.when}`, `- **Then** ${c.then}`);
    out.push(`- Test: ${c.test ? `\`${c.test}\`` : "none"}`, `- Checks: ${c.checks}`, "");
  });

  if (s.sources.length) {
    out.push("## Sources", "", ...s.sources.map((src) => `- ${src.type} \`${src.ref}\``), "");
  }

  if (s.session) {
    const x = s.session;
    out.push("## Session", "");
    out.push(`- State: ${x.state}`);
    out.push(`- Branch: \`${x.branch}\``);
    out.push(`- Attempts: ${x.attempts}`);
    if (x.tests_commit) out.push(`- Tests commit: \`${x.tests_commit}\``);
    if (x.final_commit) out.push(`- Final commit: \`${x.final_commit}\``);
    if (x.stop) out.push(`- ${stopText(x.stop)}`);
    if (x.failure) out.push(`- Failure: ${x.failure}`);
    out.push("");
  }

  if (s.results.length) {
    out.push("## Results", "", ...s.results.map((r) => `- ${r.at}: ${r.result}${r.commit ? ` (\`${r.commit}\`)` : ""}`), "");
  }
  return out.join("\n");
}

export type StoryAction = "approve" | "changes" | "discard" | "run" | "accept" | "reject";
export const STORY_ACTIONS: StoryAction[] = ["approve", "changes", "discard", "run", "accept", "reject"];

/** A session that is under way or waits for review; a failed one no longer holds the story. */
export const hasActiveSession = (s: Story): boolean => s.session !== null && s.session.state !== "failed";

/** A session whose process may still run: not ready and not failed. */
export const isRunningSession = (s: Story): boolean => hasActiveSession(s) && s.session!.state !== "ready";

/** Why `action` is not legal for the story now, or null when it is. */
export function storyActionError(s: Story, action: StoryAction): string | null {
  if (s.status !== "not_implemented") return `Story ${s.id} is ${s.status}.`;
  const state = s.session?.state;
  switch (action) {
    case "approve":
      if (s.approval !== "pending") return `Story ${s.id} has approval ${s.approval}; only a pending story can be approved.`;
      return null;
    case "discard":
      return s.session ? `Story ${s.id} has a story session in state ${state}. Run agentsi story reject ${s.id} first.` : null;
    case "changes":
    case "run":
      if (hasActiveSession(s)) return `Story ${s.id} has a story session in state ${state}.`;
      if (action === "run" && s.approval !== "approved") return `Story ${s.id} has approval ${s.approval}; approve it before the run.`;
      return null;
    case "accept":
      return state === "ready" ? null : `Story ${s.id} has no story session in state ready.`;
    case "reject":
      return state ? null : `Story ${s.id} has no story session.`;
  }
}

/** The actions that are legal for the story now. */
export const storyActions = (s: Story): StoryAction[] => STORY_ACTIONS.filter((a) => storyActionError(s, a) === null);

function assertAction(s: Story, action: StoryAction): void {
  const error = storyActionError(s, action);
  if (error) throw new Error(error);
}

/** These return a new story and write nothing; pass the result to `saveStory`. */
export function approveStory(s: Story): Story {
  assertAction(s, "approve");
  return { ...s, approval: "approved" };
}

export function requestChanges(s: Story, note: string): Story {
  assertAction(s, "changes");
  if (!note.trim()) throw new Error("A change request needs a note.");
  return { ...s, approval: "changes_requested", changes_note: note.trim() };
}

export function discardStory(s: Story): Story {
  assertAction(s, "discard");
  return { ...s, status: "discarded" };
}
