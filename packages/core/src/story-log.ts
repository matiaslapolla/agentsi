import { getPool } from "./db.ts";

/** What `session.log` holds: where to read the log of the story. */
export const storyLogRef = (product: string, id: string): string => `story_logs ${product}/${id}`;

/** Appends text to the log of one story in the story_logs table, by product and story id. */
export async function appendStoryLog(product: string, id: string, text: string): Promise<void> {
  await getPool().query(
    `insert into story_logs (product, story_id, body) values ($1, $2, $3)
     on conflict (product, story_id) do update set body = story_logs.body || excluded.body`,
    [product, id, text],
  );
}

/** The log text of one story; empty when the story has no log. */
export async function readStoryLog(product: string, id: string): Promise<string> {
  const { rows } = await getPool().query("select body from story_logs where product = $1 and story_id = $2", [product, id]);
  return rows[0] ? String(rows[0].body) : "";
}
