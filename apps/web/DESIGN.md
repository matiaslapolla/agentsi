# AgentSI web design system

## Purpose

The web shows what the agency of roles produced: what waits for the owner, the products, the minutes and the memory. It is read-only. The design makes the waiting work stand out first, and keeps long text calm and easy to scan.

The direction is a quiet paper look: pale blue-grey and white areas, one blue for actions, soft shadows in place of hard borders. Status colors carry meaning. Decoration does not.

## Tokens

All tokens live in `app/globals.css`. Light values sit on `:root`. Dark values sit on `:root[data-theme="dark"]`. The names follow shadcn/ui, so its components work with no change. `@theme inline` exposes each token to Tailwind as `bg-<name>`, `text-<name>` and `border-<name>`.

| Token | Light | Dark | Use |
|---|---|---|---|
| `--background` | `#f2f5f7` | `#1c1f24` | Page and pale sections |
| `--foreground` | `#303336` | `#e6e9ee` | Body text |
| `--card` | `#ffffff` | `#24282e` | White sections, cards |
| `--muted` | `#e1e4e6` | `#2e3136` | Quiet fills: chips, code, tab lists |
| `--muted-foreground` | `#44474b` | `#b5bac2` | Secondary text, labels |
| `--accent` | `#e1e4e6` | `#2e3136` | Hover fill of menus and ghost buttons |
| `--primary` | `#1a5bd0` | `#6ea6ff` | Links, the one filled button, focus ring |
| `--primary-foreground` | `#ffffff` | `#0e1116` | Text on a primary fill |
| `--destructive` | `#c81e1e` | `#f87171` | Errors |
| `--border`, `--input` | `#dbdee0` | `#363b43` | Hairlines where a shadow is not enough |
| `--sidebar-*` | white panel | card panel | The sidebar, with its own accent and primary |

`--secondary`, `--popover` and the `-foreground` pairs follow the same values as their neighbours.

### Contrast

Each text and fill pair reaches WCAG AA, 4.5:1, in light and dark. `lib/tokens.test.ts` reads `globals.css` and checks every pair, status chips included. If you change a color, run `pnpm --filter @agentsi/web test`.

The reference blue `#4f91fb` gives 3.1:1 on white. Thus light `--primary` is the darker `#1a5bd0`. Dark `--primary` is lighter, so text on a dark-mode primary fill is near-black.

## Type

- Sans: Inter Variable through Fontsource (`@fontsource-variable/inter`). It sets all UI text.
- Mono: JetBrains Mono through Fontsource. It sets ids, code spans and CLI hints.
- Body: 15px, line height 1.5.

| Role | Class | Weight |
|---|---|---|
| Page title | `text-3xl` (30px) | 600 |
| Minutes question | `text-[28px]`, `text-balance` | 600 |
| Section heading | `text-[17px]` | 600 |
| Card title | 15px | 600 |
| Body | 15px | 400 |
| Label, meta | 12.5px (`.label`) | 500 |
| Mono id | `text-xs` | 400 |

Headings use a small negative tracking (`-0.02em` for titles, `-0.01em` for sections). Body text keeps the default tracking.

## Color roles

- Neutrals carry nearly all of the page. Text is `foreground` or `muted-foreground`.
- `primary` marks what you can act on: links and the single filled button of a section. Do not use it for decoration.
- Status colors mark state only (see below).
- Role colors (`--role-pm`, `--role-dev`, `--role-qa`, `--role-scout`, `--role-social`) mark which role speaks. They fill avatars and dots, not text.

## Status colors

Each `Health` value of the inbox maps to one status token. The map lives in `lib/inbox.ts` as `healthStyle` (stage 1, U2).

| Token | Light | Dark | Meaning |
|---|---|---|---|
| `--status-waiting` | `#875000` amber | `#f2b84b` | Something waits for the owner |
| `--status-neglected` | `#a51f15` red | `#ff8f8f` | No meeting for too long |
| `--status-fresh` | `#0f6a3f` green | `#4ade80` | Up to date |
| `--status-idle` | `#52575e` grey | `#a9afb7` | Idle, never met, or planned |

Use a status token as text or icon color (`text-status-waiting`), or as a 15% tint behind that text (`bg-status-waiting/15`). Both pass 4.5:1. Always pair the color with an icon or a word, so state never depends on color alone.

## Radius and shadow

- `--radius` is 10px. `rounded-lg` is 10px, `rounded-md` 8px, `rounded-sm` 6px, `rounded-xl` 14px.
- `rounded-paper` is 18px. Paper cards use it.
- `shadow-soft` is a wide, faint shadow. White cards use it in place of a border. In dark mode the shadow is darker, and the card fill does most of the separation.

## Signature

These moves make the web recognisable. Use them, and little else.

1. **Alternating sections.** A page is a stack of full-width sections. Pale blue-grey (`bg-background`) sections alternate with white (`bg-card`) sections. No borders between them.
2. **Glyph headings.** A section heading has a 48px glyph beside it: a lucide icon on a soft tinted square of the section color.
3. **White cards.** Items that wait for the owner ("Te toca") are white cards with `shadow-soft` and `rounded-lg`. No border.
4. **Paper cards.** Product cards have no border and no shadow. They use `rounded-paper` and a fill one step off the section behind them. Each has a status chip.
5. **Speech bubbles.** Disagreements and role answers render as bubbles with a tail. The role avatar and name sit under the tail.
6. **One filled button.** A section has at most one filled `primary` button, for example "Escuchar la llamada". Other actions are outline or ghost buttons, or links.

## Sidebar

- The sidebar is the shadcn `Sidebar`, on the left, 16rem wide. It replaces the top header.
- Its panel is `--sidebar` (white in light, card grey in dark), with a `--sidebar-border` hairline on the right.
- Items, in order: Agencia, one group for each product (with Minutes, Roles and Memory), Guía, and the theme toggle at the bottom.
- The active item uses `--sidebar-accent` as fill and `--sidebar-accent-foreground` as text. A product shows its status icon and its waiting count.
- On mobile the sidebar opens as a sheet from a button in a top bar.

## Light and dark

- The theme lives in `data-theme` on `<html>`. An inline script in `app/layout.tsx` sets it before paint, from `localStorage` (`agentsi-theme`) or the system setting. `components/theme-toggle.tsx` flips it.
- Tailwind's `dark:` variant follows `[data-theme=dark]`, not a `.dark` class.
- Every token has a light and a dark value. Do not hard-code a color in a component. If a color is missing, add a token and a contrast pair to the test.

## Stage 1 aliases

The old Tailwind names stay as aliases until stage 2 removes them: `bg`, `surface`, `surface-2`, `line`, `line-soft`, `fg`, `faint`, `accent-soft`, `ok`, `warn` and `bad`. New code uses the token names above. `text-muted` is gone: shadcn's `muted` is a fill, so secondary text is `text-muted-foreground`.
