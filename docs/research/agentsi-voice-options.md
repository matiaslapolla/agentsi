# Voice options and licenses for AgentSI calls

**Status:** research · **Date:** 2026-10-21 · **Questions:** (1) Which licenses and notices do the voices need? (2) Which paid API fits users without a GPU? All claims come from primary sources (model cards, package metadata, license texts, vendor pages). Access date for every link: 2026-10-21. "Not found" means I could not prove the claim on a primary page. License readings are not legal advice.

## 1. Summary

1. The repo needs no license for the voices. It needs a `VOICES.md` that lists each voice, its license and the notice it requires. It also needs a `NOTICE` text for each CC BY or Apache voice the repo ships or names.
2. Ship no weights and no reference clips. Tell the user to download them. This removes every share-alike duty from the repo itself.
3. The Chatterbox clips (`ar-female-295` and the rest) come from Google's crowdsourced Spanish sets, OpenSLR 61, 71 and 72, under CC BY-SA 4.0. Nobody documented consent for cloning these speakers.
4. For the public English default, use Kokoro (Apache-2.0 weights, MIT package). Its `af_heart` voice has the best grade (A).
5. For users without a GPU, add one `api` engine. Azure Neural costs about $0.03 per call and has es-AR voices. OpenAI TTS is a poor base because OpenAI plans to remove it on 2027-01-06.

## 2. Local engines

### 2.1 Kokoro

| Part | License | Source |
|---|---|---|
| Model `hexgrad/Kokoro-82M` | Apache-2.0 | https://huggingface.co/hexgrad/Kokoro-82M/raw/main/README.md |
| `kokoro-onnx` 0.6.1 package | MIT | https://raw.githubusercontent.com/thewh1teagle/kokoro-onnx/main/LICENSE |
| `phonemizer` (dependency) | GPL-3.0 | https://pypi.org/pypi/phonemizer/3.4.0/json |
| `espeakng-loader` (dependency) | MIT on GitHub, bundles espeak-ng (GPL-3.0) | https://api.github.com/repos/thewh1teagle/espeakng-loader/license |
| `kokoro-v1.0.onnx`, `voices-v1.0.bin` | no separate license file found | https://api.github.com/repos/thewh1teagle/kokoro-onnx/releases/tag/model-files-v1.0 |

- The model card says the training audio was permissive or non-copyrighted. Two CC BY sets were part of it: SIWIS (CC BY 4.0) and Koniwa tnc (CC BY 3.0). The card does not say whether the weights inherit the attribution duty.
- `kokoro-onnx` 0.6.1 does not use `phonemizer-fork`. It depends on `phonemizer>=3.4.0`, which is GPL-3.0. The voice package is already GPL-3.0-or-later, so this fits.
- Grades from `VOICES.md` of the model: `af_heart` A, `af_bella` A-, `af_nicole` B-, `bf_emma` B-, `am_michael` C+, `am_fenrir` C+, `am_puck` C+, `am_adam` F+. The Spanish voices `ef_dora`, `em_alex` and `em_santa` have no grade. The card says the grades are estimates. https://huggingface.co/hexgrad/Kokoro-82M/raw/main/VOICES.md
- To comply: keep the Apache-2.0 text and notices, and keep the MIT notice of the package. The license permits redistribution of the weights. The ONNX files are a conversion with no license of their own, so a user download is the safer path.

### 2.2 Piper

- `piper-tts` 1.8.0 is GPL-3.0-or-later. The wheels embed espeak-ng. https://pypi.org/pypi/piper-tts/1.8.0/json and https://raw.githubusercontent.com/OHF-voice/piper1-gpl/main/README.md
- The repo `rhasspy/piper-voices` says MIT at repo level. Each `MODEL_CARD` governs its own voice. https://huggingface.co/rhasspy/piper-voices/raw/main/README.md
- Card URL pattern: `https://huggingface.co/rhasspy/piper-voices/raw/main/<lang>/<locale>/<name>/<quality>/MODEL_CARD`.

