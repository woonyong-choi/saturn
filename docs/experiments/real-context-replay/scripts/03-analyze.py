"""원응답에서 조건별 대응 정답과 실제 사용량을 다시 계산한다."""

from __future__ import annotations

import hashlib
import json
import os
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

PUBLIC = Path(__file__).resolve().parents[1]
ROOT = PUBLIC.parents[2]
PRIVATE = Path(
    os.environ.get(
        "SATURN_REPLAY_HOME", ROOT / ".local/experiments/real-context-replay/corrected"
    )
)


def parsed(text: str) -> dict[str, Any]:
    start, end = text.find("{"), text.rfind("}")
    try:
        value = json.loads(text[start : end + 1])
    except (ValueError, TypeError):
        return {}
    return value if isinstance(value, dict) else {}


def tokens(result: dict[str, Any]) -> int:
    if result["provider"] == "claude":
        return sum(
            sum(
                int(usage.get(key, 0))
                for key in [
                    "input_tokens",
                    "cache_creation_input_tokens",
                    "cache_read_input_tokens",
                ]
            )
            for usage in result.get("usages", [])
        )
    return sum(int(usage.get("input_tokens", 0)) for usage in result.get("usages", []))


def normalize(value: Any) -> str:
    return str(value).strip().strip('`"').casefold()


def score(
    case: dict[str, Any], arm: str, provider: str, questions: list[dict[str, Any]]
) -> dict[str, Any]:
    path = PRIVATE / "raw" / f"{provider}-{case['id']}-{arm}" / "result.json"
    result = (
        json.loads(path.read_text())
        if path.exists()
        else {"status": "not_run", "provider": provider}
    )
    value = parsed(result.get("text", ""))
    valid = result["status"] == "ok" and result.get("tool_calls", 0) == 0
    if provider == "claude":
        valid = (
            valid
            and result.get("ready") == "Ready"
            and result.get("same_session") is True
        )
    answer_checks = {
        q["id"]: normalize(value.get(q["id"])) in [normalize(e) for e in q["expected"]]
        for q in questions
    }
    checks = {
        q["id"]: valid
        and normalize(value.get(q["id"])) in [normalize(e) for e in q["expected"]]
        for q in questions
    }
    failure_kinds = {}
    for question in questions:
        key = question["id"]
        if checks[key]:
            continue
        answer = normalize(value.get(key))
        if not valid:
            failure_kinds[key] = "invalid_session"
        elif answer == "unknown" and question["expected"] != ["unknown"]:
            failure_kinds[key] = "abstention"
        elif key == "prompt" and answer == "하나":
            failure_kinds[key] = "numeric_format_only"
        else:
            failure_kinds[key] = "wrong_or_missing_answer"
    return {
        "failure_kinds": failure_kinds,
        "answer_correct": sum(answer_checks.values()),
        "answer_checks": answer_checks,
        "case": case["id"],
        "cluster": case["cluster"],
        "provider": provider,
        "arm": arm,
        "status": result["status"],
        "valid": valid,
        "correct": sum(checks.values()),
        "questions": len(questions),
        "checks": checks,
        "answers": value,
        "input_tokens": tokens(result),
        "elapsed_s": result.get("elapsed_s"),
        "raw_sha256": hashlib.sha256(path.read_bytes()).hexdigest()
        if path.exists()
        else None,
    }


def main() -> None:
    questions = json.loads((PUBLIC / "questions.json").read_text())
    cases = json.loads((PRIVATE / "cases.json").read_text())
    trials = [
        score(case, arm, provider, questions[case["cluster"]])
        for provider in ["claude", "codex"]
        for case in cases
        for arm in ["full", "1000", "4000", "16000"]
    ]
    groups: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for trial in trials:
        groups[f"{trial['provider']}/{trial['arm']}"].append(trial)
    totals = {
        key: {
            "correct": sum(t["correct"] for t in group),
            "answer_correct": sum(t["answer_correct"] for t in group),
            "questions": sum(t["questions"] for t in group),
            "valid_sessions": sum(t["valid"] for t in group),
            "sessions": len(group),
            "input_tokens": sum(t["input_tokens"] for t in group),
            "not_run": sum(t["status"] == "not_run" for t in group),
        }
        for key, group in groups.items()
    }
    for key, total in totals.items():
        provider, arm = key.split("/")
        baseline = totals[f"{provider}/full"]
        total["correct_delta_vs_full"] = total["correct"] - baseline["correct"]
        total["input_tokens_delta_vs_full"] = (
            total["input_tokens"] - baseline["input_tokens"]
        )
        total["input_reduction_percent"] = (
            round(100 * (1 - total["input_tokens"] / baseline["input_tokens"]), 2)
            if baseline["input_tokens"]
            else None
        )
        pairs = [
            (
                trial,
                next(
                    t for t in groups[f"{provider}/full"] if t["case"] == trial["case"]
                ),
            )
            for trial in groups[key]
        ]
        total["lost_correct_questions"] = sum(
            sum(old["checks"][q] and not new["checks"][q] for q in old["checks"])
            for new, old in pairs
        )
        total["gained_correct_questions"] = sum(
            sum(new["checks"][q] and not old["checks"][q] for q in old["checks"])
            for new, old in pairs
        )
    for key, group in groups.items():
        totals[key]["failure_kinds"] = dict(
            Counter(kind for trial in group for kind in trial["failure_kinds"].values())
        )
    sizes = []
    for case in cases:
        for budget in [1000, 4000, 16000]:
            packet = json.loads(
                (PRIVATE / "packets" / f"{case['id']}-{budget}.json").read_text()
            )
            sizes.append(
                {
                    "case": case["id"],
                    "budget": budget,
                    "full_chars": len(case["full"]),
                    "packet_chars": len(packet.get("text", "")),
                    "status": packet["status"],
                    "estimated_tokens": packet.get("estimated_tokens"),
                    "over_limit": packet.get("over_limit"),
                }
            )
    summary = {
        "design_commit": "184ba2e2",
        "cases": len(cases),
        "source_clusters": len({c["cluster"] for c in cases}),
        "natural_conversation_clusters": 2,
        "functional_record_clusters": 3,
        "planned_sessions": len(trials),
        "collected_sessions": sum(t["status"] != "not_run" for t in trials),
        "failed_sessions": sum(t["status"] == "error" for t in trials),
        "invalid_sessions": sum(t["status"] == "ok" and not t["valid"] for t in trials),
        "totals": totals,
        "sizes": sizes,
        "trials": trials,
        "inference": "purposive dependent sample; no population confidence interval or general reliability claim",
    }
    evidence = ROOT / ".local/verification/cleanup-context/summary.json"
    if evidence.exists():
        summary["verification"] = json.loads(evidence.read_text())
    initial = ROOT / ".local/experiments/real-context-replay"
    initial_count = len(list((initial / "raw").glob("*/result.json")))
    summary["archived_sessions"] = {
        "initial": initial_count,
        "corrected": summary["collected_sessions"],
        "total": initial_count + summary["collected_sessions"],
    }
    summary["packet_estimator_chars_per_token"] = 4
    (PUBLIC / "results/summary.json").write_text(
        json.dumps(summary, ensure_ascii=False, indent=2) + "\n"
    )
    print(
        json.dumps(
            {
                "collected": summary["collected_sessions"],
                "planned": len(trials),
                "totals": totals,
            },
            ensure_ascii=False,
        )
    )


if __name__ == "__main__":
    main()
