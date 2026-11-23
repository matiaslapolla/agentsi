# Product — AgentSI

**Stage:** building · **Updated:** 2026-11-23

## What it is
AgentSI is an agency of role agents for the products of the owner. The roles read a product, meet and rank what to do next. The owner approves each change. AgentSI is a native Linux app, a core daemon, a command line client and a Claude Code plugin.

## Audience
- The owner is the first audience and the only developer.
- Devs who see AgentSI on X are the second audience. They have no objective.

## Vision
The owner opens a product, and AgentSI says what to do next and why. The owner approves, and the roles do the work. The means is one place for each product: terminals, git, tickets, meetings and voice.

## Objectives
The `due` of each objective is the date of the monthly review. The data is in `agentsi.db` of the data root. The queries use the default data root, `~/.local/share/agentsi/data`. Each query below is for December 2026. For another month, change `2026-12`.

1. **The app is the daily workplace** (`o1`).
   - Sections of `ROADMAP.md`:
     - UI polish,
     - File management and day-to-day tasks,
     - More git,
     - The switch-over.
   - Signal 1: 20 days or more in each month with an owner message at the front desk of any product. The days are UTC days. The query gives the number of days:

     ```sh
     sqlite3 ~/.local/share/agentsi/data/agentsi.db \
       "select count(distinct substr(at, 1, 10)) from desk_items
        where kind = 'owner' and substr(at, 1, 7) = '2026-12'"
     ```

   - Signal 2: the outside list under `## Review` gets shorter at each review. The owner counts its items.
   - Review 2026-12-31.
2. **The roles know the product and are correct** (`o2`).
   - Sections of `ROADMAP.md`:
     - Context injection into agents,
     - More audits,
     - the issue trackers and the docs of Third-party integrations.
   - Signal: 75% or more of the marks of the month are `accepted`. The value 75% is a guess. The month of a mark is the month of its minutes. The query gives the share in percent:

     ```sh
     sqlite3 ~/.local/share/agentsi/data/agentsi.db \
       "select round(100.0 * sum(p.mark = 'accepted') / count(*), 1)
        from priorities p join minutes m on m.id = p.minutes_id
        where p.mark is not null
          and substr(m.created_at, 1, 7) = '2026-12'"
     ```

   - Review 2026-12-31.

## Later
These sections of `ROADMAP.md` have no objective. They come later:
- VPS setup.
- Cost monitoring and optimization.
- Claude Agent SDK on the Claude subscription.
- Other agent harnesses.
- More connectivity: more devices.
- Third-party integrations: chat and calendars.

Owner override: the owner's own top 3 was Cost monitoring, Context injection and the Claude Agent SDK. Only Context injection has an objective. The owner kept objective 1 and objective 2 anyway, on 2026-11-23.

## Out of scope
- Platforms other than Linux, such as macOS and a mobile app.
- A remote client. Claude Code remote control covers it.
- Any action without the approval of the owner.

## Review
The owner does the review once a month, on the review date of the objectives. The owner checks these items:
1. The days with an owner message, from the query of objective 1.
2. The number of items of the outside list, against the last review.
3. The share of `accepted` marks, from the query of objective 2.
4. The number of meetings on AgentSI:

   ```sh
   sqlite3 ~/.local/share/agentsi/data/agentsi.db \
     "select count(*) from minutes where product = 'agentsi' and status = 'valid'"
   ```

After the review, the owner moves the review date of each objective to the last day of the next month. The owner applies the new date with `/agentsi setup` in update mode.

The owner stops this north star and writes a new one in one of these cases:
- AgentSI has fewer than 2 meetings by 2026-12-31.
- Less than 50% of the marks are `accepted` at the review of 2026-12-31.

Outside list, on 2026-11-23. The items are a first guess from `ROADMAP.md`. The owner corrects them at the first review.
- Push, pull and fetch in a terminal.
- Make and switch branches, and read the history, in a terminal.
- Open and review pull requests on GitHub.
- Browse and open the files of a product in an editor.

## Sources of truth
- `ROADMAP.md`: the sections that the objectives cover.
- `docs/ideas/agentsi-north-star.md` and `docs/ideas/agentsi-north-star-eval.md`: the decisions and the map of the sections.
- The issues of `matiaslapolla/agentsi` on GitHub.
- The tables `desk_items`, `minutes` and `priorities`: the signals.
