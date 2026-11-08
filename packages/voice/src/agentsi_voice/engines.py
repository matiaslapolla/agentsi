import atexit
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import wave
from functools import cache
from pathlib import Path
from typing import Protocol

import numpy as np

from agentsi_voice.config import CHATTERBOX_DIR, CHATTERBOX_REFS, KOKORO_MODEL, KOKORO_VOICES, PIPER_DIR, VoiceRef, load_lang


class Engine(Protocol):
    def say(self, text: str, voice: str) -> tuple[np.ndarray, int]: ...


class PiperEngine:
    def __init__(self, voice: str) -> None:
        from piper import PiperVoice

        self.model = PiperVoice.load(PIPER_DIR / f"{voice}.onnx")

    def say(self, text: str, voice: str) -> tuple[np.ndarray, int]:
        chunks = [c.audio_float_array for c in self.model.synthesize(text)]
        audio = np.concatenate(chunks) if chunks else np.zeros(1, np.float32)
        return audio.astype(np.float32), self.model.config.sample_rate


# espeak-ng keeps its data path in a 160-byte buffer and resolves symlinks, so a venv in a deep worktree fails
# with "Error processing file .../phontab". Past this length, use a copy of the data in a short cache folder.
ESPEAK_PATH_LIMIT = 100


def short_espeak_data(data: str, cache_dir: Path | None = None) -> str:
    """`data`, or a copy of it in a short cache folder when `data` is too long for espeak-ng (19 MB, made once)."""
    if len(data) <= ESPEAK_PATH_LIMIT:
        return data
    cache = cache_dir or Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / "agentsi"
    copy = cache / f"espeak-{hashlib.sha1(data.encode()).hexdigest()[:10]}"
    if not (copy / "phontab").exists():
        cache.mkdir(parents=True, exist_ok=True)
        tmp = Path(tempfile.mkdtemp(prefix=f"{copy.name}.", dir=cache))
        shutil.copytree(data, tmp, dirs_exist_ok=True)
        try:
            tmp.rename(copy)
        except OSError:
            shutil.rmtree(tmp, ignore_errors=True)  # Another process made the copy first.
    return str(copy)


class KokoroEngine:
    def __init__(self) -> None:
        import espeakng_loader
        from kokoro_onnx import Kokoro
        from kokoro_onnx.config import EspeakConfig

        espeak = EspeakConfig(lib_path=espeakng_loader.get_library_path(), data_path=short_espeak_data(espeakng_loader.get_data_path()))
        self.model = Kokoro(str(KOKORO_MODEL), str(KOKORO_VOICES), espeak_config=espeak)

    def say(self, text: str, voice: str) -> tuple[np.ndarray, int]:
        audio, rate = self.model.create(text, voice=voice, speed=1.0, lang=load_lang())
        return audio.astype(np.float32), rate

    def voices(self) -> list[str]:
        return list(self.model.get_voices())


class ChatterboxEngine:
    """Client of the warm `cbx.py serve` worker (contract: CHATTERBOX_DIR/README.md).

    Chatterbox pins its own torch stack, so it runs in its own uv project as a subprocess.
    """

    def __init__(self, command: list[str] | None = None) -> None:
        command = command or ["uv", "run", "--project", str(CHATTERBOX_DIR), "python", str(CHATTERBOX_DIR / "cbx.py"), "serve"]
        self.log = Path(tempfile.gettempdir()) / "agentsi-chatterbox.log"
        self.scratch = Path(tempfile.mkdtemp(prefix="agentsi-chatterbox-"))
        with self.log.open("w") as stderr:
            self.proc = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr, text=True, bufsize=1)
        atexit.register(self.close)
        self._receive()

    def _receive(self) -> dict:
        line = self.proc.stdout.readline()
        if not line:
            raise RuntimeError(f"chatterbox worker exited (code {self.proc.poll()}); see {self.log}")
        return json.loads(line)

    def say(self, text: str, voice: str) -> tuple[np.ndarray, int]:
        out = self.scratch / "speech.wav"
        request = {"text": text, "language": load_lang().partition("-")[0], "ref": str(CHATTERBOX_REFS / f"{voice}.wav"), "out": str(out)}
        self.proc.stdin.write(json.dumps(request) + "\n")
        self.proc.stdin.flush()
        reply = self._receive()
        if not reply["ok"]:
            raise RuntimeError(f"chatterbox: {reply['error']}")
        with wave.open(str(out), "rb") as f:
            rate, frames = f.getframerate(), f.readframes(f.getnframes())
        return np.frombuffer(frames, dtype=np.int16).astype(np.float32) / 32768, rate

    def close(self) -> None:
        if self.proc.poll() is None:
            self.proc.stdin.close()
            try:
                self.proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.proc.kill()


def engine_for(ref: VoiceRef) -> Engine:
    """Piper loads one model per voice; Kokoro and Chatterbox share one model across voices."""
    return _engine(ref.engine, ref.name if ref.engine == "piper" else "")


@cache
def _engine(engine: str, name: str) -> Engine:
    if engine == "piper":
        return PiperEngine(name)
    if engine == "chatterbox":
        return ChatterboxEngine()
    return KokoroEngine()
