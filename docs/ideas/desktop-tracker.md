# Desktop tracker

**Status:** shaped · **Date:** 2026-11-09 · **AI level:** 0, no AI

## Idea

The owner and Claude Code sessions keep the notes, to-dos and open questions of a product in AgentSI. The desktop app shows them on a Tickets screen. Claude reads and writes them through MCP tools, and the roles see the open ones in each meeting.

## Term

Term in `CONTEXT.md` (owner's choice, 2026-11-09):

```markdown
**Ticket**:
One note, to-do or question of a product that the owner or Claude writes and the owner closes. It gates nothing and runs nothing. A ticket that becomes work turns into a story.
_Avoid_: Card, issue, item, entry, task (when you mean a ticket)
```

Story no longer lists "Ticket" in its _Avoid_ line.

## Problem

- **Who:** the owner, and each Claude Code session inside a product.
- **Today:** a session keeps its open ends in `.claude/JOURNAL.md:3-8`, and the next session rewrites them. The synthesis never sees the journal (`crates/agentsi-agency/src/prompt_inputs.rs:113-121`).

## Data it reads

A new `tickets` table in `agentsi.db`, its links to stories, decisions, minutes and priorities, and the product of the folder (`crates/agentsi-cli/src/mcp.rs:48-80`).

## AI level

Level 0. Each read and write is a core method and a SQL query. Claude is only a client.

## Shape

The eval file holds each part:

- Core methods and four MCP tools.
- A `## Open tickets` snapshot section and a source type `ticket`.
- A Tickets screen, a Ticket screen and menu entries.
- The line between a ticket and a story, a decision and a priority.
- A later GitHub Issues sync through links of type `issue`.

## Pass/fail check

A session in product P calls `add_ticket`. In 1 s or less, the Tickets screen of P shows the ticket with that session as author. The next `data.snapshot` of P lists it.

## Conditions to stop

1. The owner adds fewer than 10 tickets in the first 14 days.
2. The owner drops more than 50% of the session tickets in those 14 days.
3. No priority cites a ticket in the first 5 meetings. This condition removes only the snapshot section.

Cheapest test (1 day): the owner and the sessions write open ends to `.claude/tickets.md`. If the owner keeps fewer than 5 lines, stop.

## Open tests

- Does one day give 5 or more lines that the owner keeps?
- Does `agentsi mcp` get `CLAUDE_CODE_SESSION_ID` for the author (`docs/specs/agentsi-desktop-protocol.md:153`)?
- Do the golden tests stay equal when an empty `## Open tickets` section stays out?
- Collect real inputs: 6 seed cases exist, fewer than 20.

## Next step

Prototype the first open test: one day of `.claude/tickets.md`, then count the kept lines.
