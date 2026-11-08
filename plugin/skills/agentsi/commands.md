# AgentSI subcommands and desktop app

Read from the front desk when the owner types `/agentsi <subcommand>` or wants the desktop app.

## Desktop app and call

The web app is gone. The desktop app, `agentsi-app`, takes its place. `agentsi web` starts `agentsi-app` and prints one line that says so. It prints no URL and starts no server. If it cannot find `agentsi-app`, it says where to put it. Use it when the owner wants to see the minutes, listen to the call, or audit the memory.

The app has a screen for the front desk, the minutes, the call, the stories, the roles, the memory and the meeting. The owner opens a screen from the sidebar or from the command palette (`Ctrl+K`). `Ctrl+Shift+L` opens the minutes, `Ctrl+Shift+P` the call, `Ctrl+Shift+S` the stories, `Ctrl+Shift+R` the roles and `Ctrl+Shift+Y` the memory. A screen of the app can also write. Approve and Reject of a decision, a mark and Approve of a story write at once. Retire and Revert of the memory, and Changes, Discard, Run, Accept and Reject of a story first show what they will do, and write after a click on Confirm (ADR 0002). A click that writes is the owner's word, as `approve <id>` in the front desk is.

`agentsi render <minutes-id>` makes the call audio for a meeting that has none.

## Subcommands

When the owner types `/agentsi <subcommand>`, skip the greeting of section 1 of SKILL.md (still resolve the product with `agentsi whoami`) and do this:

- `/agentsi setup`: read `setup.md` and follow it. It configures the roster and the brief of the product.
- `/agentsi audit`: run `agentsi audit --product <id>`. Explain it in plain words: config problems, meeting quality, marks, memory. If "Listo para reflexionar" lists agents, offer `/agentsi reflect`. If it says "no hay evidencia suficiente", say so and stop.
- `/agentsi story`: follow section 6 of SKILL.md. Without more words, run `agentsi story list --product <id>` and show it.
- `/agentsi mark`: follow step 4 of section 3 of SKILL.md for the pending items of the product. If the list is empty, say so and stop.
- `/agentsi roles`: run `agentsi roles --product <id>` and show the roster. With an agent id, run `agentsi roles <agent>` and show its effective prompt. With `validate`, run `agentsi roles validate` and list the failures.
- `/agentsi reset <agent>`: say what the reset removes (the overlay of that agent, and its memory only with `--memory`). Wait for the owner's yes. Then run `agentsi reset <agent> --yes` (add `--memory` only if the owner asked for it). `/agentsi reset --all` also needs the owner's yes: it drops the brief, overlays, specialists and custom roles.
- `/agentsi reflect`: read `reflect.md` and follow it.
