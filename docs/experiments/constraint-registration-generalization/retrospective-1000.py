"""Jev 층화 표본 1,000건을 봉인하고 Astra 독립 판정을 실행한다."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent


def load(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"script unavailable: {path.name}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ret = load("retrospective", HERE / "retrospective.py")
audit = load("retrospective_analyze", HERE / "retrospective-analyze.py")
rounding = load("rounding_audit", HERE / "rounding_audit.py")
BASE = ret.OUT
QUALITY = BASE / "quality1000"


def prepare() -> None:
    cases = json.loads((BASE / "cases.json").read_text())
    cases += json.loads((BASE / "recovered/cases.json").read_text())
    by_id = {row["id"]: row for row in cases}
    probabilities = {}
    questions = json.loads((HERE / "questions.json").read_text())
    for path in (ret.PRIVATE / "raw").glob("*.json"):
        row = json.loads(path.read_text())
        value = rounding.recovered_probabilities(row, questions)
        if value is None:
            raise RuntimeError("unusable Jev response")
        probabilities[row["sample_id"]] = value["korean"]
    if len(by_id) != 4000 or by_id.keys() != probabilities.keys():
        raise RuntimeError("source and Jev IDs differ")
    automatic = [sid for sid, value in probabilities.items() if value >= 0.77]
    ask = [sid for sid, value in probabilities.items() if 0.43 <= value < 0.77]
    low = [sid for sid, value in probabilities.items() if value < 0.43]
    if (len(automatic), len(ask), len(low)) != (57, 301, 3642):
        raise RuntimeError("unexpected Jev strata")
    low.sort(key=lambda sid: ret.digest(f"saturn-1000-20261005:{sid}".encode()))
    selected_ids = set(automatic + ask + low[:642])
    selected = [by_id[sid] for sid in sorted(selected_ids)]
    if len(selected) != 1000:
        raise RuntimeError("selection is not 1000 unique cases")
    ret.write_once(QUALITY / "cases.json", selected)
    ret.write_once(
        QUALITY / "selection.json",
        {
            "ts_utc": ret.now(),
            "population": 4000,
            "strata": {
                "automatic": 57,
                "ask": 301,
                "low_population": 3642,
                "low_selected": 642,
            },
            "ids": sorted(selected_ids),
            "cases_sha256": ret.digest((QUALITY / "cases.json").read_bytes()),
            "design_sha256": ret.digest((HERE / "retrospective-1000.md").read_bytes()),
            "selector_sha256": ret.digest(Path(__file__).read_bytes()),
        },
    )
    _, existing, old_audit = audit.load_labels(BASE)
    if old_audit["bad_evidence"]:
        for sid in old_audit["bad_evidence"]:
            existing.pop(sid, None)
    primary = [case for case in selected if case["id"] not in existing]
    ret.write_once(QUALITY / "primary/cases.json", primary)
    ret.write_once(QUALITY / "secondary/cases.json", selected)
    print(
        json.dumps(
            {
                "selected": 1000,
                "reused": 1000 - len(primary),
                "primary_pending": len(primary),
                "secondary_pending": 1000,
            }
        )
    )


def label(group: str) -> None:
    ret.OUT = QUALITY / group
    if not (ret.OUT / "seal.json").exists():
        ret.seal()
    ret.label(None)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("action", choices=("prepare", "label"))
    parser.add_argument("--group", choices=("primary", "secondary", "third"))
    args = parser.parse_args()
    if args.action == "prepare":
        prepare()
    elif args.group is None:
        parser.error("--group is required for label")
    else:
        label(args.group)


if __name__ == "__main__":
    main()
