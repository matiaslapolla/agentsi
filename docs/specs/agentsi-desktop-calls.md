# AgentSI desktop: calls

**Status:** built in U26 · **Source:** `docs/specs/agentsi-desktop-parity.md` (the row `render`, the route of the call, "Unknown: the audio format of calls"), `packages/voice/src/agentsi_voice/render.py`, `packages/cli/src/commands/render.ts`, `apps/web/components/call-player.tsx`, `apps/web/lib/inbox-data.ts`

## Problem

A meeting can end as a call: each role speaks its part with its own voice. Today the Python package `packages/voice` renders it with `ffmpeg`, and the web app plays it. The desktop build has no Python and no web app. It must render a call, play a new one, and still play each call that the owner made before.

## Goal

- The core renders the call of stored minutes with Kokoro, one voice for each role, and writes the audio and the times of each segment.
- The Call screen plays old and new calls, shows the speaking role and keeps the listened mark.
- `agentsi render <minutes-id>` and `agentsi meet --audio` call the core.

## Non-goals

- Piper and Chatterbox voices. A role with another engine in `voices.toml` falls back to the default Kokoro voice.
- Streaming playback of a call while it renders.
- Audio from the browser. The web app stays until the switch-over and plays only `call.webm`.

## Terms

- **Call**: the folder `<data root>/workspace/<product>/audio/<minutes id>/`. It holds the audio file and `timings.json`.
- **Old call**: `call.webm`, Opus in WebM, written by the Python package.
- **New call**: `call.opus`, Ogg Opus, written by the core.
- **Listened mark**: one row of the table `listens` for each call that the owner heard.

## Design

### The audio format (decision)

A test on one call of 37.53 s of synthetic text, Kokoro voices, 24 kHz mono:

| Format | Size | Encode | Decode | Needs |
|---|---|---|---|---|
| WAV, 16 bit | 1,801,294 B | none | none | nothing |
| FLAC (`flacenc` 0.5.1) | 1,032,494 B (57 %) | 8.5 ms | pure Rust | an encoder crate. In the test it dropped the last 3,601 samples |
| Opus 32 kbit/s VoIP (libopus) | 146,888 B (8.2 %) | 283 ms | 11 ms for 9.4 s | libopus (C, BSD-3-Clause, built from source with cmake) |
| WebM from the Python package (Opus 32 kbit/s) | 153,398 B | 19 s with the voice | see Opus | `ffmpeg` |

Decoders of an old `call.webm`, on a real file of 9.36 s that the Python package wrote:

| Decoder | Match with `ffmpeg -c:a libopus` | Time |
|---|---|---|
| libopus (`opusic-sys` 0.7.7) | 79.4 dB | 11 ms |
| pure Rust (`opus-decoder` 0.1.1) | 64.5 dB | 936 ms (10 times real time) |
| native decoder of `ffmpeg` | 15.8 dB against libopus | not measured |

The format is **Ogg Opus, 24 kHz, mono, 32 kbit/s, VoIP**, the same codec and settings as the Python package. A new call has 8 % of the size of WAV, so a call of 5 minutes takes about 1.2 MB. libopus reads both Opus files. The core links no `ffmpeg`. The pure Rust decoder is exact enough but 86 times slower, so it stays out. The cost is a C build step: `cmake` must be present when the core builds.

### Files

- `call.opus` is written whole or not at all. `timings.json` has the shape of the Python package: an array of `{idx, role, text, start, end}` with 3 decimals, 2 spaces of indent and one trailing newline.
- When a call has both files, `call.opus` wins. The core never deletes `call.webm`.
- A name of a product or of minutes that is not `[A-Za-z0-9_][A-Za-z0-9._-]*`, or has `..`, is not found.

### Render

