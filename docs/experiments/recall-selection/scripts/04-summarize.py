"""실제 호출과 입력 재사용을 구분해 보고서 수치를 다시 계산한다."""

from __future__ import annotations

from collections import Counter
import json
import os
from pathlib import Path
import re
import statistics
import subprocess

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = ROOT / ".local/experiments/recall-selection"


def main() -> None:
    phases = {}
    for phase in [
        "primary",
        "coverage",
        "focused",
        "conservative",
        "format",
        "explicit",
        "replication",
        "control-replication",
    ]:
        if not (PRIVATE / phase / "calls-plan.json").exists():
            continue
        subprocess.run(
            ["python3", str(PUBLIC / "scripts/01-run.py"), "analyze"],
            env=dict(os.environ, SATURN_SELECTION_PHASE=phase),
            stdout=subprocess.DEVNULL,
            check=True,
        )
        phases[phase] = json.loads((PUBLIC / "results" / f"{phase}.json").read_text())
    final = phases["explicit"]
    repetition = phases.get("replication", {"records": []})
    counts = {}
    for run in sorted(PRIVATE.iterdir()):
        if not (run / "calls-plan.json").exists():
            continue
        results = [
            json.loads(p.read_text()) for p in (run / "raw").glob("*/result.json")
        ]
        reuse = (
            json.loads((run / "reuse.json").read_text())
            if (run / "reuse.json").exists()
            else {}
        )
        counts[run.name] = dict(
            prepared_conditions=len(json.loads((run / "calls-plan.json").read_text()))
            * 2,
            actual_calls=len(results),
            status=dict(Counter(r["status"] for r in results)),
            reused_conditions=len(reuse) * 2,
        )
    combined = []
    paired = []
    for provider in ["claude", "codex"]:
        records = [
            r
            for r in final["records"] + repetition["records"]
            if r["provider"] == provider and r["arm"] == "candidate"
        ]
        combined.append(
            dict(
                provider=provider,
                correct=sum(sum(r["checks"].values()) for r in records),
                questions=sum(len(r["checks"]) for r in records),
                runs=len(records),
                valid_runs=sum(r["valid"] for r in records),
                failures=[
                    dict(
                        case=r["case"],
                        trial=r["trial"],
                        question=key,
                        actual=r["answers"].get(key),
                    )
                    for r in records
                    for key, value in r["checks"].items()
                    if not value
                ],
            )
        )
        candidates = [
            r
            for r in final["records"]
            if r["provider"] == provider and r["arm"] == "candidate"
        ]
        deltas = []
        for candidate in candidates:
            baseline = next(
                r
                for r in final["records"]
                if r["provider"] == provider
                and r["arm"] == "baseline"
                and r["case"] == candidate["case"]
            )
            deltas.append(
                dict(
                    case=candidate["case"],
                    input_tokens=candidate["input_tokens"] - baseline["input_tokens"],
                    correct=sum(candidate["checks"].values())
                    - sum(baseline["checks"].values()),
                )
            )
        paired.append(
            dict(
                provider=provider,
                conditions=deltas,
                token_delta_sum=sum(d["input_tokens"] for d in deltas),
                token_delta_min=min(d["input_tokens"] for d in deltas),
                token_delta_max=max(d["input_tokens"] for d in deltas),
                token_delta_median=statistics.median(d["input_tokens"] for d in deltas),
            )
        )
    log = (
        ROOT / ".local/verification/recall-selection/accepted-workspace-tests.log"
    ).read_text()
    tests = [
        tuple(map(int, item))
        for item in re.findall(
            r"test result: \w+\. (\d+) passed; (\d+) failed; (\d+) ignored", log
        )
    ]
    controls = phases.get("control-replication", {"records": []})
    all_records = final["records"] + repetition["records"] + controls["records"]
    repeated_groups = []
    disagreements = []
    questions = json.loads((PRIVATE / "explicit/questions.json").read_text())
    for provider in ["claude", "codex"]:
        provider_rows = [r for r in all_records if r["provider"] == provider]
        baseline_tokens = sum(
            r["input_tokens"] for r in provider_rows if r["arm"] == "baseline"
        )
        full_tokens = sum(
            r["input_tokens"] for r in provider_rows if r["arm"] == "full"
        )
        for arm in ["full", "baseline", "candidate"]:
            rows = [r for r in provider_rows if r["arm"] == arm]
            tokens = sum(r["input_tokens"] for r in rows)
            known_correct = sum(
                r["checks"][q["id"]]
                for r in rows
                for q in questions[r["case"]]
                if q["expected"] != ["unknown"]
            )
            repeated_groups.append(
                dict(
                    provider=provider,
                    arm=arm,
                    runs=len(rows),
                    valid_runs=sum(r["valid"] for r in rows),
                    correct=sum(sum(r["checks"].values()) for r in rows),
                    questions=sum(len(r["checks"]) for r in rows),
                    known_correct=known_correct,
                    known_questions=sum(
                        q["expected"] != ["unknown"]
                        for r in rows
                        for q in questions[r["case"]]
                    ),
                    input_tokens=tokens,
                    reduction_from_baseline=1 - tokens / baseline_tokens
                    if baseline_tokens
                    else None,
                    reduction_from_full=1 - tokens / full_tokens
                    if full_tokens
                    else None,
                )
            )
        for candidate in [r for r in provider_rows if r["arm"] == "candidate"]:
            for arm in ["full", "baseline"]:
                reference = next(
                    r
                    for r in provider_rows
                    if r["arm"] == arm and r["trial"] == candidate["trial"]
                )
                for key, correct in candidate["checks"].items():
                    if correct != reference["checks"][key]:
                        disagreements.append(
                            dict(
                                provider=provider,
                                trial=candidate["trial"],
                                question=key,
                                reference_arm=arm,
                                reference_correct=reference["checks"][key],
                                candidate_correct=correct,
                            )
                        )
    load_results = {}
    for phase in [
        "load",
        "load-index",
        "load-index-verified",
        "load-postings",
        "load-final",
        "load-release",
    ]:
        raw = PRIVATE / phase / "measurements.json"
        if not raw.exists():
            continue
        records = json.loads(raw.read_text())
        groups = []
        for multiplier in [1, 10, 100]:
            for arm in ["baseline", "candidate"]:
                rows = [
                    r
                    for r in records
                    if r["arm"] == arm and r["multiplier"] == multiplier
                ]
                times = [r["elapsed_s"] for r in rows]
                groups.append(
                    dict(
                        arm=arm,
                        multiplier=multiplier,
                        source_rows=rows[0]["source_rows"],
                        runs=len(rows),
                        successes=sum(r["exit_code"] == 0 for r in rows),
                        median_s=statistics.median(times),
                        min_s=min(times),
                        max_s=max(times),
                    )
                )
        result = dict(groups=groups, records=records)
        (PUBLIC / "results" / f"{phase}.json").write_text(
            json.dumps(result, indent=2) + "\n"
        )
        load_results[phase] = groups
    summary = dict(
        groups=final["groups"],
        repeated_groups=repeated_groups,
        disagreements=disagreements,
        load_results=load_results,
        invalid_load_phase={
            "load-index": "candidate binary matched the pre-index binary; excluded from performance conclusions"
        },
        regressions=final["regressions"],
        combined=combined,
        paired=paired,
        phases=counts,
        actual_calls=sum(c["actual_calls"] for c in counts.values()),
        actual_successful_calls=sum(c["status"].get("ok", 0) for c in counts.values()),
        earlier_groups={
            phase: data["groups"]
            for phase, data in phases.items()
            if phase not in {"explicit", "replication", "control-replication"}
        },
        source_scope=dict(
            conversation_clusters=6,
            conversation_conditions=10,
            conversation_questions=80,
            documents=4,
            document_batch_questions=20,
            repeated_single_questions=4,
            conditions=18,
            questions_per_provider=104,
            known_questions=87,
            unknown_questions=17,
            unique_question_strings=len(
                {q["question"] for items in questions.values() for q in items}
            ),
        ),
        implementation_tests=dict(
            passed=sum(t[0] for t in tests),
            failed=sum(t[1] for t in tests),
            ignored=sum(t[2] for t in tests),
        ),
        equivalence=json.loads((PRIVATE / "final-equivalence.json").read_text()),
        release_equivalence=json.loads(
            (PRIVATE / "release-equivalence.json").read_text()
        ),
        acceptance=dict(
            accepted=False,
            reason="two previously correct Claude answers regressed in repetition",
            state="local candidate implementation; no release adoption",
        ),
        policy=dict(
            packet_limit=4000,
            max_chars=8000,
            pool_count=2,
            plain_chunk_chars=600,
            relative_score=0.25,
            minimum_candidates=2,
            candidates_per_question=2,
            single_question_cap=None,
        ),
    )
    (PUBLIC / "results/summary.json").write_text(
        json.dumps(summary, ensure_ascii=False, indent=2) + "\n"
    )
    print(
        json.dumps(
            dict(
                actual_calls=summary["actual_calls"],
                combined=combined,
                groups=[g for g in final["groups"] if g["subset"] == "all"],
            ),
            ensure_ascii=False,
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
