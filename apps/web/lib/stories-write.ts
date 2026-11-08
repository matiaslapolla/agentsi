import { decideStory, type StoryVerdict } from "@agentsi/core/story-write";
import type { Product } from "@agentsi/core";
import { archiveStory as archiveStoryCore, restoreStory as restoreStoryCore, type Story } from "@agentsi/core/story";
import { ROOT } from "./env";

export type WebStoryAction = StoryVerdict;

/** The core call of `agentsi story approve|discard`; "changes" records the note, and the owner then runs `agentsi story changes`. */
export const changeStory = (product: Product, id: string, action: WebStoryAction, note = "", root = ROOT): Promise<Story> =>
  decideStory({ product, root }, id, action, note);

/** The core call of `agentsi archive <story-id>`; refused while a story session is running. */
export const archiveStory = async (product: Product, id: string, root = ROOT): Promise<void> => {
  const commit = await archiveStoryCore(product.id, id, root);
  if (commit.error) throw new Error(`Commit skipped: ${commit.error}`);
};

/** The core call of `agentsi restore <story-id>`. */
export const restoreStory = async (product: Product, id: string, root = ROOT): Promise<void> => {
  const commit = await restoreStoryCore(product.id, id, root);
  if (commit.error) throw new Error(`Commit skipped: ${commit.error}`);
};
