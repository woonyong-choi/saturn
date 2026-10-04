"""봉인한 기준으로 질문별 곡선과 가설 판정 및 공개 집계를 만든다."""

from __future__ import annotations

import csv
import hashlib
import json
import math
import re
from collections import Counter
from statistics import NormalDist

from metrics import (
    cluster_metrics,
    compare,
    evaluate,
    judge_difference,
    performance,
    quantile,
    rate,
)
from runtime import PRIOR, PRIVATE, PUBLIC, read, rows, write

CONDITIONS = ("current", "scope", "scope_gate", "astra")
THRESHOLDS = [n / 100 for n in range(50, 100, 5)]
LOWERS = (0.7, 0.75, 0.8, 0.85)


def repeat_agreement(data: list[dict], threshold: float, lower: float) -> dict:
    grouped = {sid: [] for sid in sorted({r["sample_id"] for r in data})}
    for r in data:
        value = (
            None
            if not r["valid"]
            else (
                "auto"
                if r["probability"] >= threshold
                else "ask"
                if r["probability"] >= lower
                else "skip"
            )
        )
        grouped[r["sample_id"]].append(value)
    return rate(
        sum(
            len(v) == 3 and None not in v and len(set(v)) == 1 for v in grouped.values()
        ),
        len(grouped),
    )


def make_policy(
    data: list[dict], condition: str, threshold: float, lower: float
) -> dict:
    result = dict(
        condition=condition,
        threshold=threshold,
        lower=lower,
        **evaluate(data, threshold, lower),
    )
    result["simultaneous"] = evaluate(data, threshold, lower, 0.0125 / 36)
    p, c, a = (result[k]["value"] for k in ("precision", "combined_recall", "ask_rate"))
    result["point_pass"] = (
        p is not None
        and c is not None
        and p >= 0.9
        and c >= 0.7
        and a <= 0.2
        and result["precision"]["n"] >= 10
    )
    sim = result["simultaneous"]
    limits = [sim[k]["ci"] for k in ("precision", "combined_recall", "ask_rate")]
    result["interval_pass"] = (
        all(limits)
        and limits[0][0] >= 0.9
        and limits[1][0] >= 0.7
        and limits[2][1] <= 0.2
    )
    result["interval_fail"] = all(limits) and (
        limits[0][1] < 0.9 or limits[1][1] < 0.7 or limits[2][0] > 0.2
    )
    return result


def summarize_audit() -> dict:
    result = read(PUBLIC / "results/audit.json")
    result["agreement_rate"] = rate(result["agreement"], result["n"])
    samples = {c["sample_id"]: c for c in read(PRIOR / "samples.json")}
    old = {r["sample_id"]: r for r in read(PRIOR / "gold.json")}
    new = read(PRIVATE / "audit-astra.json")
    result["by_kind"] = {
        kind: rate(
            sum(
                old[r["sample_id"]]["label"] == r["value"]["label"]
                for r in new
                if samples[r["sample_id"]]["input_kind"] == kind
            ),
            sum(samples[r["sample_id"]]["input_kind"] == kind for r in new),
        )
        for kind in ("human_likely", "ai_instruction_likely")
    }
    old_counts = Counter(old[r["sample_id"]]["label"] for r in new)
    new_counts = Counter(r["value"]["label"] if r["value"] else "invalid" for r in new)
    expected = sum(old_counts[k] * new_counts[k] for k in old_counts) / len(new) ** 2
    result["kappa"] = (result["agreement"] / len(new) - expected) / (1 - expected)
    result["old_positive"] = old_counts["constraint"]
    result["new_positive"] = new_counts["constraint"]
    result["new_uncertain"] = new_counts["uncertain"]
    result["old_positive_retained"] = rate(
        result["transitions"].get("constraint->constraint", 0), old_counts["constraint"]
    )
    records = [read(f) for f in sorted((PRIOR / "raw").glob("query-jev-*.json"))]
    result["source_hashes"] = {
        n: hashlib.sha256((PRIOR / n).read_bytes()).hexdigest()
        for n in ("samples.json", "gold.json", "calls.jsonl")
    }
    result["question_audit"] = dict(
        requests=len(records),
        unique_questions=len(
            {json.dumps(r["request"]["questions"], sort_keys=True) for r in records}
        ),
        state_keys=sorted(records[0]["request"]["state"]),
        explicit_task_exclusion=False,
        llm_explicit_task_exclusion=True,
        current_core_implemented=True,
        prior_declarative_current_interrogative=True,
    )
    return result