| Voice | Dataset | License on card | Commercial | Credit |
|---|---|---|---|---|
| en_GB-cori-high | LibriVox | public domain | yes | no |
| en_US-lessac-high | Blizzard 2013 Lessac | URL only, page unreachable: not found | not found | not found |
| en_US-ryan-high | RyanSpeech | CC BY-NC-SA 4.0 | **no** | yes, share-alike |
| es_AR-daniela-high | OpenSLR 61 | CC BY-SA 4.0 | yes | yes, share-alike |
| es_ES-carlfm-x_low | carlfm01 | public domain | yes | no |
| es_ES-davefx-medium | OHF-Voice | CC0 | yes | no |
| es_ES-mls_10246-low | OpenSLR 94 | CC BY 4.0 | yes | yes |
| es_ES-mls_9972-low | OpenSLR 94 | CC BY 4.0 | yes | yes |
| es_ES-sharvard-medium | Univ. Edinburgh | CC BY 3.0 | yes | yes |
| es_MX-ald-medium | Ald (HF) | Unlicense | yes | no |
| es_MX-claude-high | HirCoir | Apache-2.0 | yes | Apache notice |

- Several voices are finetuned from the `lessac` or `ryan` checkpoint. The card lists only the dataset license of the new voice. The effect of the parent license (NC-SA for `ryan`, unknown for `lessac`) is not found. The `mls_*` voices start from `ryan`, so treat them as a risk. Do not ship or recommend `ryan`.
- Piper cards give only a quality tier (`x_low` to `high`). They give no clarity grade.

### 2.3 Chatterbox

- Model `ResembleAI/chatterbox` (Multilingual): MIT. https://huggingface.co/ResembleAI/chatterbox/raw/main/README.md
- Package `chatterbox-tts` 0.1.7: MIT. https://pypi.org/pypi/chatterbox-tts/json
- English and Spanish are both supported (23 languages in V3). V3 needs `t3_model="v3"`. Single-language finetunes exist, for example `es-mx-latam` (MIT).
- Every output carries the Perth watermark. `resemble-perth` is MIT. Whether a user may remove it is not found. https://raw.githubusercontent.com/resemble-ai/chatterbox/master/README.md
- The only use rule is "Don't use this model to do bad things". The model card and README have no consent rule for cloning. Same links.
- To comply: keep the MIT text if the repo redistributes code or weights. The stack is torch-heavy (about 3 GB of weights), so a user download is practical.

## 3. The Chatterbox reference clips

**Finding: proven.** The clips come from Google's crowdsourced Latin American Spanish sets.

- Local proof: `~/dev/tools/chatterbox/refs.toml` names each clip's dataset, speaker and OpenSLR number. `~/dev/tools/models/chatterbox/ATTRIBUTION.md` lists the source utterances. `cbx.py prepare` builds each clip from the Hugging Face mirrors by `ylacombe`.

| Clip | Dataset | OpenSLR |
|---|---|---|
| ar-female-295, ar-female-4310, ar-male-610, ar-male-5223 | google-argentinian-spanish | 61 |
| cl-male-3397 | google-chilean-spanish | 71 |
| co-female-9334 | google-colombian-spanish | 72 |

- License: each OpenSLR page says "Attribution-ShareAlike 4.0 International" with "Copyright 2018, 2019 Google, Inc.". https://www.openslr.org/61/ , https://www.openslr.org/71/ , https://www.openslr.org/72/ . The paper confirms CC BY-SA 4.0 in section 2.3. https://aclanthology.org/2020.lrec-1.801/
- The Hugging Face cards state a license only for the Chilean set. For the Argentinian and Colombian sets, cite OpenSLR.
- Intended use is TTS. The Chilean card says the data "can be used to train a model for Text-To-Speech". The paper names "adaptation data" as a use.
- Consent and cloning: **not found.** The OpenSLR pages and the paper do not mention consent, release forms, privacy or voice cloning. The Chilean card asks users "to not attempt to determine the identity of speakers". That sentence is not a CC term.
- Citation error in the local notes: `ATTRIBUTION.md` cites "Kjartansson et al." The first author is Guevara-Rukoz. Kjartansson is the last of nine.

