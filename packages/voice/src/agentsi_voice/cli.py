import argparse
import json
import sys
from pathlib import Path

from agentsi_voice.config import MODELS_DIR, load_voices, voice_for, voices_file


def _doctor() -> int:
    from agentsi_voice.doctor import pill_status, player_status, problems

    print(player_status())
    print(pill_status())
    found = problems()
    if found:
        for p in found:
            print(f"FAIL {p}", file=sys.stderr)
        return 1
    roles, default = load_voices()
    print(json.dumps({"ok": True, "models_dir": str(MODELS_DIR), "voices_file": str(voices_file()), "default": default.id, "roles": {r: v.id for r, v in roles.items()}}))
    return 0


def _render(script_path: Path, out_dir: Path) -> int:
    from agentsi_voice.render import render

    script = json.loads(script_path.read_text())
    if not isinstance(script, list) or not script:
        print("script must be a non-empty JSON array of {role, text}", file=sys.stderr)
        return 2
    for i, item in enumerate(script):
        if not isinstance(item, dict) or not str(item.get("role", "")) or not str(item.get("text", "")).strip():
            print(f"segment {i} needs non-empty role and text", file=sys.stderr)
            return 2
    print(json.dumps(render(script, out_dir)))
    return 0


def _say(role: str, out_dir: Path) -> int:
    """Speak the sentences on stdin, one per line, with the voice of a role.

    Writes <out_dir>/<n>.wav for each and prints `WAV <path>` once the file is complete.
    """
    import numpy as np
    from scipy.io import wavfile

    from agentsi_voice.engines import engine_for
    from agentsi_voice.render import RATE, _level
    from agentsi_voice.render import _say as take

    ref = voice_for(role)
    engine = engine_for(ref)
    out_dir.mkdir(parents=True, exist_ok=True)
    n = 0
    for line in sys.stdin:
        text = line.strip()
        if not text:
            continue
        audio = _level(take(engine, text, ref.name))
        path = out_dir / f"{n}.wav"
        wavfile.write(str(path), RATE, (np.clip(audio, -1, 1) * 32767).astype("<i2"))
        print(f"WAV {path}", flush=True)
        n += 1
    if n == 0:
        print("nothing to say: stdin is empty", file=sys.stderr)
        return 2
    return 0


def main() -> None:
    parser = argparse.ArgumentParser(prog="agentsi-voice")
    sub = parser.add_subparsers(dest="command", required=True)
    r = sub.add_parser("render", help="render a call script to call.webm and timings.json")
    r.add_argument("script", type=Path)
    r.add_argument("out_dir", type=Path)
    sub.add_parser("doctor", help="check models, ffmpeg, voices, the audio player and qs")
    sy = sub.add_parser("say", help="speak the sentences on stdin (one per line) with a role voice into numbered wav files")
    sy.add_argument("role")
    sy.add_argument("out", type=Path, help="folder for the <n>.wav files")
    args = parser.parse_args()
    if args.command == "doctor":
        sys.exit(_doctor())
    if args.command == "say":
        sys.exit(_say(args.role, args.out))
    sys.exit(_render(args.script, args.out_dir))
