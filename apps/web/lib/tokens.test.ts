import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const css = readFileSync(new URL("../app/globals.css", import.meta.url), "utf8");

function block(selector: string): Record<string, string> {
  const start = css.indexOf(`${selector} {`);
  assert.ok(start >= 0, `missing ${selector} block`);
  const body = css.slice(start, css.indexOf("}", start));
  return Object.fromEntries([...body.matchAll(/--([\w-]+):\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]));
}

const light = block(":root");
const themes = { light, dark: { ...light, ...block(':root[data-theme="dark"]') } };

function luminance(hex: string): number {
  assert.match(hex, /^#[0-9a-f]{6}$/i, `${hex} must be a 6-digit hex color`);
  const [r, g, b] = [1, 3, 5].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

// Tailwind mixes `/15` in oklab. An sRGB blend is close enough for a 4.5 floor.
function tint(hex: string, over: string, alpha: number): string {
  const channels = (h: string) => [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16));
  const [a, b] = [channels(hex), channels(over)];
  return `#${a.map((v, i) => Math.round(v * alpha + b[i] * (1 - alpha)).toString(16).padStart(2, "0")).join("")}`;
}

function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const STATUSES = ["waiting", "neglected", "fresh", "idle"] as const;

const TEXT_ON = [
  ["foreground", "background"],
  ["foreground", "card"],
  ["foreground", "muted"],
  ["card-foreground", "card"],
  ["popover-foreground", "popover"],
  ["muted-foreground", "background"],
  ["muted-foreground", "card"],
  ["muted-foreground", "muted"],
  ["primary", "background"],
  ["primary", "card"],
  ["primary-foreground", "primary"],
  ["secondary-foreground", "secondary"],
  ["accent-foreground", "accent"],
  ["destructive", "background"],
  ["destructive", "card"],
  ["destructive-foreground", "destructive"],
  ["sidebar-foreground", "sidebar"],
  ["sidebar-foreground", "sidebar-accent"],
  ["sidebar-accent-foreground", "sidebar-accent"],
  ["sidebar-primary", "sidebar"],
  ["sidebar-primary-foreground", "sidebar-primary"],
  ...STATUSES.flatMap((s) => [
    [`status-${s}`, "background"],
    [`status-${s}`, "card"],
    [`status-${s}`, "sidebar"],
  ]),
] as const;

for (const [name, tokens] of Object.entries(themes)) {
  test(`${name} text pairs reach WCAG AA (4.5:1)`, () => {
    const pairs = TEXT_ON.map(([fg, bg]) => ({ fg, bg, ratio: contrast(tokens[fg], tokens[bg]) }));
    for (const status of STATUSES) {
      for (const surface of ["background", "card"]) {
        const color = tokens[`status-${status}`];
        const chip = tint(color, tokens[surface], 0.15);
        pairs.push({ fg: `status-${status}`, bg: `status-${status}/15 on ${surface}`, ratio: contrast(color, chip) });
      }
    }
    const failures = pairs
      .filter(({ ratio }) => ratio < 4.5)
      .map(({ fg, bg, ratio }) => `${fg} on ${bg}: ${ratio.toFixed(2)}`);
    assert.deepEqual(failures, []);
  });
}
