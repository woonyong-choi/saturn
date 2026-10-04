"""봉인·예산·눈가림·정규화 관측 수와 공개 집계를 검사한다."""

from __future__ import annotations

import json
import subprocess
from collections import Counter

from protocol import REFERENCE, make_body
from runtime import LIMITS, PRIVATE, PUBLIC, ROOT, digest, read, rows


def check_seal(name: str) -> None:
    for file, checksum in read(PRIVATE / name).items():
        if digest(PRIVATE / file) != checksum:
            raise RuntimeError("private seal mismatch: " + file)


# cost: io raw receipts and seals; basis: estimate
def main() -> None:
    for seal in ("sample-seal.json", "gold-seal.json"):
        check_seal(seal)
    for name, checksum in read(PRIVATE / "collection-seal.json")["files"].items():
        if digest(PUBLIC / name) != checksum:
            raise RuntimeError("preregistered collection changed: " + name)
    ledger = rows(PRIVATE / "calls.jsonl")
    trials = [r["trial_id"] for r in ledger]
    counts = Counter(r["kind"] for r in ledger)
    if len(trials) != len(set(trials)) or any(counts[k] > v for k, v in LIMITS.items()):
        raise RuntimeError("budget or reservation uniqueness failure")
    raw = {p.stem: read(p) for p in (PRIVATE / "raw").glob("*.json")}
    if not set(raw) <= set(trials):
        raise RuntimeError("unreserved completed calls")
    samples = read(PRIVATE / "samples.json")
    if len(samples) != 100 or len({s["sample_id"] for s in samples}) != 100:
        raise RuntimeError("sample identity failure")
    for case in samples:
        for lane in ["jev", *REFERENCE]:
            for repeat in range(1, 4 if lane == "jev" else 2):
                record = raw.get(lane + "-" + case["sample_id"] + "-" + str(repeat))
                if record is None:
                    continue
                if lane == "jev" and record["request"] != make_body(case):
                    raise RuntimeError("request drift")
                if lane != "jev":
                    prompt = record["prompt"]
                    if any(text in prompt for text in case["future"] if len(text) > 40):
                        raise RuntimeError("possible future text leakage")
    gold_last = max(r["ts_utc"] for r in ledger if r["trial_id"].startswith("gold-"))
    query_first = min(
        r["ts_utc"]
        for r in ledger
        if r["trial_id"].split("-")[0] in ["jev", *REFERENCE]
    )
    if gold_last >= query_first:
        raise RuntimeError("queries started before gold completion")
    observations = rows(PRIVATE / "processed/observations.jsonl")
    if len(observations) != 700:
        raise RuntimeError("wrong observation count")
    summary = read(PUBLIC / "results/summary.json")
    if summary["calls"] != dict(counts):
        raise RuntimeError("summary budget mismatch")
    if (
        summary["adjudication"]["analyzed"]
        != summary["conditions"]["jev-1"]["effective"]["n"]
    ):
        raise RuntimeError("analysis denominator mismatch")
    files = subprocess.check_output(
        ["git", "ls-files", "docs/experiments/target-model-choice"], cwd=ROOT, text=True
    ).splitlines()
    banned = ('"prompt":', '"stdout":', '"subsequent_user_inputs":', '"source_path":')
    for name in files:
        path = ROOT / name
        if path.suffix not in (".json", ".jsonl", ".csv", ".md"):
            continue
        if any(value in path.read_text() for value in banned):
            raise RuntimeError("private field in public output: " + name)
    print(
        json.dumps(
            dict(
                verified=True,
                samples=len(samples),
                observations=len(observations),
                calls=dict(counts),
            )
        )
    )


if __name__ == "__main__":
    main()
