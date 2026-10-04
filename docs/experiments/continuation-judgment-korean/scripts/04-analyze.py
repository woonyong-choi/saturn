"""저장한 라벨·응답만 읽어 재현 가능한 공개 집계를 만든다."""

from __future__ import annotations

import csv
import hashlib
import json
import re
from collections import Counter, defaultdict
from statistics import pstdev

from metrics import (
    THRESHOLDS,
    bootstrap,
    bootstrap_delta,
    cluster_counts,
    confusion,
    mcnemar,
    measure,
    quantile,
    ratio,
)
from storage import MODELS, PRIVATE, PUBLIC, SEED, read, rows, write


def agreed_labels(sample: list[dict]) -> tuple[list[dict], dict, dict]:
    lanes = [
        {r["id"]: r for r in read(PRIVATE / f"labels-{m}.json")}
        for m in (*MODELS, "adjudicated")
    ]
    result, agreements = [], 0
    cross = Counter()
    invalid_pairs = 0
    invalid_reasons = {"missing", "format failure or incomplete call"}
    for case in sample:
        a, b = [
            lane.get(case["id"], {"label": "uncertain", "reason": "missing"})
            for lane in lanes[:2]
        ]
        valid_pair = all(r["reason"] not in invalid_reasons for r in (a, b))
        if valid_pair:
            cross[(a["label"], b["label"])] += 1
        else:
            invalid_pairs += 1
        same = valid_pair and a["label"] == b["label"]
        agreements += same
        final = (
            a
            if same and a["label"] != "uncertain"
            else lanes[2].get(
                case["id"], {"label": "uncertain", "reason": "unresolved"}
            )
        )
        result.append(
            {
                **case,
                "label": final["label"],
                "reason": final["reason"],
                "initial_agreement": same,
                "initial_labels": [a, b],
            }
        )
    n = len(sample) - invalid_pairs
    expected = (
        sum(
            sum(v for (a, b), v in cross.items() if a == label)
            * sum(v for (a, b), v in cross.items() if b == label)
            for label in ("continue", "new", "uncertain")
        )
        / n**2
    )
    stats = {
        "agreement": ratio(agreements, n),
        "agreement_all_selected": ratio(agreements, len(sample)),
        "valid_initial_pair_coverage": ratio(n, len(sample)),
        "kappa": (agreements / n - expected) / (1 - expected) if expected < 1 else None,
        "cross": [{"a": a, "b": b, "n": v} for (a, b), v in sorted(cross.items())],
        "final": dict(Counter(c["label"] for c in result)),
        "adjudicated": len(lanes[2]),
        "lane_missing": [len(sample) - len(lane) for lane in lanes[:2]],
        "invalid_initial_pairs": invalid_pairs,
        "invalid_initial_labels": [
            sum(r["reason"] in invalid_reasons for r in lane.values())
            for lane in lanes[:2]
        ],
    }
    write(PRIVATE / "final-labels.json", result)
    return result, stats, {c["id"]: c for c in result}


def picked(probabilities: dict | None, options: list[str]) -> str | None:
    if not probabilities:
        return None
    top = max(options, key=lambda key: probabilities[key])
    return (
        top
        if (len(options) * probabilities[top] - 1) / (len(options) - 1) >= 0.6
        else None
    )


def engine_prediction(case: dict, receipt: dict | None) -> bool:
    answers = (
        receipt.get("answers", {}) if receipt and receipt["status"] == "ok" else {}
    )
    if not case["running"]:
        p = answers.get("keep_current")
        return p is None or p >= 0.3
    relation = picked(
        answers.get("relation_to_running"),
        ["refines", "continues", "independent", "conflicts", "other"],
    )
    if relation == "independent":
        return False
    if relation in ("refines", "continues"):
        return (
            picked(answers.get("steer_or_spawn"), ["steer", "queue", "spawn", "other"])
            != "spawn"
        )
    return True


