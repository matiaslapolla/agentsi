import json
import subprocess
import sys
import tempfile
from pathlib import Path

import numpy as np
from scipy.signal import resample_poly

from agentsi_voice.config import load_lang, load_lexicon, voice_for
from agentsi_voice.engines import Engine, engine_for
from agentsi_voice.text import halve, sentences

RATE = 24_000
SEGMENT_GAP = 0.35
SENTENCE_GAP = 0.2
TARGET_RMS = 0.08
PEAK_LIMIT = 0.95
# The role voices speak at 15-19 chars/s; a take far past that is Chatterbox missing its stop token.
CHARS_PER_SECOND = 16
RUNAWAY_FACTOR = 1.3
RUNAWAY_SLACK = 1.5
ATTEMPTS = 3
OPUS_ARGS = ["-c:a", "libopus", "-b:a", "32k", "-application", "voip", "-ac", "1", "-ar", str(RATE)]


def _resample(audio: np.ndarray, rate: int) -> np.ndarray:
    if rate == RATE:
        return audio
    g = np.gcd(rate, RATE)
    return resample_poly(audio, RATE // g, rate // g).astype(np.float32)


def _level(audio: np.ndarray) -> np.ndarray:
    rms = float(np.sqrt(np.mean(audio**2))) or 1.0
    audio = audio * (TARGET_RMS / rms)
    peak = float(np.max(np.abs(audio)))
    return audio * (PEAK_LIMIT / peak) if peak > PEAK_LIMIT else audio


def _silence(seconds: float) -> np.ndarray:
    return np.zeros(int(seconds * RATE), dtype=np.float32)


def _plausible(audio: np.ndarray, text: str) -> bool:
    return len(audio) / RATE <= len(text) / CHARS_PER_SECOND * RUNAWAY_FACTOR + RUNAWAY_SLACK


def _say(engine: Engine, text: str, voice: str) -> np.ndarray:
    """Retake a sentence the engine babbled past its end; split it at a comma when every take does."""
    takes = []
    for _ in range(ATTEMPTS):
        audio, rate = engine.say(text, voice)
        audio = _resample(audio, rate)
        if _plausible(audio, text):
            return audio
        takes.append(audio)
    parts = halve(text)
    if parts:
        left, right = parts
        return np.concatenate([_say(engine, left, voice), _silence(SENTENCE_GAP), _say(engine, right, voice)])
    print(f"agentsi-voice: no clean take after {ATTEMPTS} tries: {text!r}", file=sys.stderr)
    return min(takes, key=len)


def _segment_audio(role: str, text: str, lexicon: dict[str, str]) -> np.ndarray:
    ref = voice_for(role)
    engine = engine_for(ref)
    pieces: list[np.ndarray] = []
    for sentence in sentences(text, lexicon):
        if pieces:
            pieces.append(_silence(SENTENCE_GAP))
        pieces.append(_say(engine, sentence, ref.name))
    if not pieces:
        return _silence(0.1)
    return _level(np.concatenate(pieces))


def _encode(audio: np.ndarray, out: Path) -> None:
    pcm = (np.clip(audio, -1, 1) * 32767).astype("<i2").tobytes()
    cmd = ["ffmpeg", "-y", "-loglevel", "error", "-f", "s16le", "-ar", str(RATE), "-ac", "1", "-i", "-", *OPUS_ARGS, str(out)]
    subprocess.run(cmd, input=pcm, check=True)


def render(script: list[dict], out_dir: Path) -> dict:
    out_dir.mkdir(parents=True, exist_ok=True)
    lexicon = load_lexicon(load_lang())
    pieces: list[np.ndarray] = []
    timings: list[dict] = []
    cursor = 0
    for idx, item in enumerate(script):
        if idx:
            gap = _silence(SEGMENT_GAP)
            pieces.append(gap)
            cursor += len(gap)
        audio = _segment_audio(item["role"], item["text"], lexicon)
        timings.append(
            {
                "idx": idx,
                "role": item["role"],
                "text": item["text"],
                "start": round(cursor / RATE, 3),
                "end": round((cursor + len(audio)) / RATE, 3),
            }
        )
        pieces.append(audio)
        cursor += len(audio)
    full = np.concatenate(pieces)
    with tempfile.TemporaryDirectory(prefix="agentsi-voice-") as tmp:
        staged = Path(tmp) / "call.webm"
        _encode(full, staged)
        webm = out_dir / "call.webm"
        webm.write_bytes(staged.read_bytes())
    (out_dir / "timings.json").write_text(json.dumps(timings, ensure_ascii=False, indent=2) + "\n")
    return {
        "duration": round(len(full) / RATE, 3),
        "bytes": webm.stat().st_size,
        "segments": len(timings),
        "out": str(out_dir),
    }
