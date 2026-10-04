"""조건별 집계와 보고서의 모든 수치를 재생성한다."""

from __future__ import annotations

import csv
import hashlib
from collections import Counter

from metrics import compare_pairs, summarize_condition
from support import (
    PRIVATE,
    PUBLIC,
    SOURCE,
    binomial_tail,
    load_original,
    percentile,
    ratio,
    read_json,
    read_rows,
    write_json,
    source_manifest,
)
from trials import load_extension


def describe(values: list) -> dict:
    return {
        "n": len(values),
        "min": min(values) if values else None,
        "median": percentile(values, 0.5),
        "p95": percentile(values, 0.95),
        "max": max(values) if values else None,
    }


def request_metrics(receipts: list[dict], infos: list[dict]) -> dict:
    import json

    sizes = [
        len(json.dumps(r["request"], ensure_ascii=False).encode()) for r in receipts
    ]
    return {
        "calls": len(receipts),
        "statuses": dict(Counter(r["status"] for r in receipts)),
        "http_failures": sum(r["status"] == "http_error" for r in receipts),
        "http_statuses": dict(Counter(str(r.get("http_status")) for r in receipts)),
        "bytes": describe(sizes),
        "before_trim_bytes": describe(
            [r["original_bytes"] for r in infos if "original_bytes" in r]
        ),
        "trimmed_trials": sum(r.get("trimmed", 0) > 0 for r in infos),
        "trimmed_candidates": sum(r.get("trimmed", 0) for r in infos),
        "skipped": dict(Counter(r["status"] for r in infos if r["status"] != "ready")),
        "candidates": describe([len(r["candidate_ids"]) for r in infos]),
    }


def extension_summary() -> dict:
    conversations, labels = load_extension()
    counts = Counter()
    ambiguous = agreement = compared = invalid = 0
    for gold in labels.values():
        for row in gold.values():
            ambiguous += row["ambiguous"]
            if not row["ambiguous"]:
                counts[row["operation"]] += len(row["targets"])
    lane_statuses = {
        lane: Counter() for lane in ("gpt-6-astra", "gpt-5.6-luna", "adjudicated")
    }
    calls = read_rows(PRIVATE / "calls.jsonl")
    for index, _ in enumerate(read_json(PRIVATE / "plan.json")["label_batches"]):
        batches = {}
        for lane in lane_statuses:
            path = PRIVATE / "extension" / f"{lane}-{index}.json"
            batch = read_json(path) if path.exists() else None
            batches[lane] = batch
            if batch is None:
                was_reserved = any(
                    r["trial_id"].startswith(f"extension-{lane}-{index}-")
                    for r in calls
                )
                lane_statuses[lane][
                    "missing_reserved" if was_reserved else "not_started"
                ] += 1
            else:
                lane_statuses[lane][
                    "invalid" if batch.get("invalid_batch") else "ok"
                ] += 1
        final = batches["adjudicated"]
        invalid += bool(final and final.get("invalid_batch"))
        first, second = batches["gpt-6-astra"], batches["gpt-5.6-luna"]
        if (
            not first
            or not second
            or first.get("invalid_batch")
            or second.get("invalid_batch")
        ):
            continue
        for cid in first:
            for a, b in zip(first[cid]["turns"], second[cid]["turns"]):
                compared += 1
                agreement += (
                    a["is_constraint"],
                    a["operation"],
                    sorted(a["targets"]),
                    a["ambiguous"],
                ) == (
                    b["is_constraint"],
                    b["operation"],
                    sorted(b["targets"]),
                    b["ambiguous"],
                )
    planned = [
        m
        for m in read_json(SOURCE / "selection.json")["projects"]
        if 0 < m["user_turns"] < 10
    ]
    planned_turns = sum(m["user_turns"] for m in planned)
    return {
        "planned_projects": len(planned),
        "planned_turns": planned_turns,
        "excluded_turns": planned_turns - sum(len(c["turns"]) for c in conversations),
        "lane_statuses": {k: dict(v) for k, v in lane_statuses.items()},
        "projects": len(conversations),
        "turns": sum(len(c["turns"]) for c in conversations),
        "ambiguous": ambiguous,
        "invalid_batches": invalid,
        "transition_targets": dict(counts),
        "initial_agreement": ratio(agreement, compared),
        "remaining_positive_targets": {
            "replace": max(0, 124 - 2 - counts["replace"]),
            "release": max(0, 124 - counts["release"]),
        },
    }


def power() -> dict:
    for n in range(1, 1000):
        k = next((k for k in range(n + 1) if binomial_tail(k, n, 0.9) <= 0.0125), n + 1)
        achieved = binomial_tail(k, n, 0.97)
        if achieved >= 0.8:
            return dict(
                n=n,
                critical=k,
                power=achieved,
                null=0.9,
                alternative=0.97,
                alpha=0.0125,
                target=0.8,
            )
    raise RuntimeError("power search exhausted")


