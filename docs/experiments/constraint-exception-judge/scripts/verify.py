"""호출 상한, 봉인, 관측 완전성과 공개 자료의 원문 경계를 검사한다."""

from __future__ import annotations

import hashlib
import importlib
import json
import subprocess
from collections import Counter

import exploration
import publication
import runtime
from inference import binomial_tail, ratio
from protocol import MODEL, scope_candidates, validate
from publication import check_raw_seal, redact_fragments, source_fragments
from runtime import LIMITS, PRIVATE, PUBLIC, ROOT, read, rows


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


def check_interrupted() -> None:
    collector = importlib.import_module("02-collect")
    original = collector.PRIVATE
    fixture = PRIVATE / "runtime/interrupted-fixture"
    fixture.mkdir(parents=True, exist_ok=True)
    for path in (fixture / "workflows").glob("*.json"):
        path.unlink()
    runtime.write(fixture / "run.json", {"run_id": "fixture"})
    tid = "fixture-L1-r1"
    (fixture / "calls.jsonl").write_text(
        json.dumps({"kind": "claude", "trial_id": tid, "ts_utc": "fixture"}) + "\n"
    )
    try:
        collector.PRIVATE = fixture
        tasks = [({"id": "fixture", "task": "constraint"}, "L1", r) for r in (1, 2)]
        collector.preserve_interrupted(tasks)
        path = fixture / "workflows" / (tid + ".json")
        value = read(path)
        require(value["status"] == "incomplete", "interrupted call not preserved")
        require(value["stages"] == [tid], "interrupted reservation missing")
        require(
            not (fixture / "workflows/fixture-L1-r2.json").exists(),
            "uncalled trial marked interrupted",
        )
        before = path.read_bytes()
        collector.preserve_interrupted(tasks)
        require(before == path.read_bytes(), "recovery overwrote completed workflow")
    finally:
        collector.PRIVATE = original


def check_decimal_boundary() -> None:
    original = exploration.PRIVATE
    fixture = PRIVATE / "runtime/decimal-fixture"
    item = {"rules": ["오류 문구는 영어로 쓴다."], "text": "이 규칙은 없애."}
    row = {"condition": "J1", "status": "invalid", "trial_id": "fixture"}
    scope = dict.fromkeys(scope_candidates(item["text"]), 0)
    scope["none"] = 1
    reply = {
        "model": MODEL,
        "answers": {
            "release_target": {"probabilities": {"c1": 0.99, "none": 0}},
            "kind": {"probabilities": {"permanent": 1, "once": 0, "scoped": 0}},
            "scope": {"probabilities": scope},
        },
    }
    try:
        exploration.PRIVATE = fixture
        for total, status in ((0.99, "ok"), (0.98, "invalid")):
            reply["answers"]["release_target"]["probabilities"]["c1"] = total
            runtime.write(
                fixture / "jev/fixture.json",
                {"status": "invalid", "raw_response": json.dumps(reply)},
            )
            result = exploration.decimal_sensitivity(row, item)
            require(result["status"] == status, "decimal tolerance boundary incorrect")
    finally:
        exploration.PRIVATE = original


def check_publication() -> None:
    fixture = PRIVATE / "runtime/publication-fixture"
    fixture.mkdir(parents=True, exist_ok=True)
    original = (
        publication.MAIN,
        publication.PRIVATE,
        publication.PUBLIC,
        publication.RAW_FILES,
    )
    try:
        publication.MAIN = fixture / "missing-source"
        try:
            publication.source_fragments()
        except ValueError as error:
            require("sources are missing" in str(error), "unexpected source failure")
        else:
            raise ValueError("missing private source accepted")
        text = "이것은 비공개원문조각이 포함된 생성문이다."
        require(
            "비공개원문조각이"
            not in publication.redact_fragments(text, {"비공개원문조각이"}),
            "partial private quote not redacted",
        )
        publication.PRIVATE = fixture / "private"
        publication.PUBLIC = fixture / "public"
        publication.RAW_FILES = {"sample.json"}
        (publication.PUBLIC / "data").mkdir(parents=True, exist_ok=True)
        seal = publication.PUBLIC / "data/RAW_SHA256SUMS"
        if seal.exists():
            seal.unlink()
        runtime.write(publication.PRIVATE / "sample.json", {"original": True})
        publication.check_raw_seal(create=True)
        original_seal = seal.read_bytes()
        runtime.write(publication.PRIVATE / "sample.json", {"original": False})
        try:
            publication.check_raw_seal(create=True)
        except ValueError:
            require(seal.read_bytes() == original_seal, "raw seal overwritten")
        else:
            raise ValueError("changed raw data accepted")
    finally:
        (
            publication.MAIN,
            publication.PRIVATE,
            publication.PUBLIC,
            publication.RAW_FILES,
        ) = original


# cost: io local receipt reads and 1 git read; basis: estimate
def main() -> None:
    check_contracts()
    check_budget()
    check_interrupted()
    check_decimal_boundary()
    check_publication()
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
    protocol = subprocess.check_output(
        [
            "git",
            "show",
            run["design_commit"]
            + ":docs/experiments/constraint-exception-judge/scripts/protocol.py",
        ],
        cwd=ROOT,
    )
    require(
        protocol == (PUBLIC / "scripts/protocol.py").read_bytes(),
        "question and schema seal changed",
    )
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
    check_raw_seal()
    fragments = source_fragments()
    public = read(PUBLIC / "data/generated.json")
    require(
        all(redact_fragments(row["text"], fragments) == row["text"] for row in public),
        "source fragment leaked into public generated inputs",
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
        if row["condition"] == "L1" and row["status"] != "incomplete":
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
