# AgentSI desktop: voice in, voice out and the pill

**Status:** approved 2026-11-02 · **Source:** `docs/specs/agentsi-desktop.md`, P1 (`prototypes/gpui-pill/`), `plugin/hooks/voice-mode.mjs`, `docs/research/agentsi-voice-options.md`

## Problem

Voice mode works only on local Linux. It needs Python, a Quickshell pill and audio tools. The owner has no dictation into a terminal of AgentSI.

## Goal

- The pill speaks each reply of a pane that has voice mode on.
- The owner holds a hotkey, speaks, and the text goes into the focused pane.
- Each model downloads from the settings screen, with a hash check.
- Voice works with no Python. M1 runs on Linux. macOS follows in M2.
- A custom vocabulary fixes the product names that every model mishears.

## Non-goals

- Piper and Chatterbox voices.
- Dictation into windows that are not AgentSI panes.
- Voice from an SSH host in M1. M2 forwards it.

## Terms

- **Voice mode**: a switch on one pane. When on, the pill speaks the replies of its Claude session.
- **Vocabulary**: `<config>/agentsi/vocabulary.toml`, an owner-editable list of terms. The core applies it to each transcript.
- **Model catalog**: `crates/agentsi-models/models.toml`, the models that the app can download.

## Design

### The pill process

- `agentsi-pill` is a client of the local core and subscribes to `voice.*` and `dictation.*` events.
- On Linux it uses `WindowKind::LayerShell` on the overlay layer, top center, with no keyboard focus (P1).
- On macOS (M2) it uses `WindowKind::PopUp`, a nonactivating panel. P1 did not run on macOS yet.
- States: hidden, listening, transcribing, speaking, and error (one line for 3 s). A click stops the speech or the recording.
- `cpal` gives the audio input and output.

### Model downloads

These rules copy Hex (`src/net/download.rs`, `src/transcription/models.rs`, MIT):

- Each catalog entry has an id, a license, and for each file a path in a pinned Hugging Face repo and revision (which give the URL), a size and a sha256 hash.
- The download goes to a `.partial` file and continues after a stop with an HTTP `Range` request.
- After the last byte, the code checks the hash. A correct file moves into place. A wrong file is deleted, and the error shows.
- Models go to `~/.local/share/agentsi/models/catalog/<id>/` on Linux (`$XDG_DATA_HOME` replaces `~/.local/share` if set) and to `~/Library/Application Support/agentsi/models/catalog/<id>/` on macOS. The folders `models/kokoro/`, `models/piper/` and the others belong to the Python voice code until the switch-over, and the app never writes there.
- The download never replaces or deletes a file that it did not write. A file with the name of a catalog file and another hash stops the download with a `conflict` error. A file with the right size and hash counts as downloaded.
- The settings screen shows each model with its size, license and state, with download and delete.

### Speech in

