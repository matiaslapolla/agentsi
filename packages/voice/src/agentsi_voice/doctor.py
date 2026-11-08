import shutil
import subprocess

from agentsi_voice.config import (
    CHATTERBOX_DIR,
    CHATTERBOX_REFS,
    KOKORO_MODEL,
    KOKORO_VOICES,
    MODELS_DIR,
    PIPER_DIR,
    load_voices,
    voices_file,
)

KOKORO_DOWNLOAD = "https://github.com/thewh1teagle/kokoro-onnx/releases/tag/model-files-v1.0 (see VOICES.md)"


PLAYERS = ("pw-play", "paplay", "aplay")


def player_status() -> str:
    """One line: whether an audio player is on PATH. The voice hooks use pw-play first."""
    for name in PLAYERS:
        path = shutil.which(name)
        if path:
            return f"audio player {name}: found ({path})"
    return "audio player pw-play: missing (not found on PATH; install pipewire)"


def pill_status() -> str:
    """One line: whether qs (Quickshell) is on PATH. Without it voice mode works but shows no floating pill."""
    path = shutil.which("qs")
    if path:
        return f"floating pill qs: found ({path})"
    return "floating pill qs: missing (not found on PATH; the pill is optional, install quickshell)"


def problems() -> list[str]:
    found: list[str] = []
    if shutil.which("ffmpeg") is None:
        found.append("ffmpeg not found in PATH")
    else:
        enc = subprocess.run(["ffmpeg", "-hide_banner", "-encoders"], capture_output=True, text=True).stdout
        if "libopus" not in enc:
            found.append("ffmpeg has no libopus encoder")
    if not voices_file().exists():
        return [*found, f"voice map missing: {voices_file()}"]
    roles, default = load_voices()
    refs = {*roles.values(), default}
    kokoro = sorted(r.name for r in refs if r.engine == "kokoro")
    if kokoro:
        missing = [p for p in (KOKORO_MODEL, KOKORO_VOICES) if not p.exists()]
        found += [f"kokoro model file missing: {p} (TTS_MODELS_DIR={MODELS_DIR}); download: {KOKORO_DOWNLOAD}" for p in missing]
        if not missing:
            from agentsi_voice.engines import KokoroEngine

            available = set(KokoroEngine().voices())
            found += [f"kokoro voice not in voices bin: {v}" for v in kokoro if v not in available]
    for r in sorted(refs, key=lambda r: r.id):
        if r.engine == "piper":
            for suffix in (".onnx", ".onnx.json"):
                path = PIPER_DIR / f"{r.name}{suffix}"
                if not path.exists():
                    found.append(f"piper model file missing: {path}")
        if r.engine == "chatterbox":
            if not (CHATTERBOX_DIR / ".venv").is_dir():
                found.append(f"chatterbox runtime not synced: {CHATTERBOX_DIR} (uv sync there)")
            if not (CHATTERBOX_REFS / f"{r.name}.wav").exists():
                found.append(f"chatterbox reference clip missing: {CHATTERBOX_REFS / r.name}.wav")
    return found
