import { copyFileSync, cpSync, existsSync, mkdirSync, readdirSync, readFileSync, realpathSync } from "node:fs";
import { dirname, join } from "node:path";
import { readBrief, writeBrief, type Brief } from "./brief.ts";
import { getPool } from "./db.ts";
import { parseMeetingLog } from "./meeting-log.ts";
import { readMemoryItems, type MemoryItem } from "./memory.ts";
import { storeMinutes } from "./minutes.ts";
import { readOverlay, writeOverlay } from "./overlays.ts";
import { archivePath, audioDir, dataRoot, listensPath, meetingLogPath, reflectionsPath, snapshotPath, storiesDir, workspaceDir, workspaceRoot } from "./paths.ts";
import type { ReflectionEntry } from "./reflections.ts";
import { AGENT_ID } from "./roles.ts";
import type { Minutes, UsageRow } from "./types.ts";

export interface ImportResult {
  /** Rows inserted or updated; 0 when the database already holds the data of the old root. */
  changes: number;
  stories: number;
  logs: number;
  memory: number;
  overlays: number;
  briefs: number;
  minutes: number;
  log: number;
  snapshots: number;
  listens: number;
  reflections: number;
  archive: number;
  /** Files copied into the data folder: the registry, the voice files and folder, and the audio of each meeting it lacked. */
  files: number;
}

/**
 * Copies the registry and the meeting audio of an old data root into `target`, the data folder (ADR 0003).
 * Audio and the registry stay files; a file or audio folder that `target` already has is never replaced.
 */
function importFiles(dir: string, target: string, result: ImportResult): void {
  if (!existsSync(target)) mkdirSync(target, { recursive: true });
  if (realpathSync(dir) === realpathSync(target)) return;
  // The registry, the private voice files of agentsi-voice, and the voice mode folder (state and transcripts).
  for (const name of ["agentsi.config.json", "voices.toml", "lexicon.toml"]) {
    if (!existsSync(join(dir, name)) || existsSync(join(target, name))) continue;
    copyFileSync(join(dir, name), join(target, name));
    result.files++;
    result.changes++;
  }
  if (existsSync(join(dir, "voice")) && !existsSync(join(target, "voice"))) {
    cpSync(join(dir, "voice"), join(target, "voice"), { recursive: true });
    result.files++;
    result.changes++;
  }
  const workspace = workspaceRoot(dir);
  if (!existsSync(workspace)) return;
  for (const product of readdirSync(workspace)) {
    const audio = join(workspace, product, "audio");
    if (!existsSync(audio)) continue;
    for (const id of readdirSync(audio)) {
      const to = audioDir(product, id, target);
      if (existsSync(to)) continue;
      cpSync(join(audio, id), to, { recursive: true });
      result.files++;
      result.changes++;
    }
  }
}

const STORY_JSON = /^[ST]\d{3,}-[a-z0-9-]+\.json$/;
const STORY_LOG = /^([ST]\d{3,})\.log$/;

// Where memory, overlays and briefs lived before they moved to Postgres; only this import reads them.
const memoryDir = (product: string, root: string) => join(workspaceDir(product, root), "memory");
const memoryPath = (product: string, agent: string, root: string) => join(memoryDir(product, root), `${agent}.json`);
const agentsDir = (product: string, root: string) => join(workspaceDir(product, root), "agents");
const briefPath = (product: string, root: string) => join(workspaceDir(product, root), "brief.json");

async function importStories(product: string, dir: string, result: ImportResult): Promise<void> {
  const db = getPool();
  const folder = storiesDir(product, dir);
  if (!existsSync(folder)) return;
  for (const file of readdirSync(folder).sort()) {
    if (STORY_JSON.test(file)) {
      const data = JSON.parse(readFileSync(join(folder, file), "utf8")) as { id?: unknown };
      if (typeof data.id !== "string") throw new Error(`${join(folder, file)} has no story id.`);
      const { rows } = await db.query("select 1 from stories where product = $1 and id = $2", [product, data.id]);
      if (rows[0]) continue;
      await db.query(
        `insert into stories (product, id, data) values ($1, $2, $3::jsonb)
         on conflict (product, id) do nothing`,
        [product, data.id, JSON.stringify(data)],
      );
      result.stories++;
      result.changes++;
      continue;
    }
    const log = STORY_LOG.exec(file);
    if (!log) continue;
    const body = readFileSync(join(folder, file), "utf8");
    const { rows } = await db.query("select 1 from story_logs where product = $1 and story_id = $2", [product, log[1]]);
    if (rows[0]) continue;
    await db.query(
      `insert into story_logs (product, story_id, body) values ($1, $2, $3)
       on conflict (product, story_id) do nothing`,
      [product, log[1], body],
    );
    result.logs++;
    result.changes++;
  }
}

