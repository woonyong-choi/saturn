#!/usr/bin/env python3
from __future__ import annotations

import csv
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RAW = ROOT / "data" / "raw"
OUTPUT = ROOT / "data" / "processed" / "observations.csv"


def load_rows() -> list[dict]:
    rows = []
    for path in sorted(RAW.glob("*.jsonl")):
        with path.open(encoding="utf-8") as stream:
            for line in stream:
                if line.strip():
                    rows.append(json.loads(line))
    return rows


def public_row(row: dict) -> dict:
    return {
        "run_id": row.get("run_id"),
        "trial_id": row.get("trial_id"),
        "condition": row.get("condition"),
        "ts_utc": row.get("ts_utc"),
        "provider": row.get("provider"),
        "model": row.get("model"),
        "request_result": row.get("request_result"),
        "marker_start_count": row.get("marker_start_count"),
        "marker_complete_count": row.get("marker_complete_count"),
        "marker_touch_count": row.get("marker_touch_count"),
        "resume_child_execution": row.get("resume_child_execution"),
        "observation_status": row.get("observation_status"),
        "private_log": row.get("private_log"),
    }


def write_csv(rows: list[dict]) -> bytes:
    OUTPUT.parent.mkdir(parents=True, exist_ok=True)
    fields = [
        "run_id", "trial_id", "condition", "ts_utc", "provider", "model", "request_result",
        "marker_start_count", "marker_complete_count", "marker_touch_count",
        "resume_child_execution", "observation_status", "private_log",
    ]
    from io import StringIO
    buffer = StringIO(newline="")
    writer = csv.DictWriter(buffer, fieldnames=fields, lineterminator="\n")
    writer.writeheader()
    for row in rows:
        writer.writerow(public_row(row))
    content = buffer.getvalue().encode("utf-8")
    if "--check" not in sys.argv:
        OUTPUT.write_bytes(content)
    return content


def main() -> None:
    rows = load_rows()
    if not rows:
        raise SystemExit("raw 데이터가 없습니다")
    content = write_csv(rows)
    if "--check" in sys.argv:
        if not OUTPUT.exists() or OUTPUT.read_bytes() != content:
            raise SystemExit("processed CSV가 raw와 다릅니다")


if __name__ == "__main__":
    main()
