"use client";

import { useState, useTransition } from "react";
import { Check, MessageSquareText, Trash2 } from "lucide-react";
import { approveStoryAction, discardStoryAction, requestStoryChangesAction, type ActionResult } from "@/app/actions";
import { Button } from "@/components/ui/button";
import type { Story, StoryAction } from "@/lib/stories";

const PROMPT: Record<Story["approval"], { title: string; text: string }> = {
  pending: { title: "¿La aprobás?", text: "Aprobada, la story puede correr su sesión. Si algo no cierra, pedí cambios." },
  approved: { title: "Ya está aprobada", text: "Si cambió algo antes de la sesión, pedí cambios o descartala." },
  changes_requested: { title: "¿Otra nota?", text: "Podés reemplazar la nota o descartar la story." },
};

/** Only the actions that `storyActions` permits reach this component. */
export function StoryActions({
  product,
  id,
  approval,
  actions,
}: {
  product: string;
  id: string;
  approval: Story["approval"];
  actions: StoryAction[];
}) {
  const [pending, start] = useTransition();
  const [mode, setMode] = useState<"idle" | "changes" | "discard">("idle");
  const [note, setNote] = useState("");
  const [error, setError] = useState<string | null>(null);

  function act(fn: () => Promise<ActionResult>) {
    setError(null);
    start(async () => {
      const res = await fn();
      if (!res.ok) setError(res.error);
      else setMode("idle");
    });
  }

  return (
    <div className="rounded-lg bg-card px-5 py-4 shadow-soft">
      <p className="font-medium">{PROMPT[approval].title}</p>
      <p className="mt-0.5 text-sm text-muted-foreground">{PROMPT[approval].text}</p>

      {mode === "changes" && (
        <form
          className="mt-4"
          onSubmit={(e) => {
            e.preventDefault();
            act(() => requestStoryChangesAction(product, id, note));
          }}
        >
          <label htmlFor="story-note" className="label text-muted-foreground">
            Qué cambiar
          </label>
          <textarea
            id="story-note"
            value={note}
            onChange={(e) => setNote(e.target.value)}
            rows={3}
            autoFocus
            placeholder="Por ejemplo: sumá un criterio para el modo oscuro."
            className="mt-1.5 w-full rounded-md border border-input bg-transparent px-3 py-2 text-[15px] outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 dark:bg-input/30"
          />
          <div className="mt-2 flex flex-wrap gap-2">
            <Button type="submit" disabled={pending || !note.trim()}>
              Enviar la nota
            </Button>
            <Button type="button" variant="ghost" onClick={() => setMode("idle")} disabled={pending}>
              Cancelar
            </Button>
          </div>
        </form>
      )}

      {mode === "discard" && (
        <div className="mt-4 flex flex-wrap items-center gap-2" role="group" aria-label="Confirmar descarte">
          <span className="text-sm">¿Descartar la story? La podés ver después en Descartadas.</span>
          <Button variant="destructive" size="sm" onClick={() => act(() => discardStoryAction(product, id))} disabled={pending}>
            Sí, descartar
          </Button>
          <Button variant="ghost" size="sm" onClick={() => setMode("idle")} disabled={pending}>
            No
          </Button>
        </div>
      )}

      {mode === "idle" && (
        <div className="mt-4 flex flex-wrap gap-2">
          {actions.includes("approve") && (
            <Button onClick={() => act(() => approveStoryAction(product, id))} disabled={pending}>
              <Check /> Aprobar
            </Button>
          )}
          {actions.includes("changes") && (
            <Button variant="outline" onClick={() => setMode("changes")} disabled={pending}>
              <MessageSquareText /> Pedir cambios
            </Button>
          )}
          {actions.includes("discard") && (
            <Button variant="ghost" className="text-destructive hover:text-destructive" onClick={() => setMode("discard")} disabled={pending}>
              <Trash2 /> Descartar
            </Button>
          )}
        </div>
      )}

      {error && (
        <p role="alert" className="mt-3 text-sm text-destructive">
          {error}
        </p>
      )}
    </div>
  );
}
