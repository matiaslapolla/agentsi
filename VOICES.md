# Voices

AgentSI renders a call from a script with a text-to-speech engine. The repo ships no model weights and no reference clips. The user downloads them.

License readings here are not legal advice. Each source link was checked on 2026-10-21.

## Default: Kokoro, English

The repo `packages/voice/voices.toml` maps the five roles to Kokoro voices.

| Role | Voice |
|---|---|
| pm | `af_heart` |
| dev | `am_adam` |
| social | `af_sarah` |
| qa | `am_puck` |
| scout | `af_aoede` |

These five voices scored best in a blind listening test of 12 Kokoro English voices, on 2026-10-21. The model card gives its own grades, which are estimates.

Kokoro runs on a CPU. To download the model, run these commands:

```sh
mkdir -p "${XDG_DATA_HOME:-$HOME/.local/share}/agentsi/models/kokoro"
cd "${XDG_DATA_HOME:-$HOME/.local/share}/agentsi/models/kokoro"
curl -LO https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx
curl -LO https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin
```

Set `TTS_MODELS_DIR` to use another folder. Then run `agentsi-voice doctor` in `packages/voice` to check the setup.

## Engines

| Engine | Part | License | Source |
|---|---|---|---|
| Kokoro | model `hexgrad/Kokoro-82M` | Apache-2.0 | https://huggingface.co/hexgrad/Kokoro-82M |
| Kokoro | package `kokoro-onnx` | MIT | https://github.com/thewh1teagle/kokoro-onnx |
| Kokoro | ONNX conversion files | no license file of their own | https://github.com/thewh1teagle/kokoro-onnx/releases/tag/model-files-v1.0 |
| Piper | package `piper-tts` | GPL-3.0-or-later | https://github.com/OHF-voice/piper1-gpl |
| Piper | each voice | the license on its `MODEL_CARD` | https://huggingface.co/rhasspy/piper-voices |
| Chatterbox | model and package `chatterbox-tts` | MIT | https://huggingface.co/ResembleAI/chatterbox |

Notes:

- Kokoro trained on audio that includes two attribution sets: SIWIS (CC BY 4.0) and Koniwa tnc (CC BY 3.0). `NOTICE` carries the credits.
- `phonemizer` and the bundled espeak-ng are GPL-3.0. The `agentsi-voice` package is GPL-3.0-or-later for this reason.
- Chatterbox adds an inaudible Perth watermark to every output.

## Speech in: Parakeet

The desktop app turns speech into text with NVIDIA Parakeet TDT 0.6B v3, in the int8 ONNX export of istupakov. The model is multilingual (25 European languages, including English and Spanish).

| Part | License | Source |
|---|---|---|
| Model `nvidia/parakeet-tdt-0.6b-v3` | CC BY 4.0 | https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3 |
| ONNX export `istupakov/parakeet-tdt-0.6b-v3-onnx` | CC BY 4.0 | https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx |
| ONNX Runtime and `ort` | MIT, and MIT or Apache-2.0 | https://github.com/microsoft/onnxruntime |

CC BY 4.0 allows commercial use and changes. It asks for credit and a link to the license, which `NOTICE` gives. The repo ships no weights. The settings screen downloads the files, and each file has a pinned SHA-256 in `crates/agentsi-models/models.toml`.

The model runs on a CPU. Audio never leaves the machine. The app keeps no recording: the pill holds the samples in memory until the text is ready, then drops them.

The model mishears product names, for example "Agent C" for "AgentSI". The vocabulary in `<config>/agentsi/vocabulary.toml` fixes the names that you add. The settings screen edits it.

## Voices with a restriction

| Voice | Restriction |
|---|---|
| Piper `en_US-ryan-*` | CC BY-NC-SA 4.0. Not for commercial use. Share-alike. Do not use it in a product. |
| Piper `mls_*` Spanish voices | Finetuned from `ryan`. The effect of the parent license is not settled. Treat them as a risk. |
| Piper `es_AR-daniela-*` | CC BY-SA 4.0 (OpenSLR 61). Credit Google and share adaptations under the same license. |
| Chatterbox clips from OpenSLR 61, 71 and 72 | CC BY-SA 4.0. See the next section. |
| Piper `es_ES-mls_*`, `es_ES-sharvard-*` | CC BY 4.0 and CC BY 3.0. Credit required. |

Kokoro voices carry no extra restriction beyond the model license.

### Clips from OpenSLR

Google published crowdsourced Spanish speech sets as OpenSLR 61 (Argentina), 71 (Chile) and 72 (Colombia). The license is CC BY-SA 4.0, "Copyright 2018, 2019 Google, Inc.".

Credit: Guevara-Rukoz et al., "Crowdsourcing Latin American Spanish for Low-Resource Text-to-Speech", LREC 2020. https://aclanthology.org/2020.lrec-1.801/

Rules when you share a clip:

- Keep the credit, the license notice and the disclaimer.
- State the changes, for example "modified: trimmed, resampled, joined".
- License the adapted clip under CC BY-SA 4.0.

No source settles whether audio that Chatterbox generates from a clip is an adaptation. If you share such audio, treat it as possibly CC BY-SA 4.0. The repo ships no clip.

## Add a voice

1. Put the model or clip in the models folder (`TTS_MODELS_DIR`):
   - Kokoro voices come in `kokoro/voices-v1.0.bin`.
   - A Piper voice is `piper/<name>.onnx` and `piper/<name>.onnx.json`.
   - A Chatterbox clip is `chatterbox/refs/<name>.wav`.
2. Add a line to `voices.toml`, as `role = "engine:voice"`.
3. Record the license and the source of the voice in your own notes. Add its credit if it requires one.

To keep your voice map private, put `voices.toml` in the data root. The renderer reads it there before it reads the repo file. A `lexicon.toml` in the data root replaces the repo lexicon the same way. The `lang` key sets the language of Kokoro and Chatterbox. Words are respelled with the lexicon section of the same language, so the Spanish respellings apply only when `lang` starts with `es`.

## Consent

Cloning the voice of a real person needs that person's consent. A Creative Commons license on a recording does not grant publicity, privacy or personality rights. The OpenSLR pages and the paper do not document consent for cloning.

## Privacy

Local engines keep the script on your machine. A paid speech engine, planned for a later release, sends the script to its vendor. Read the vendor's retention terms before you use it with private meetings.
