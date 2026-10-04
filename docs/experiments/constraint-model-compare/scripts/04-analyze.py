"""사후 정답·사전 질의 관측에서 공개 가능한 집계만 만든다."""

from __future__ import annotations

import csv
import hashlib
import statistics
from collections import Counter, defaultdict

from metrics import metrics, paired_difference, rate, wilson
from runtime import PRIVATE, PUBLIC, read, rows, write

NAMES = ("sonnet", "opus", "haiku", "astra", "sol", "terra", "luna", "jev")
THRESHOLDS = [i / 100 for i in range(50, 100, 5)]


def measurable(data: list[dict]) -> list[dict]:
    return [r for r in data if r["gold"] != "uncertain"]


def performance(data: list[dict]) -> dict:
    result = metrics(measurable(data))
    claimed = [r for r in measurable(data) if r["claimed_prediction"] is not None]
    result["boolean_only_exploratory"] = rate(
        sum(r["claimed_prediction"] == (r["gold"] == "constraint") for r in claimed),
        len(claimed),
    )
    result["format_reasons"] = dict(
        Counter(r["format_reason"] for r in data if not r["valid"])
    )
    result["format_failure"] = rate(
        sum(not r["valid"] and r["status"] == "ok" for r in data), len(data)
    )
    result["all_failure"] = rate(sum(not r["valid"] for r in data), len(data))
    result["wrapped"] = sum(r["wrapped"] for r in data)
    result["multiline"] = sum(r["multiline"] for r in data)
    result["statuses"] = dict(Counter(r["status"] for r in data))
    latencies = [r["latency_s"] for r in data if r["latency_s"] is not None]
    costs = [r["cost_usd"] for r in data if r["cost_usd"] is not None]
    result.update(
        latency_median_s=statistics.median(latencies) if latencies else None,
        cost_mean_usd=statistics.mean(costs) if costs else None,
        cost_known_usd=sum(costs),
        cost_unknown=len(data) - len(costs),
    )
    return result


def policy(data: list[dict], auto: float, ask: float) -> dict:
    eligible = measurable(data)
    auto_rows = [r for r in eligible if r["valid"] and r["probability"] >= auto]
    asked = [r for r in data if r["valid"] and ask <= r["probability"] < auto]
    asked_known = measurable(asked)
    positives = sum(r["gold"] == "constraint" for r in eligible)
    reached = sum(
        r["gold"] == "constraint" and r["valid"] and r["probability"] >= ask
        for r in eligible
    )
    return dict(
        auto=auto,
        ask=ask,
        auto_precision=rate(
            sum(r["gold"] == "constraint" for r in auto_rows), len(auto_rows)
        ),
        ask_rate=rate(len(asked), len(data)),
        asked_precision=rate(
            sum(r["gold"] == "constraint" for r in asked_known), len(asked_known)
        ),
        combined_recall=rate(reached, positives),
    )


def choose_threshold(curve: list[dict], jev: list[dict]) -> dict:
    candidates = [
        r for r in curve if r["precision"]["n"] >= 10 and r["precision"]["value"] >= 0.9
    ]
    auto = candidates[0]["threshold"] if candidates else None
    options = [
        policy(jev, auto if auto is not None else 0.8, ask)
        for ask in THRESHOLDS
        if ask < (auto if auto is not None else 0.8)
    ]
    feasible = [r for r in options if r["ask_rate"]["value"] <= 0.25]
    meets = [
        r
        for r in feasible
        if r["combined_recall"]["value"] is not None
        and r["combined_recall"]["value"] >= 0.9
    ]
    chosen = (
        max(meets, key=lambda r: r["ask"])
        if meets
        else max(feasible, key=lambda r: (r["combined_recall"]["value"] or 0, r["ask"]))
        if feasible
        else None
    )
    robust = False
    if candidates:
        point = candidates[0]
        cluster = point["cluster_ci95"]["precision"]
        robust = (
            point["precision"]["ci95"][0] >= 0.9
            and cluster is not None
            and cluster[0] >= 0.9
        )
    return dict(
        auto_candidate=auto,
        precision_lower_bounds_pass=robust,
        ask_targets_pass=bool(meets),
        selected_policy=chosen,
        options=options,
    )


def gold_summary() -> dict:
    gold = read(PRIVATE / "gold.json")
    pairs = [r["judgments"][:2] for r in gold]
    valid_pairs = [pair for pair in pairs if all(r["valid"] for r in pair)]
    matches = sum(
        pair[0]["value"]["label"] == pair[1]["value"]["label"] for pair in valid_pairs
    )
    judgments = [r for item in gold for r in item["judgments"]]
    return dict(
        labels=dict(Counter(r["label"] for r in gold)),
        self_agreement=rate(matches, len(valid_pairs)),
        all_pair_agreement=rate(matches, len(pairs)),
        invalid_judgments=sum(not r["valid"] for r in judgments),
        third_votes=sum(len(r["judgments"]) == 3 for r in gold),
        wrapped=sum(r["wrapped"] for r in judgments),
        multiline=sum(r["multiline"] for r in judgments),
    )