- The model is Parakeet TDT v3 int8 on `ort`, by the owner's decision of 2026-11-04. The owner dropped the recordings of open test 2, so the choice rests on the synthetic dry run. The reference for quality is Hex. No model hears product names, so the vocabulary is required.
- ONNX Runtime runs the model through the `ort` crate, with an exact version pin. CPU is the default. CUDA and CoreML are optional features.
- `agentsi-voice::stt::Transcriber` turns 16 kHz mono samples into text. It runs four files of the catalog entry `parakeet-tdt-v3-int8`: `nemo128.onnx` (audio features), the int8 encoder, the int8 decoder and joint network, and `vocab.txt`. The decoder is greedy TDT: each step takes the best token and the best duration, as onnx-asr 0.12.0 does (MIT, in `NOTICE`). It uses half the logical CPUs, with no thread spinning, like the voice. A clip of less than 0.2 s gives no text. A clip of more than 120 s is refused.
- The pill resamples the microphone to 16 kHz with a windowed-sinc filter, so a 48 kHz device does not alias into speech.
- On Linux, a compositor bind runs `agentsi-core dictate start` on key press and `agentsi-core dictate stop` on release, because a Wayland client cannot read a global key. The owner picks the key in the settings screen, section "Dictation key". AgentSI writes the bind file, and the owner adds the include line to the Hyprland config one time (`agentsi-desktop-keys.md`). `CONTRIBUTING.md` gives the hand lines for each form of the Hyprland config.
- On macOS (M2), the pill registers the global hotkey itself.
- The core sends `dictation.start` and `dictation.stop` to the pill. On `start` the pill stops any speech (the microphone wins), opens the microphone and shows `listening`. On `stop` it closes the microphone and shows `transcribing`. After 120 s of recording, the pill ends the recording by itself and transcribes it. A click on the pill asks the core to stop, like the key. A key press while the model loads, or after a failed load, opens no microphone: the pill shows the reason at once. A failed model loads again at the next press. The result of an old dictation never changes a new one.
- The model loads when the pill starts, and runs once on silence, so the first dictation has no load time. A missing model shows "Download the speech-in model in the settings" at the key press.
- The pill sends the text to the core with `voice.transcript`. The core applies the vocabulary and types the text into the focused pane with `terminal.input`. The app reports its focused pane with `focus.report`. With no focused pane, the pill copies the text with `wl-copy`.
- The core types no Return, unless the owner turns on `send_return` in `<config>/agentsi/dictation.toml`. Each control character of a transcript becomes a space, so a transcript cannot run a command or move the cursor of the program in the pane.
- A recording with no text shows "Nothing was heard" and types nothing.
- The pill opens the microphone at each key press and closes it at each release, so the device is free between two dictations. The pill shows `listening` at once, and the stream starts a moment later, so the first syllable can be lost if the owner speaks at the instant of the press. Nobody has measured the start of the stream with a real microphone. The owner's live check does.
- The pill keeps the samples in memory and drops them after the text is ready. No recording reaches the disk.

### Vocabulary

- `<config>/agentsi/vocabulary.toml` holds `[[entry]]` tables. Each has a `term` and `heard` forms, for example `term = "AgentSI"`, `heard = ["Agent C"]`.
- After the model returns the text, the core replaces each `heard` form with its `term`, as a whole word, ignoring case. The longest form wins where two start at the same place. A replaced term is not replaced again.
- On the first read, with no file, the core seeds the list from the product names of the registry, and writes the file. Each product name gets these `heard` forms: the name in lower case, its camel-case words (`Agent SI`), the same with spelled capitals (`Agent S I`), and the forms that the dry run heard for AgentSI (`Agent C`, `Agent CI`, `Agent sea`). The core seeds once. It never adds a product later and never brings back an entry that the owner removed.
- The settings screen edits it: one row for each entry with Edit and Delete, and a form with a term and heard forms separated by commas. The core saves the file at each Add, Update or Delete. The file stays plain TOML, so the owner can also edit it by hand. `hex vocabulary` in Hex (MIT) is the model for the list.

### Speech out

- Kokoro runs on ONNX Runtime. `voices.toml` gives the voice of each role, as today. An entry with another engine falls back to the default voice with a warning.
- `lexicon.toml` gives the spoken form of words, as today.
- Kokoro needs phonemes. `espeak-ng` makes them but is GPL-3.0, so no crate links it. The pill runs the `espeak-ng` program as a separate process, which keeps the Apache-2.0 app free of GPL code. `agentsi doctor` reports it if missing.
- The pill plays one sentence at a time, so the first plays before the last is ready.

### What the pill speaks

1. The Stop hook gives `last_assistant_message` to the core through the hook bridge.
2. If voice mode is on for that pane, the core takes the text above the line "On screen only", by the rules of `voice-mode.mjs`.
3. The core sends `voice.speak` with the voice of the PM.
4. The core saves the prompt and the reply as Markdown in `<data root>/voice/transcripts/`, as today.
5. A new prompt in the same pane sends `voice.stop`, and the pill stops at once.

### Speech out in the pill (U12)

