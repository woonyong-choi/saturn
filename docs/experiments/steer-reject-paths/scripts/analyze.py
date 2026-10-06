"""원자료에서 조건별 불변식과 경로 분류를 계산한다. `analyze.py [--check]`. 같은 원자료는 같은 결과 파일을 만든다."""

from __future__ import annotations

import gzip
import json
import re
import sys
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
EXP = HERE.parent
RAW = EXP / "data" / "raw"
RUNS = ["formal", "formal2"]
OUT = EXP / "results"
ORDER = ["steer_ok", "steer_review", "steer_compact", "steer_compact_late", "steer_race", "steer_compact_burst"]


def classify(trial: dict) -> dict:
    plan = trial["plan"]
    b = str(plan["b"])
    rec = trial["inputs_seen"][b]
    states = [entry[0][0] for entry in rec["states"]]
    dispositions = [entry[0][1] for entry in rec["states"]]
    final = states[-1]
    mark = f"MARK-B-{plan['rep']}"
    marks = trial["notes"].count(mark)
    db_row = next(row for row in trial["db"]["inputs"] if row["id"] == plan["b"])
    steered = db_row["steered_run"] is not None
    refused_logs = [line for line in trial["engine_steer_log"] if "steer was refused" in line]
    reasons = [re.sub(r"\x1b\[[0-9;]*m", "", line).split("reason=")[-1] for line in refused_logs]
    reasons = [re.sub(r"`[0-9a-f-]{36}`", "`<id>`", reason).strip() for reason in reasons]
    not_delivered = [re.sub(r"\x1b\[[0-9;]*m", "", line).split("reason=")[-1].strip() for line in trial["engine_steer_log"] if "not delivered" in line]
    # Delivering 뒤 Queued로 돌아온 횟수와 그 뒤 Delivering 횟수
    returns = sum(1 for i in range(1, len(states)) if states[i] == "Queued" and states[i - 1] == "Delivering")
    deliveries = sum(1 for i in range(1, len(states)) if states[i] == "Delivering" and states[i - 1] != "Delivering")
    if steered:
        path = "steered"
    elif refused_logs:
        path = "refused_queued"
    elif "Steer" in dispositions and final == "Applied":
        path = "fallback_new_turn"
    elif final == "Applied":
        path = "queued_normal"
    else:
        path = "lost"
    inv1 = final == "Applied"
    inv2 = marks == 1
    inv3 = returns <= 1 and deliveries - returns <= 1 and deliveries <= returns + 1
    return {
        "cond": plan["cond"],
        "rep": plan["rep"],
        "offset": plan.get("offset"),
        "settled": plan.get("settled"),
        "final": final,
        "states": states,
        "marks": marks,
        "path": path,
        "refused_reasons": reasons,
        "not_delivered_reasons": not_delivered,
        "inv1_applied": inv1,
        "inv2_exactly_once": inv2,
        "inv3_one_return": inv3,
        "ok": inv1 and inv2 and inv3,
        "provider_turn_requests": trial["provider_calls"],
        "key_files_with_key": trial["key_files_with_key"],
    }


def main() -> None:
    rows = []
    for run in RUNS:
        for path in sorted((RAW / run).glob("*.json.gz")):
            with gzip.open(path, "rt") as handle:
                rows.append({"run": run, **classify(json.load(handle))})
    rows.sort(key=lambda r: (ORDER.index(r["cond"]), r["rep"]))
    summary = {}
    for cond in ORDER:
        sub = [r for r in rows if r["cond"] == cond]
        summary[cond] = {
            "n": len(sub),
            "ok": sum(r["ok"] for r in sub),
            "paths": dict(sorted(Counter(r["path"] for r in sub).items())),
            "refused_reasons": dict(sorted(Counter(x for r in sub for x in r["refused_reasons"]).items())),
            "not_delivered_reasons": dict(sorted(Counter(x for r in sub for x in r["not_delivered_reasons"]).items())),
            "final": dict(sorted(Counter(r["final"] for r in sub).items())),
        }
    result = {"trials": rows, "summary": summary, "provider_turn_requests": sum(r["provider_turn_requests"] for r in rows), "key_files_with_key": sum(r["key_files_with_key"] for r in rows)}
    text = json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    target = OUT / "summary.json"
    if "--check" in sys.argv:
        assert target.read_text() == text, "analysis is not byte-identical"
        print("identical")
        return
    OUT.mkdir(exist_ok=True)
    target.write_text(text)
    for cond, s in summary.items():
        print(cond, s["ok"], "/", s["n"], s["paths"], s["final"])


if __name__ == "__main__":
    main()
