"""Run each speech model on each WAV; print WER, time, transcript.

Usage: uv run python eval.py --wavs recordings [--models a,b] [--provider cpu|cuda] [--out results/x.json]
WAV <id>.wav is scored against the sentence with the same id in sentences.json.
"""
import argparse, json, os, re, subprocess, sys, time
from pathlib import Path

CACHE = Path.home() / ".cache" / "agentsi-stt-eval"
os.environ.setdefault("HF_HOME", str(CACHE / "hf"))
HERE = Path(__file__).parent

import onnxruntime as _ort
if hasattr(_ort, 'preload_dlls'):
    _ort.preload_dlls()  # finds the pip nvidia-* CUDA 13 libs
import jiwer, onnx_asr, soundfile as sf, numpy as np  # noqa: E402

# name -> (onnx-asr model id, quantization)
ONNX_MODELS = {
    "whisper-base": ("onnx-community/whisper-base", None),
    "whisper-large-v3-turbo": ("onnx-community/whisper-large-v3-turbo", "int8"),
    "parakeet-v2": ("nemo-parakeet-tdt-0.6b-v2", "int8"),
    "parakeet-v3": ("nemo-parakeet-tdt-0.6b-v3", "int8"),
}
HEX_MODELS = {"hex-parakeet_unified_en": "parakeet_unified_en", "hex-whisper_large_v3_turbo": "whisper_large_v3_turbo"}


def norm(t: str) -> str:
    t = t.lower().replace("-", " ")
    t = re.sub(r"[^a-z0-9' ]+", " ", t)
    return re.sub(r"\s+", " ", t).strip()


def providers(kind: str):
    import onnxruntime as ort
    if kind == "cuda":
        avail = ort.get_available_providers()
        if "CUDAExecutionProvider" not in avail:
            raise SystemExit(f"CUDAExecutionProvider missing; have {avail}")
        return ["CUDAExecutionProvider", "CPUExecutionProvider"]
    return ["CPUExecutionProvider"]


def dir_size(p: Path) -> float:
    return sum(f.stat().st_size for f in p.rglob("*") if f.is_file()) / 1e6


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--wavs", required=True)
    ap.add_argument("--models", default=",".join(n for n in list(ONNX_MODELS) + list(HEX_MODELS) if not n.endswith("fp32")))
    ap.add_argument("--provider", default="cpu", choices=["cpu", "cuda"])
    ap.add_argument("--out")
    a = ap.parse_args()
    sentences = {s["id"]: s["text"] for s in json.loads((HERE / "sentences.json").read_text())}
    wavs = sorted(Path(a.wavs).glob("*.wav"), key=lambda p: int(p.stem))
    if not wavs:
        raise SystemExit(f"no WAV in {a.wavs}")
    results = []
    for name in a.models.split(","):
        rows, load_s = [], 0.0
        if name in ONNX_MODELS:
            mid, q = ONNX_MODELS[name]
            t0 = time.perf_counter()
            model = onnx_asr.load_model(mid, quantization=q, providers=providers(a.provider))
            load_s = time.perf_counter() - t0
            # warm-up: first call includes session setup
            model.recognize(np.zeros(16000, dtype=np.float32), sample_rate=16000)
            def run(p, model=model):
                wav, sr = sf.read(p, dtype="float32")
                if wav.ndim > 1: wav = wav.mean(axis=1)
                return str(model.recognize(wav, sample_rate=sr))
        else:
            def run(p, name=name):
                out = subprocess.run(["hex", "model", "transcribe", str(p), "--model", HEX_MODELS[name], "--language", "en"],
                                     capture_output=True, text=True, check=True).stdout
                return out.strip()
        for p in wavs:
            ref = sentences[int(p.stem)]
            dur = sf.info(p).duration
            t0 = time.perf_counter()
            hyp = run(p)
            dt = time.perf_counter() - t0
            rows.append({"id": int(p.stem), "audio_s": dur, "time_s": dt, "wer": jiwer.wer(norm(ref), norm(hyp)), "ref": ref, "hyp": hyp})
            print(f"{name:28} {p.stem:>3} wer={rows[-1]['wer']:.2f} t={dt:.2f}s  {hyp[:70]}", flush=True)
        results.append({"model": name, "provider": a.provider if name in ONNX_MODELS else "hex", "load_s": load_s, "rows": rows})
    print(f"\n{'model':28} {'provider':8} {'load s':>7} {'WER %':>6} {'s/sentence':>10} {'RTF':>6}")
    for r in results:
        rs = r["rows"]
        refs = " ".join(norm(x["ref"]) for x in rs); hyps = " ".join(norm(x["hyp"]) for x in rs)
        wer = jiwer.wer(refs, hyps) * 100
        t = sum(x["time_s"] for x in rs); d = sum(x["audio_s"] for x in rs)
        print(f"{r['model']:28} {r['provider']:8} {r['load_s']:7.1f} {wer:6.1f} {t/len(rs):10.2f} {t/d:6.2f}")
    if a.out:
        Path(a.out).parent.mkdir(parents=True, exist_ok=True)
        Path(a.out).write_text(json.dumps(results, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main())