# cost: io 1 saved sample read; basis: estimate
def evidence_quality(gold: list[dict]) -> dict:
    cases = {c["sample_id"]: c for c in read(PRIVATE / "samples.json")}
    invalid = []
    for g in gold:
        for index, vote in enumerate(g["judgments"]):
            if not vote:
                continue
            for ref in vote["evidence"]:
                match = re.fullmatch(r"future:(\d+)", str(ref))
                if ref != "target" and (
                    not match or int(match[1]) >= len(cases[g["sample_id"]]["future"])
                ):
                    invalid.append((g["sample_id"], index, g["label"]))
    return dict(
        invalid_references=len(invalid),
        affected_inputs=len({r[0] for r in invalid}),
        by_vote=dict(Counter(str(r[1]) for r in invalid)),
        affected_labels=dict(Counter(r[2] for r in invalid)),
    )


def hypotheses(first: dict, all_data: dict, policies: list[dict]) -> dict:
    h1 = [
        dict(threshold=t, **compare(first["current"], first["scope"], t))
        for t in (0.8, 0.9)
    ]
    h3 = [
        dict(threshold=t, **compare(first["scope"], first["scope_gate"], t))
        for t in (0.8, 0.9)
    ]
    p9 = [p for p in policies if p["threshold"] == 0.9]
    h2 = (
        "채택"
        if any(p["interval_pass"] for p in p9)
        else "기각"
        if all(p["interval_fail"] for p in p9)
        else "보류"
    )
    h4 = {
        c: performance(all_data["current"], all_data[c])
        for c in ("scope", "scope_gate")
    }
    values = list(h4.values())
    if any(not v["p95_ci"] or not v["cost_ratio_ci"] for v in values):
        h4_verdict = "보류"
    elif all(v["p95_ci"][1] < 1 and v["cost_ratio_ci"][1] <= 1.5 for v in values):
        h4_verdict = "채택"
    elif any(v["p95_ci"][0] >= 1 or v["cost_ratio_ci"][0] > 1.5 for v in values):
        h4_verdict = "기각"
    else:
        h4_verdict = "보류"
    return dict(
        H1=dict(verdict=judge_difference(h1), comparisons=h1),
        H2=dict(
            verdict=h2,
            candidates=len(p9),
            point_pass=sum(p["point_pass"] for p in p9),
            interval_pass=sum(p["interval_pass"] for p in p9),
        ),
        H3=dict(verdict=judge_difference(h3), comparisons=h3),
        H4=dict(verdict=h4_verdict, conditions=h4),
    )


# cost: io 2 saved accounting reads; basis: estimate
def summarize_calls() -> dict:
    reservations = rows(PRIVATE / "calls.jsonl")
    accounting = read(PRIVATE / "accounting.json")
    known_costs = [a["cost_usd"] for a in accounting if a["cost_usd"] is not None]
    return dict(
        reserved=dict(Counter(r["kind"] for r in reservations)),
        by_phase=dict(
            Counter(
                "-".join(r["trial_id"].split("-")[:2])
                if r["kind"] == "codex"
                else "jev"
                for r in reservations
            )
        ),
        saved=len(accounting),
        incomplete=len(reservations) - len(accounting),
        statuses=dict(Counter(a["status"] for a in accounting)),
        known_cost_usd=sum(known_costs),
        cost_unknown=len(accounting) - len(known_costs),
        cost_by_kind={
            k: sum(
                a["cost_usd"]
                for a in accounting
                if a["kind"] == k and a["cost_usd"] is not None
            )
            for k in ("jev", "codex")
        },
    )


def summarize_performance(data: list[dict]) -> dict:
    times = [r["latency_s"] for r in data if r["latency_s"] is not None]
    amortized = [
        r["amortized_latency_s"] for r in data if r["amortized_latency_s"] is not None
    ]
    costs = [r["cost_usd"] for r in data if r["cost_usd"] is not None]
    return dict(
        median_request_s=quantile(times, 0.5),
        median_amortized_s=quantile(amortized, 0.5),
        p95_request_s=quantile(times, 0.95),
        mean_cost_usd=sum(costs) / len(costs) if costs else None,
        valid=sum(r["valid"] for r in data),
    )


# cost: io aggregate writes and input checksum reads; basis: estimate
def write_public(summary: dict) -> None:
    write(PUBLIC / "results/summary.json", summary)
    for name, items in [
        ("thresholds", summary["curves"]),
        ("policies", summary["policies"]),
    ]:
        target = PUBLIC / f"results/tables/{name}.csv"
        fields = [
            "condition",
            "threshold",
            "lower",
            "precision",
            "recall",
            "ask_rate",
            "combined_recall",
            "precision_n",
        ]
        with target.open("w", newline="") as stream:
            writer = csv.DictWriter(stream, fieldnames=fields)
            writer.writeheader()
            for r in items:
                writer.writerow(
                    {
                        **{k: r[k] for k in fields[:3]},
                        **{k: r[k]["value"] for k in fields[3:-1]},
                        "precision_n": r["precision"]["n"],
                    }
                )
    names = (
        "samples.json",
        "gold.json",
        "calls.jsonl",
        "processed.jsonl",
        "design-seal.json",
        "audit-astra.json",
        "review-followup.json",
    )
    (PUBLIC / "data/SHA256SUMS").write_text(
        "".join(
            hashlib.sha256((PRIVATE / n).read_bytes()).hexdigest() + "  " + n + "\n"
            for n in names
        )
    )