def label_call_summary() -> dict:
    statuses, observed, repairs = Counter(), Counter(), 0
    for path in sorted((PRIVATE / "codex").glob("*/receipt.json")):
        receipt = read(path)
        repairs += receipt["trial_id"].endswith("-repair")
        observed.update(re.findall(r"^model: (.+)$", receipt.get("stderr", ""), re.M))
        if receipt["returncode"] != 0:
            statuses["process_failed"] += 1
            continue
        try:
            payload = json.loads(
                (path.parent / "prompt.txt").read_text().rsplit("평가 데이터:\n", 1)[1]
            )
            text = receipt["stdout"]
            labels = json.loads(text[text.find("{") : text.rfind("}") + 1])["labels"]
            if [r.get("id") for r in labels] != [r["id"] for r in payload]:
                statuses["id_coverage_failed"] += 1
            elif any(
                r.get("label") not in ("continue", "new", "uncertain")
                or not isinstance(r.get("reason"), str)
                for r in labels
            ):
                statuses["label_fields_failed"] += 1
            else:
                statuses["ok"] += 1
        except (ValueError, KeyError, TypeError, IndexError):
            statuses["json_failed"] += 1
    return {
        "status": dict(statuses),
        "observed_models": dict(observed),
        "repair_calls": repairs,
    }


