#!/usr/bin/env python3
"""route별 반복 일치 판정을 results/summary.json으로 만든다."""
from __future__ import annotations

import csv
import json
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INPUT = ROOT / "data" / "processed" / "trials.csv"
OUT = ROOT / "results" / "summary.json"


def main() -> int:
    grouped = defaultdict(list)
    with INPUT.open(encoding="utf-8", newline="") as stream:
        for row in csv.DictReader(stream):
            grouped[row["condition"]].append(row)
    routes = {}
    for condition, rows in sorted(grouped.items()):
        fingerprints = [tuple(row[field] for field in ("request_result", "tool_decision", "fixture_effect", "next_turn_effect", "restart_effect")) for row in rows]
        routes[condition] = {"n": len(rows), "fingerprints": [list(value) for value in fingerprints],
                            "classification": "확인" if len(rows) >= 3 and len(set(fingerprints)) == 1 else "확인 못 함"}
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({"routes": routes}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
