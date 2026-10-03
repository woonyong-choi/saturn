#!/usr/bin/env python3
from __future__ import annotations

import csv
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RAW = ROOT / "data" / "raw"
OUT = ROOT / "data" / "processed" / "trials.csv"
FIELDS = [
    "run_id", "trial_id", "condition", "ts_utc", "provider", "model", "request_result",
    "process_id", "thread_id", "user_input_request", "approval_request", "marker_effect",
    "rule_state", "reload_path", "reload_request_result", "private_log", "model_calls",
]


def main() -> int:
    by_trial = {}
    for path in sorted(RAW.glob("*.jsonl")):
        for line in path.read_text(encoding="utf-8").splitlines():
            value = json.loads(line)
            if value.get("condition", "").startswith("inventory."):
                continue
            by_trial[value["trial_id"]] = {field: value.get(field) for field in FIELDS}
    rows = list(by_trial.values())
    OUT.parent.mkdir(parents=True, exist_ok=True)
    with OUT.open("w", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=FIELDS)
        writer.writeheader()
        writer.writerows(rows)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