def main() -> None:
    sample = read(PRIVATE / "sample.json")
    labelled, label_stats, by_id = agreed_labels(sample)
    clear = [c for c in labelled if c["label"] != "uncertain"]
    receipts = rows(PRIVATE / "jev.jsonl")
    receipt_index = {r["trial_id"]: r for r in receipts}
    first = {
        condition: {
            r["id"]: r
            for r in receipts
            if r["condition"] == condition and r["repeat"] == 1
        }
        for condition in ("A", "B")
    }
    probabilities = {
        condition: {
            key: r["answers"]["keep_current"]
            for key, r in first[condition].items()
            if r["status"] == "ok"
        }
        for condition in ("A", "B")
    }
    draws = cluster_counts(clear, SEED)
    curves, comparisons = {}, []
    for condition in ("A", "B"):
        probs = probabilities[condition]
        curve = []
        for threshold in THRESHOLDS:
            predictions = {key: p >= threshold for key, p in probs.items()}
            valid = [c for c in clear if c["id"] in probs]
            metric = measure(
                confusion(
                    [(c["label"] == "continue", predictions[c["id"]]) for c in valid]
                )
            )
            metric.update(
                threshold=threshold, cluster=bootstrap(clear, predictions, draws)
            )
            metric["ask_rate_hypothetical"] = ratio(
                sum(
                    c["id"] not in probs or 0.3 <= probs[c["id"]] < threshold
                    for c in sample
                ),
                len(sample),
            )
            metric["ask_rate_current_router"] = ratio(0, len(sample))
            metric["operational_recall"] = ratio(
                metric["tp"], sum(c["label"] == "continue" for c in clear)
            )
            repeat_labels, deviations = [], []
            for c in sample:
                group = [
                    receipt_index.get(f"{c['id']}-{condition}-r{r}", {})
                    for r in (1, 2, 3)
                ]
                if all(r.get("status") == "ok" for r in group):
                    ps = [r["answers"]["keep_current"] for r in group]
                    repeat_labels.append(len({p >= threshold for p in ps}) == 1)
                    deviations.append(pstdev(ps))
            metric["repeat_agreement"] = ratio(sum(repeat_labels), len(repeat_labels))
            metric["repeat_incomplete"] = len(sample) - len(repeat_labels)
            metric["mean_probability_stddev"] = (
                sum(deviations) / len(deviations) if deviations else None
            )
            eligible = []
            for name in ("precision", "recall"):
                ci = metric["cluster"].get(name, {}).get("ci")
                eligible.append(
                    metric[name]["half_width"] is not None
                    and metric[name]["half_width"] <= 0.07
                    and ci is not None
                    and (ci[1] - ci[0]) / 2 <= 0.07
                )
            ci = metric["cluster"].get("precision", {}).get("ci")
            metric["recommendation_eligible"] = (
                all(eligible) and metric["precision"]["ci"][0] >= 0.8 and ci[0] >= 0.8
            )
            curve.append(metric)
        curves[condition] = curve
    paired = [
        c
        for c in clear
        if all(c["id"] in probabilities[condition] for condition in ("A", "B"))
    ]
    for threshold in THRESHOLDS:
        ab = [
            {c["id"]: probabilities[condition][c["id"]] >= threshold for c in paired}
            for condition in ("A", "B")
        ]
        scores = [
            measure(
                confusion([(c["label"] == "continue", preds[c["id"]]) for c in paired])
            )
            for preds in ab
        ]
        b = sum(
            ab[0][c["id"]] == (c["label"] == "continue")
            and ab[1][c["id"]] != (c["label"] == "continue")
            for c in paired
        )
        c_count = sum(
            ab[1][c["id"]] == (c["label"] == "continue")
            and ab[0][c["id"]] != (c["label"] == "continue")
            for c in paired
        )
        delta = {}
        for name in ("precision", "recall", "accuracy", "false_join_rate"):
            vals = [s[name]["value"] for s in scores]
            delta[name] = (
                vals[1] - vals[0] if all(v is not None for v in vals) else None
            )
        delta["f1"] = (
            scores[1]["f1"] - scores[0]["f1"]
            if all(s["f1"] is not None for s in scores)
            else None
        )
        comparisons.append(
            {
                "threshold": threshold,
                "n": len(paired),
                "a_only_correct": b,
                "b_only_correct": c_count,
                "mcnemar_p": mcnemar(b, c_count),
                "difference": delta,
                "cluster": bootstrap_delta(paired, *ab, draws),
                "A": scores[0],
                "B": scores[1],
            }
        )
    sizes, engine, stratified, sensitivity, relation_counts = {}, {}, {}, {}, {}
    for condition in ("A", "B"):
        rs = [r for r in receipts if r["condition"] == condition]
        sizes[condition] = {
            "quantiles": {
                name: quantile([r["request_bytes"] for r in rs], q)
                for name, q in [
                    ("min", 0),
                    ("p50", 0.5),
                    ("p90", 0.9),
                    ("p95", 0.95),
                    ("p99", 0.99),
                    ("max", 1),
                ]
            },
            "status": dict(Counter(r["status"] for r in rs)),
            "http_status": dict(Counter(str(r.get("http_status")) for r in rs)),
            "models": dict(Counter(r.get("model") for r in rs if r.get("model"))),
        }
        engine[condition] = measure(
            confusion(
                [
                    (
                        c["label"] == "continue",
                        engine_prediction(c, first[condition].get(c["id"])),
                    )
                    for c in clear
                ]
            )
        )
        stratified[condition] = {}
        for field in ("project", "activity", "length"):
            groups = defaultdict(list)
            for case in clear:
                if case["id"] not in probabilities[condition]:
                    continue
                key = (
                    case["project"]
                    if field == "project"
                    else ("running" if case["running"] else "idle")
                    if field == "activity"
                    else (
                        "<100"
                        if len(case["input"]) < 100
                        else "100-999"
                        if len(case["input"]) < 1000
                        else "1000+"
                    )
                )
                groups[key].append(case)
            stratified[condition][field] = {
                key: measure(
                    confusion(
                        [
                            (
                                c["label"] == "continue",
                                probabilities[condition][c["id"]] >= 0.8,
                            )
                            for c in cases
                        ]
                    )
                )
                for key, cases in sorted(groups.items())
            }
        sensitivity[condition] = measure(
            confusion(
                [
                    (c["label"] == "continue", probabilities[condition][c["id"]] >= 0.8)
                    for c in clear
                    if c["initial_agreement"]
                    and c["initial_labels"][0]["label"] != "uncertain"
                    and c["id"] in probabilities[condition]
                ]
            )
        )
        relation_counts[condition] = dict(
            Counter(
                picked(
                    r.get("answers", {}).get("relation_to_running"),
                    ["refines", "continues", "independent", "conflicts", "other"],
                )
                or "fallback"
                for r in first[condition].values()
                if by_id[r["id"]]["running"]
            )
        )
    boundary = sorted(
        labelled,
        key=lambda c: (
            c["initial_agreement"],
            c["label"] != "uncertain",
            abs(probabilities["B"].get(c["id"], 0.8) - 0.8),
            c["id"],
        ),
    )[:30]
    write(
        PRIVATE / "human-review.json",
        {
            "status": "not human reviewed",
            "cases": [
                {
                    **c,
                    "responses": {
                        condition: first[condition].get(c["id"])
                        for condition in ("A", "B")
                    },
                }
                for c in boundary
            ],
        },
    )
    h1 = next(c for c in comparisons if c["threshold"] == 0.8)
    ci = h1["cluster"].get("accuracy", {}).get("ci")
    verdict = (
        "채택"
        if ci and ci[0] > 0 and h1["mcnemar_p"] < 0.05
        else "기각"
        if ci and ci[1] <= 0
        else "보류"
    )
    recommendations = {
        condition: next(
            (r["threshold"] for r in curve if r["recommendation_eligible"]), None
        )
        for condition, curve in curves.items()
    }
    engine_preds = [
        {c["id"]: engine_prediction(c, first[condition].get(c["id"])) for c in clear}
        for condition in ("A", "B")
    ]
    engine_comparison = {
        "accuracy_difference": engine["B"]["accuracy"]["value"]
        - engine["A"]["accuracy"]["value"],
        "cluster": bootstrap_delta(clear, *engine_preds, draws),
    }
    summary = {
        "run": read(PRIVATE / "run.json"),
        "planning": read(PUBLIC / "env.json")["planning"],
        "sample_target": read(PUBLIC / "env.json")["sample_target"],
        "selection": read(PRIVATE / "selection.json"),
        "labels": label_stats,
        "label_calls": label_call_summary(),
        "clear_n": len(clear),
        "paired_n": len(paired),
        "paired_excluded": len(clear) - len(paired),
        "curves": curves,
        "comparisons": comparisons,
        "recommendations": recommendations,
        "h1": {"verdict": verdict, **h1},
        "request_sizes": sizes,
        "engine_replay": engine,
        "engine_comparison": engine_comparison,
        "stratified": stratified,
        "initial_agreement_sensitivity": sensitivity,
        "relation_top": relation_counts,
        "calls": dict(Counter(r["kind"] for r in rows(PRIVATE / "calls.jsonl"))),
        "receipts": len(receipts),
        "expected_receipts": len(sample) * 6,
        "human_review_n": len(boundary),
        "context": {
            "activity": dict(
                Counter("running" if c["running"] else "idle" for c in sample)
            ),
            "missing_stop_reason": sum(c["stop_reason"] is None for c in sample),
            "no_assistant": sum(c["assistant_count"] == 0 for c in sample),
            "goal_truncated": sum(c["goal_omitted_chars"] > 0 for c in sample),
            "progress_truncated": sum(c["progress_omitted_chars"] > 0 for c in sample),
            "sessions": len({c["session"] for c in sample}),
            "saturn_n": sum(c["is_saturn"] for c in sample),
            "single_session_strata": sum(
                len({c["session"] for c in sample if c["project"] == p}) == 1
                for p in {c["project"] for c in sample}
            ),
        },
    }
    write(PUBLIC / "results/summary.json", summary)
    target = PUBLIC / "results/tables/thresholds.csv"
    target.parent.mkdir(parents=True, exist_ok=True)
    with target.open("w", newline="") as out:
        writer = csv.writer(out)
        writer.writerow(
            [
                "condition",
                "threshold",
                "n",
                "tp",
                "fp",
                "fn",
                "tn",
                "precision",
                "recall",
                "f1",
                "false_join_rate",
                "ask_rate_hypothetical",
                "repeat_agreement",
            ]
        )
        for condition, curve in curves.items():
            for r in curve:
                writer.writerow(
                    [
                        condition,
                        r["threshold"],
                        r["n"],
                        r["tp"],
                        r["fp"],
                        r["fn"],
                        r["tn"],
                        r["precision"]["value"],
                        r["recall"]["value"],
                        r["f1"],
                        r["false_join_rate"]["value"],
                        r["ask_rate_hypothetical"]["value"],
                        r["repeat_agreement"]["value"],
                    ]
                )
    sources = [
        "census.json",
        "population.json",
        "sample.json",
        "selection.json",
        "run.json",
        "calls.jsonl",
        "jev.jsonl",
        *[f"labels-{m}.json" for m in (*MODELS, "adjudicated")],
    ]
    (PUBLIC / "data/SHA256SUMS").write_text(
        "".join(
            hashlib.sha256((PRIVATE / name).read_bytes()).hexdigest()
            + "  "
            + name
            + "\n"
            for name in sources
        )
    )
    print(
        {
            "clear_n": len(clear),
            "agreement": label_stats["agreement"],
            "recommendations": recommendations,
            "h1": verdict,
            "calls": summary["calls"],
        }
    )


if __name__ == "__main__":
    main()