# cost: io saved observations and aggregate artifacts; basis: estimate
def main() -> None:
    data = rows(PRIVATE / "processed.jsonl")
    all_data = {c: [r for r in data if r["condition"] == c] for c in CONDITIONS}
    first = {c: [r for r in all_data[c] if r["repeat"] == 1] for c in CONDITIONS}
    policies = [
        make_policy(first[c], c, t, lo)
        for c in CONDITIONS[:3]
        for t in (0.8, 0.9)
        for lo in LOWERS
        if lo < t
    ]
    curves = [
        dict(
            condition=c,
            threshold=t,
            lower=min(0.7, t),
            **evaluate(first[c], t, min(0.7, t)),
        )
        for c in CONDITIONS
        for t in THRESHOLDS
    ]
    candidates = [p for p in policies if p["point_pass"]]
    candidates.sort(
        key=lambda p: (CONDITIONS.index(p["condition"]), p["threshold"], -p["lower"])
    )
    gold = read(PRIVATE / "gold.json")
    counts = Counter(g["label"] for g in gold)
    has_sol = {g["sample_id"]: g["judgments"][0] is not None for g in gold}
    groups = {
        c: {
            "with_sol_vote": evaluate(
                [r for r in first[c] if has_sol[r["sample_id"]]], 0.9, 0.7
            ),
            "new": evaluate(
                [r for r in first[c] if not r["previously_seen"]], 0.9, 0.7
            ),
            **{
                kind: evaluate(
                    [r for r in first[c] if r["input_kind"] == kind], 0.9, 0.7
                )
                for kind in ("human_likely", "ai_instruction_likely")
            },
        }
        for c in CONDITIONS
    }
    z, zb = NormalDist().inv_cdf(1 - 0.0125), NormalDist().inv_cdf(0.8)
    needed = math.ceil(
        ((z * math.sqrt(0.7 * 0.3) + zb * math.sqrt(0.9 * 0.1)) / 0.2) ** 2 + 1 / 0.2
    )
    summary = dict(
        design_seal=read(PRIVATE / "design-seal.json"),
        audit=summarize_audit(),
        sampling=read(PRIVATE / "sampling.json"),
        power=dict(
            p0=0.7,
            p1=0.9,
            alpha=0.0125,
            power=0.8,
            positive_required=needed,
            input_required=math.ceil(needed / (16 / 90 * 0.9)),
            actual_positive=counts["constraint"],
        ),
        execution_integrity={
            k: v
            for k, v in read(PRIVATE / "review-followup.json").items()
            if k != "ancestor_instruction_files"
        },
        evidence_quality=evidence_quality(gold),
        gold=dict(
            n=len(gold),
            labels=dict(counts),
            initial_agreement=rate(sum(g["initially_agreed"] for g in gold), len(gold)),
            third=sum(not g["initially_agreed"] for g in gold),
            valid_pair_agreement=rate(
                sum(g["initially_agreed"] for g in gold),
                sum(bool(g["judgments"][0] and g["judgments"][1]) for g in gold),
            ),
            semantic_disagreements=sum(
                bool(g["judgments"][0] and g["judgments"][1])
                and not g["initially_agreed"]
                for g in gold
            ),
            invalid_judgments={
                m: sum(g["judgments"][i] is None for g in gold)
                for i, m in enumerate(("sol", "astra"))
            },
            invalid_batches={
                m: len(
                    {
                        r["trial_id"]
                        for r in read(PRIVATE / f"gold-{m}.json")
                        if r["value"] is None
                    }
                )
                for m in ("sol", "astra")
            },
            astra_only_resolved=sum(
                g["judgments"][0] is None and g["label"] != "uncertain" for g in gold
            ),
        ),
        invalid_by_condition={
            c: sum(not r["valid"] for r in all_data[c]) for c in CONDITIONS
        },
        curves=curves,
        policies=policies,
        hypotheses=hypotheses(first, all_data, policies),
        recommendation=candidates[0] if candidates else None,
        groups=groups,
        cluster_ci95={
            c: {str(t): cluster_metrics(first[c], t) for t in (0.8, 0.9)}
            for c in CONDITIONS
        },
        repeats={
            c: {
                str(t): repeat_agreement(all_data[c], t, min(0.7, t))
                for t in THRESHOLDS
            }
            for c in CONDITIONS[:3]
        },
        calls=summarize_calls(),
        performance={c: summarize_performance(first[c]) for c in CONDITIONS},
    )
    write_public(summary)
    print(
        json.dumps(
            {k: v["verdict"] for k, v in summary["hypotheses"].items()},
            ensure_ascii=False,
        )
    )


if __name__ == "__main__":
    main()
