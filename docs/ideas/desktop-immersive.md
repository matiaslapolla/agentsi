# Immersive desktop app

**Status:** shaped · **Date:** 2026-11-09 · **AI level:** Level 0, no AI

## Idea

The owner wants the desktop app to show the work of the agency without a visit to each screen. The app adds motion, notification sounds, a live card for each agent, and alerts (a glowing border, a toast, a log row) in the color of the state. Each part reads events that the core or the app already has.

## Problem

- **Who:** the owner, with many panes, meetings and story sessions open.
- **Today:** a waiting pane shows a static marker, and its tab shows nothing (`crates/agentsi-app/src/tab_bar.rs:481`). The app plays no sound. Thus the owner finds a waiting pane or a ready story only when they look.

## Data it reads

- Core events: `terminal.state`, `terminal.exit`, `meeting.progress`, `meeting.event`, `meeting.render`, `models.*` and `data.changed`, and reads of `story.list` and `product.desk`.
- `voice.speak` and `converse.state`, after the app subscribes to `voice`.
- App sources: the bell of a pane, the call player and the host links.

`desktop-immersive-eval.md` holds the 33 events, the state map, the design, the findings and 9 questions for the owner.

## AI level

Level 0, no AI. A fixed table maps each event to a state, and each state to a color, an icon, a word, a sound and a motion.

## Pass/fail check

A test replays a meeting with a failed check, a story session that becomes ready, and a waiting pane. It passes when each alert matches the state map and has an icon and a word. Then the idle bench must report 0 frames within 10 s of the last alert.

## Conditions to stop

1. If the median time from `blocked` to the next input does not fall by 30 % by 2026-12-07, stop the sounds and the pulse.
2. If the idle bench is not at 0 frames 10 s after the last alert by 2026-11-23, remove the pulse.
3. If the owner mutes all sounds or turns off motion before 2026-12-07, stop that part.

Cheapest test (1 day): only the waiting pane gets its glow and one sound. If the owner turns it off that day, stop the idea.

## Open tests

- The baseline of condition 1.
- Can 3 pulses at 30 frames each second return the idle bench to 0 frames?
- How long does `cpal` take to open the output on ALSA and on Core Audio?
- Record one real meeting and one story session as test inputs.

## Next step

`prototype`: measure the baseline. Then answer: do a glow and one sound on a waiting pane cut the time to the owner's input?
