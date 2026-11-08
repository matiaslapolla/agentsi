# Web UI stage 1

**Status:** draft · **Source:** `docs/ideas/agentsi-web-ui.md` (29c639e) and a design exploration of 2026-10-20 (Thinks)

## Problem

Each page of `apps/web` shows long text with one visual weight. "Te toca" does not stand out on the home. The top nav reaches only Agencia, one product, Logros and Guía. The owner cannot see at a glance what waits for them.

## Goal

- A left sidebar reaches every route and highlights the active URL.
- Each `Health` value has one color and one icon.
- "Te toca" is the first and strongest block on the home.
- The minutes page shows a summary first. Tabs hold the sections, and the long detail folds.
- All pages use the new tokens of `apps/web/DESIGN.md`.

## Non-goals

- No write path. The web stays read-only. `finishCall` stays its only action.
- No SaaS cards on the product, roles, memory and call pages. Stage 2 does that, with its own spec.
- No tabs on the roles page. Stage 2 does that.
- No change in `@agentsi/core`, the CLI or the MCP server.

## Design

### 1. Direction

The direction is the Thinks guideline, chosen in a design exploration. The exploration gave a theme, a handoff and a reference `Screen.tsx`. The reference shows intent. Its code is not pasted.

- Pale blue-grey sections alternate with white sections.
- A section heading has a 48px color glyph beside it.
- "Te toca" items are white cards with a soft shadow.
- Product cards are borderless paper cards with a status chip.
- Disagreements and role answers are speech bubbles with the role under the tail.

`apps/web/DESIGN.md` describes the system in our own words. It never copies the vendor brief.

### 2. Tokens and shadcn/ui

- shadcn/ui is set up in `apps/web` (`components.json`). Components: accordion, badge, breadcrumb, button, pagination, separator, sidebar, table, tabs, collapsible, tooltip.
- The global CSS uses the shadcn variable names (`--background`, `--card`, `--primary`, `--sidebar-*`).
- The theme toggle keeps `data-theme` on `<html>`. A Tailwind custom variant maps `dark:` to `[data-theme="dark"]`.
- Status tokens: `--status-waiting` (amber), `--status-neglected` (red), `--status-fresh` (green) and `--status-idle` (gray), each with a light and a dark value.
- Role tokens `--role-pm`, `--role-dev`, `--role-qa`, `--role-scout` and `--role-social` keep their current values.
- Each text pair reaches WCAG AA (4.5:1). The Thinks blue `#4f91fb` gives 3.3:1 on white. Thus `--primary` for text and links becomes a darker blue that reaches 4.5:1.
- Inter loads through Fontsource, as the current fonts do. JetBrains Mono stays for code.
- During stage 1 the old Tailwind names (`text-muted`, `bg-surface`, `border-line`, `text-faint`, `text-ok`, `text-warn`, `text-bad`) stay as aliases of the new tokens. Stage 2 removes the aliases.

### 3. Shell and sidebar

- `app/layout.tsx` renders the shadcn `Sidebar` on the left and the page on the right. The top header goes.
- Sidebar items, in order:
  1. Agencia (`/`), with the count of "Te toca"
  2. one group for each product of the registry: the product (`/p/<id>`) with its status icon and its count, then Minutes (`/p/<id>#minutes`), Roles (`/p/<id>/roles`) and Memory (`/p/<id>/memory`)
  3. Guía (`/guia`)
  4. the theme toggle at the bottom
- The minutes page and the call page highlight their product group and its Minutes item.
- A pure function `navState(pathname, products)` returns the active item. It has a unit test.
- On mobile, the sidebar opens as a sheet from a button in a top bar.

### 4. Game layer

These go:

- `/logros` (`app/logros/page.tsx`)
- `components/game/meters.tsx` (`AgencyBadge`, streak)
- `components/game/party.tsx` (`Party`)
- achievements and streak in `lib/game.ts` and `lib/game-data.ts`

