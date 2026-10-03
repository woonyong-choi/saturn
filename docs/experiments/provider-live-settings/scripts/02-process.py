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
          "restart_effect", "private_log", "fixture_distinguishes"]


def main() -> int:
    by_trial = {}
    for path in sorted(RAW.glob("*.jsonl")):
        for line in path.read_text(encoding="utf-8").splitlines():
            value = json.loads(line)
            # 같은 trial을 계측 필드 추가 뒤 다시 실행한 경우 최신 raw를 사용한다.
            normalized = {field: value.get(field) for field in FIELDS}
            if value.get("condition", "").startswith("codex.file."):
                normalized["fixture_distinguishes"] = False
            by_trial[value["trial_id"]] = normalized
    rows = list(by_trial.values())
    OUT.parent.mkdir(parents=True, exist_ok=True)
    with OUT.open("w", encoding="utf-8", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=FIELDS)
        writer.writeheader()
        writer.writerows(rows)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
