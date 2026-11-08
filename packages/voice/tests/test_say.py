import io
import wave
from pathlib import Path

import numpy as np

from agentsi_voice import cli, engines
from agentsi_voice.config import VoiceRef
from agentsi_voice.render import RATE


class FakeEngine:
    def __init__(self) -> None:
        self.calls: list[tuple[str, str]] = []

    def say(self, text: str, voice: str) -> tuple[np.ndarray, int]:
        self.calls.append((text, voice))
        return np.full(len(text) * RATE // 16, 0.1, np.float32), RATE


def test_say_speaks_each_stdin_line_with_one_engine(monkeypatch, tmp_path, capsys):
    engine = FakeEngine()
    loads: list[VoiceRef] = []
    monkeypatch.setattr(cli, "voice_for", lambda role: VoiceRef("kokoro", f"{role}-voice"))
    monkeypatch.setattr(engines, "engine_for", lambda ref: loads.append(ref) or engine)
    monkeypatch.setattr("sys.stdin", io.StringIO("The build passes.\n\nI fixed two tests.\n"))

    assert cli._say("pm", tmp_path / "out") == 0

    assert len(loads) == 1, "the model loads once for the whole reply"
    assert engine.calls == [("The build passes.", "pm-voice"), ("I fixed two tests.", "pm-voice")]
    printed = capsys.readouterr().out.splitlines()
    assert printed == [f"WAV {tmp_path / 'out' / '0.wav'}", f"WAV {tmp_path / 'out' / '1.wav'}"]
    with wave.open(str(tmp_path / "out" / "0.wav")) as f:
        assert f.getframerate() == RATE and f.getnframes() > 0


def test_say_fails_on_empty_stdin(monkeypatch, tmp_path):
    monkeypatch.setattr(cli, "voice_for", lambda role: VoiceRef("kokoro", "v"))
    monkeypatch.setattr(engines, "engine_for", lambda ref: FakeEngine())
    monkeypatch.setattr("sys.stdin", io.StringIO("\n  \n"))
    assert cli._say("pm", tmp_path) == 2


def test_short_espeak_data_copies_a_long_path_once(tmp_path):
    short = str(tmp_path / "data")
    assert engines.short_espeak_data(short) == short

    long_dir = tmp_path / ("x" * 120) / "espeak-ng-data"
    long_dir.mkdir(parents=True)
    (long_dir / "phontab").write_text("ok")
    copy = engines.short_espeak_data(str(long_dir), cache_dir=tmp_path / "cache")
    assert len(copy) <= len(str(tmp_path)) + 30
    assert not Path(copy).is_symlink(), "espeak-ng resolves symlinks back to the long path"
    assert (Path(copy) / "phontab").read_text() == "ok"
    (long_dir / "phontab").write_text("changed")
    assert engines.short_espeak_data(str(long_dir), cache_dir=tmp_path / "cache") == copy
    assert (Path(copy) / "phontab").read_text() == "ok", "the copy is made once"