def repeats(data: list[dict]) -> dict:
    by_sample = defaultdict(list)
    for row in data:
        by_sample[row["sample_id"]].append(row)
    result = {}
    complete = [
        group
        for group in by_sample.values()
        if len(group) == 3 and all(r["valid"] for r in group)
    ]
    for threshold in (0.5, 0.7, 0.8):
        matching = sum(
            len({r["probability"] >= threshold for r in group}) == 1
            for group in complete
        )
        result[str(threshold)] = rate(matching, len(by_sample))
    exact = sum(len({r["probability"] for r in group}) == 1 for group in complete)
    deviations = [
        statistics.pstdev(r["probability"] for r in group) for group in complete
    ]
    return dict(
        classification=result,
        exact_probability=rate(exact, len(by_sample)),
        complete=len(complete),
        probability_sd_mean=statistics.mean(deviations) if deviations else None,
    )


def hypothesis(metric: dict, target: float) -> dict:
    ci = wilson(metric["k"], metric["n"], 2.241402727604947)
    decision = (
        "보류"
        if ci is None
        else "채택"
        if ci[0] >= target
        else "기각"
        if ci[1] < target
        else "보류"
    )
    return dict(target=target, metric=metric, simultaneous_ci975=ci, decision=decision)


def export_tables(summary: dict) -> None:
    tables = PUBLIC / "results/tables"
    tables.mkdir(parents=True, exist_ok=True)
    with (tables / "models.csv").open("w", newline="") as stream:
        writer = csv.writer(stream, lineterminator="\n")
        writer.writerow(
            [
                "model",
                "n",
                "accuracy",
                "precision",
                "recall",
                "f1",
                "format_failure",
                "latency_median_s",
                "cost_mean_usd",
            ]
        )
        for name, row in summary["models"].items():
            writer.writerow(
                [
                    name,
                    row["counts"]["n"],
                    *[
                        row[key]["value"]
                        for key in (
                            "accuracy",
                            "precision",
                            "recall",
                            "f1",
                            "format_failure",
                        )
                    ],
                    row["latency_median_s"],
                    row["cost_mean_usd"],
                ]
            )
    with (tables / "thresholds.csv").open("w", newline="") as stream:
        writer = csv.writer(stream, lineterminator="\n")
        writer.writerow(
            [
                "threshold",
                "predicted_positive",
                "precision",
                "precision_low",
                "precision_high",
                "recall",
                "f1",
            ]
        )
        for row in summary["jev_curve"]:
            writer.writerow(
                [
                    row["threshold"],
                    row["precision"]["n"],
                    row["precision"]["value"],
                    *(row["precision"]["ci95"] or [None, None]),
                    row["recall"]["value"],
                    row["f1"]["value"],
                ]
            )


def main() -> None:
    observations = rows(PRIVATE / "processed.jsonl")
    first = {
        name: [r for r in observations if r["condition"] == name and r["repeat"] == 1]
        for name in NAMES
    }
    models = {name: performance(data) for name, data in first.items()}
    strata = {
        kind: {
            name: performance([r for r in data if r["input_kind"] == kind])
            for name, data in first.items()
        }
        for kind in ("human_likely", "ai_instruction_likely")
    }
    curve = [
        dict(threshold=t, **metrics(measurable(first["jev"]), t)) for t in THRESHOLDS
    ]
    current = policy(first["jev"], 0.8, 0.7)
    ledger = rows(PRIVATE / "calls.jsonl")
    summary = dict(
        design_commit="2438948",
        sampling=read(PRIVATE / "sampling.json"),
        deviations=read(PRIVATE / "deviations.json"),
        source_audit=read(PRIVATE / "source-audit.json"),
        gold=gold_summary(),
        models=models,
        input_kinds=strata,
        jev_curve=curve,
        current_policy=current,
        selected=choose_threshold(curve, first["jev"]),
        jev_repeats=repeats([r for r in observations if r["condition"] == "jev"]),
        paired_vs_jev={
            name: paired_difference(measurable(first["jev"]), measurable(data))
            for name, data in first.items()
            if name != "jev"
        },
        calls=dict(Counter(r["kind"] for r in ledger)),
        call_phases=dict(Counter(r["trial_id"].split("-")[0] for r in ledger)),
        hypotheses={
            "H1": hypothesis(current["auto_precision"], 0.9),
            "H2": hypothesis(current["combined_recall"], 0.8),
        },
        actual_models={
            name: dict(Counter(m for r in data for m in r["actual_models"] if m))
            for name, data in first.items()
        },
        costs=dict(
            known_query_usd=sum(
                r["cost_usd"] for r in observations if r["cost_usd"] is not None
            ),
            unknown_query_calls=sum(r["cost_usd"] is None for r in observations),
        ),
        hashes={
            name: hashlib.sha256((PRIVATE / name).read_bytes()).hexdigest()
            for name in ("samples.json", "gold.json", "calls.jsonl", "processed.jsonl")
        },
    )
    write(PUBLIC / "results/summary.json", summary)
    export_tables(summary)
    print("analysis complete")


if __name__ == "__main__":
    main()
