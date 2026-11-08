"""Show one sentence at a time and record the microphone (16 kHz mono WAV) until Enter.

Usage: uv run python record.py [--from N]
Saves recordings/<id>.wav. Press Enter to stop a take; type r + Enter to redo; q to quit.
"""
import argparse, json, signal, subprocess, sys, time
from pathlib import Path

HERE = Path(__file__).parent


def record(path: Path) -> None:
    proc = subprocess.Popen(
        ["pw-record", "--rate", "16000", "--channels", "1", "--format", "s16", str(path)],
        stderr=subprocess.DEVNULL,
    )
    try:
        input("  [recording] press Enter to stop ")
    finally:
        proc.send_signal(signal.SIGINT)
        proc.wait()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--from", dest="start", type=int, default=1)
    args = ap.parse_args()
    sentences = json.loads((HERE / "sentences.json").read_text())
    out = HERE / "recordings"
    out.mkdir(exist_ok=True)
    for s in sentences:
        if s["id"] < args.start:
            continue
        while True:
            print(f"\n[{s['id']}/{len(sentences)}] Read this aloud, then press Enter to stop:\n\n  {s['text']}\n")
            input("  press Enter to start ")
            time.sleep(0.2)
            record(out / f"{s['id']}.wav")
            ans = input("  Enter = keep, r = redo, q = quit: ").strip().lower()
            if ans == "q":
                return 0
            if ans != "r":
                break
    print("\nDone. Next: uv run python eval.py --wavs recordings")
    return 0


if __name__ == "__main__":
    sys.exit(main())