What CC BY-SA 4.0 requires (legal code https://creativecommons.org/licenses/by-sa/4.0/legalcode.en):

1. Section 3(a): when you share the material, keep the creator and copyright notices, a license notice and a disclaimer. State that you modified it.
2. Section 3(b): when you share adapted material, license it under the same terms.
3. Section 2(b)(1): publicity, privacy and personality rights are not licensed.

Readings (not rulings):

- The trimmed, resampled and joined clips are adapted material. Shipping them triggers 3(a) and 3(b) for the clip files.
- The CC FAQ says share-alike applies only when a work is modified and shared. A collection need not be share-alike. https://creativecommons.org/faq/ So the repo code stays MIT if the clips sit in their own directory.
- Chatterbox output conditioned on a clip: **no source settles whether it is an adaptation.** CC's AI guidance says it assumes the most restrictive reading. https://creativecommons.org/using-cc-licensed-works-for-ai-training-2/ Treat shared audio as possibly BY-SA.

## 4. Paid APIs

Cost of one call: 2,000 characters, about 2 minutes. Prices come from the vendor pages. Quality is not measured here.

| Vendor and model | Per 1M chars | Per call | Free tier | Spanish and es-AR | Formats | Key header |
|---|---|---|---|---|---|---|
| OpenAI tts-1 | $15 | $0.03 | none found | about 57 languages, "optimized for English", no es-AR | mp3, opus, aac, flac, wav, pcm 24 kHz | `Authorization: Bearer` |
| OpenAI tts-1-hd | $30 | $0.06 | none found | same | same | same |
| OpenAI gpt-4o-mini-tts | tokens: $0.60 in, $12 audio out | not provable | none found | `instructions` can steer accent | same | same |
| ElevenLabs Flash v2.5 | $40 | $0.08 | 10K credits, non-commercial | Spanish yes, es-AR not found | mp3, opus, pcm, wav, ulaw, alaw | `xi-api-key` |
| ElevenLabs Multilingual v2 and v3 | $80 | $0.16 | same | Spanish yes, es-AR not found | same | same |
| Google Standard | $4 | $0.008 | 4M chars a month | es-US 3, es-ES 5, no es-AR | LINEAR16 and others | OAuth |
| Google Neural2 | $16 | $0.032 | 1M a month | es-US 3, es-ES 5 | same | OAuth |
| Google Chirp 3 HD | $30 | $0.06 | 1M a month | es-US 30, es-ES 30, no es-AR | same | OAuth |
| Azure Neural | $15 | $0.03 | 0.5M a month | `es-AR-ElenaNeural`, `es-AR-TomasNeural` | mp3, opus, wav, pcm, 8 to 48 kHz | `Ocp-Apim-Subscription-Key` |
| Azure Neural HD | $22 | $0.044 | none found | es-AR listed as HD locale, names not found | same | same |
| Cartesia Sonic 3.6 | Pro $5 for 100K credits, overage $65 | $0.08 to $0.10 | 20K credits | `es` only, es-AR not found | wav, mp3, raw | `Authorization: Bearer` |
| Deepgram Aura-2 | $30 | $0.06 | $200 credit | 17 Spanish voices, 1 Argentine (`antonia`) | wav, mp3, opus, flac, aac | not found |

Sources: https://developers.openai.com/api/docs/pricing , https://elevenlabs.io/pricing/api , https://cloud.google.com/text-to-speech/pricing , https://prices.azure.com/api/retail/prices (Azure's pricing page shows placeholders), https://cartesia.ai/pricing , https://deepgram.com/pricing . Formats and headers come from each vendor's API reference, listed in the per-vendor notes below.

Per vendor:

- **OpenAI.** The deprecation page lists `tts-1`, `tts-1-hd` and both `gpt-4o-mini-tts` snapshots for removal on 2027-01-06. The named replacement does not work on `/v1/audio/speech`. https://developers.openai.com/api/docs/deprecations The TTS guide says the usage policies require a clear disclosure that the voice is AI-generated. https://developers.openai.com/api/docs/guides/text-to-speech The usage policy page itself returned 403, so I did not read it. Abuse logs are kept up to 30 days. https://developers.openai.com/api/docs/guides/your-data
- **ElevenLabs.** The two pricing pages disagree on plan sizes. Free users may use the output only for non-commercial purposes. Paid users may use it commercially. The terms state no attribution or AI-disclosure duty. https://elevenlabs.io/terms-of-use Zero retention is enterprise-only. The terms grant ElevenLabs a perpetual license to content, including output. https://elevenlabs.io/docs/api-reference/text-to-speech/convert
- **Google.** The WaveNet price row reads $4, which looks like a page error. The API needs a Cloud project and OAuth. An API-key header is not found on the reference page. Google does not log TTS text or audio. https://docs.cloud.google.com/text-to-speech/docs/data-logging
- **Azure.** `POST https://{region}.tts.speech.microsoft.com/cognitiveservices/v1` with SSML. The user needs an Azure account, a Speech resource and a region. Microsoft does not retain real-time input text or output audio. https://learn.microsoft.com/en-us/azure/ai-services/speech-service/rest-text-to-speech and https://learn.microsoft.com/en-us/azure/foundry/responsible-ai/speech-service/text-to-speech/data-privacy-security
- **Cartesia.** Commercial use starts at the Pro tier. The terms let Cartesia train on inputs and outputs, with an opt-out form. https://cartesia.ai/terms That matters for private call content.
- **Deepgram.** Endpoint, key header, output terms and retention are not found on the pages I read.

All vendors let the user hold the key on their own machine. For Google, the credential is an OAuth token or service account, not one key.

## 5. Fit with the code

Facts from the current code:

- `engines.py`: an engine has one method, `say(text, voice) -> (float32 array, rate)`. `engine_for` builds one engine per `(engine, name)`.
- `config.py`: `ENGINES` is a tuple. `_parse` splits `engine:voice` at the first colon. `voices.toml` maps roles to these specs.
- `render.py`: `_say` calls the engine up to 3 times per sentence when the take is too long (`_plausible`). A paid API would be billed for each retake. The renderer already resamples any rate to 24 kHz.
- `cli.py`: `render` prints JSON with `duration`, `bytes`, `segments` and `out`. It has no cost field.
- `packages/core/src/meeting.ts`: `renderAudio` returns only the output folder. The meeting cost is `totalCostUsd`, and the audio cost is not part of it.

An `api` engine needs:

1. **Engine names.** Add one name per provider to `ENGINES`, for example `azure` and `elevenlabs`. A spec reads `azure:es-AR-ElenaNeural`. This keeps `_parse` as it is.
2. **Keys from the environment.** `AZURE_SPEECH_KEY`, `AZURE_SPEECH_REGION`, `ELEVENLABS_API_KEY`. The doctor reports a missing key by name and never prints it.
3. **Output format.** Ask for raw 24 kHz PCM. No resample and no decode step.
4. **Cache.** Key: hash of provider, voice, format and text. Store WAV files under the output folder or a cache folder. A re-render of the same script then costs nothing.
5. **No paid retakes.** For an API engine, skip the retake loop. The runaway take is a Chatterbox fault.
6. **Cost report.** The engine counts billed characters. `render` adds `chars` and `cost_usd` to its JSON, using a price table in the config. `renderAudio` parses that and returns it, and the CLI prints it beside the meeting cost.
7. **Pronunciation.** The lexicon respells words for local engines. An API engine needs its own setting, because a respelling can be wrong for a vendor model.
8. **Privacy notice.** Sending the script to a vendor leaves the machine. The doctor and `VOICES.md` must say so.

## 6. Open gaps

- OpenSLR LICENSE files were not opened.
- The lessac dataset license page was unreachable.
- I did not listen to any voice. "Clearer" is untested for the API voices.
- No source shows whether a Chatterbox output is an adaptation of its clip.
