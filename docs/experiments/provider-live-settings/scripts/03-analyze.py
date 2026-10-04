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

# 성공 응답이나 설정 readback만 있고 실행 중 적용을 행동으로 입증하지 못한 route.
# 세 회차가 같아도 `확인`으로 분류하지 않는다.
RESPONSE_ONLY = {
    "claude.control.set_model": "control response만 성공. 실제 모델 변경은 검증하지 않음",
    "codex.config.batchWrite.reloadUserConfig": "config/read의 readback만 확인. 다음 턴 행동 차이는 입증하지 못함",
    "codex.experimentalFeature.enablement.set": "feature list의 값만 확인. 후속 codex-live-reload에서 같은 thread 행동은 3/3 바뀌지 않음",
    "codex.turn.start.approvalPolicy": "approval request가 발생하지 않아 승인 정책 차이를 입증하지 못함",
}


def main() -> int:
    grouped = defaultdict(list)
    with INPUT.open(encoding="utf-8", newline="") as stream:
        for row in csv.DictReader(stream):
            grouped[row["condition"]].append(row)
    routes = {}
    for condition, rows in sorted(grouped.items()):
        fingerprints = [tuple(row[field] for field in ("request_result", "tool_decision", "fixture_effect", "next_turn_effect", "restart_effect")) for row in rows]
        if len(rows) < 3 or all(row.get("fixture_distinguishes") == "False" for row in rows) or condition in RESPONSE_ONLY:
            classification = "확인 못 함"
        elif len(set(fingerprints)) == 1:
            classification = "확인"
        else:
            classification = "불안정"
        routes[condition] = {"n": len(rows), "fingerprints": [list(value) for value in fingerprints],
                            "classification": classification}
        if condition in RESPONSE_ONLY:
            routes[condition]["reason"] = RESPONSE_ONLY[condition]
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({"routes": routes}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
