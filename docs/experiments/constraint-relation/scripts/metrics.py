"""같은 후보의 대응을 보존한 비율과 프로젝트 구간을 계산한다."""

from __future__ import annotations

import random

from support import SEED, binomial_tail, percentile, ratio


def classify(probabilities: list, threshold: float) -> str:
    replace, release = probabilities
    if replace is None or release is None:
        return "invalid"
    if release >= threshold:
        return "release"
    if replace >= threshold:
        return "replace"
    return "partial" if replace >= 0.5 else "compatible"


def operation_counts(rows: list[dict], operation: str, threshold: float) -> dict:
    tp = predicted = gold = valid_tp = valid_gold = 0
    for row in rows:
        outcome = classify(row["probabilities"][0], threshold)
        expected = row["gold"] == operation
        emitted = outcome == operation
        gold += expected
        predicted += emitted
        tp += expected and emitted
        if outcome != "invalid" and row["measured"]:
            valid_tp += expected and emitted
            valid_gold += expected
    return {
        "tp": tp,
        "predicted": predicted,
        "gold": gold,
        "precision": ratio(tp, predicted),
        "recall": ratio(tp, gold),
        "conditional_recall": ratio(valid_tp, valid_gold),
        "p_value": binomial_tail(tp, predicted, 0.9),
    }


def bootstrap_precision(
    rows: list[dict], operation: str, threshold: float, project_ids: list[str]
) -> dict:
    groups = {cid: [] for cid in project_ids}
    for row in rows:
        groups[row["conversation_id"]].append(row)
    counts = [
        operation_counts(group, operation, threshold) for group in groups.values()
    ]
    randomizer = random.Random(SEED)
    values, recalls = [], []
    for _ in range(2000):
        sample = randomizer.choices(counts, k=len(counts))
        tp = sum(c["tp"] for c in sample)
        n = sum(c["predicted"] for c in sample)
        gold = sum(c["gold"] for c in sample)
        if n:
            values.append(tp / n)
        if gold:
            recalls.append(tp / gold)
    return {
        "clusters": len(counts),
        "precision_ci95": [percentile(values, 0.025), percentile(values, 0.975)],
        "recall_ci95": [percentile(recalls, 0.025), percentile(recalls, 0.975)],
        "empty_precision_draws": 2000 - len(values),
        "empty_recall_draws": 2000 - len(recalls),
    }


