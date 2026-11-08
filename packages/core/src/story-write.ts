import { withLock } from "./lock.ts";
import { dataRoot, workspaceLockPath } from "./paths.ts";
import type { Product } from "./registry.ts";
import { approveStory, discardStory, loadStory, requestChanges, saveStory, type Story } from "./story.ts";

export interface StoryStore {
  product: Product;
  /** AgentSI checkout that holds the workspace lock and the archive record; tests pass a temp dir. */
  root?: string;
}

/**
 * Runs `fn` under the workspace lock of the product, so story transactions of one product run one at a time.
 * The stories are in the database: nothing is committed to the data root and no file is restored.
 * `_message` is accepted for old callers and ignored. `fn` must not take the same lock.
 */
export async function storyTransaction<T>(store: StoryStore, fn: () => T | Promise<T>, _message?: (result: T) => string | null): Promise<T> {
  const root = store.root ?? dataRoot;
  return withLock(workspaceLockPath(store.product.id, root), fn);
}

/** Loads the story, applies `change` and saves it, in one story transaction. */
export function updateStory(store: StoryStore, id: string, change: (s: Story) => Story): Promise<Story> {
  const root = store.root ?? dataRoot;
  return storyTransaction(store, async () => saveStory(store.product, change(await loadStory(store.product.id, id, root)), root));
}

export type StoryVerdict = "approve" | "discard" | "changes";

/**
 * `story approve` and `story discard` in the CLI, and the three story actions of the web; no model call.
 * `changes` records the note only: the PM rewrite of `agentsi story changes` needs a model call.
 */
export function decideStory(store: StoryStore, id: string, verdict: StoryVerdict, note = ""): Promise<Story> {
  const apply: Record<StoryVerdict, (s: Story) => Story> = {
    approve: approveStory,
    discard: discardStory,
    changes: (s) => requestChanges(s, note),
  };
  return updateStory(store, id, apply[verdict]);
}
