"""수집 뒤 확인하는 근거 유실 경로다. 확인 가설의 판정을 바꾸지 않는다."""

from __future__ import annotations

import gzip
import json
from pathlib import Path

EXP = Path(__file__).resolve().parents[1]


def main() -> None:
    rows = []
    for path in sorted((EXP / "data/raw").glob("*.gz")):
        raw = json.load(gzip.open(path, "rt"))
        if raw["seed"] >= 1000 or not raw.get("store"):
            continue
        store = raw["store"]
        header = raw["facts"]["header_new"]
        inputs = [i for i in store["inputs"] if header in i.get("text", "")]
        observed = []
        for judgment in store["judgments"]:
            if "compact" not in judgment["question_sets"] or not judgment.get("sent"):
                continue
            sent = json.loads(judgment["sent"])
            observed.append(
                {
                    "judgment_id": judgment["id"],
                    "header_in_state": header in str(sent.get("state", "")),
                    "header_anywhere_in_request": header in judgment["sent"],
                }
            )
        rows.append(
            {
                "trial": raw["trial"],
                "source_input_ids": [i["id"] for i in inputs],
                "constraint_packet_items": sum(
                    i["zone"] == "Constraints" and i["form"] is not None
                    for i in store["handoff_packet_items"]
                ),
                "compact_requests": observed,
                "last_grade_header_ok": raw["grades"].get("f3", {}).get("header_ok"),
                "last_grade_retries_ok": raw["grades"].get("f3", {}).get("retries_ok"),
                "f3_retries_disagreement": bool(raw["grades"].get("f3"))
                and not raw["grades"]["f3"].get("retries_ok", False),
                "nonliteral_grade_values": [
                    {"label": label, "name": name}
                    for label, grade in raw["grades"].items()
                    for name, value in grade.get("values", {}).items()
                    if value is None
                ],
                "input_failures": [
                    {"label": t["label"], "status": t["status"]}
                    for t in raw["turns"]
                    if t["status"] != "ok"
                ],
            }
        )
    (EXP / "results/exploratory-audit.json").write_text(
        json.dumps(
            {"scope": "exploratory; not an adoption criterion", "trials": rows},
            ensure_ascii=False,
            sort_keys=True,
            indent=2,
        )
        + "\n"
    )
    print("audited", len(rows), "trials")


if __name__ == "__main__":
    main()
