import { roleColorVar } from "@/lib/roles";

/** Tone of a mark: a role id, a status colour, the primary colour, or the plain card. */
export type Tone = "plain" | "primary" | "you" | "repo" | "fail" | string;

const toneColor = (tone: Tone): string | null =>
  tone === "plain" ? null : tone === "primary" ? "var(--primary)" : tone === "you" ? "var(--status-waiting)" : tone === "repo" ? "var(--status-fresh)" : tone === "fail" ? "var(--status-neglected)" : roleColorVar(tone);

export function Diagram({ id, w, h, label, minWidth = 640, children }: { id: string; w: number; h: number; label: string; minWidth?: number; children: React.ReactNode }) {
  return (
    <div className="-mx-1 overflow-x-auto px-1 pb-1">
      <svg viewBox={`0 0 ${w} ${h}`} role="img" aria-label={label} className="block h-auto w-full text-foreground" style={{ minWidth }}>
        <defs>
          <marker id={`${id}-arrow`} viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
            <path d="M0,0 L10,5 L0,10 z" fill="currentColor" />
          </marker>
        </defs>
        {children}
      </svg>
    </div>
  );
}

export function Box({ x, y, w, h, title, sub, tone = "plain", dashed, start }: { x: number; y: number; w: number; h: number; title: string; sub?: string; tone?: Tone; dashed?: boolean; start?: boolean }) {
  const color = toneColor(tone);
  const cx = start ? x + 14 : x + w / 2;
  const anchor = start ? "start" : "middle";
  const ty = sub ? y + h / 2 - 3 : y + h / 2 + 4.5;
  return (
    <g>
      <rect
        x={x}
        y={y}
        width={w}
        height={h}
        rx={10}
        fill={color ? `color-mix(in oklch, ${color} 14%, var(--card))` : "var(--card)"}
        stroke={color ?? "var(--border)"}
        strokeWidth={color ? 1.5 : 1}
        strokeDasharray={dashed ? "5 4" : undefined}
      />
      <text x={cx} y={ty} textAnchor={anchor} fontSize={13} fontWeight={600} fill="currentColor">
        {title}
      </text>
      {sub && (
        <text x={cx} y={ty + 17} textAnchor={anchor} fontSize={11} fill="var(--muted-foreground)" className="mono">
          {sub}
        </text>
      )}
    </g>
  );
}

/** A labelled arrow along the points; the label sits at the middle of the first segment unless `at` moves it. */
export function Arrow({ id, points, label, at, dashed, flow, tone }: { id: string; points: [number, number][]; label?: string; at?: [number, number]; dashed?: boolean; flow?: boolean; tone?: Tone }) {
  const color = tone ? toneColor(tone) : null;
  const d = points.map(([px, py], i) => `${i ? "L" : "M"}${px},${py}`).join(" ");
  const [[x1, y1], [x2, y2]] = points as [[number, number], [number, number]];
  const [lx, ly] = at ?? [(x1 + x2) / 2, (y1 + y2) / 2 - 7];
  return (
    <g style={color ? { color } : undefined} className={color ? undefined : "text-muted-foreground"}>
      <path d={d} fill="none" stroke="currentColor" strokeWidth={1.4} strokeDasharray={dashed ? "4 4" : undefined} markerEnd={`url(#${id}-arrow)`} />
      {flow && <path d={d} fill="none" stroke="currentColor" strokeWidth={2.4} strokeLinecap="round" pathLength={100} className="flow-dot" />}
      {label && (
        <text x={lx} y={ly} textAnchor="middle" fontSize={11.5} fill="currentColor" stroke="var(--background)" strokeWidth={4} paintOrder="stroke">
          {label}
        </text>
      )}
    </g>
  );
}

/** A dashed frame that groups boxes, with its name at the top left or the bottom left. */
export function Zone({ x, y, w, h, title, tone = "plain", bottom }: { x: number; y: number; w: number; h: number; title: string; tone?: Tone; bottom?: boolean }) {
  const color = toneColor(tone) ?? "var(--border)";
  return (
    <g>
      <rect x={x} y={y} width={w} height={h} rx={14} fill={`color-mix(in oklch, ${color} 5%, transparent)`} stroke={color} strokeDasharray="6 5" />
      <text x={x + 14} y={bottom ? y + h - 10 : y + 20} fontSize={11} fontWeight={600} letterSpacing="0.06em" fill={color === "var(--border)" ? "var(--muted-foreground)" : color}>
        {title.toUpperCase()}
      </text>
    </g>
  );
}

/** A numbered step marker. */
export function Step({ x, y, n, tone = "primary" }: { x: number; y: number; n: number; tone?: Tone }) {
  const color = toneColor(tone) ?? "var(--primary)";
  return (
    <g>
      <circle cx={x} cy={y} r={10} fill={color} />
      <text x={x} y={y + 4} textAnchor="middle" fontSize={11} fontWeight={700} fill="var(--background)">
        {n}
      </text>
    </g>
  );
}

export function Figure({ title, caption, children }: { title: string; caption: React.ReactNode; children: React.ReactNode }) {
  return (
    <figure className="space-y-4 rounded-[18px] border border-border bg-card p-4 shadow-[var(--soft-shadow)] sm:p-6">
      <h2 className="text-lg font-semibold tracking-[-0.01em]">{title}</h2>
      {children}
      <p className="text-[12.5px] text-muted-foreground sm:hidden">Deslizá el diagrama para verlo entero.</p>
      <figcaption className="space-y-1.5 text-[14px] text-muted-foreground">{caption}</figcaption>
    </figure>
  );
}