def main() -> None:
    if source_manifest() != read_json(PRIVATE / "plan.json")["source_manifest"]:
        raise RuntimeError("source changed")
    rows = read_rows(PRIVATE / "processed.jsonl")
    infos = read_json(PRIVATE / "trial-info.json")
    receipts = read_rows(PRIVATE / "jev.jsonl")
    calls = read_rows(PRIVATE / "calls.jsonl")
    _, _, old = load_original()
    summary = {
        "run": read_json(PRIVATE / "run.json"),
        "power": power(),
        "constants": {
            "threshold": 0.8,
            "target_precision": 0.9,
            "sample": 400,
            "extension_sample": 44,
            "original_turns": 931,
            "original_clear_turns": 834,
            "original_transition_targets": 58,
            "original_replace_targets": 2,
            "original_partial_targets": 56,
            "original_release_targets": 0,
            "recent_k": 8,
            "byte_limit": 100000,
            "jev_limit": 4000,
            "codex_limit": 300,
            "repeats": 3,
        },
        "calls": dict(Counter(r["kind"] for r in calls)),
        "codex_call_roles": dict(
            Counter(
                "review" if r["trial_id"].startswith("review-") else "label"
                for r in calls
                if r["kind"] == "codex"
            )
        ),
        "incomplete_calls": len(
            {r["trial_id"] for r in calls if r["kind"] == "jev"}
            - {r["trial_id"] for r in receipts}
        ),
        "models": dict(Counter(str(r.get("model")) for r in receipts)),
        "coverage_full": read_json(PRIVATE / "coverage.json"),
        "extension": extension_summary(),
        "cohorts": {},
        "baseline_full": read_json(
            PUBLIC.parent / "constraint-deep/results/summary.json"
        )["relations"],
    }
    for cohort in ("original", "extension"):
        summary["cohorts"][cohort] = {}
        for condition in ("C0", "C1", "C2", "C3"):
            if cohort == "extension" and condition == "C0":
                continue
            group = [
                r for r in rows if r["cohort"] == cohort and r["condition"] == condition
            ]
            trials = [
                r
                for r in infos
                if r["cohort"] == cohort and r["condition"] == condition
            ]
            stats = summarize_condition(group, trials)
            if cohort != "original" or condition not in ("C1", "C2"):
                stats["hypotheses"] = {op: "탐색" for op in ("replace", "release")}
                stats["recommendation"] = "탐색: 자동 판정 대상 아님"
            if condition == "C0":
                sample = {tuple(x) for x in read_json(PRIVATE / "plan.json")["sample"]}
                responses = [
                    r
                    for r in old.values()
                    if r["meta"]["kind"] == "relation"
                    and (r["meta"]["conversation_id"], r["meta"]["turn_id"]) in sample
                ]
            else:
                responses = [
                    r
                    for r in receipts
                    if r["meta"]["cohort"] == cohort
                    and r["meta"]["condition"] == condition
                ]
            stats["requests"] = request_metrics(responses, trials)
            summary["cohorts"][cohort][condition] = stats
    original = [r for r in rows if r["cohort"] == "original"]
    summary["paired"] = {
        left + "_" + right: compare_pairs(
            [r for r in original if r["condition"] == left],
            [r for r in original if r["condition"] == right],
            sorted({i["conversation_id"] for i in infos if i["cohort"] == "original"}),
        )
        for left, right in [("C0", "C1"), ("C1", "C2"), ("C2", "C3")]
    }
    summary["source_manifest_sha256"] = hashlib.sha256(
        __import__("json")
        .dumps(read_json(PRIVATE / "plan.json")["source_manifest"], sort_keys=True)
        .encode()
    ).hexdigest()
    write_json(PUBLIC / "results/summary.json", summary)
    with (PUBLIC / "results/tables/thresholds.csv").open("w", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(
            [
                "cohort",
                "condition",
                "threshold",
                "operation",
                "tp",
                "predicted",
                "gold",
                "precision",
                "recall",
                "precision_lower",
                "precision_upper",
                "recall_lower",
                "recall_upper",
            ]
        )
        for cohort, conditions in summary["cohorts"].items():
            for condition, stats in conditions.items():
                for step in stats["curve"]:
                    for operation in ("replace", "release"):
                        m = step[operation]
                        writer.writerow(
                            [
                                cohort,
                                condition,
                                step["threshold"],
                                operation,
                                m["tp"],
                                m["predicted"],
                                m["gold"],
                                m["precision"]["value"],
                                m["recall"]["value"],
                                *m["precision"]["ci95"],
                                *m["recall"]["ci95"],
                            ]
                        )
    print({"calls": summary["calls"], "extension": summary["extension"]})
    from report import write_report

    write_report(summary)


if __name__ == "__main__":
    main()
