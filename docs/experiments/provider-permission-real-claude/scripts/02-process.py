#!/usr/bin/env python3
"""raw의 회차 행을 trials.csv 한 표로 만든다."""
from __future__ import annotations

import csv
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FIELDS = ["run_id", "condition", "trial_id", "model_call_ordinal", "ts_utc", "turn_status", "classification",
          "request_count", "touch_request_count", "request_tools", "saturn_responses", "tools_attempted",
          "marker_effect", "hook_calls", "mcp_fixture_calls", "token_seen", "tool_result_errors"]


def main() -> int:
    rows = []
    for path in sorted((ROOT / "data" / "raw").glob("claude-*.jsonl")):
        for line in path.read_text(encoding="utf-8").splitlines():
            row = json.loads(line)
            if row.get("kind") == "meta":
                continue
            run = row["run"]
            rows.append({"run_id": row["run_id"], **{k: run[k] for k in FIELDS if k != "run_id"}})
    out = ROOT / "data" / "processed" / "trials.csv"
    out.parent.mkdir(parents=True, exist_ok=True)
    with out.open("w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=FIELDS, lineterminator="\n")
        writer.writeheader()
        for row in rows:
            writer.writerow({k: ("|".join(map(str, v)) if isinstance(v, list) else ("" if v is None else v))
                             for k, v in row.items()})
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
