"""앞 실험의 추출기로 남아 있는 원기록에서 입력 쌍을 다시 고른다."""

from __future__ import annotations

import importlib
import json
import random
import sys
from collections import Counter
from pathlib import Path

PUBLIC = Path(__file__).resolve().parents[1]
PRIVATE = PUBLIC.parents[2] / ".local/experiments/constraint-exception"
sys.path.insert(0, str(PUBLIC.parent / "continuation-newtask/scripts"))
collector = importlib.import_module("01-collect")
audit = importlib.import_module("audit")


# cost: io O(f) reads; vars: f = session files; basis: estimate
def main() -> None:
    cases, census = [], []
    for path in sorted((Path.home() / ".codex/sessions").rglob("*.jsonl")):
        extracted, meta = collector.codex_cases(path, Counter())
        census.append(meta)
        for case in extracted:
            if str(case.get("timestamp", "")) >= "2026-10-04T05:55:43":
                continue
            if any(audit.internal_tag(case[f]) for f in ("input", "previous_input")):
                continue
            if max(len(case["input"]), len(case["previous_input"])) > 6000:
                continue
            cases.append(case)
    rng = random.Random(38204)
    rng.shuffle(cases)
    enriched = [c for c in cases if collector.is_candidate(c)][:150]
    ids = {c["id"] for c in enriched}
    ordinary = [c for c in cases if c["id"] not in ids][:150]
    selected = [c for pair in zip(enriched, ordinary) for c in pair]
    (PRIVATE / "continuation-candidates.json").write_text(
        json.dumps(selected, ensure_ascii=False, indent=2) + "\n"
    )
    (PRIVATE / "continuation-census.json").write_text(
        json.dumps(census, ensure_ascii=False, indent=2) + "\n"
    )
    print(
        json.dumps(
            {"continuation_population": len(cases), "candidates": len(selected)}
        ),
        flush=True,
    )


if __name__ == "__main__":
    main()
