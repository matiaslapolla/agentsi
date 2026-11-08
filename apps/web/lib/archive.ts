import { archiveMinutes as archiveCore, restoreMinutes as restoreCore, restoreProduct as restoreProductCore, type CommitResult, type Product } from "@agentsi/core";
import { ROOT } from "./env";
import { archivedRefs } from "./workspace";

/** The core call of `agentsi archive`: the web app and the CLI share one archive record. */
export const archiveMinutes = (product: Product, id: string, root = ROOT): Promise<CommitResult> => archiveCore(product.id, id, root);

/** True when the whole product is archived (`agentsi archive`, no id). */
export const productArchived = async (id: string): Promise<boolean> => (await archivedRefs(id, "product")).size > 0;

/** The core call of `agentsi restore` for a whole product. */
export const restoreProduct = (product: Product, root = ROOT): Promise<CommitResult> => restoreProductCore(product.id, root);

/** The core call of `agentsi restore`. */
export const restoreMinutes = (product: Product, id: string, root = ROOT): Promise<CommitResult> => restoreCore(product.id, id, root);
