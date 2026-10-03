#!/usr/bin/env python3
"""공개 raw를 그대로 정규화한다. 큰 원문은 메인 저장소 private log를 가리킨다."""
from __future__ import annotations

import csv
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RAW = ROOT / "data" / "raw"
OUT = ROOT / "data" / "processed" / "trials.csv"
FIELDS = ["run_id", "trial_id", "condition", "ts_utc", "provider", "request_result",
          "process_id", "tool_decision", "fixture_effect", "next_turn_effect",
          "restart_effect", "private_log"]


def main() -> int:
    rows = []
    for path in sorted(RAW.glob("*.jsonl")):
        for line in path.read_text(encoding="utf-8").splitlines():
            value = json.loads(line)
            rows.append({field: value.get(field) for field in FIELDS})
    OUT.parent.mkdir(parents=True, exist_ok=True)
    with OUT.open("w", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=FIELDS)
        writer.writeheader()
        writer.writerows(rows)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
