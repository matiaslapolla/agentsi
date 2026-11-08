# Voice defaults and licenses

**Status:** done · **Source:** unit 5 of `docs/plans/agentsi-open-source.md`, brief `docs/ideas/agentsi-voice-api.md`

## Problem

The public repo names the owner's Chatterbox clips and products. It points at the owner's tool directories. It has no voice license notice. The default voice is Spanish and needs a GPU.

## Goal

- A new user renders a call on a CPU, in English, with Kokoro and no personal files.
- A user keeps a private voice map outside the repo.
- The repo states the license of each supported engine and voice.

## Non-goals

- No paid API engine (`azure`). It waits for the owner to shape the brief.
- No root `LICENSE`. Unit 6 adds it.
- No change to `packages/core`, `packages/cli`, `apps/web` or `plugin/`.

## Design

### 1. Public default voices

`packages/voice/voices.toml` maps the five roles to distinct Kokoro English voices. It has a `lang` key, default `"en-us"`. Kokoro and Chatterbox read the language from this key. The lexicon section follows the same language prefix (`en`, `es`).

### 2. Private override

The renderer reads `voices.toml` from the data root when that file exists. Otherwise it reads the repo file. The data root resolves as in `packages/core/src/paths.ts`: `AGENTSI_HOME`, then `${XDG_CONFIG_HOME:-~/.config}/agentsi/home`, then the code checkout.

### 3. No clips, no weights, no personal paths

The repo file names no Chatterbox clip. `TTS_MODELS_DIR` defaults to `${XDG_DATA_HOME:-~/.local/share}/agentsi/models`. `CHATTERBOX_DIR` defaults to `${XDG_DATA_HOME:-~/.local/share}/agentsi/chatterbox`. The env vars still override both.

### 4. Licenses

- `packages/voice/LICENSE`: GPL-3.0-or-later. `pyproject.toml` declares it.
- `VOICES.md`: engines, voices, licenses, links, notices, download and add-a-voice steps, consent and privacy notes.
- `NOTICE`: third-party notices.

### 5. Scrub

No product or owner name remains in `packages/voice`. Tests stay meaningful.

## Acceptance criteria

- [x] `voices.toml` maps five roles to five distinct `kokoro:` voices and has `lang = "en-us"`.
- [x] Kokoro and Chatterbox use `lang` from the voice map, not a fixed constant.
- [x] A `voices.toml` in the data root overrides the repo file, and the repo file is used when the data root has none. Both paths have tests.
- [x] The default models dir and Chatterbox dir follow `XDG_DATA_HOME`, and `TTS_MODELS_DIR` and `CHATTERBOX_DIR` override them. Tests cover it.
- [x] `packages/voice/LICENSE` holds the GPL-3.0 text and `pyproject.toml` says `GPL-3.0-or-later`.
- [x] `VOICES.md` and `NOTICE` exist at the repo root and cover the items in the brief's license plan.
- [x] A case-insensitive `git grep` in `packages/voice` for the owner's product names, the owner's name and `dev/tools` returns nothing.
- [x] `cd packages/voice && uv run pytest`, `pnpm typecheck`, `pnpm lint` and `pnpm test` pass.
- [x] The Kokoro English sample renders with the default voice map when the model exists locally.
