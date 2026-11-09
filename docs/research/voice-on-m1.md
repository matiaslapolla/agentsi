# Voice on a MacBook Air M1 (Asahi Linux)

**Status:** research · **Date:** 2026-11-09 · **Question:** Is the CPU path of the AgentSI voice (Kokoro and Parakeet on ONNX Runtime) fast enough for calls and voice mode on this machine?

## 1. Verdict

1. Yes. The CPU path is good enough for voice mode and for calls on this machine.
2. Kokoro renders speech at a real-time factor (RTF) of 0.27 to 0.37. The RTF is the render time divided by the audio time. So each sentence is ready before the sentence before it stops.
3. The first sentence of a reply of 2 sentences renders in 0.88 s. The target is 1.5 s from the Stop hook to the first sound. The pill keeps the voice open, so 0.62 s remain for the hook, the core and the output stream.
4. Parakeet v3 turns 7.5 s of speech into text in 0.31 s. The target is 1.5 s for 5 s of speech.
5. A cold start misses the target. The voice takes 0.95 s to open. If the pill opens the voice when the reply arrives, the first sound comes after about 1.83 s.
6. A call of 5 minutes takes about 80 s of render time at an RTF of 0.27.

## 2. Machine

| Item | Value |
|---|---|
| Computer | MacBook Air M1, 16 GB memory |
| System | Asahi Linux (Omarchy), kernel 7.2.11-14-1-ARCH, aarch64 |
| CPU | 8 cores: 4 Firestorm (performance) and 4 Icestorm (efficiency) |
| Load before the runs | load average 0.57. Other desktop apps ran. 5 GB of swap was in use. |
| Phonemizer | espeak-ng 1.52.0 |
| ONNX Runtime | 1.31 through `ort` 2.0.0-rc.15 (prebuilt library) |
| Threads | Kokoro and Parakeet use half the logical CPUs: 4 intra-op threads, no spinning |

## 3. Commands

All commands ran from the repo root.

```sh
cargo build --release -p agentsi-voice --examples          # 18 s, the dependencies were in the cache
cargo run --release -p agentsi-voice --example fetch -- kokoro
cargo run --release -p agentsi-voice --example fetch -- parakeet-tdt-v3-int8
cargo run --release -p agentsi-voice --example fetch -- parakeet-tdt-v2-int8
cargo run --release -p agentsi-voice --example fetch -- silero-vad-v5
agentsi doctor
target/release/examples/say <out.wav> <role> "<text>"
target/release/examples/transcribe <out.wav>
```

- `/usr/bin/time` is not on this machine. A Python script ran each command in its place. The script read the peak memory from `getrusage` (`ru_maxrss`). It read the thread count from `/proc/<pid>/status` every 20 ms.
- The WAV files are in the scratchpad of the session, in `scratchpad/voice/`. They are not in the repo.

## 4. Downloads

| Catalog id | Size on disk | Time | Result |
|---|---|---|---|
| `kokoro` | 338 MB | 95 s | ok, hashes match |
| `parakeet-tdt-v3-int8` | 640 MB | 173 s | ok, hashes match |
| `parakeet-tdt-v2-int8` | 631 MB | 170 s | ok, hashes match |
| `silero-vad-v5` | 2.2 MB | 2 s | ok, hashes match |

Before the downloads, `agentsi doctor` showed `fail voice models (optional) 0 of 4 downloaded`. After the downloads, it shows `ok voice models (optional) 4 of 4 downloaded`.

## 5. Results

### 5.1 Speech out (Kokoro), reply of 2 sentences, role pm

Text (24 words): "I merged the voice branch and the tests pass on Linux. Next, we should measure the desktop pill on the M1 before the release."

| Run | Open | Sentence 1 (3.07 s audio) | Sentence 2 (4.20 s audio) | Render total | RTF | Wall time | Peak memory |
|---|---|---|---|---|---|---|---|
| 1 | 0.89 s | 0.89 s | 1.14 s | 2.03 s | 0.28 | 2.95 s | 597 MB |
| 2 | 0.98 s | 0.88 s | 1.14 s | 2.01 s | 0.28 | 3.02 s | 596 MB |
| 3 | 0.98 s | 0.87 s | 1.14 s | 2.02 s | 0.28 | 3.03 s | 597 MB |
| Mean | 0.95 s | 0.88 s | 1.14 s | 2.02 s | 0.28 | 3.00 s | 597 MB |