async function importMemory(product: string, dir: string, result: ImportResult): Promise<void> {
  const db = getPool();
  const folder = memoryDir(product, dir);
  if (!existsSync(folder)) return;
  for (const file of readdirSync(folder).filter((f) => f.endsWith(".json")).sort()) {
    const agent = file.slice(0, -".json".length);
    if (!AGENT_ID.test(agent)) continue;
    const parsed = JSON.parse(readFileSync(memoryPath(product, agent, dir), "utf8")) as { items?: MemoryItem[] };
    const items: MemoryItem[] = (Array.isArray(parsed.items) ? parsed.items : []).map((i) => ({
      id: i.id,
      text: i.text,
      kind: i.kind,
      helpful: i.helpful ?? 0,
      harmful: i.harmful ?? 0,
      source: i.source,
      created_at: i.created_at,
      retired_at: i.retired_at ?? null,
      retired_why: i.retired_why ?? null,
    }));
    const stored = new Map((await readMemoryItems(product, agent)).map((i) => [i.id, i]));
    for (const [position, i] of items.entries()) {
      if (stored.has(i.id)) continue;
      await db.query(
        `insert into memory (product, agent, id, position, text, kind, helpful, harmful, source, created_at, retired_at, retired_why)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         on conflict (product, agent, id) do nothing`,
        [product, agent, i.id, position, i.text, i.kind, i.helpful, i.harmful, i.source, i.created_at, i.retired_at, i.retired_why],
      );
      result.memory++;
      result.changes++;
    }
  }
}

async function importOverlays(product: string, dir: string, result: ImportResult): Promise<void> {
  const folder = agentsDir(product, dir);
  if (!existsSync(folder)) return;
  for (const file of readdirSync(folder).filter((f) => f.endsWith(".md")).sort()) {
    const agent = file.slice(0, -".md".length);
    if (!AGENT_ID.test(agent)) continue;
    const text = readFileSync(join(folder, file), "utf8");
    if ((await readOverlay(product, agent)) !== null) continue;
    await writeOverlay(product, agent, text);
    result.overlays++;
    result.changes++;
  }
}

async function importBrief(product: string, dir: string, result: ImportResult): Promise<void> {
  const file = briefPath(product, dir);
  if (!existsSync(file)) return;
  let brief: Brief;
  try {
    brief = JSON.parse(readFileSync(file, "utf8")) as Brief;
  } catch (err) {
    throw new Error(`${file} is not valid JSON: ${(err as Error).message}`);
  }
  if ((await readBrief(product)) !== null) return;
  await writeBrief(product, brief);
  result.briefs++;
  result.changes++;
}

const MINUTES_JSON = /^(?!.*\.usage\.json$).+\.json$/;

const readJson = <T>(file: string): T => {
  try {
    return JSON.parse(readFileSync(file, "utf8")) as T;
  } catch (err) {
    throw new Error(`${file} is not valid JSON: ${(err as Error).message}`);
  }
};

/** The non-empty lines of a JSON Lines file, parsed. A line that is not JSON is skipped, as the old readers did. */
function jsonLines<T>(file: string): T[] {
  if (!existsSync(file)) return [];
  return readFileSync(file, "utf8")
    .split("\n")
    .flatMap((line) => {
      if (!line.trim()) return [];
      try {
        return [JSON.parse(line) as T];
      } catch {
        return [];
      }
    });
}

/**
 * The minutes id of `product` in the database. Before minutes ids were unique across products (37c489c), two
 * products could hold the same id; the import stores the second one as `<id>-<product>`.
 */
async function localMinutesId(product: string, id: string): Promise<string> {
  const { rows } = await getPool().query<{ id: string }>("select id from minutes where product = $1 and id in ($2, $3)", [product, id, `${id}-${product}`]);
  return rows.find((r) => r.id === id)?.id ?? rows[0]?.id ?? id;
}

async function importMinutes(product: string, dir: string, result: ImportResult): Promise<void> {
  const db = getPool();
  const folder = join(workspaceDir(product, dir), "minutes");
  if (!existsSync(folder)) return;
  for (const file of readdirSync(folder).filter((f) => MINUTES_JSON.test(f)).sort()) {
    const m = readJson<Minutes>(join(folder, file));
    if (typeof m.id !== "string" || m.product !== product) continue;
    const usageFile = join(folder, file.replace(/\.json$/, ".usage.json"));
    const usage = existsSync(usageFile) ? readJson<UsageRow[]>(usageFile) : null;
    const taken = (await db.query<{ product: string }>("select product from minutes where id = $1", [m.id])).rows[0];
    if (taken && taken.product !== product) m.id = `${m.id}-${product}`;
    const { rows } = await db.query<{ product: string; stored: boolean }>("select product, data is not null as stored from minutes where id = $1", [m.id]);
    const row = rows[0];
    if (row && row.product !== product) continue;
    if (row?.stored) continue;
    if (row) {
      // A row from before the `data` column: it keeps its priorities, decisions and usage rows, and gets the JSON.
      await db.query("update minutes set data = $2::jsonb, usage = coalesce(usage, $3::jsonb) where id = $1", [m.id, JSON.stringify(m), usage ? JSON.stringify(usage) : null]);
    } else {
      await storeMinutes(m, usage);
    }
    result.minutes++;
    result.changes++;
  }
}

