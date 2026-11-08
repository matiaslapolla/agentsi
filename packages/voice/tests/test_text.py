from agentsi_voice.config import VOICES_FILE, voice_for
from agentsi_voice.text import sentences


def test_splits_sentences_and_applies_lexicon():
    out = sentences("Hola, ZetaApp anda. ¿Y QA? Sí", {"zetaapp": "zeta ap", "qa": "kiu ei"})
    assert out == ["Hola, zeta ap anda.", "¿Y kiu ei?", "Sí"]


def test_unknown_role_falls_back_to_default():
    assert voice_for("nobody", VOICES_FILE).id == "kokoro:af_heart"
    assert voice_for("qa", VOICES_FILE).id == "kokoro:am_puck"


def test_specialist_falls_back_to_its_role_voice():
    assert voice_for("qa.mobile", VOICES_FILE).id == voice_for("qa", VOICES_FILE).id
    assert voice_for("nobody.x", VOICES_FILE).id == voice_for("nobody", VOICES_FILE).id