- `call.render` reads `script` of the stored minutes, `[{role, text}]`. An empty script, or a segment with no role or no text, is `invalid`.
- Each segment is spoken with the voice of its role (`voices.toml`), sentence by sentence, with 0.2 s between sentences. The whole segment is leveled once to an RMS of 0.08. 0.35 s of silence separates two segments, as in `render.py`.
- Difference to the Python package: the voice levels each sentence first (U12), then the segment. Python levels only the segment. The sound is close, not equal. The Python retake step for Chatterbox is not ported.
- A render takes minutes on a CPU. It runs off the event loop. `meeting.render` events tell `start` (`of` segments), `segment` (`done` of `of`), `encode` and `done` (`note` holds the folder, or `error`). `call.get` shows the running render with its progress, so a screen that has no event feed can read it.
- `call.cancel` stops a render before its next segment. A canceled render writes nothing and the old files stay. A core that stops cancels each render, as it does for meetings.
- A call renders once at a time. A second `call.render` of the same call is `conflict`.
- Without the Kokoro model or `espeak-ng`, `call.render` is `unavailable` with the hint of the settings screen. It writes nothing.

### Listened mark

It ports `recordListen` and `removeListenRecord`:

- `call.heard {heard: true}` stores a row `(product, minutes_id, at)`. A second mark changes nothing and keeps the first time. A call with no audio file cannot be marked: the answer is `changed: false`.
- `call.heard {heard: false}` removes the row of that call and no other.
- A call that plays to its end gets the mark.
- A change sends `data.changed {product, kind: "call"}`.

### Protocol

`agentsi-proto::call_api`. The schema is `schema/core.openrpc.json`.

- `call.get {product, minutes_id}` returns the question, the segments, `timed`, `audio {format, bytes}`, `heard` and `render {render, done, of}` while a render runs. Without `timings.json` the segments are the script with times of 0.
- `call.audio {product, minutes_id, from, count}` returns up to 2 minutes of samples as 16-bit little-endian integers at 24 kHz, in base64, and the samples of the whole call. The core decodes the file at each request, which takes about 10 ms for each 10 s of audio.
- `call.render {product, minutes_id, render?}`, `call.cancel {render}` and `call.heard {product, minutes_id, heard}`.
- The event is `meeting.render`, so a subscriber of the topic `meeting` gets it.

### CLI

- `agentsi render <minutes-id>` finds the product of the minutes, renders, prints the folder and writes the progress on stderr. `Ctrl+C` cancels the render. The messages of the errors are those of the TypeScript CLI.
- `agentsi meet --audio` renders after the minutes are stored. A failed render prints the `WARNING: audio failed, minutes were kept:` line and keeps the minutes.

### Call screen

- It opens with `Ctrl+Shift+P` on the open minutes, and from the palette. The panel title is `Call`, the argument is the minutes id.
- It shows the question, the id, the state, the file format, the speaking role (a circle for each role of the call, lit for the one that speaks), the figures and the segments. The segment that plays has the accent border. A click on a segment plays from its start.
- Buttons: Play or Pause, Previous, Next, Speed (1, 1.25, 1.5, 2), Mark as listened or Remove the mark, Render the call or Render again, Cancel the render, Open the minutes.
- Keys of the focused screen: `Space` plays or pauses, `Left` and `Right` move between segments, `L` flips the listened mark. The palette has an entry for each action.
- The samples load when the owner first plays. The device opens at the first play. An open screen costs one `call.get` and draws no frame while idle. A playing call repaints each 50 ms.
- A call with no timings cannot play. A call with no audio shows its script and the Render button.

## Acceptance criteria

- [ ] `cargo test -p agentsi-agency --test call_old` passes: a real `call.webm` of the Python package decodes to what `ffmpeg` decodes, its pauses fall on the pauses of `timings.json`, and it can be marked as listened.
- [ ] `cargo test -p agentsi-core --test call` passes: the old call plays through the core, a render with Kokoro writes `call.opus` and `timings.json`, and a cancel writes nothing.
- [ ] `cargo test -p agentsi-cli --test golden` passes, with the five `render-*` cases that the TypeScript CLI wrote.
- [ ] The Call screen plays an old `call.webm` and a new call. The tests of the screen use an output that makes no sound.
- [ ] `agentsi meet --audio` calls `call.render` and keeps the minutes when the render fails.
- [ ] `rg -n 'rgb\(|hsla\(|px\(' crates/agentsi-app/src` finds nothing.
