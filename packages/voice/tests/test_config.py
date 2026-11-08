import importlib

from agentsi_voice import config
from agentsi_voice.config import VOICES_FILE, data_root, LEXICON_FILE, lexicon_file, load_lang, load_lexicon, load_voices, voices_file


def test_repo_voice_map_has_five_distinct_kokoro_voices():
    roles, default = load_voices(VOICES_FILE)
    assert {r.engine for r in roles.values()} == {"kokoro"}
    assert len({r.name for r in roles.values()}) == len(roles) == 5
    assert default.engine == "kokoro"
    assert load_lang(VOICES_FILE) == "en-us"


def test_data_root_voices_file_overrides_the_repo_file(tmp_path):
    (tmp_path / "voices.toml").write_text('lang = "es"\ndefault = "piper:x"\n')
    env = {"AGENTSI_HOME": str(tmp_path)}
    assert voices_file(env) == tmp_path / "voices.toml"
    assert load_lang(voices_file(env)) == "es"


def test_repo_voices_file_is_used_when_the_data_root_has_none(tmp_path):
    assert voices_file({"AGENTSI_HOME": str(tmp_path)}) == VOICES_FILE


def test_data_root_follows_the_home_file_after_the_env(tmp_path):
    (tmp_path / "agentsi").mkdir()
    (tmp_path / "agentsi" / "home").write_text(f"{tmp_path / 'data'}\n")
    assert data_root({"XDG_CONFIG_HOME": str(tmp_path)}) == tmp_path / "data"
    assert data_root({"XDG_CONFIG_HOME": str(tmp_path), "AGENTSI_HOME": str(tmp_path / "env")}) == tmp_path / "env"
    assert data_root({"XDG_CONFIG_HOME": str(tmp_path / "none")}) == config.CODE_ROOT


def test_lexicon_section_follows_the_language_prefix():
    assert load_lexicon("en-us", LEXICON_FILE)["qa"] == "Q A"
    assert load_lexicon("es", LEXICON_FILE)["qa"] == "kiu ei"


def test_model_dirs_default_to_xdg_data_home_and_env_overrides(monkeypatch, tmp_path):
    monkeypatch.delenv("TTS_MODELS_DIR", raising=False)
    monkeypatch.delenv("CHATTERBOX_DIR", raising=False)
    monkeypatch.setenv("XDG_DATA_HOME", str(tmp_path))
    try:
        reloaded = importlib.reload(config)
        assert reloaded.MODELS_DIR == tmp_path / "agentsi/models"
        assert reloaded.CHATTERBOX_DIR == tmp_path / "agentsi/chatterbox"
        monkeypatch.setenv("TTS_MODELS_DIR", str(tmp_path / "m"))
        monkeypatch.setenv("CHATTERBOX_DIR", str(tmp_path / "c"))
        reloaded = importlib.reload(config)
        assert (reloaded.MODELS_DIR, reloaded.CHATTERBOX_DIR) == (tmp_path / "m", tmp_path / "c")
    finally:
        monkeypatch.undo()
        importlib.reload(config)


def test_data_root_lexicon_replaces_the_repo_lexicon(tmp_path):
    env = {"AGENTSI_HOME": str(tmp_path)}
    assert lexicon_file(env) == LEXICON_FILE
    (tmp_path / "lexicon.toml").write_text('[en]\nacme = "ak me"\n')
    assert lexicon_file(env) == tmp_path / "lexicon.toml"
    assert load_lexicon("en-us", lexicon_file(env)) == {"acme": "ak me"}