- An earlier set of 3 runs gave the same sentence times: 0.86 to 0.88 s and 1.13 to 1.14 s.
- The phonemes (espeak-ng) take 13 to 19 ms for each sentence. The model takes the rest of the time.
- The WAV file holds 7.47 s: the 2 sentences and a gap of 0.2 s.
- The first sentence is not slower than the second sentence for each second of audio. So the first render after the open has no large warm-up cost.

### 5.2 Speech out (Kokoro), paragraph, role pm

The paragraph has 128 words in 8 sentences. One run.

| Measure | Value |
|---|---|
| Open | 0.98 s |
| Fastest sentence | 1.06 s for 3.78 s of audio |
| Slowest sentence | 1.40 s for 5.42 s of audio |
| Render total | 10.06 s |
| Audio total (no gaps) | 37.54 s |
| RTF | 0.27 |
| Wall time | 11.07 s |
| Peak memory | 616 MB |

### 5.3 Speech out (Kokoro), one sentence for each role

Text: "The build is green and the tests pass." The roles and voices come from `packages/voice/voices.toml`.

| Role | Voice | Open | Render | Audio | RTF | Peak memory |
|---|---|---|---|---|---|---|
| pm | `af_heart` | 0.92 s | 0.59 s | 1.92 s | 0.31 | 542 MB |
| dev | `am_adam` | 0.98 s | 0.67 s | 1.98 s | 0.34 | 546 MB |
| social | `af_sarah` | 1.02 s | 0.72 s | 1.92 s | 0.37 | 554 MB |
| qa | `am_puck` | 0.98 s | 0.59 s | 1.92 s | 0.31 | 543 MB |
| scout | `af_aoede` | 0.95 s | 0.65 s | 1.94 s | 0.34 | 545 MB |

A short sentence has a higher RTF than a long sentence. Each render has a fixed cost.

### 5.4 Speech in (Parakeet TDT v3, int8), one run

The input is the WAV file of run 1 in 5.1: 24 kHz, mono, 16 bit. The example resamples it to 16 kHz.

| Measure | Value |
|---|---|
| Open | 1.69 s |
| Audio | 7.47 s |
| Resample to 16 kHz | 0.075 s |
| Resample and transcribe | 0.31 s |
| RTF | 0.04 |
| Wall time | 2.04 s |
| Peak memory | 1231 MB |
| Threads seen | 10 |

Text: "I merge the voice branch, and the tests pass on Linux. Next, we should measure the desktop pill on the M, one before the release."

The model wrote "merge" for "merged" and "M, one" for "M1". The other words are correct.

### 5.5 Memory and threads

- Kokoro uses 540 to 620 MB of resident memory. The peak goes up a little with the length of the text.
- Parakeet v3 uses 1.23 GB of resident memory.
- Kokoro and Parakeet in one process need about 1.8 GB. The machine has 16 GB.
- The `say` process shows 4 threads at most. ONNX Runtime uses 4 intra-op threads.
- The `transcribe` process shows 10 threads at most. Parakeet opens 3 sessions: the preprocessor, the encoder and the decoder.

## 6. Comparison with the spec targets

The targets come from `docs/specs/agentsi-desktop-voice.md`, section "Targets".

| Case | Target | Measured | Result |
|---|---|---|---|
| Stop hook to the first sound, reply of 2 sentences, CPU | ≤ 1.5 s | 0.88 s to render the first sentence, with the voice open. 1.83 s if the voice opens first. | Pass with a warm pill. Fail with a cold open. |
| Key release to text in the pane, 5 s of speech | p95 ≤ 1.5 s | 0.31 s for 7.47 s of speech, one run, with the model open | Pass, from one sample |
| Pill shown after the key press | ≤ 100 ms | Not measured | Not measured |

- The first-sound value excludes the Stop hook, the core, the event to the pill and the start of the output stream. These steps have 0.62 s of the budget.
- `Speaker::start` in `crates/agentsi-pill/src/speak.rs` opens the voice when the pill starts. The voice stays open between replies. So the warm case is the normal case.
- The speech-in result is one sample, not a p95. More runs on real microphone audio are necessary for a p95.

## 7. Limits of this measurement

- The voice is synthetic. Parakeet hears clean Kokoro speech, not a microphone.
- The examples do not run the pill. They do not measure the Stop hook, the IPC or the audio output.
- Other apps ran on the machine. The scheduler can put the 4 threads on the efficiency cores. This can change the times.
- The Parakeet v2 model and the Silero detector were downloaded but not measured.