def summarize_condition(rows: list[dict], infos: list[dict]) -> dict:
    curves = []
    for step in range(10):
        threshold = round(0.5 + 0.05 * step, 2)
        curves.append(
            {
                "threshold": threshold,
                **{
                    op: operation_counts(rows, op, threshold)
                    for op in ("replace", "release")
                },
            }
        )
    primary = curves[6]
    hypotheses = {}
    for op in ("replace", "release"):
        primary[op]["bootstrap"] = bootstrap_precision(
            rows, op, 0.8, sorted({i["conversation_id"] for i in infos})
        )
        metric = primary[op]
        lower = metric["precision"]["ci95"][0]
        upper = metric["precision"]["ci95"][1]
        boot = metric["bootstrap"]["precision_ci95"][0]
        accepted = (
            lower is not None
            and boot is not None
            and lower >= 0.9
            and boot >= 0.9
            and metric["p_value"] <= 0.0125
        )
        hypotheses[op] = (
            "채택"
            if accepted
            else ("기각" if upper is not None and upper < 0.9 else "보류")
        )
    measured = [r for r in rows if r["measured"]]
    repeats = {kind: [] for kind in ("classification", "replace", "release")}
    incomplete = 0
    for row in measured:
        outcomes = [classify(p, 0.8) for p in row["probabilities"]]
        if "invalid" in outcomes:
            incomplete += 1
            continue
        repeats["classification"].append(len(set(outcomes)) == 1)
        for op in ("replace", "release"):
            repeats[op].append(len({x == op for x in outcomes}) == 1)
    by_distance = {}
    for lo, hi, name in (
        (1, 5, "1-5"),
        (6, 20, "6-20"),
        (21, 100, "21-100"),
        (101, 100000, "101+"),
    ):
        subset = [r for r in measured if lo <= r["distance"] <= hi]
        all_rows = [r for r in rows if lo <= r["distance"] <= hi]
        by_distance[name] = {
            "accuracy": ratio(
                sum(classify(r["probabilities"][0], 0.8) == r["gold"] for r in subset),
                len(subset),
            ),
            **{
                op: operation_counts(all_rows, op, 0.8)["recall"]
                for op in ("replace", "release")
            },
        }
    valid = [r for r in measured if classify(r["probabilities"][0], 0.8) != "invalid"]
    for band, group in by_distance.items():
        low, high = {
            "1-5": (1, 5),
            "6-20": (6, 20),
            "21-100": (21, 100),
            "101+": (101, 100000),
        }[band]
        subset = [r for r in valid if low <= r["distance"] <= high]
        group["conditional_accuracy"] = ratio(
            sum(classify(r["probabilities"][0], 0.8) == r["gold"] for r in subset),
            len(subset),
        )
    coverage = {}
    for op in ("all", "replace", "release", "partial"):
        values = [
            i.get("coverage_by_operation", {}).get(
                op, {"n": 0, "before": 0, "after": 0}
            )
            for i in infos
        ]
        n = sum(v["n"] for v in values)
        coverage[op] = {
            phase: ratio(sum(v[phase] for v in values), n)
            for phase in ("before", "after")
        }
    return {
        "coverage_by_operation": coverage,
        "conditional_accuracy": ratio(
            sum(classify(r["probabilities"][0], 0.8) == r["gold"] for r in valid),
            len(valid),
        ),
        "selected_turns": len(infos),
        "pairs": len(measured),
        "missing_targets": sum(not r["candidate_included"] for r in rows),
        "unevaluable_targets": sum(
            not r["measured"] and r["candidate_included"] for r in rows
        ),
        "ambiguous_current_turns": sum(
            i.get("ambiguous_current", False) for i in infos
        ),
        "sample_coverage": ratio(
            sum(i.get("covered", 0) for i in infos),
            sum(i.get("gold_targets", 0) for i in infos),
        ),
        "curve": curves,
        "primary": primary,
        "hypotheses": hypotheses,
        "recommendation": "자동 가능"
        if all(v == "채택" for v in hypotheses.values())
        else "사용자 선택",
        "repeats": {k: ratio(sum(v), len(v)) for k, v in repeats.items()},
        "incomplete_triplets": incomplete,
        "distance": by_distance,
        "invalid_pairs": sum(
            classify(r["probabilities"][0], 0.8) == "invalid" for r in measured
        ),
    }


def compare_pairs(left: list[dict], right: list[dict], project_ids: list[str]) -> dict:
    index = {
        (r["conversation_id"], r["turn_id"], r["target"]): r
        for r in right
        if r["measured"]
    }
    groups = {cid: [] for cid in project_ids}
    b = c = 0
    for row in left:
        other = index.get((row["conversation_id"], row["turn_id"], row["target"]))
        if not row["measured"] or not other:
            continue
        a = classify(row["probabilities"][0], 0.8) == row["gold"]
        z = classify(other["probabilities"][0], 0.8) == other["gold"]
        b += a and not z
        c += z and not a
        groups[row["conversation_id"]].append(int(z) - int(a))
    diffs = [x for g in groups.values() for x in g]
    clusters = list(groups.values())
    randomizer = random.Random(SEED)
    samples = []
    for _ in range(2000):
        selected = [x for g in randomizer.choices(clusters, k=len(clusters)) for x in g]
        if selected:
            samples.append(sum(selected) / len(selected))
    return {
        "empty_draws": 2000 - len(samples),
        "n": len(diffs),
        "left_only_correct": b,
        "right_only_correct": c,
        "difference": sum(diffs) / len(diffs) if diffs else None,
        "ci95": [percentile(samples, 0.025), percentile(samples, 0.975)],
    }
