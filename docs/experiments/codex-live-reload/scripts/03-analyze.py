#!/usr/bin/env python3
from __future__ import annotations

import csv
import json
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INPUT = ROOT / "data" / "processed" / "trials.csv"
OUT = ROOT / "results" / "summary.json"


def boolean(value: str | None) -> bool:
    return value == "True"


def classify(rows: list[dict], expected: list[tuple[str, bool]] | None = None) -> str:
    if len(rows) < 3:
        return "확인 못 함"
    if expected is None:
        fingerprints = [tuple(row.get(key) for key in ("request_result", "approval_request", "marker_effect")) for row in rows]
    else:
        fingerprints = [
            (row.get("condition"), row.get("request_result"), row.get("approval_request"), row.get("marker_effect"))
            for row in rows
        ]
    if any(row.get("request_result") in {"timeout", "model_call_cap"} for row in rows):
        return "확인 못 함"
    return "확인" if len(set(fingerprints)) == 1 else "불안정"


def main() -> int:
    rows = list(csv.DictReader(INPUT.open(encoding="utf-8", newline="")))
    grouped = defaultdict(list)
    for row in rows:
        grouped[row["condition"]].append(row)
    routes = {}
    question_states = defaultdict(list)
    for row in rows:
        if row["condition"].startswith("codex.questions."):
            question_states[row["condition"].rsplit(".", 1)[-1]].append(row)
    question_trials = defaultdict(dict)
    reverse_trials = defaultdict(dict)
    for row in rows:
        if row["condition"].startswith("codex.questions."):
            trial_parts = row["trial_id"].split("-")
            trial = "-".join(trial_parts[:3] if ".reverse." in row["condition"] else trial_parts[:2])
            state = row["condition"].rsplit(".", 1)[-1]
            if ".reverse." in row["condition"]:
                reverse_trials[trial][state] = boolean(row["user_input_request"])
            else:
                question_trials[trial][state] = boolean(row["user_input_request"])
    question_sequences = [tuple(question_trials[trial].get(state) for state in ("on-before", "off", "on-after")) for trial in sorted(question_trials)]
    routes["H1.codex.questions.toggle"] = {
        "n": len(question_sequences), "sequences": question_sequences,
        "classification": "확인" if question_sequences and len(set(question_sequences)) == 1 else ("불안정" if question_sequences else "확인 못 함"),
    }
    reverse_sequences = [tuple(reverse_trials[trial].get(state) for state in ("off-before", "on-after")) for trial in sorted(reverse_trials)]
    routes["H1.codex.questions.reverse"] = {
        "n": len(reverse_sequences), "sequences": reverse_sequences,
        "classification": "확인" if reverse_sequences and len(set(reverse_sequences)) == 1 else ("불안정" if reverse_sequences else "확인 못 함"),
    }
    for condition, condition_rows in sorted(grouped.items()):
        if condition.startswith("codex.questions."):
            continue
        routes[condition] = {
            "n": len(condition_rows),
            "fingerprints": [[row.get(key) for key in ("request_result", "approval_request", "marker_effect")] for row in condition_rows],
            "classification": classify(condition_rows),
        }
    for direction in ("allow", "forbidden"):
        for path in ("none", "config_batch", "new_process"):
            matches = []
            for condition, condition_rows in grouped.items():
                if direction in condition and path in condition:
                    matches.extend(condition_rows)
            if matches:
                routes[f"H2.{direction}.{path}"] = {
                    "n": len(matches),
                    "fingerprints": [[row.get(key) for key in ("request_result", "approval_request", "marker_effect")] for row in matches],
                    "classification": classify(matches),
                }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps({"routes": routes, "turn_rows": len(rows)}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
