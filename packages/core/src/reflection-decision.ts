import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { getPool } from "./db.ts";
import { commitFiles, type CommitResult } from "./exec.ts";
import { sha256 } from "./meeting-log.ts";
import { withLock } from "./lock.ts";
import { readOverlay, writeOverlay } from "./overlays.ts";
import { dataRoot, workspaceLockPath } from "./paths.ts";
import { writeAllowed } from "./workspace-write.ts";
import { readReflections, updateReflections, writeReflections, type Proposal } from "./reflections.ts";

/** Applies an approved proposal's diff to its target and marks the proposal approved. A global role file is committed in the code repo; an overlay and the reflection are database rows and are not. */
export type ApplyProposal = (reflectionId: string, o: { product: string; root: string }) => Promise<CommitResult>;

/** A proposal may only write its agent's overlay in its product, or a global role file. */
const allowedTarget = (product: string, p: Proposal): boolean =>
  p.target === `workspace/${product}/agents/${p.agent}.md` || /^plugin\/agents\/[a-z][a-z-]*\.md$/.test(p.target);

/** The overlay a target names (`workspace/<product>/agents/<agent>.md`); overlays live in the database, not in files. */
const overlayOf = (target: string): { product: string; agent: string } | null => {
  const m = /^workspace\/([^/]+)\/agents\/([^/]+)\.md$/.exec(target);
  return m ? { product: m[1]!, agent: m[2]! } : null;
};

export const applyProposal: ApplyProposal = async (reflectionId, { product, root }) => {
  const proposal = await withLock(workspaceLockPath(product, root), async () => {
    const entries = await readReflections(product, root);
    const proposal = entries.find((e) => e.id === reflectionId)?.proposal;
    if (!proposal) throw new Error(`Reflection "${reflectionId}" has no proposal in ${product}.`);
    if (proposal.status !== "pending") throw new Error(`The proposal of "${reflectionId}" is already ${proposal.status}.`);
    if (!allowedTarget(product, proposal)) throw new Error(`The proposal of "${reflectionId}" targets "${proposal.target}", which it may not write.`);
    if ((await targetHash(proposal.target, root)) !== proposal.target_hash) throw new Error(`The target of "${reflectionId}" changed since the proposal.`);

    const overlay = overlayOf(proposal.target);
    if (overlay) await writeOverlay(overlay.product, overlay.agent, proposal.content);
    else writeAllowed(proposal.target, proposal.content, { root, allowPlugin: proposal.target.startsWith("plugin/") });
    await writeReflections(product, entries.map((e) => (e.id === reflectionId ? { ...e, proposal: { ...proposal, status: "approved" as const } } : e)), root);
    return proposal;
  });
  if (overlayOf(proposal.target)) return { committed: false };
  return commitFiles(root, [proposal.target], `reflect(${product}): ${reflectionId} proposal approved`);
};

/** sha256 of the target's text (an overlay in the database, or a repo-relative file), or null when it does not exist. */
export async function targetHash(target: string, root = dataRoot): Promise<string | null> {
  const overlay = overlayOf(target);
  if (overlay) {
    const text = await readOverlay(overlay.product, overlay.agent);
    return text === null ? null : sha256(text);
  }
  const file = join(root, target);
  return existsSync(file) ? sha256(readFileSync(file, "utf8")) : null;
}

/** The part of a pg pool the decision writers use, so tests can stub the store. */
export interface DecisionStore {
  query<R extends object>(sql: string, params: unknown[]): Promise<{ rows: R[] }>;
}

export async function recordReflectionDecision(
  product: string,
  reflectionId: string,
  text: string,
  why: string,
  db: DecisionStore = getPool(),
): Promise<number> {
  const { rows } = await db.query<{ id: string }>(
    "insert into decisions (product, kind, reflection_id, text, why) values ($1, 'reflection', $2, $3, $4) returning id",
    [product, reflectionId, text, why],
  );
  return Number(rows[0]!.id);
}

export type ReflectionVerdict = "approved" | "rejected" | "stale";

/** The decision's reflection has no proposal on disk, so the decision can never be applied. */
export class MissingProposal extends Error {}

async function setProposalStatus(product: string, reflectionId: string, status: Proposal["status"], root: string): Promise<CommitResult> {
  await updateReflections(product, (entries) => entries.map((e) => (e.id === reflectionId && e.proposal ? { ...e, proposal: { ...e.proposal, status } } : e)), root);
  return { committed: false };
}

/**
 * Settles the pending proposal of one reflection. Approval first checks that the target file still has the hash
 * it had when the proposal was made; if it changed, the proposal is marked stale and nothing is applied.
 * Settling again with the same verdict returns the earlier outcome, so a retry after a failed database update succeeds.
 */
export async function settleReflection(
  product: string,
  reflectionId: string,
  verdict: "approved" | "rejected",
  o: { root?: string; apply?: ApplyProposal } = {},
): Promise<{ status: ReflectionVerdict; commit: CommitResult }> {
  const root = o.root ?? dataRoot;
  const proposal = (await readReflections(product, root)).find((e) => e.id === reflectionId)?.proposal;
  if (!proposal) throw new MissingProposal(`Reflection "${reflectionId}" has no proposal in ${product}.`);
  if (proposal.status === verdict || (proposal.status === "stale" && verdict === "approved")) {
    return { status: proposal.status, commit: { committed: false } };
  }
  if (proposal.status !== "pending") throw new Error(`The proposal of "${reflectionId}" is already ${proposal.status}.`);

  if (verdict === "rejected") return { status: "rejected", commit: await setProposalStatus(product, reflectionId, "rejected", root) };
  if ((await targetHash(proposal.target, root)) !== proposal.target_hash) {
    return { status: "stale", commit: await setProposalStatus(product, reflectionId, "stale", root) };
  }
  return { status: "approved", commit: await (o.apply ?? applyProposal)(reflectionId, { product, root }) };
}
