---
name: voice
description: Turn voice mode on or off for this Claude Code session. With it on, each reply is spoken with the pm voice (only the short prose above the "On screen only" line) and each turn is saved to a Markdown transcript in the AgentSI data folder. Use on "/voice", "voice on", "voice off", "talk to me", "stop talking", "activá la voz", "apagá la voz", "hablame".
---

# Voice mode

Voice mode belongs to one Claude Code session. Inside a pane of the AgentSI app, it belongs to that pane. Other sessions, such as meetings and story sessions, never speak.

1. Read the argument: `on`, `off`, or none. With none, run `agentsi voice status` and switch to the other state.
2. Run `agentsi voice on` or `agentsi voice off` with Bash. Use one plain command per Bash call.
3. Say in one short sentence whether voice mode is now on or off. When you turn it on, add that details stay on screen below the "On screen only" line, and give the transcripts folder that the command prints. If the command also prints a warning that `agentsi-pill` is not running, say so and give `systemctl --user enable --now agentsi-pill`.
4. If the owner hears nothing, run `agentsi doctor`. It reports `espeak-ng`, the voice models and the audio devices. Then check that `agentsi-pill` runs, because the pill speaks. The hook log is `logs/hook.log` in the AgentSI data folder.

Voice mode applies from the next prompt.
