#!/usr/bin/env python3
from __future__ import annotations

import csv
import json
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PROCESSED = ROOT / "data" / "processed" / "trials.csv"
SUMMARY = ROOT / "results" / "summary.json"
FORMAL = {"deny_shell", "workdir_edit", "readonly_command", "outside_edit", "add_dir_edit", "git_edit", "mcp_prompt"}


def main() -> int:
    groups = defaultdict(list)
    calls = 0
    with PROCESSED.open(encoding="utf-8", newline="") as stream:
        for row in csv.DictReader(stream):
            groups[row["condition"]].append(row)
            if row["model_call_ordinal"] not in {"", "None"}:
                calls += 1
    routes = []
    non_formal = []
    for condition, rows in sorted(groups.items()):
        if condition not in FORMAL:
            non_formal.append({"condition": condition, "n": len(rows), "rows": rows})
            continue
        classifications = [row["classification"] for row in rows]
        routes.append({
            "condition": condition,
            "n": len(rows),
            "classification_counts": {value: classifications.count(value) for value in sorted(set(classifications))},
            "all_same": len(set(classifications)) == 1 and len(rows) == 3,
            "trials": [
                {
                    "trial_id": row["trial_id"],
                    "approval_request": row["approval_request"] == "True",
                    "approval_methods": json.loads(row["approval_methods"] or "[]"),
                    "marker_effect": row["marker_effect"] == "True",
                    "command_effect": row["command_effect"] == "True",
                    "mcp_effect": row["mcp_effect"] == "True",
                    "classification": row["classification"],
                }
                for row in rows
            ],
        })
    SUMMARY.parent.mkdir(parents=True, exist_ok=True)
    SUMMARY.write_text(json.dumps({"routes": routes, "non_formal": non_formal, "codex_turn_start_calls": calls, "codex_call_limit": 80}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
