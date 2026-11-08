#!/usr/bin/env bash
# Synthetic WAVs from sentences.json with Kokoro (voice "pm"), resampled to 16 kHz mono. Not the owner's voice.
set -euo pipefail
cd "$(dirname "$0")"
tmp=$(mktemp -d)
python3 -c "import json;[print(s['text']) for s in json.load(open('sentences.json'))]" \
  | uv run --project ../../packages/voice agentsi-voice say pm "$tmp"
mkdir -p dry-run
for i in $(seq 0 19); do
  ffmpeg -loglevel error -y -i "$tmp/$i.wav" -ar 16000 -ac 1 -c:a pcm_s16le "dry-run/$((i+1)).wav"
done
rm -rf "$tmp"
