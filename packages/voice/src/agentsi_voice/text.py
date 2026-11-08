import re

SENTENCE_END = re.compile(r"[.!?…][\"'»)]*$")
EDGE = re.compile(r"^(\W*)(.*?)(\W*)$")


def _speak(token: str, lexicon: dict[str, str]) -> str:
    prefix, core, suffix = EDGE.match(token).groups()
    replacement = lexicon.get(core.lower())
    return token if replacement is None else f"{prefix}{replacement}{suffix}"


def sentences(text: str, lexicon: dict[str, str]) -> list[str]:
    """Split on sentence-final punctuation and apply the pronunciation lexicon."""
    out: list[str] = []
    current: list[str] = []
    for token in text.split():
        current.append(_speak(token, lexicon))
        if SENTENCE_END.search(token):
            out.append(" ".join(current))
            current = []
    if current:
        out.append(" ".join(current))
    return out


def halve(sentence: str) -> tuple[str, str] | None:
    """Split at the comma nearest the middle, so a long sentence can be spoken in two takes."""
    commas = [
        i
        for i, c in enumerate(sentence)
        if c == "," and sentence[i + 1 : i + 2] == " " and any(ch.isalnum() for ch in sentence[:i])
    ]
    if not commas:
        return None
    at = min(commas, key=lambda i: abs(i + 1 - len(sentence) / 2))
    return sentence[: at + 1], sentence[at + 2 :]
