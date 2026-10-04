"""공개 생성문을 원문 조각과 대조하고 원자료 해시를 불변으로 봉인한다."""

from __future__ import annotations

import hashlib
from pathlib import Path

from runtime import MAIN, PRIVATE, PUBLIC, read, write

SPAN = 8
RAW_FILES = {
    "calls.jsonl",
    "run.json",
    "restored.json",
    "generated-private.json",
    "constraint-preliminary.json",
    "excluded.json",
    "continuation-candidates.json",
    "continuation-census.json",
    "continuation-labels.json",
    "continuation.json",
    "continuation-exclusions.json",
    "scope-label-pairs.json",
    "scope-label-grades.json",
    "items.json",
    "j2-ids.json",
    "input-hashes.json",
    "flow.json",
    "scope-prediction-pairs.json",
    "scope-prediction-grades.json",
    "scope-rounding-pairs.json",
    "scope-rounding-grades.json",
    "privacy-sources.json",
}


def source_fragments() -> set[str]:
    paths = sorted(
        (MAIN / ".local/experiments/constraint-deep/conversations").glob("*.json")
    )
    if not paths:
        raise ValueError("private redaction sources are missing")
    hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
    expected = {}
    for line in (
        (PUBLIC.parent / "constraint-deep/data/SHA256SUMS").read_text().splitlines()
    ):
        digest, relative = line.split("  ", 1)
        if relative.startswith("conversations/"):
            expected[Path(relative).name] = digest
    if not expected or hashes != expected:
        raise ValueError("redaction sources differ from the prior experiment seal")
    manifest = PRIVATE / "privacy-sources.json"
    if manifest.exists() and read(manifest) != hashes:
        raise ValueError("private redaction source inventory changed")
    fragments = set()
    for path in paths:
        turns = read(path).get("turns")
        if not isinstance(turns, list):
            raise ValueError("invalid redaction source: " + path.name)
        for turn in turns:
            text = turn["text"]
            if not isinstance(text, str):
                raise ValueError("invalid source text")
            fragments.update(text[i : i + SPAN] for i in range(len(text) - SPAN + 1))
    if not fragments:
        raise ValueError("private redaction fragments are empty")
    if not manifest.exists():
        write(manifest, hashes)
    return fragments


def redact_fragments(text: str, fragments: set[str]) -> str:
    intervals = []
    for start in range(len(text) - SPAN + 1):
        if text[start : start + SPAN] not in fragments:
            continue
        if intervals and intervals[-1][1] >= start:
            intervals[-1][1] = start + SPAN
        else:
            intervals.append([start, start + SPAN])
    end = 0
    parts = []
    for start, stop in intervals:
        parts.extend((text[end:start], "[원문 가림]"))
        end = stop
    return "".join(parts) + text[end:]


def raw_paths() -> list[Path]:
    files = [PRIVATE / name for name in RAW_FILES]
    if any(not p.is_file() for p in files):
        raise ValueError("raw seal inputs are incomplete")
    for name in ("codex", "claude", "jev", "workflows"):
        files.extend(p for p in (PRIVATE / name).rglob("*") if p.is_file())
    return sorted(files)


# cost: io O(n) file reads; vars: n = raw files; basis: estimate
def check_raw_seal(*, create: bool = False) -> None:
    manifest = PUBLIC / "data/RAW_SHA256SUMS"
    paths = raw_paths()
    actual = "".join(
        hashlib.sha256(p.read_bytes()).hexdigest()
        + "  "
        + str(p.relative_to(PRIVATE))
        + "\n"
        for p in paths
    )
    if manifest.exists():
        if manifest.read_text() != actual:
            raise ValueError("immutable raw seal mismatch; analysis refused")
    elif create:
        with manifest.open("x") as stream:
            stream.write(actual)
    else:
        raise ValueError("immutable raw seal missing")


def public_rows(items: list[dict]) -> list[dict]:
    fragments = source_fragments()
    result = []
    for item in items:
        if item["task"] != "constraint" or item["source"] == "real":
            continue
        text = redact_fragments(item["text"], fragments)
        result.append(
            dict(
                id=item["id"],
                source=item["source"],
                text=text,
                text_redacted=text != item["text"],
                rule_ids=item["rule_ids"],
                kind=item["gold"]["kind"],
                target=item["gold"]["target"],
            )
        )
    return result
