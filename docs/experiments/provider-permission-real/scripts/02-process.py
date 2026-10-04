#!/usr/bin/env python3
from __future__ import annotations

import csv
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RAW = ROOT / "data" / "raw"
OUT = ROOT / "data" / "processed" / "trials.csv"


def main() -> int:
    rows = []
    for path in sorted(RAW.glob("*.jsonl")):
        with path.open(encoding="utf-8") as stream:
            rows.extend(json.loads(line) for line in stream if line.strip())
    fields = [
        "run_id", "trial_id", "condition", "ts_utc", "provider", "model", "process_id", "thread_id",
        "request_result", "approval_request", "approval_methods", "approval_response", "marker_effect",
        "command_effect", "mcp_effect", "turn_status", "model_call_ordinal", "classification", "private_log",
    ]
    OUT.parent.mkdir(parents=True, exist_ok=True)
    with OUT.open("w", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields)
        writer.writeheader()
        for row in rows:
            writer.writerow({field: json.dumps(row[field], ensure_ascii=False) if isinstance(row[field], (list, dict)) else row[field] for field in fields})
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