async function importLog(product: string, dir: string, result: ImportResult): Promise<void> {
  const db = getPool();
  for (const entry of parseMeetingLog(existsSync(meetingLogPath(product, dir)) ? readFileSync(meetingLogPath(product, dir), "utf8") : "", () => {})) {
    const { rows } = await db.query(
      "insert into meeting_log (product, minutes_id, entry) values ($1, $2, $3::jsonb) on conflict (product, minutes_id) do nothing returning minutes_id",
      [product, await localMinutesId(product, entry.minutes_id), JSON.stringify({ ...entry, minutes_id: await localMinutesId(product, entry.minutes_id) })],
    );
    result.log += rows.length;
    result.changes += rows.length;
  }
  const folder = dirname(snapshotPath(product, "x", dir));
  if (!existsSync(folder)) return;
  for (const file of readdirSync(folder).filter((f) => f.endsWith(".md")).sort()) {
    const id = await localMinutesId(product, file.slice(0, -".md".length));
    const { rows } = await db.query(
      "insert into snapshots (product, minutes_id, body) values ($1, $2, $3) on conflict (product, minutes_id) do nothing returning minutes_id",
      [product, id, readFileSync(join(folder, file), "utf8")],
    );
    result.snapshots += rows.length;
    result.changes += rows.length;
  }
}

async function importListens(product: string, dir: string, result: ImportResult): Promise<void> {
  const db = getPool();
  for (const l of jsonLines<{ minutes_id?: unknown; at?: unknown }>(listensPath(product, dir))) {
    if (typeof l.minutes_id !== "string" || typeof l.at !== "string") continue;
    const { rows } = await db.query(
      "insert into listens (product, minutes_id, at) values ($1, $2, $3) on conflict (product, minutes_id) do nothing returning minutes_id",
      [product, await localMinutesId(product, l.minutes_id), l.at],
    );
    result.listens += rows.length;
    result.changes += rows.length;
  }
}

async function importReflections(product: string, dir: string, result: ImportResult): Promise<void> {
  const db = getPool();
  for (const entry of jsonLines<ReflectionEntry>(reflectionsPath(product, dir))) {
    if (typeof entry.id !== "string") continue;
    const { rows } = await db.query(
      `insert into reflections (product, id, position, entry)
       values ($1, $2, (select coalesce(max(position) + 1, 0) from reflections where product = $1), $3::jsonb)
       on conflict (product, id) do nothing returning id`,
      [product, entry.id, JSON.stringify(entry)],
    );
    result.reflections += rows.length;
    result.changes += rows.length;
  }
}

async function importArchive(product: string, dir: string, result: ImportResult): Promise<void> {
  const file = archivePath(product, dir);
  if (!existsSync(file)) return;
  const rec = readJson<{ minutes?: unknown; decisions?: unknown; stories?: unknown; product?: unknown }>(file);
  const list = (v: unknown): string[] => (Array.isArray(v) ? v.filter((x) => typeof x === "string" || Number.isInteger(x)).map(String) : []);
  const wanted: [string, string][] = [
    ...list(rec.minutes).map((r): [string, string] => ["minutes", r]),
    ...list(rec.decisions).map((r): [string, string] => ["decision", r]),
    ...list(rec.stories).map((r): [string, string] => ["story", r]),
    ...(rec.product === true ? [["product", ""] as [string, string]] : []),
  ];
  for (const [kind, ref] of wanted) {
    const { rows } = await getPool().query(
      "insert into archive_records (product, kind, ref) values ($1, $2, $3) on conflict do nothing returning ref",
      [product, kind, ref],
    );
    result.archive += rows.length;
    result.changes += rows.length;
  }
}

/**
 * `agentsi data import <dir>`: loads what an old data root holds into the database. It only adds what the
 * database lacks: a row that exists is newer than the old file and is never replaced, and a second run changes nothing. That is the stories (`stories`) and their logs (`story_logs`), the role memory
 * (`memory`), the roster overlays (`agent_overlays`) and the briefs (`briefs`), and the minutes with their usage
 * (`minutes`), the meeting log (`meeting_log`), the snapshots (`snapshots`), the listens (`listens`), the reflections
 * (`reflections`) and the archive records (`archive_records`).
 */
export async function importData(dir: string, target = dataRoot): Promise<ImportResult> {
  const result: ImportResult = { changes: 0, stories: 0, logs: 0, memory: 0, overlays: 0, briefs: 0, minutes: 0, log: 0, snapshots: 0, listens: 0, reflections: 0, archive: 0, files: 0 };
  importFiles(dir, target, result);
  const workspace = workspaceRoot(dir);
  if (!existsSync(workspace)) return result;
  for (const product of readdirSync(workspace)) {
    await importStories(product, dir, result);
    await importMemory(product, dir, result);
    await importOverlays(product, dir, result);
    await importBrief(product, dir, result);
    await importMinutes(product, dir, result);
    await importLog(product, dir, result);
    await importListens(product, dir, result);
    await importReflections(product, dir, result);
    await importArchive(product, dir, result);
  }
  return result;
}
