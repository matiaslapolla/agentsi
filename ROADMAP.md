# Roadmap

AgentSI is open source and free forever. It is made in Argentina. This file lists the work that the owner plans. The order can change. Issues and pull requests are welcome.

## UI polish

- Refine the spacing, the motion and the empty states of each screen.
- Check each screen for contrast and keyboard use.
- Add more choices to the appearance settings.

## Third-party integrations for work tasks

- Issue trackers, so the roles read and update the issues of a product.
- Chat, so the owner gets the minutes and the pending decisions where the team talks.
- Calendars, so a meeting can have a place in the day of the owner.
- Docs, so the roles read the documents of a product where the team keeps them.

## VPS setup

- Run the core on a server, so meetings and stories continue while your laptop is off.
- Make the setup of a new server one command.
- Improve the screens of an SSH host. The app already connects to the core of an SSH host.

## Cost monitoring and optimization

- Show the tokens and the plan use of each role.
- Show the same numbers for each meeting and each story.
- Suggest a cheaper model or a shorter context where the cost is high.

## Context injection into agents

- Give a running agent the context of the product: the brief, the objectives and the knowledge base.
- Give a running agent its memory and the decisions of the owner.
- Let the owner see what each agent received.

## Claude Agent SDK on the Claude subscription

- Run the roles through the Claude Agent SDK with the Claude subscription of the owner.
- Use Claude as a provider in the same way that OpenCode does.

## Other agent harnesses

- Add Codex as a harness.
- Add OpenCode as a harness.
- Add more harnesses. The roles, the meetings and the stories must work the same on each one.

## File management and day-to-day tasks

- Browse and open the files of a product in the app.
- Do small daily tasks from the app, without a terminal.

## More git

- Add push, pull and fetch to the git panel.
- Add branches, diffs between branches and a view of the history.
- Add pull requests.

## More audits

- Audit the memory of each agent.
- Audit the cost, the sources and the failed checks of each meeting.
- Audit the stories that the agents merged.

## More connectivity

- Connect more devices to the same agency.
- Control the agency from a remote client.
- Add a mobile app.

## macOS and the switch-over

- Port the app to macOS.
- Run the switch-over: the Rust programs replace the TypeScript app, and the repo drops `apps/` and `packages/`.

## And much more

The list above is a start. If you have an idea, open an issue.

Everything stays open source and free forever. Made in Argentina.
