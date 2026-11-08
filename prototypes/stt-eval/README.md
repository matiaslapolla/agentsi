# stt-eval (open test 2)

Throwaway harness. Question: which ONNX speech-in model matches Hex on 20 real dictations of the owner, and with what delay?

## Owner: two commands

1. Record. Read each sentence aloud, press Enter to stop. The WAVs go to `recordings/` (git-ignored, it is your voice).

   ```
   cd prototypes/stt-eval && uv sync --extra cpu && uv run python record.py
   ```

2. Evaluate. Runs 4 ONNX models and 2 Hex models on every WAV.

   ```
   uv run python eval.py --wavs recordings --out results/real-cpu.json
   ```

Options: `--models whisper-base,parakeet-v2` picks models. `--provider cuda` uses the GPU (needs `uv sync --extra cuda`).

Models download once into `~/.cache/agentsi-stt-eval/hf`. Hex rows call `hex model transcribe` (read-only, no settings change). Hex time includes process start and model load.

`sentences.json` holds 20 sentences from the owner's Hex history. Three mis-heard names were fixed (gitignore, Herdr, AgentSI).
`dry-run/` holds synthetic WAVs (Kokoro voice, not the owner), to prove the pipeline.

## Dry run (no owner)

```
./make_dry_run.sh
uv sync --extra cpu && uv run python eval.py --wavs dry-run --out results/dry-cpu.json
UV_PROJECT_ENVIRONMENT=.venv-cuda uv sync --extra cuda && UV_PROJECT_ENVIRONMENT=.venv-cuda uv run python eval.py --wavs dry-run --provider cuda --models whisper-base,whisper-large-v3-turbo,parakeet-v2,parakeet-v3 --out results/dry-cuda.json
```

Run `uv sync --extra cpu` once before the owner commands too. Results of the first dry run are in `results/`.
