import numpy as np

from agentsi_voice import render
from agentsi_voice.text import halve

LONG = "Alex, necesitamos que consigas las credenciales de Mercado Pago, decidas los puntos legales y resuelvas las ramas viejas."


class FakeEngine:
    """Speaks at 16 chars/s, except the first `runaways` calls, which babble to Chatterbox's 40 s cap."""

    def __init__(self, runaways: int) -> None:
        self.runaways = runaways
        self.calls: list[str] = []

    def say(self, text: str, voice: str) -> tuple[np.ndarray, int]:
        self.calls.append(text)
        seconds = 40.0 if len(self.calls) <= self.runaways else len(text) / 16
        return np.full(int(seconds * render.RATE), 0.1, np.float32), render.RATE


def seconds(audio: np.ndarray) -> float:
    return len(audio) / render.RATE


def test_resynthesizes_a_runaway_sentence():
    engine = FakeEngine(runaways=1)
    audio = render._say(engine, LONG, "v")
    assert len(engine.calls) == 2
    assert seconds(audio) < 10


def test_splits_at_a_comma_when_every_attempt_runs_away():
    engine = FakeEngine(runaways=render.ATTEMPTS)
    audio = render._say(engine, LONG, "v")
    assert engine.calls[render.ATTEMPTS :] == list(halve(LONG))
    assert seconds(audio) < 10


def test_halves_at_the_comma_nearest_the_middle():
    assert halve("uno dos tres, cuatro, cinco seis siete ocho.") == ("uno dos tres, cuatro,", "cinco seis siete ocho.")
    assert halve("a, bb cc dd ee, ff gg hh ii jj.") == ("a, bb cc dd ee,", "ff gg hh ii jj.")
    assert halve("sin comas") is None
    assert halve(", hola.") is None
