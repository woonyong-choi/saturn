"""보존된 응답에서 대응 정답·전체 토큰과 단계 진입 기준을 다시 계산한다."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import statistics
from typing import Any

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
RUN = ROOT / ".local/experiments/joint-context-delivery/formal-v2"


def load(path: Path) -> Any:
    return json.loads(path.read_text())


def analyze() -> None:
    spec = importlib.util.spec_from_file_location(
        "scoring", PUBLIC.parent / "real-context-replay/scripts/03-analyze.py"
    )
    scoring = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(scoring)
    questions = load(RUN / "questions.json")
    records = []
    for job in load(RUN / "calls-plan.json"):
        for provider in ["claude", "codex"]:
            path = RUN / "raw" / f"{provider}-{job['id']}-{job['arm']}" / "result.json"
            result = (
                load(path)
                if path.exists()
                else dict(status="missing", provider=provider)
            )
            valid = result["status"] == "ok" and result.get("tool_calls", 0) == 0
            if provider in ["claude", "codex"]:
                valid = (
                    valid
                    and result.get("ready") == "Ready"
                    and result.get("same_session") is True
                )
            answers = scoring.parsed(result.get("text", ""))
            checks = {
                q["id"]: valid
                and scoring.normalize(answers.get(q["id"]))
                in [scoring.normalize(e) for e in q["expected"]]
                for q in questions[job["case"]]
            }
            measured = result["status"] == "ok" and bool(result.get("usages"))
            input_tokens = scoring.tokens(result) if measured else None
            output_tokens = (
                sum(int(u.get("output_tokens", 0)) for u in result.get("usages", []))
                if measured
                else None
            )
            records.append(
                dict(
                    provider=provider,
                    case=job["case"],
                    trial=job["id"],
                    repeat=job["repeat"],
                    arm=job["arm"],
                    status=result["status"],
                    valid=valid,
                    answers=answers,
                    checks=checks,
                    input_tokens=input_tokens,
                    output_tokens=output_tokens,
                    total_tokens=input_tokens + output_tokens if measured else None,
                    elapsed_s=result.get("elapsed_s"),
                )
            )
    groups = [
        group(records, provider, arm, subset)
        for provider in ["claude", "codex"]
        for arm in ["baseline", "candidate"]
        for subset in ["all", "development", "heldout"]
    ]
    paired = pairs(records)
    decisions = []
    for provider in ["claude", "codex"]:
        baseline = next(
            g
            for g in groups
            if g["provider"] == provider
            and g["arm"] == "baseline"
            and g["subset"] == "all"
        )
        candidate = next(
            g
            for g in groups
            if g["provider"] == provider
            and g["arm"] == "candidate"
            and g["subset"] == "all"
        )
        heldout = next(
            g
            for g in groups
            if g["provider"] == provider
            and g["arm"] == "candidate"
            and g["subset"] == "heldout"
        )
        provider_pairs = [p for p in paired if p["provider"] == provider]
        complete = (
            baseline["valid_runs"] == baseline["runs"]
            and candidate["valid_runs"] == candidate["runs"]
        )
        saving = (
            1 - candidate["input_tokens"] / baseline["input_tokens"]
            if baseline["input_tokens"] and candidate["input_tokens"] is not None
            else None
        )
        fewer_total = (
            candidate["total_tokens"] < baseline["total_tokens"]
            if baseline["total_tokens"] and candidate["total_tokens"] is not None
            else False
        )
        losses = sum(len(p["losses"]) for p in provider_pairs)
        h2 = complete and saving is not None and saving >= 0.05 and fewer_total
        h3 = complete and losses == 0 and heldout["correct"] == heldout["questions"]
        deltas = [
            p["input_reduction"]
            for p in provider_pairs
            if p["input_reduction"] is not None
        ]
        decisions.append(
            dict(
                provider=provider,
                complete=complete,
                input_reduction=saving,
                input_saved=baseline["input_tokens"] - candidate["input_tokens"]
                if saving is not None
                else None,
                total_saved=baseline["total_tokens"] - candidate["total_tokens"]
                if baseline["total_tokens"] is not None
                and candidate["total_tokens"] is not None
                else None,
                losses=losses,
                gains=sum(len(p["gains"]) for p in provider_pairs),
                h2=h2,
                h3=h3,
                paired_reduction=dict(
                    min=min(deltas), median=statistics.median(deltas), max=max(deltas)
                )
                if deltas
                else None,
            )
        )
    payloads = load(RUN / "payload-checks.json")
    h1 = all(p["reversible"] for p in payloads)
    result = dict(
        planned_calls=len(records),
        actual_calls=sum(r["status"] != "missing" for r in records),
        successful_calls=sum(r["status"] == "ok" for r in records),
        h1=h1,
        prototype_passed=h1 and all(d["h2"] and d["h3"] for d in decisions),
        groups=groups,
        decisions=decisions,
        paired=paired,
        payloads=payloads,
        repeats=[
            group([r for r in records if r["repeat"] == repeat], provider, arm, "all")
            | dict(repeat=repeat)
            for repeat in [1, 2, 3]
            for provider in ["claude", "codex"]
            for arm in ["baseline", "candidate"]
        ],
        records=records,
    )
    result["next_stage_allowed"] = False
    result["engine_integration_verified"] = False
    (PUBLIC / "results/summary.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    )
    print(
        json.dumps(
            {
                k: result[k]
                for k in [
                    "actual_calls",
                    "successful_calls",
                    "h1",
                    "prototype_passed",
                    "decisions",
                ]
            },
            ensure_ascii=False,
            indent=2,
        )
    )


def group(records: list[dict], provider: str, arm: str, subset: str) -> dict:
    selected = [
        r
        for r in records
        if r["provider"] == provider
        and r["arm"] == arm
        and (
            subset == "all" or r["case"].startswith("heldout-") == (subset == "heldout")
        )
    ]
    result = dict(
        provider=provider,
        arm=arm,
        subset=subset,
        runs=len(selected),
        valid_runs=sum(r["valid"] for r in selected),
        correct=sum(sum(r["checks"].values()) for r in selected)
        if any(r["status"] != "missing" for r in selected)
        else None,
        questions=sum(len(r["checks"]) for r in selected),
        observed_questions=sum(
            len(r["checks"]) for r in selected if r["status"] != "missing"
        ),
    )
    for field in ["input_tokens", "output_tokens", "total_tokens"]:
        result[field] = (
            sum(r[field] for r in selected)
            if all(r[field] is not None for r in selected)
            else None
        )
    return result


def pairs(records: list[dict]) -> list[dict]:
    paired = []
    for candidate in [r for r in records if r["arm"] == "candidate"]:
        baseline = next(
            r
            for r in records
            if r["provider"] == candidate["provider"]
            and r["trial"] == candidate["trial"]
            and r["arm"] == "baseline"
        )
        reduction = (
            1 - candidate["input_tokens"] / baseline["input_tokens"]
            if baseline["input_tokens"] and candidate["input_tokens"] is not None
            else None
        )
        paired.append(
            dict(
                provider=candidate["provider"],
                trial=candidate["trial"],
                case=candidate["case"],
                repeat=candidate["repeat"],
                input_reduction=reduction,
                losses=[
                    key
                    for key, correct in baseline["checks"].items()
                    if correct and not candidate["checks"][key]
                ],
                gains=[
                    key
                    for key, correct in candidate["checks"].items()
                    if correct and not baseline["checks"][key]
                ],
            )
        )
    return paired


if __name__ == "__main__":
    analyze()
