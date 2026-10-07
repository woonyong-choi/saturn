"""복원 수정과 동시 전달 효과를 서로 다른 대응 비교로 판정한다."""

from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import statistics

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
RUN = ROOT / ".local/experiments/recall-structure/formal"
ARMS = ["baseline", "repaired", "joint"]


def load(path):
    return json.loads(path.read_text())


def module(path):
    spec = importlib.util.spec_from_file_location(path.stem, path)
    value = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(value)
    return value


def group(records, provider, arm, subset):
    selected = [
        r
        for r in records
        if r["provider"] == provider
        and r["arm"] == arm
        and (subset == "all" or r["case"].startswith(subset + "-"))
    ]
    complete = all(r["status"] == "ok" and r["valid"] for r in selected)
    return dict(
        provider=provider,
        arm=arm,
        subset=subset,
        runs=len(selected),
        complete=complete,
        observed=sum(r["status"] != "missing" for r in selected),
        correct=sum(sum(r["checks"].values()) for r in selected),
        questions=sum(len(r["checks"]) for r in selected),
        input_tokens=sum(r["input_tokens"] for r in selected) if complete else None,
        output_tokens=sum(r["output_tokens"] for r in selected) if complete else None,
        total_tokens=sum(r["total_tokens"] for r in selected) if complete else None,
    )


def comparison(records, provider, before, after):
    pairs = []
    for b in [r for r in records if r["provider"] == provider and r["arm"] == after]:
        a = next(
            r
            for r in records
            if r["provider"] == provider
            and r["trial_id"] == b["trial_id"]
            and r["arm"] == before
        )
        observed = a["valid"] and b["valid"]
        pairs.append(
            dict(
                case=b["case"],
                repeat=b["repeat"],
                trial_id=b["trial_id"],
                observed=observed,
                losses=[
                    key
                    for key in a["checks"]
                    if a["checks"][key] and not b["checks"][key]
                ]
                if observed
                else None,
                gains=[
                    key
                    for key in a["checks"]
                    if b["checks"][key] and not a["checks"][key]
                ]
                if observed
                else None,
                input_reduction=1 - b["input_tokens"] / a["input_tokens"]
                if observed
                else None,
            )
        )
    a, b = (
        group(records, provider, before, "all"),
        group(records, provider, after, "all"),
    )
    complete = a["complete"] and b["complete"]
    losses = sum(len(p["losses"]) for p in pairs) if complete else None
    gains = sum(len(p["gains"]) for p in pairs) if complete else None
    reduction = 1 - b["input_tokens"] / a["input_tokens"] if complete else None
    probes = [
        group(records, provider, after, kind) for kind in ["regression", "heldout"]
    ]
    quality = (
        complete and losses == 0 and all(g["correct"] == g["questions"] for g in probes)
    )
    deltas = [p["input_reduction"] for p in pairs if p["observed"]]
    return dict(
        provider=provider,
        before=before,
        after=after,
        complete=complete,
        losses=losses,
        gains=gains,
        quality=quality,
        input_reduction=reduction,
        input_saved=a["input_tokens"] - b["input_tokens"] if complete else None,
        total_saved=a["total_tokens"] - b["total_tokens"] if complete else None,
        cost=complete and reduction >= 0.05 and b["total_tokens"] < a["total_tokens"],
        paired_reduction=dict(
            min=min(deltas), median=statistics.median(deltas), max=max(deltas)
        )
        if deltas
        else None,
        pairs=pairs,
    )


def observation(job, provider, questions, scoring):
    run_id = f"{provider}-{job['id']}-{job['arm']}"
    folder = RUN / "raw" / run_id
    result = (
        load(folder / "result.json")
        if (folder / "result.json").exists()
        else dict(status="missing", provider=provider)
    )
    measured = result["status"] == "ok" and len(result.get("usages", [])) == 2
    valid = (
        measured
        and result.get("ready") == "Ready"
        and result.get("same_session") is True
        and result.get("tool_calls", 0) == 0
    )
    answers = scoring.parsed(result.get("text", ""))
    checks = {
        q["id"]: valid
        and scoring.normalize(answers.get(q["id"]))
        in [scoring.normalize(e) for e in q["expected"]]
        for q in questions[job["case"]]
    }
    inputs = scoring.tokens(result) if measured else None
    outputs = (
        sum(u.get("output_tokens", 0) for u in result["usages"]) if measured else None
    )
    return dict(
        run_id=run_id,
        trial_id=job["id"],
        condition=job["arm"],
        ts_utc=load(folder / "request.json")["started_at"]
        if (folder / "request.json").exists()
        else None,
        provider=provider,
        case=job["case"],
        repeat=job["repeat"],
        arm=job["arm"],
        status=result["status"],
        valid=valid,
        answers=answers,
        checks=checks,
        input_tokens=inputs,
        output_tokens=outputs,
        total_tokens=inputs + outputs if measured else None,
    )


def analyze():
    scoring = module(PUBLIC.parent / "real-context-replay/scripts/03-analyze.py")
    records = []
    questions = load(RUN / "questions.json")
    for job in load(RUN / "calls-plan.json"):
        for provider in ["claude", "codex"]:
            records.append(observation(job, provider, questions, scoring))
    comparisons = [
        comparison(records, provider, before, after)
        for provider in ["claude", "codex"]
        for before, after in [
            ("baseline", "repaired"),
            ("repaired", "joint"),
            ("baseline", "joint"),
        ]
    ]
    private = RUN.parent / "processed"
    private.mkdir(exist_ok=True)
    (private / "records.jsonl").write_text(
        "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in records)
    )
    result = dict(
        sample=dict(
            conditions=len(questions),
            questions=sum(len(q) for q in questions.values()),
            repeats=3,
        ),
        planned_calls=len(records),
        actual_calls=sum(r["status"] != "missing" for r in records),
        successful_calls=sum(r["status"] == "ok" for r in records),
        h1=all(p["reversible"] for p in load(RUN / "payload-checks.json")),
        h2=all(c["quality"] for c in comparisons if c["after"] == "repaired"),
        h3=all(c["quality"] for c in comparisons if c["before"] == "repaired"),
        h4=all(c["cost"] for c in comparisons if c["after"] == "joint"),
        next_stage_allowed=False,
        engine_integration_verified=False,
        groups=[
            group(records, provider, arm, subset)
            for provider in ["claude", "codex"]
            for arm in ARMS
            for subset in ["all", "regression", "heldout"]
        ],
        comparisons=comparisons,
        repeats=[
            group([r for r in records if r["repeat"] == repeat], provider, arm, "all")
            | dict(repeat=repeat)
            for repeat in [1, 2, 3]
            for provider in ["claude", "codex"]
            for arm in ARMS
        ],
        records=[
            {key: value for key, value in r.items() if key != "answers"}
            for r in records
        ],
    )
    result["prototype_passed"] = all(result[key] for key in ["h1", "h2", "h3", "h4"])
    (PUBLIC / "results/summary.json").write_text(
        json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    )
    print(
        json.dumps(
            {
                key: result[key]
                for key in [
                    "actual_calls",
                    "successful_calls",
                    "h1",
                    "h2",
                    "h3",
                    "h4",
                    "prototype_passed",
                ]
            }
        )
    )


if __name__ == "__main__":
    analyze()