- `agentsi-voice::tts::Voice` turns one sentence into 24 kHz samples: the lexicon, the phonemes, Kokoro v1.0 and the level. It reads the catalog files in `models/catalog/kokoro/`, never the Python files in `models/kokoro/`.
- ONNX Runtime 1.31 comes through `ort` `=2.0.0-rc.15` with `download-binaries`: the build downloads the prebuilt MIT library once and links it. A fresh clone builds with no other step. The `tts` feature of `agentsi-voice` keeps ONNX Runtime out of a crate that needs only the text rules, when that crate builds alone. A build of the whole workspace merges the features.
- The pill runs `espeak-ng -q --ipa -v <lang>` for each piece of a sentence between punctuation runs, and puts the marks back as phonemizer does. `AGENTSI_ESPEAK` names another program.
- The program gives a primary stress to a clause that has none, and the library call of kokoro-onnx does not. So each piece also runs after a stressed dummy word, and that output wins when the piece has no primary stress there. On 1540 lines, 1538 have the phonemes of kokoro-onnx. A lone "what" or "when" between marks still has a primary stress.
- The catalog model has no `duration` output. With it, kokoro-onnx lengthens the pause after a comma inside a sentence. The owner's Python model has the output, so the Python voice pauses a little longer at commas.
- Each sentence is leveled on its own, because the pill does not wait for the whole reply. Python levels the whole reply.
- `render.py` retakes a sentence that runs too long and then splits it at a comma. That step is for Chatterbox, which can miss its stop token. Kokoro gives the same output each time, so the pill does not retake.
- ONNX Runtime uses half the logical CPUs with no thread spinning. On a busy machine this was faster than one spinning thread per CPU.
- `voice.speak`: the sentences render one at a time into a queue, and the output stream plays from it. `voice.stop` or a click empties the queue, and the stream plays silence from its next buffer of 20 ms.

### Settings screen (U15c)

- The palette entry "Open the settings" and `Ctrl+,` (`Cmd+,` on macOS) open the settings screen. It belongs to no product.
- The screen reads `models.list` and shows one row for each model: the name, the kind, the license, the size and the state.
- A missing, partial or failed model has a Download button. A partial model continues from its bytes. A downloaded, partial or failed model has a Delete button. Delete removes only the files that the download wrote.
- While a model downloads, the row shows a bar and the bytes. The core sends at most 10 `models.progress` events each second for each download. The screen draws at most 10 times each second, also with two downloads at once.
- A failed download shows its error line from the core in the row. A wrong hash shows the `hash_mismatch` message, which says that the file was deleted.
- A conflict shows the name of the foreign file. The core leaves that file in place.
- The events that arrive while the screen reads `models.list` apply after the answer. A `core.lagged` event makes the screen read the list again.
- The screen shows the voice of each role from `models.voices`. The section "Keys" shows the keys of the app, among them the voice key (`Ctrl+Shift+M`), and the section "Dictation key" shows the dictation key. The owner changes both in the screen (`agentsi-desktop-keys.md`).
- The vocabulary editor (U14) sits between the models and the voices. See "Vocabulary".
- With no download, the screen draws no frame.

### Targets

| Case | Target |
|---|---|
| Stop hook to the first sound, reply of 2 sentences, CPU | ≤ 1.5 s |
| Key release to text in the pane, 5 s of speech | p95 ≤ 1.5 s, if the model passes open test 2 |
| Pill shown after the key press | ≤ 100 ms |

## Acceptance criteria

- [ ] On Linux, `hyprctl layers -j` lists the pill on the overlay layer while it speaks, and `hyprctl activewindow` shows no focus change.
- [ ] M2: on macOS, the pill shows over a full-screen app and does not take the focus.
- [ ] A download stopped at 50 % continues from 50 % when restarted.
- [ ] A file with a wrong hash is deleted, and the settings screen shows the error.
- [ ] `rg -l espeak crates/*/Cargo.toml` finds nothing.
- [ ] With voice mode on, a reply of 2 sentences plays within 1.5 s of the Stop hook.
- [ ] A new prompt in the same pane stops the speech within 200 ms.
- [x] If the speech-in model is on: with the hotkey held, a spoken sentence appears in the focused pane, and no Return follows. (A WAV through the real path into a real pane: `crates/agentsi-pill/tests/dictate.rs`. The live microphone check is the owner's.)
- [x] M2 (U14, speech in, is out of M1): `cargo test -p agentsi-voice` has a test that turns "Agent C" into "AgentSI" with the vocabulary. (`vocabulary::tests::agent_c_becomes_agentsi`)
- [ ] `cargo test -p agentsi-voice` passes, with the cases of `plugin/hooks/voice-mode.test.mjs`.
- [ ] After a reply in a voice-mode pane, `<data root>/voice/transcripts/` holds a new Markdown file with the prompt and the reply.
