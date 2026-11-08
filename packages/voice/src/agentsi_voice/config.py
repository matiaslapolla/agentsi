import os
import tomllib
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path

PACKAGE_ROOT = Path(__file__).resolve().parents[2]
CODE_ROOT = PACKAGE_ROOT.parents[1]
VOICES_FILE = PACKAGE_ROOT / "voices.toml"
LEXICON_FILE = PACKAGE_ROOT / "lexicon.toml"
DATA_HOME = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
MODELS_DIR = Path(os.environ.get("TTS_MODELS_DIR", DATA_HOME / "agentsi/models"))
KOKORO_MODEL = MODELS_DIR / "kokoro" / "kokoro-v1.0.onnx"
KOKORO_VOICES = MODELS_DIR / "kokoro" / "voices-v1.0.bin"
PIPER_DIR = MODELS_DIR / "piper"
CHATTERBOX_DIR = Path(os.environ.get("CHATTERBOX_DIR", DATA_HOME / "agentsi/chatterbox"))
CHATTERBOX_REFS = MODELS_DIR / "chatterbox" / "refs"
ENGINES = ("kokoro", "piper", "chatterbox")
DEFAULT_LANG = "en-us"


@dataclass(frozen=True)
class VoiceRef:
    engine: str
    name: str

    @property
    def id(self) -> str:
        return f"{self.engine}:{self.name}"


def _parse(spec: str) -> VoiceRef:
    engine, _, name = spec.partition(":")
    if engine not in ENGINES or not name:
        raise ValueError(f"invalid voice spec {spec!r}, expected <{'|'.join(ENGINES)}>:<voice>")
    return VoiceRef(engine, name)


def data_root(env: Mapping[str, str] = os.environ) -> Path:
    """Same order as `resolveDataRoot` in packages/core/src/paths.ts."""

    def from_home(value: str) -> Path:
        return (Path.home() / value.replace("~", str(Path.home()), 1)) if value.startswith("~") else Path.home() / value

    if fromenv := env.get("AGENTSI_HOME", "").strip():
        return from_home(fromenv)
    home_file = Path(env.get("XDG_CONFIG_HOME", "").strip() or Path.home() / ".config") / "agentsi" / "home"
    if home_file.is_file() and (line := (home_file.read_text().splitlines() or [""])[0].strip()):
        return from_home(line)
    return CODE_ROOT


def voices_file(env: Mapping[str, str] = os.environ) -> Path:
    """The data root's `voices.toml` is the private map; the repo file is the public default."""
    private = data_root(env) / "voices.toml"
    return private if private.is_file() else VOICES_FILE


def lexicon_file(env: Mapping[str, str] = os.environ) -> Path:
    """Same rule as `voices_file`: the data root's `lexicon.toml` replaces the repo file."""
    private = data_root(env) / "lexicon.toml"
    return private if private.is_file() else LEXICON_FILE


def _read_voices(path: Path | None) -> dict:
    with (path or voices_file()).open("rb") as f:
        return tomllib.load(f)


def load_lang(path: Path | None = None) -> str:
    return _read_voices(path).get("lang", DEFAULT_LANG)


def load_voices(path: Path | None = None) -> tuple[dict[str, VoiceRef], VoiceRef]:
    data = _read_voices(path)
    roles = {role: _parse(spec) for role, spec in data.get("roles", {}).items()}
    return roles, _parse(data["default"])


def voice_for(role: str, path: Path | None = None) -> VoiceRef:
    roles, default = load_voices(path)
    return roles.get(role) or roles.get(role.partition(".")[0], default)


def load_lexicon(lang: str, path: Path | None = None) -> dict[str, str]:
    """The lexicon section is the language prefix of `lang`: `en` for `en-us`."""
    with (path or lexicon_file()).open("rb") as f:
        return {k.lower(): v for k, v in tomllib.load(f).get(lang.partition("-")[0], {}).items()}
