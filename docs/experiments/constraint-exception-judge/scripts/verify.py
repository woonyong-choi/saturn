"""호출 상한, 봉인, 관측 완전성과 공개 자료의 원문 경계를 검사한다."""

from __future__ import annotations

import hashlib
import importlib
import json
import subprocess
from collections import Counter

import runtime
from inference import binomial_tail, ratio
from protocol import validate
from runtime import LIMITS, MAIN, PRIVATE, PUBLIC, ROOT, read, rows


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def check_budget() -> None:
    original_private, original_limits = runtime.PRIVATE, runtime.LIMITS
    fixture = original_private / "runtime/budget-fixture"
    fixture.mkdir(parents=True, exist_ok=True)
    for name in ("calls.jsonl", "budget.lock"):
        path = fixture / name
        if path.exists():
            path.unlink()
    try:
        runtime.PRIVATE, runtime.LIMITS = fixture, {"jev": 1}
        runtime.reserve("jev", "fixture-first")
        try:
            runtime.reserve("jev", "fixture-second")
        except RuntimeError:
            require(
                len(runtime.rows(fixture / "calls.jsonl")) == 1,
                "budget rejection appended a call",
            )
        else:
            raise ValueError("call beyond budget accepted")
    finally:
        runtime.PRIVATE, runtime.LIMITS = original_private, original_limits


def check_contracts() -> None:
    item = {"rules": ["오류 문구는 영어로 쓴다."], "text": "이번 작업만 예외로 해."}
    good = {
        "request": "exception",
        "target": "c1",
        "kind": "once",
        "scope_text": "이번 작업만",
    }
    require(validate(good, item), "valid example rejected")
    bads = [
        {**good, "target": "c9"},
        {**good, "scope_text": "내일까지"},
        {**good, "extra": 1},
        {**good, "kind": "permanent"},
        {**good, "request": "none"},
    ]
    require(all(not validate(v, item) for v in bads), "invalid contract accepted")
    require(ratio(0, 450)["ci"][1] < 0.01, "wilson rare error bound incorrect")
    require(
        abs(binomial_tail(0, 450, 0.01, upper=False) - 0.99**450) < 1e-12,
        "binomial tail incorrect",
    )
    analyze = importlib.import_module("04-analyze")
    sample = {
        **item,
        "id": "fixture",
        "task": "constraint",
        "source": "synthetic",
        "cluster": "fixture",
        "gold": {
            "request": "exception",
            "target": "c1",
            "kind": "once",
            "scope_text": "이번 작업만",
        },
    }
    row = {
        "run_id": "fixture",
        "trial_id": "fixture",
        "item_id": "fixture",
        "task": "constraint",
        "condition": "J1",
        "repeat": 1,
        "ts_utc": None,
        "status": "ok",
        "prediction": {
            "request": "release",
            "target": "c1",
            "kind": "permanent",
            "scope_text": None,
        },
        "stages": [],
    }
    scored = analyze.normalized(sample, row, {})
    require(
        scored["false_permanent"] and not scored["correct"],
        "exception falsely released was not counted",
    )
    failed = analyze.normalized(sample, {**row, "status": "invalid"}, {})
    require(
        failed["failure"] and not failed["correct"] and not failed["pred_positive"],
        "invalid result did not count as a miss",
    )


# cost: io local receipt reads and 1 git read; basis: estimate
def main() -> None:
    check_contracts()
    check_budget()
    if not (PRIVATE / "items.json").exists():
        print("contract checks passed; no collection yet")
        return
    ledger = rows(PRIVATE / "calls.jsonl")
    counts = Counter(r["kind"] for r in ledger)
    require(all(counts[k] <= n for k, n in LIMITS.items()), "call budget exceeded")
    require(
        len({r["trial_id"] for r in ledger}) == len(ledger),
        "duplicate call reservation",
    )
    run = read(PRIVATE / "run.json")
    sealed = subprocess.check_output(
        [
            "git",
            "show",
            run["design_commit"]
            + ":docs/experiments/constraint-exception-judge/design.md",
        ],
        cwd=ROOT,
    )
    require(sealed == (PUBLIC / "design.md").read_bytes(), "design seal changed")
    for line in (PUBLIC / "data/SHA256SUMS").read_text().splitlines():
        expected, relative = line.split("  ", 1)
        require(
            hashlib.sha256((PRIVATE / relative).read_bytes()).hexdigest() == expected,
            "raw hash mismatch: " + relative,
        )
    summary = read(PUBLIC / "results/summary.json")
    require(dict(counts) == summary["calls"], "call count mismatch")
    observations = rows(PRIVATE / "processed.jsonl")
    require(
        sum(r["call_count"] for r in observations) == counts["jev"] + counts["claude"],
        "reserved evaluation call coverage mismatch",
    )
    item_ids = {i["id"] for i in read(PRIVATE / "items.json")}
    require(
        all(r["item_id"] in item_ids for r in observations), "unknown observation input"
    )
    require(
        all(
            len(rs) == 3
            for rs in (
                [r for r in observations if (r["item_id"], r["condition"]) == key]
                for key in {(r["item_id"], r["condition"]) for r in observations}
            )
        ),
        "repeat coverage mismatch",
    )
    originals = {
        t["text"]
        for p in (MAIN / ".local/experiments/constraint-deep/conversations").glob(
            "*.json"
        )
        for t in read(p)["turns"]
        if len(t["text"]) >= 8
    }
    public = read(PUBLIC / "data/generated.json")
    require(
        all(
            not any(original in row["text"] for original in originals) for row in public
        ),
        "original text leaked into public generated inputs",
    )
    require(all(r["source"] != "real" for r in public), "real input published")
    require(
        not any(
            p.name in ("receipt.json", "items.json", "prompt.txt")
            for p in PUBLIC.rglob("*")
        ),
        "raw private artifact under public directory",
    )
    for p in (PRIVATE / "workflows").glob("*.json"):
        row = read(p)
        if row["condition"] == "L1" and row["status"] == "ok":
            require(row.get("num_turns") == 1, "claude used more than one turn")
            require(
                set(row.get("model_usage", {}))
                <= {"claude-haiku-4-5-20251001", "claude-haiku-4-5"},
                "unexpected claude model",
            )
    print(
        json.dumps(
            {
                "verify": "passed",
                "calls": dict(counts),
                "observations": len(observations),
            }
        )
    )


if __name__ == "__main__":
    main()
