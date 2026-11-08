# AgentSI desktop: hearing mode

**Status:** approved 2026-11-07 · **Source:** owner decisions of 2026-11-07 (log "Owner decisions after the first owner checks")

## Problem

Dictation works only while the owner holds the key, and one recording stops at 120 s. The owner cannot talk to a Claude session with no hands.

## Goal

- **Toggle dictation:** one press starts the dictation and the next press stops it. The owner speaks freely between the two presses, with no 120 s limit.
- **Conversation mode:** the owner speaks to one Claude pane with no hands. Each spoken turn goes to the pane, and the PM voice speaks the reply.

## Non-goals

- macOS. U18 stands by.
- Text typed into windows that are not AgentSI panes. Dictation keeps its targets: the focused pane, else the clipboard.
- Barge-in. The owner cannot interrupt the voice by speaking. A press of the key or a click on the pill stops the voice.
- A wake word.

## Terms

- **Toggle dictation:** dictation that starts on one key press and stops on the next press.
- **Conversation mode:** a state of the pill in which it listens, sends each turn to one pane, and speaks the reply.
- **Turn:** the speech between the start of the speech and a silence of the end-of-turn length.
- **Voice detector:** the model that marks each 32 ms frame of audio as speech or no speech.

## Design

### Voice detector

- Silero VAD v5 (MIT) runs on `ort` in `agentsi-voice::vad`. It is a new catalog entry, `silero-vad-v5`, with a pinned revision and hash, about 2 MB. The pill loads it with Parakeet.
- It reads 16 kHz audio in frames of 512 samples and gives a speech chance for each frame. A frame above 0.5 counts as speech.
- A test gives the detector a known speech clip and a silent clip, and checks the marks.

### Toggle dictation

- The settings section "Dictation key" gets a choice: "Hold to dictate" (default, as today) or "Press to start, press to stop". `keys.toml` keeps it as `[dictation] mode = "hold" | "toggle"`.
- In toggle form, the bind file runs `agentsi-core dictate toggle` on the press and has no release listener. The core sends `dictation.start` or `dictation.stop` from the state of the pill. A click on the pill also stops.
- While it listens, the pill cuts the audio into pieces at each silence of 0.6 s or more. It transcribes each piece at once and sends its text with `voice.transcript`. A piece longer than 30 s is cut at its quietest frame. So the text arrives during the speech, and no limit applies.
- Each piece after the first starts with a space. The vocabulary applies to each piece.
- After 10 min with no speech, the pill stops the dictation and shows "Dictation stopped: no speech for 10 min".

### Conversation mode

- The palette entry "Talk to this pane", a context menu entry of a pane, and an optional key start it. The key is a second row in the section "Dictation key", "Conversation key", in the same bind file. A press starts the mode, and the next press stops it.
- The target is the pane that has the focus at the start. A later focus change does not move it. A start with no focused pane shows "Focus a pane first" on the pill.
- At the start, the core turns on voice mode for that pane (`voice.mode`), so the Stop hook speaks the reply. At the stop, the core puts voice mode back as it was.
- The pill opens the microphone and shows "Listening". A turn ends after 1.2 s of silence. The pill transcribes the turn, the core applies the vocabulary and sends the text with `terminal.input` and `submit = true` to the target pane, and the pill shows "Waiting".
- While the pane works and while the voice speaks, the pill ignores the microphone, so the voice does not hear itself. After the last sentence of the reply, it listens again.
- A turn with no text sends nothing. A turn of a single word "stop" or "para" ends the mode.
- The mode stops when the target pane closes, when the core stops, on a click on the pill, on the key, and after 10 min with no turn.
- New protocol parts, all additions: `voice.converse {action: "start" | "stop" | "toggle", terminal?}`, the event `converse.state {state, terminal}`, and `dictate toggle` with `voice.dictate {action: "toggle"}`. The owner reads them before the merge.

### Pill

- New states: "Waiting" in conversation mode, and a small mark that shows toggle dictation or conversation mode, so the owner knows that the microphone is open.

## Acceptance criteria

- [ ] The voice detector test passes on a speech clip and a silent clip.
- [ ] A test feeds 3 min of speech with pauses to toggle dictation and gets the text in pieces, in order, with no piece over 30 s.
- [ ] A test of conversation mode with a fake pane sends one submit for each turn, ignores audio while the voice speaks, and puts voice mode back at the stop.
- [ ] The word "stop" alone ends conversation mode, and a turn with no text sends nothing.
- [ ] `schema/core.openrpc.json` changes only by the additions in this spec.
- [ ] The idle app draws 0 frames, and the pill draws 0 frames while it waits.
- [ ] The owner dictates for more than 2 min in toggle form, and talks to a Claude pane for 3 turns in conversation mode.
