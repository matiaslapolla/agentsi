"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CircleCheck, Pause, Play, Terminal } from "lucide-react";
import { finishCall } from "@/app/actions";
import { RoleAvatar } from "@/components/roster";
import { cn } from "@/lib/cn";
import { clock } from "@/lib/format";
import { roleColorVar, roleMeta } from "@/lib/roles";

export interface Segment {
  idx: number;
  role: string;
  text: string;
  start: number;
  end: number;
}

const SPEEDS = [1, 1.25, 1.5];

export function CallPlayer({
  segments,
  participants,
  audioSrc,
  renderCommand,
  timed,
  product,
  minutesId,
  heard,
}: {
  segments: Segment[];
  participants: string[];
  audioSrc: string | null;
  renderCommand: string;
  timed: boolean;
  product: string;
  minutesId: string;
  heard: boolean;
}) {
  const audioRef = useRef<HTMLAudioElement>(null);
  const listRef = useRef<HTMLOListElement>(null);
  const [time, setTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [speed, setSpeed] = useState(1);
  const [finished, setFinished] = useState(false);
  const canPlay = !!audioSrc && timed;

  async function onEnded() {
    setPlaying(false);
    setFinished(true);
    await finishCall(product, minutesId);
  }

  const total = useMemo(
    () => Math.max(duration || 0, segments.reduce((m, s) => Math.max(m, s.end), 0)),
    [duration, segments],
  );

  const activeIdx = useMemo(() => {
    if (!timed) return -1;
    let found = -1;
    for (let i = 0; i < segments.length; i++) if (segments[i].start <= time + 0.05) found = i;
    return found;
  }, [segments, time, timed]);
  const active = activeIdx >= 0 ? segments[activeIdx] : null;
  const speaking = playing && active && time < active.end + 0.15 ? active.role : null;

  useEffect(() => {
    const a = audioRef.current;
    if (!a || !playing) return;
    let raf = 0;
    let last = -1;
    const tick = () => {
      if (Math.abs(a.currentTime - last) > 0.08) {
        last = a.currentTime;
        setTime(a.currentTime);
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [playing]);

  useEffect(() => {
    const list = listRef.current;
    if (!list || activeIdx < 0) return;
    const el = list.children[activeIdx] as HTMLElement | undefined;
    if (!el) return;
    list.scrollTo({
      top: el.offsetTop - list.clientHeight / 2 + el.clientHeight / 2,
      behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth",
    });
  }, [activeIdx]);

  const seek = useCallback((t: number, play = false) => {
    const a = audioRef.current;
    if (!a) return;
    a.currentTime = t;
    setTime(t);
    if (play) void a.play();
  }, []);

  function toggle() {
    const a = audioRef.current;
    if (!a) return;
    if (a.paused) void a.play();
    else a.pause();
  }

  function setRate(r: number) {
    setSpeed(r);
    if (audioRef.current) audioRef.current.playbackRate = r;
  }

  return (
    <div>
      {audioSrc && (
        <audio
          ref={audioRef}
          src={audioSrc}
          preload="metadata"
          onLoadedMetadata={(e) => setDuration(e.currentTarget.duration)}
          onPlay={() => {
            setPlaying(true);
            setFinished(false);
          }}
          onPause={() => setPlaying(false)}
          onEnded={() => void onEnded()}
          onSeeked={(e) => setTime(e.currentTarget.currentTime)}
        />
      )}

      <div className="rounded-xl border border-border bg-card p-5 sm:p-8">
        <div className="mb-6 flex items-center justify-between text-xs">
          <span className="flex items-center gap-2 text-muted-foreground">
            <span className={cn("size-2 rounded-full", playing ? "bg-status-fresh" : "bg-muted-foreground")} />
            {playing ? "En llamada" : canPlay ? "En pausa" : "Sin audio"}
            {(heard || finished) && !playing && (
              <span className="inline-flex items-center gap-1 text-status-fresh">
                <CircleCheck className="size-3.5" /> escuchada
              </span>
            )}
          </span>
          <span className="mono text-muted-foreground">
            {timed && activeIdx >= 0 ? `segmento ${activeIdx + 1} de ${segments.length}` : `${participants.length} participantes`} ·{" "}
            {clock(total)}
          </span>
        </div>

        <div className="flex flex-wrap items-start justify-center gap-x-6 gap-y-8 sm:gap-x-10">
          {participants.map((r) => {
            const on = speaking === r;
            const dim = !!speaking && !on;
            const meta = roleMeta(r);
            return (
              <div
                key={r}
                className={cn("flex w-[88px] flex-col items-center gap-3 transition-opacity duration-200 sm:w-[120px]", dim && "opacity-45")}
              >
                <div className="relative grid place-items-center">
                  {on && (
                    <span
                      className="speak-ring absolute inset-0 rounded-full"
                      style={{ boxShadow: `0 0 0 3px ${roleColorVar(r)}` }}
                      aria-hidden
                    />
                  )}
                  <RoleAvatar
                    role={r}
                    className={cn("relative [--a:56px] transition-transform duration-200 sm:[--a:72px]", on && "scale-105")}
                  />
                </div>
                <div className="text-center">
                  <p className="font-semibold leading-tight" style={{ color: on ? roleColorVar(r) : undefined }}>
                    {meta.label}
                  </p>
                  <p className="mt-0.5 hidden min-h-[2lh] text-[11.5px] leading-tight text-muted-foreground sm:block">{meta.blurb}</p>
                </div>
                <div className="flex h-4 items-end gap-[3px]" aria-hidden>
                  {[0, 1, 2, 3, 4].map((b) => (
                    <span
                      key={b}
                      className={cn("w-[3px] rounded-full", on ? "speak-bar h-4" : "h-[3px]")}
                      style={{
                        background: on ? roleColorVar(r) : "var(--border)",
                        animationDelay: `${b * 0.13}s`,
                      }}
                    />
                  ))}
                </div>
              </div>
            );
          })}
        </div>

        <div className="mx-auto mt-8 flex min-h-[5.5lh] max-w-2xl flex-col items-center justify-center text-center" aria-hidden={playing}>
          {finished ? (
            <p className="cleared-in flex items-center gap-2 text-[17px] font-semibold text-status-fresh">
              <CircleCheck className="size-5" /> Llamada completa
            </p>
          ) : active && (playing || time > 0) ? (
            <>
              <p className="text-[13px] font-semibold" style={{ color: roleColorVar(active.role) }}>
                {roleMeta(active.role).label}
              </p>
              <p key={active.idx} className="cleared-in mt-1 line-clamp-4 text-[17px] leading-relaxed text-foreground">
                {active.text}
              </p>
            </>
          ) : (
            <p className="text-sm text-muted-foreground">
              {canPlay ? `Dale play: ${roleMeta(segments[0]?.role ?? "pm").label} abre la reunión.` : "Los subtítulos aparecen acá durante la llamada."}
            </p>
          )}
        </div>

        <div className="mt-6 flex flex-wrap items-center gap-x-4 gap-y-3 sm:flex-nowrap">
          <button
            type="button"
            onClick={toggle}
            disabled={!canPlay}
            aria-label={playing ? "Pausar" : "Reproducir"}
            className="grid size-12 shrink-0 cursor-pointer place-items-center rounded-full bg-foreground text-background transition-opacity hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {playing ? <Pause className="size-5" fill="currentColor" /> : <Play className="size-5 translate-x-px" fill="currentColor" />}
          </button>

          <div className="min-w-0 flex-1">
            <div className="relative h-7 rounded-full has-[input:focus-visible]:outline-2 has-[input:focus-visible]:outline-offset-2 has-[input:focus-visible]:outline-foreground">
              <div className="absolute inset-x-0 top-1/2 flex h-2.5 -translate-y-1/2 gap-[2px] overflow-hidden rounded-full">
                {timed && total > 0
                  ? segments.map((s) => (
                      <span
                        key={s.idx}
                        className="h-full"
                        style={{
                          width: `${((s.end - s.start) / total) * 100}%`,
                          background: roleColorVar(s.role),
                          opacity: s.start <= time ? 0.95 : 0.28,
                        }}
                      />
                    ))
                  : <span className="h-full w-full bg-muted" />}
              </div>
              {timed && total > 0 && (
                <span
                  className="pointer-events-none absolute top-1/2 h-5 w-[3px] -translate-x-1/2 -translate-y-1/2 rounded-full bg-foreground"
                  style={{ left: `${Math.min(100, (time / total) * 100)}%` }}
                />
              )}
              <input
                type="range"
                aria-label="Posición de la llamada"
                min={0}
                max={total || 1}
                step={0.1}
                value={time}
                disabled={!canPlay}
                onChange={(e) => seek(Number(e.target.value))}
                className="absolute inset-0 h-full w-full cursor-pointer opacity-0 outline-none disabled:cursor-not-allowed"
              />
            </div>
            <div className="mono mt-0.5 flex justify-between text-xs text-muted-foreground">
              <span>{clock(time)}</span>
              <span>{clock(total)}</span>
            </div>
          </div>

          <div className="flex shrink-0 overflow-hidden rounded-md border border-border max-sm:order-3 max-sm:w-full" role="group" aria-label="Velocidad">
            {SPEEDS.map((r) => (
              <button
                key={r}
                type="button"
                onClick={() => setRate(r)}
                aria-pressed={speed === r}
                disabled={!canPlay}
                className={cn(
                  "mono h-10 cursor-pointer border-r max-sm:flex-1 border-border px-2.5 sm:h-8 text-xs transition-colors last:border-r-0 disabled:cursor-not-allowed disabled:opacity-40",
                  speed === r ? "bg-muted text-foreground" : "text-muted-foreground hover:bg-muted",
                )}
              >
                {r}x
              </button>
            ))}
          </div>
        </div>

        {!canPlay && (
          <div className="mt-6 flex items-start gap-3 rounded-lg border border-dashed border-border px-4 py-3 text-sm text-muted-foreground">
            <Terminal className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
            <p>
              {timed
                ? "Todavía no hay audio para esta llamada."
                : "Esta llamada todavía no se renderizó."}{" "}
              Generalo con <code className="mono rounded bg-muted px-1.5 py-0.5 text-foreground">{renderCommand}</code>
            </p>
          </div>
        )}
      </div>

      <h2 className="mb-2 mt-12 text-[17px] font-semibold tracking-[-0.01em]">Transcripción</h2>
      <ol
        ref={listRef}
        className="relative max-h-[460px] overflow-y-auto border-y border-border py-2"
      >
        {segments.map((s, i) => {
          const isActive = i === activeIdx;
          return (
            <li key={s.idx}>
              <button
                type="button"
                onClick={() => canPlay && seek(s.start, true)}
                disabled={!canPlay}
                aria-current={isActive ? "true" : undefined}
                className={cn(
                  "flex w-full gap-3 rounded-md px-3 py-3 text-left transition-colors enabled:cursor-pointer enabled:hover:bg-card",
                  isActive && "bg-card",
                )}
              >
                <RoleAvatar role={s.role} size={28} className="mt-0.5" />
                <span className="min-w-0 flex-1">
                  <span className="flex items-baseline gap-2">
                    <span className="text-[13px] font-semibold" style={{ color: roleColorVar(s.role) }}>
                      {roleMeta(s.role).label}
                    </span>
                    {timed && <span className="mono text-[11px] text-muted-foreground">{clock(s.start)}</span>}
                  </span>
                  <span className={cn("mt-0.5 block text-[15px] leading-relaxed", isActive ? "text-foreground" : "text-muted-foreground")}>
                    {s.text}
                  </span>
                </span>
              </button>
            </li>
          );
        })}
      </ol>
    </div>
  );
}
