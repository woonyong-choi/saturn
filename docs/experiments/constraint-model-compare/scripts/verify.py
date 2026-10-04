"""호출 예산·눈가림·정답 다수결·집계 재현의 불변 조건을 검사한다."""

from __future__ import annotations

import hashlib
import json
import os
from collections import Counter
from datetime import datetime, timedelta
from pathlib import Path

from protocol import gold_prompt, parse, query_prompt
from runtime import LIMITS, PRIVATE, PUBLIC, QUESTION, read, rows
from seal import check_seals


def require(condition: bool, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def verify_gold(cases: list[dict]) -> None:
    gold = {r["sample_id"]: r for r in read(PRIVATE / "gold.json")}
    require(set(gold) == {c["sample_id"] for c in cases}, "gold coverage mismatch")
    for case in cases:
        item = gold[case["sample_id"]]
        votes = []
        for repeat in range(1, len(item["judgments"]) + 1):
            path = PRIVATE / "raw" / f"gold-sol-{case['sample_id']}-r{repeat}.json"
            if not path.exists():
                continue
            record = read(path)
            require(record["prompt"] == gold_prompt(case), "gold prompt mismatch")
            parsed = parse(record, "gold")
            if parsed["valid"]:
                votes.append(parsed["value"]["label"])
        count = Counter(votes)
        label, n = count.most_common(1)[0] if count else ("uncertain", 0)
        require(
            item["label"] == (label if n >= 2 else "uncertain"),
            "gold majority mismatch",
        )
        first = item["judgments"][:2]
        disagreement = (
            not all(r["valid"] for r in first)
            or len({r["value"]["label"] for r in first if r["valid"]}) != 1
        )
        require(
            len(item["judgments"]) == (3 if disagreement else 2),
            "third vote allocation mismatch",
        )


def verify_queries(cases: list[dict]) -> None:
    for case in cases:
        for name in ("sonnet", "opus", "haiku", "astra", "sol", "terra", "luna", "jev"):
            for repeat in (1, 2, 3) if name == "jev" else (1,):
                path = (
                    PRIVATE / "raw" / f"query-{name}-{case['sample_id']}-r{repeat}.json"
                )
                require(path.exists(), "missing query receipt")
                record = read(path)
                if name == "jev":
                    require(
                        record["request"]
                        == {
                            "model": "jev-1.13.0",
                            "state": case["state"],
                            "questions": {
                                "is_constraint": {
                                    "type": "noul",
                                    "instructions": QUESTION,
                                }
                            },
                        },
                        "jev request leaked or changed",
                    )
                else:
                    require(
                        record["prompt"] == query_prompt(case),
                        "query prompt leaked or changed",
                    )


def verify_order(ledger: list[dict]) -> None:
    first_query = min(
        datetime.fromisoformat(r["ts_utc"])
        for r in ledger
        if r["trial_id"].startswith("query-")
    )
    gold_ends = []
    for path in (PRIVATE / "raw").glob("gold-*.json"):
        record = read(path)
        gold_ends.append(
            datetime.fromisoformat(record["ts_utc"])
            + timedelta(seconds=record["latency_s"])
        )
    require(first_query > max(gold_ends), "query started before gold completed")


def verify_no_secret(paths: list[Path]) -> None:
    key = os.environ.get("SATURN_JUDGE_KEY")
    require(bool(key), "exact key verification requires process environment")
    needle = key.encode()
    require(
        all(needle not in path.read_bytes() for path in paths),
        "exact key found in artifacts",
    )


def main() -> None:
    cases = read(PRIVATE / "samples.json")
    require(len(cases) == 100, "sample count mismatch")
    require(len({c["sample_id"] for c in cases}) == 100, "duplicate sample")
    require(
        all(c["future_user_count"] >= 3 for c in cases), "insufficient future context"
    )
    ledger = rows(PRIVATE / "calls.jsonl")
    require(
        len({r["trial_id"] for r in ledger}) == len(ledger),
        "duplicate call reservation",
    )
    calls = Counter(r["kind"] for r in ledger)
    require(
        all(calls[k] <= limit for k, limit in LIMITS.items()), "call budget exceeded"
    )
    check_seals()
    verify_gold(cases)
    verify_queries(cases)
    verify_order(ledger)
    summary = read(PUBLIC / "results/summary.json")
    for name, checksum in summary["hashes"].items():
        require(
            hashlib.sha256((PRIVATE / name).read_bytes()).hexdigest() == checksum,
            "stale summary input: " + name,
        )
    require(summary["calls"] == dict(calls), "summary call count mismatch")
    for result in summary["models"].values():
        require(result["format_failure"]["n"] == 100, "format denominator mismatch")
    files = [
        p
        for directory in (PUBLIC, PRIVATE)
        for p in directory.rglob("*")
        if p.is_file() and "runtime" not in p.parts and "__pycache__" not in p.parts
    ]
    verify_no_secret(files)
    print(
        json.dumps(
            dict(
                verification="passed",
                cases=len(cases),
                calls=dict(calls),
                exact_secret_matches=0,
            )
        )
    )


if __name__ == "__main__":
    main()
