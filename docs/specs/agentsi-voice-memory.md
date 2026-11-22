# AgentSI voice: the models stay in memory only as long as the owner wants

**Status:** approved 2026-11-22 · **Source:** owner report of 2026-11-22 ("agentsi usa 1.6gb de memoria, eso tiene que reducirse a mínimos"; "permitir configurar esto en las settings")

## Problem

`agentsi-pill` loads the speech model (parakeet, 640 MB on disk) and the voice model (kokoro, 311 MB) when it starts, and it keeps them for its whole life (`crates/agentsi-pill/src/main.rs`, `speak.rs` `run`, `dictate.rs` `run`). Its resident memory is 1.68 GB on the M1, also when the owner never speaks or dictates. The core uses 26 MB and the app 117 MB.

## Goal

- An idle pill uses as little memory as the owner chooses.
- The owner chooses in Settings how long the voice models stay in memory after their last use.
- Dictation stays as fast to start as now.

## Non-goals

- A smaller or another model.
- A change of the memory of the app or of the core.
- A new download flow for the models.

## Terms

- **Voice models**: the speech model (dictation, parakeet), the voice detector (silero) and the voice model (speech out, kokoro).
- **Keep time**: how long a voice model stays in memory after its last use.
- **Load on use**: a voice model loads at its first use, not at the start of the pill.

## Decisions

- D1. Each voice model loads on use. At its start the pill loads no voice model.
- D2. The owner picks the keep time in Settings, section "Voice models": "1 minute", "5 minutes", "15 minutes", "1 hour" or "Always". "Always" keeps a model once it loaded. The default is "5 minutes".
- D3. One more choice, "Load at start", keeps today's behavior: the models load when the pill starts and stay. It is for an owner who wants the first word with no wait.
- D4. The speech model starts to load when a dictation starts. The microphone records at once into the buffer, and the transcription waits for the model. So a dictation never loses audio, and the wait shows only at the end of a short dictation.
- D5. The voice model loads when the first speech job arrives. The speech starts when the model is ready (about 1.2 s on the M1).
- D6. The pill frees a model when the keep time passes with no use. A model in use is never freed.
- D7. A change of the keep time applies at once, with no restart of the pill.

## Design

### Pill

- `speak.rs` and `dictate.rs` keep each engine as an `Option`. The worker thread loads it when a job needs it and drops it when the keep time passes since the last job.
- The worker waits with `recv_timeout` until the next drop time, so an idle pill wakes once for each drop and never polls.
- The silero detector is small (2 MB). It loads and drops with the speech model.
- Check with `/proc/self/status` (`VmRSS`) in a test or a bench that a drop gives the memory back. ONNX Runtime can keep an arena: if a drop does not give memory back, set the session options that disable the arena (`with_memory_pattern(false)` and the CPU arena allocator off), and measure again. Report the numbers.
- The logs keep their lines "voice ready in …" and "speech in ready in …", and add one line for each drop.

### Setting

- The keep time lives in `pill.toml`, as `[models] keep = "5m"`. Valid values: `"1m"`, `"5m"`, `"15m"`, `"1h"`, `"always"`, `"start"`. A missing value is `"5m"`. A bad value gives one clear line, and the pill uses `"5m"`.
- `Config` of `crates/agentsi-pill/src/config.rs` reads it. Keep `deny_unknown_fields`, and add the new table.
- The app writes the value through the core. Use the path that a live setting of the pill already takes from the app, if one exists. Else add `voice.models.keep {keep}` to the core: the core writes `pill.toml` with `toml_edit` (keep the other keys and the comments) and sends the value to the pill over its link, so D7 holds.

### App

- Settings, section "Voice models", gets the choice "Keep the models in memory" with the six values of D2 and D3, as a select or as a row of buttons. All colors and sizes come from `agentsi-ui` tokens.
- Under the choice, one line shows the memory of the pill now, from `/proc/<pid>/status` of `agentsi-pill` (`VmRSS`). It updates when the section opens and after a change, not on a timer.

## Acceptance criteria

- [ ] With the keep time "5 minutes", a new pill holds no voice model: a test checks that no engine opens at the start.
- [ ] The first dictation loads the speech model, and no audio of that dictation is lost. A test with a fake microphone and a slow fake recognizer checks it.
- [ ] The first speech loads the voice model and speaks the whole text.
- [ ] With a keep time of 1 minute, a model with no use for 1 minute is dropped. A test with a fake clock checks it, and checks that a model in use is not dropped.
- [ ] "Always" keeps a model after it loaded. "Load at start" loads the models at the start.
- [ ] A change in Settings applies with no restart of the pill.
- [ ] `pill.toml` keeps its other keys after the app writes the keep time. A bad value gives one line and the default.
- [ ] The Voice models section shows the choice and the memory of the pill.
- [ ] The memory of an idle pill after the drop is under 150 MB on the M1. The report gives the numbers before and after.
- [ ] An idle pill wakes only at a drop time, and never on a fixed timer.
- [ ] Owner check: dictation and the pm voice still work on the M1, and the first dictation after a drop feels as fast as before.
