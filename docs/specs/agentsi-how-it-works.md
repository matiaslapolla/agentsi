---
status: done
date: 2026-10-21
---

# How it works page

## Problem

The guide shows the commands one by one. A new user cannot see how the pieces connect. A new user also cannot see what AgentSI reads and writes in a product repo. The open-source release needs this view.

## Decision

- The web app gets the page `/como-funciona`, with the item "Cómo funciona" in the sidebar under "Guía".
- The page explains AgentSI with five diagrams: the pieces, the product repo, a meeting, a story and the learning loop.
- Each diagram is inline SVG in React. It uses the theme tokens, so it works in light and dark mode.
- The text is in Spanish and follows STE. Each diagram has a short caption with the rule that it shows.
- The page uses no product data. It uses a sample product named "miapp", as `agentsi.config.example.json` does.
- The page is static. It reads no registry, workspace or database.

## Out of scope

- Animation beyond a subtle flow on the arrows. The page respects `prefers-reduced-motion`.
- Changes to the generated guide in `docs/guide/`.

## Acceptance criteria

- [x] `GET /como-funciona` returns 200 and shows the five diagrams with their captions.
- [x] The sidebar shows "Cómo funciona", and `navState("/como-funciona")` marks it.
- [x] The diagrams read well in light and dark mode, and at 390 px wide with no horizontal page scroll.
- [x] The page names no real product, person or path of the owner.
- [x] Each fact on the page matches the code: the sources of the index, the steps of a meeting, the steps of a story and what is written in the product repo.
- [x] Typecheck, lint and the web tests pass.
