"use server";

import { revalidatePath } from "next/cache";
import { archiveMinutes, restoreMinutes, restoreProduct } from "@/lib/archive";
import { recordListen, removeListenRecord } from "@/lib/inbox-data";
import { findProduct } from "@/lib/registry";
import { changeStory, type WebStoryAction } from "@/lib/stories-write";
import { safeSegment } from "@/lib/workspace";

export type ActionResult = { ok: true } | { ok: false; error: string };

export async function finishCall(product: string, minutesId: string): Promise<ActionResult> {
  if (!safeSegment(product) || !safeSegment(minutesId)) return { ok: false, error: "parámetros inválidos" };
  try {
    if (await recordListen(product, minutesId)) revalidatePath("/", "layout");
    return { ok: true };
  } catch (e) {
    return { ok: false, error: (e as Error).message.slice(0, 300) };
  }
}

/** The unmark press of the listened button: removes the meeting's listen row. */
export async function removeListen(product: string, minutesId: string): Promise<ActionResult> {
  if (!safeSegment(product) || !safeSegment(minutesId)) return { ok: false, error: "parámetros inválidos" };
  try {
    if (await removeListenRecord(product, minutesId)) revalidatePath("/", "layout");
    return { ok: true };
  } catch (e) {
    return { ok: false, error: (e as Error).message.slice(0, 300) };
  }
}

async function archiveAction(product: string, minutesId: string, change: typeof archiveMinutes): Promise<ActionResult> {
  if (!safeSegment(product) || !safeSegment(minutesId)) return { ok: false, error: "parámetros inválidos" };
  const p = findProduct(product);
  if (!p) return { ok: false, error: `producto desconocido: ${product}` };
  try {
    await change(p, minutesId);
    revalidatePath("/", "layout");
    return { ok: true };
  } catch (e) {
    return { ok: false, error: (e as Error).message.slice(0, 300) };
  }
}

/** Same as `agentsi archive`: records the minutes in the archive record; the file stays. */
export async function archiveMinutesAction(product: string, minutesId: string): Promise<ActionResult> {
  return archiveAction(product, minutesId, archiveMinutes);
}

/** Same as `agentsi restore`. */
export async function restoreMinutesAction(product: string, minutesId: string): Promise<ActionResult> {
  return archiveAction(product, minutesId, restoreMinutes);
}

/** Same as `agentsi restore --product <id>`: clears the archived mark of the whole product. */
export async function restoreProductAction(product: string): Promise<ActionResult> {
  if (!safeSegment(product)) return { ok: false, error: "parámetros inválidos" };
  const p = findProduct(product);
  if (!p) return { ok: false, error: `producto desconocido: ${product}` };
  try {
    await restoreProduct(p);
    revalidatePath("/", "layout");
    return { ok: true };
  } catch (e) {
    return { ok: false, error: (e as Error).message.slice(0, 300) };
  }
}

async function storyAction(product: string, storyId: string, action: WebStoryAction, note = ""): Promise<ActionResult> {
  if (!safeSegment(product) || !safeSegment(storyId)) return { ok: false, error: "parámetros inválidos" };
  const p = findProduct(product);
  if (!p?.stories) return { ok: false, error: `Stories mode está apagado para ${product}` };
  try {
    await changeStory(p, storyId, action, note);
    revalidatePath(`/p/${product}/stories`, "layout");
    return { ok: true };
  } catch (e) {
    return { ok: false, error: (e as Error).message.slice(0, 300) };
  }
}

export async function approveStoryAction(product: string, storyId: string): Promise<ActionResult> {
  return storyAction(product, storyId, "approve");
}

/** Records the note only; the PM rewrite needs a model call, so the owner runs `agentsi story changes`. */
export async function requestStoryChangesAction(product: string, storyId: string, note: string): Promise<ActionResult> {
  if (!note.trim()) return { ok: false, error: "escribí qué cambiar" };
  return storyAction(product, storyId, "changes", note);
}

export async function discardStoryAction(product: string, storyId: string): Promise<ActionResult> {
  return storyAction(product, storyId, "discard");
}
