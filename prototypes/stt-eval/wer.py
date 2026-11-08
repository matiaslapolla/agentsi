"""Word error rate of transcripts against a sentence list. Python standard library only.

Usage: python3 -I wer.py <sentences.json> <transcripts.jsonl> [<transcripts.jsonl> ...]
A transcripts file has one JSON object for each line: {"id": 1, "hyp": "text", "time_s": 0.4}.
Prints the corpus word error rate (all words together) and the mean time for each file.
"""
import json
import re
import sys
import unicodedata


def norm(text: str) -> list[str]:
    text = unicodedata.normalize("NFC", text).lower().replace("-", " ")
    text = re.sub(r"[^\w' ]+", " ", text)
    return text.split()


def edits(ref: list[str], hyp: list[str]) -> int:
    row = list(range(len(hyp) + 1))
    for i, r in enumerate(ref, 1):
        prev, row[0] = row[0], i
        for j, h in enumerate(hyp, 1):
            prev, row[j] = row[j], min(row[j] + 1, row[j - 1] + 1, prev + (r != h))
    return row[-1]


def main(argv: list[str]) -> int:
    sentences = {s["id"]: s["text"] for s in json.load(open(argv[1]))}
    for path in argv[2:]:
        rows = [json.loads(line) for line in open(path) if line.strip()]
        errors = words = 0
        for row in rows:
            ref = norm(sentences[row["id"]])
            errors += edits(ref, norm(row["hyp"]))
            words += len(ref)
        times = [r["time_s"] for r in rows if "time_s" in r]
        mean = sum(times) / len(times) if times else float("nan")
        print(f"{path}: WER {100 * errors / words:.1f}% ({errors}/{words} words, {len(rows)} sentences), mean {mean:.2f} s")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