`lib/game.ts` becomes `lib/inbox.ts`. It keeps `questBoard`, `Health`, `NEGLECT_DAYS` and `meetingProgress`. `lib/game-data.ts` becomes `lib/inbox-data.ts`, and keeps `recordListen`. `components/game/` becomes `components/inbox/`. The tests move with the code.

### 5. Status

One map `healthStyle(health)` in `lib/inbox.ts` returns a label, a token and a lucide icon:

| `Health` | Label | Token | Icon |
|---|---|---|---|
| `waiting` | te espera | `--status-waiting` | `Clock` |
| `neglected` | `{d}` días sin reunión | `--status-neglected` | `TriangleAlert` |
| `fresh` | al día | `--status-fresh` | `CircleCheck` |
| `idle` | `{d}` días sin reunión | `--status-idle` | `Circle` |
| `never` | sin reuniones | `--status-idle` | `CircleDashed` |
| `planned` | planeado | `--status-idle` | `CalendarClock` |

The sidebar, the home and the product page use this map. A unit test makes sure that each `Health` value has an entry.

### 6. Home

1. "Te toca" comes first. It is a grid of white cards, 2 columns on desktop and 1 on mobile, with the pager.
2. "Productos" follows. It is a grid of paper cards, each with name, status chip, waiting counts and meeting count.
3. The "Roles" table (`Party`) goes.

### 7. Minutes page

1. The header keeps the id, status, date, check result and the question.
2. A summary block follows. It shows the closed or open state, the next step, the call card and 4 tiles: priorities marked, decisions decided, disagreements and cost.
3. Tabs follow: Priorities, Disagreements, Decisiones, Respuesta de cada rol, Uso de esta reunión. Each tab label shows its count.
4. Priorities and role answers fold in an accordion. The title, the role and the mark stay visible when an item is folded.
5. Disagreements and role answers render as speech bubbles.
6. The CLI hints (`agentsi mark`, `agentsi decide`) stay as code spans.
7. A URL hash (`#priorities`, `#decisiones`) opens its tab, so the links that exist keep working.

### 8. Other pages

The product, call, roles, memory and guide pages keep their layout. They get the new tokens and the shell.

## Acceptance criteria

- [ ] `apps/web/components.json` exists, and the global CSS uses the shadcn variable names.
- [ ] `apps/web/DESIGN.md` exists in our own words, with tokens, type, color roles, status colors and the sidebar.
- [ ] Each text and background pair of the tokens reaches 4.5:1 in light and dark (unit test over the token values).
- [ ] The sidebar reaches `/`, each `/p/<id>`, each `/p/<id>/roles`, each `/p/<id>/memory`, the minutes list of each product and `/guia` in 1 click.
- [ ] `navState` marks the right item for `/`, `/p/agentsi`, `/p/agentsi/roles`, `/p/agentsi/memory`, a minutes page, a call page and `/guia` (unit test).
- [ ] `healthStyle` has one label, one token and one icon for each `Health` value (unit test).
- [ ] `/logros`, `AgencyBadge`, `Party`, the streak and the achievements are gone. `rg -i "logros|streak|achievement|AgencyBadge|Party" apps/web` finds nothing outside `node_modules`.
- [x] "Te toca" is the first block on the home, and its first card shows above the fold at 1280×800.
  Evidence: `docs/reviews/web-smoke-demo.md`, row 1 at 1280×800, first card top 172px in light and dark.
- [ ] The minutes page shows the summary before the tabs. Priorities and role answers fold. `#decisiones` opens the Decisiones tab.
- [ ] The web has no new server action, and `finishCall` is still its only action (the read-only test passes).
- [x] Light and dark render on the routes of `docs/ideas/agentsi-web-ui-eval.md` rows 1, 2, 8, 18, 19, 20 and 21 with no console error.
  Evidence: `docs/reviews/web-smoke-demo.md`, rows 1 to 21 on the demo data, 0 console errors in light and dark at both viewports.
- [ ] Typecheck, lint and the web tests pass.
