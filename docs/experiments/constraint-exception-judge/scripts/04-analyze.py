"""봉인 입력과 원응답에서 정규화 관측과 공개 집계를 재생성한다."""

from __future__ import annotations

import csv
import hashlib
import importlib
import json
import sys
from collections import Counter
from pathlib import Path

from exploration import decimal_sensitivity, tie_order_audit, unwrap
from inference import (
    binomial_power,
    binomial_tail,
    cluster_intervals,
    distribution,
    holm,
    metric_counts,
    paired,
    paired_power,
    power,
    ratio,
)
from publication import check_raw_seal, public_rows, source_fragments
from runtime import PRIVATE, PUBLIC, read, rows, setup, write

grader = importlib.import_module("03-grade")
collector = importlib.import_module("02-collect")


def workflow_cost(row: dict) -> dict:
    calls = []
    for trial in row.get("stages", []):
        path = (
            PRIVATE / "claude" / trial / "receipt.json"
            if row["condition"] == "L1"
            else PRIVATE / "jev" / (trial + ".json")
        )
        if not path.exists():
            calls.append(
                dict(
                    cost_usd=None,
                    estimated_cost_usd=None,
                    latency_ms=None,
                    status="incomplete",
                    usage={},
                )
            )
            continue
        receipt = read(path)
        cost = None
        if row["condition"] == "L1":
            try:
                wrapper = json.loads(receipt["stdout"])
            except (ValueError, KeyError):
                wrapper = {}
            cost = wrapper.get("total_cost_usd")
            usage = wrapper.get("usage") or {}
            if "input_tokens" in usage and "output_tokens" in usage:
                estimate = (
                    usage["input_tokens"]
                    + 5 * usage["output_tokens"]
                    + 1.25 * usage.get("cache_creation_input_tokens", 0)
                    + 0.75
                    * usage.get("cache_creation", {}).get(
                        "ephemeral_1h_input_tokens", 0
                    )
                    + 0.1 * usage.get("cache_read_input_tokens", 0)
                ) / 1_000_000
            else:
                estimate = None
        else:
            reply = json.loads(receipt.get("raw_response", "{}"))
            usage = receipt.get("usage") or reply.get("usage") or {}
            tokens = usage.get("input_tokens")
            estimate = tokens * 0.042 / 1_000_000 if tokens is not None else None
            cost = estimate
        calls.append(
            dict(
                cost_usd=cost,
                estimated_cost_usd=estimate,
                latency_ms=receipt.get("latency_ms"),
                status=receipt.get("status"),
                usage=usage,
            )
        )
    costs = [c["cost_usd"] for c in calls if c["cost_usd"] is not None]
    return dict(
        call_count=len(calls),
        unknown_cost_calls=len(calls) - len(costs),
        cost_usd=sum(costs) if len(costs) == len(calls) and calls else None,
        latency_ms=sum(c["latency_ms"] for c in calls)
        if calls and all(c["latency_ms"] is not None for c in calls)
        else None,
        calls=calls,
    )


def normalized(item: dict, row: dict, grades: dict) -> dict:
    valid = row.get("status") == "ok"
    pred = row.get("prediction") or {}
    gold = item["gold"]
    result = {
        k: row.get(k)
        for k in (
            "run_id",
            "trial_id",
            "item_id",
            "task",
            "condition",
            "repeat",
            "ts_utc",
            "status",
        )
    }
    result.update(
        cluster=item["cluster"],
        source=item["source"],
        valid=valid,
        format_error=row.get("status") == "invalid",
        failure=not valid,
        prediction_signature=hashlib.sha256(
            json.dumps(pred, sort_keys=True, ensure_ascii=False).encode()
        ).hexdigest(),
        **workflow_cost(row),
    )
    if item["task"] == "continuation":
        result.update(
            gold_positive=gold["continue"],
            pred_positive=valid and pred.get("continue", False),
            gold_negative=not gold["continue"],
            correct=valid and pred == gold,
        )
        return result
    positive = gold["request"] != "none"
    predicted = valid and pred.get("request") != "none"
    exception = gold["kind"] in ("once", "scoped")
    scope = pred.get("scope_text")
    semantic = (
        valid
        and exception
        and isinstance(scope, str)
        and grades.get(grader.pair_id(item["id"], scope), False)
    )
    target = valid and pred.get("target") == gold["target"]
    kind = valid and pred.get("kind") == gold["kind"]
    request = valid and pred.get("request") == gold["request"]
    permanent = valid and pred.get("kind") == "permanent"
    result.update(
        gold_kind=gold["kind"],
        active_count=len(item["rules"]),
        gold_positive=positive,
        gold_negative=not positive,
        gold_exception=exception,
        gold_nonpermanent=gold["kind"] != "permanent",
        pred_positive=predicted,
        true_positive=positive and predicted,
        target_correct=target,
        kind_correct=kind,
        request_correct=request,
        scope_semantic=semantic,
        scope_exact=valid and exception and scope == gold["scope_text"],
        pred_permanent=permanent,
        false_permanent=permanent and (gold["kind"] != "permanent" or not target),
        correct=request and target and kind and (not exception or semantic),
    )
    return result


# cost: io O(n) local reads and 3 derived writes; vars: n = receipts; basis: estimate
def process() -> None:
    source_fragments()
    check_raw_seal(create=True)
    items = read(PRIVATE / "items.json")
    grades = {
        r["id"]: r["equivalent"] for r in read(PRIVATE / "scope-prediction-grades.json")
    }
    grades.update(
        {r["id"]: r["equivalent"] for r in read(PRIVATE / "scope-rounding-grades.json")}
    )
    j2 = set(read(PRIVATE / "j2-ids.json"))
    reserved = {
        r["trial_id"]
        for r in rows(PRIVATE / "calls.jsonl")
        if r["kind"] in ("jev", "claude")
    }
    observations = []
    unwrapped = []
    rounded = []
    for item in items:
        conditions = (
            ("B1", "L1")
            if item["task"] == "continuation"
            else ("J1", "L1", "J2")
            if item["id"] in j2
            else ("J1", "L1")
        )
        for condition in conditions:
            for repeat in (1, 2, 3):
                tid = f"{item['id']}-{condition}-r{repeat}"
                path = PRIVATE / "workflows" / (tid + ".json")
                row = (
                    read(path)
                    if path.exists()
                    else dict(
                        run_id=read(PRIVATE / "run.json")["run_id"],
                        trial_id=tid,
                        item_id=item["id"],
                        task=item["task"],
                        condition=condition,
                        repeat=repeat,
                        status="incomplete",
                        stages=sorted(
                            t
                            for t in reserved
                            if t == tid
                            or t in [tid + "-" + stage for stage in ("A", "B", "C")]
                        ),
                    )
                )
                observations.append(normalized(item, row, grades))
                if condition == "L1":
                    unwrapped.append(normalized(item, unwrap(row, item), grades))
                if condition == "J1":
                    rounded.append(
                        normalized(item, decimal_sensitivity(row, item), grades)
                    )
    (PRIVATE / "processed.jsonl").write_text(
        "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in observations)
    )
    (PRIVATE / "exploratory-processed.jsonl").write_text(
        "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in unwrapped)
    )
    (PRIVATE / "exploratory-rounding.jsonl").write_text(
        "".join(json.dumps(r, ensure_ascii=False) + "\n" for r in rounded)
    )
    public = public_rows(items)
    write(PUBLIC / "data/generated.json", public)
    files = sorted(
        p
        for p in PRIVATE.rglob("*")
        if p.is_file()
        and "runtime" not in p.relative_to(PRIVATE).parts
        and not p.name.endswith((".log", ".lock"))
    )
    (PUBLIC / "data/SHA256SUMS").write_text(
        "".join(
            hashlib.sha256(p.read_bytes()).hexdigest()
            + "  "
            + str(p.relative_to(PRIVATE))
            + "\n"
            for p in files
        )
    )
    print(json.dumps({"processed": len(observations), "public_generated": len(public)}))


def summarize(group: list[dict]) -> dict:
    first = [r for r in group if r["repeat"] == 1]
    metrics = metric_counts(first)
    costs = [r["cost_usd"] for r in group if r["cost_usd"] is not None]
    calls = [c for r in group for c in r["calls"]]
    by_id = {
        i: [r for r in group if r["item_id"] == i]
        for i in {r["item_id"] for r in group}
    }
    repeats = sum(
        len(rs) == 3
        and all(r["valid"] for r in rs)
        and len({r["prediction_signature"] for r in rs}) == 1
        for rs in by_id.values()
    )
    return dict(
        n=len(first),
        metrics=metrics,
        statuses=dict(Counter(r["status"] for r in group)),
        repeat_agreement=ratio(repeats, len(by_id)),
        latency_ms=distribution(
            [r["latency_ms"] for r in group if r["latency_ms"] is not None]
        ),
        cost_usd=distribution(costs),
        known_total_cost_usd=sum(
            c["cost_usd"] for c in calls if c["cost_usd"] is not None
        ),
        unknown_cost_calls=sum(r["unknown_cost_calls"] for r in group),
        call_count=sum(r["call_count"] for r in group),
        per_call_latency_ms=distribution(
            [c["latency_ms"] for c in calls if c["latency_ms"] is not None]
        ),
        per_call_cost_usd=distribution(
            [c["cost_usd"] for c in calls if c["cost_usd"] is not None]
        ),
        estimated_total_cost_usd=sum(
            c["estimated_cost_usd"]
            for c in calls
            if c["estimated_cost_usd"] is not None
        ),
    )


def hypotheses(conditions: dict, comparison: dict) -> dict:
    h = {}
    for name, condition in [("H1", "constraint/J1"), ("H2", "constraint/L1")]:
        metrics = conditions[condition]["metrics"]
        pvalues = {}
        for metric, p0, upper in [
            ("false_permanent", 0.01, False),
            ("request_recall", 0.95, True),
            ("kind_accuracy", 0.9, True),
        ]:
            m = metrics[metric]
            pvalues[metric] = binomial_tail(m["k"], m["n"], p0, upper=upper)
        point = (
            metrics["false_permanent"]["value"] <= 0.01
            and metrics["request_recall"]["value"] >= 0.95
            and metrics["kind_accuracy"]["value"] >= 0.9
        )
        ci = conditions[condition]["cluster_intervals"]
        cluster = (
            ci["false_permanent"]["ci"][1] <= 0.01
            and ci["request_recall"]["ci"][0] >= 0.95
            and ci["kind_accuracy"]["ci"][0] >= 0.9
        )
        rejected = (
            metrics["false_permanent"]["ci"][0] > 0.01
            or metrics["request_recall"]["ci"][1] < 0.95
            or metrics["kind_accuracy"]["ci"][1] < 0.9
        )
        h[name] = dict(
            condition=condition,
            p=max(pvalues.values()),
            component_p=pvalues,
            point_pass=point,
            cluster_pass=cluster,
            rejected=rejected,
        )
    h["H3"] = dict(p=comparison["p"], comparison="continuation/L1-minus-B1")
    corrected = holm({k: v["p"] for k, v in h.items()})
    for name, value in h.items():
        value["holm_p"] = corrected[name]
        if name == "H3":
            lo, hi = comparison["ci"]
            value["verdict"] = (
                "채택" if corrected[name] < 0.05 and (lo > 0 or hi < 0) else "보류"
            )
        else:
            value["verdict"] = (
                "채택"
                if value["point_pass"]
                and value["cluster_pass"]
                and corrected[name] < 0.05
                else "기각"
                if value["rejected"]
                else "보류"
            )
    return h


# cost: time O(b*n), heap O(n), io 2 aggregate writes; vars: b = bootstrap count, n = observations; basis: estimate
def analyze() -> None:
    observations = rows(PRIVATE / "processed.jsonl")
    conditions = {}
    groups = {
        key: [r for r in observations if r["task"] + "/" + r["condition"] == key]
        for key in sorted({r["task"] + "/" + r["condition"] for r in observations})
    }
    for key, group in groups.items():
        conditions[key] = summarize(group)
        names = (
            ["accuracy", "false_join", "recall"]
            if key.startswith("continuation")
            else ["accuracy", "false_permanent", "request_recall", "kind_accuracy"]
        )
        conditions[key]["cluster_intervals"] = cluster_intervals(
            [r for r in group if r["repeat"] == 1], names
        )

    def first(key: str) -> list[dict]:
        return [r for r in groups[key] if r["repeat"] == 1]

    comparisons = {
        "constraint/L1-minus-J1": paired(
            first("constraint/J1"), first("constraint/L1")
        ),
        "continuation/L1-minus-B1": paired(
            first("continuation/B1"), first("continuation/L1")
        ),
        "constraint/J2-minus-J1": paired(
            first("constraint/J1"), first("constraint/J2")
        ),
        "constraint/L1-minus-J2": paired(
            first("constraint/J2"), first("constraint/L1")
        ),
    }
    strata = {}
    for key, group in groups.items():
        for field in (
            ("source", "gold_kind", "active_count")
            if key.startswith("constraint")
            else ("source",)
        ):
            for value in sorted({str(r[field]) for r in group}):
                strata[key + "/" + field + "/" + value] = summarize(
                    [r for r in group if str(r[field]) == value]
                )
    hs = hypotheses(conditions, comparisons["continuation/L1-minus-B1"])
    point = [
        h["condition"] for name, h in hs.items() if name != "H3" and h["point_pass"]
    ]
    confirmed = [
        h["condition"]
        for name, h in hs.items()
        if name != "H3" and h["verdict"] == "채택"
    ]

    def order(key: str) -> tuple:
        cost = conditions[key]["cost_usd"]["mean"]
        return cost if cost is not None else float("inf"), conditions[key][
            "latency_ms"
        ]["median"]

    def has_costs(keys: list[str]) -> bool:
        return bool(keys) and all(
            conditions[k]["unknown_cost_calls"] == 0
            and conditions[k]["cost_usd"]["mean"] is not None
            for k in keys
        )

    summary = dict(
        run=read(PRIVATE / "run.json"),
        flow=read(PRIVATE / "flow.json"),
        conditions=conditions,
        comparisons=comparisons,
        strata=strata,
        hypotheses=hs,
        power=power(),
        environment=read(PUBLIC / "env.json"),
        calls=dict(Counter(r["kind"] for r in rows(PRIVATE / "calls.jsonl"))),
        recommendation=sorted(confirmed, key=order)[0]
        if has_costs(confirmed)
        else None,
        quality_qualified=confirmed,
        point_qualified=point,
        cost_ranking_available=has_costs(point),
        point_candidate=sorted(point, key=order)[0] if has_costs(point) else None,
    )
    exploratory = []
    inputs = {i["id"]: i for i in read(PRIVATE / "items.json")}
    grades = {
        g["id"]: g["equivalent"] for g in read(PRIVATE / "scope-prediction-grades.json")
    }
    for path in sorted((PRIVATE / "workflows").glob("*-J1-r1.json")):
        row = read(path)
        if row.get("request_p", 0) < 0.8 and row.get("status") == "ok":
            row["prediction"] = dict(
                request="none", target="none", kind="none", scope_text=None
            )
        exploratory.append(normalized(inputs[row["item_id"]], row, grades))
    summary["exploratory_j1_threshold_080"] = metric_counts(exploratory)
    cm = conditions["constraint/J1"]["metrics"]
    summary["continuation_flow"] = {
        "reviewed": len(read(PRIVATE / "continuation-labels.json"))
        + len(read(PRIVATE / "continuation-exclusions.json")),
        "agreed": len(read(PRIVATE / "continuation-labels.json")),
        "excluded": dict(
            Counter(r["reason"] for r in read(PRIVATE / "continuation-exclusions.json"))
        ),
        "selected": len([i for i in inputs.values() if i["task"] == "continuation"]),
    }
    summary["exclusion_reasons"] = dict(
        Counter(r["reason"] for r in read(PRIVATE / "excluded.json"))
    )
    summary["observed_power"] = {
        "recall": binomial_power(cm["request_recall"]["n"], 0.95, 0.99, upper=True),
        "false_permanent": binomial_power(
            cm["false_permanent"]["n"], 0.01, 0.0001, upper=False
        ),
        "continuation": paired_power(comparisons["continuation/L1-minus-B1"]["n"]),
        "kinds": {
            k: binomial_power(
                sum(
                    i["task"] == "constraint" and i["gold"]["kind"] == k
                    for i in inputs.values()
                ),
                0.9,
                0.99,
                upper=True,
            )
            for k in ("permanent", "once", "scoped")
        },
    }
    summary["claude_models"] = dict(
        Counter(
            model
            for path in (PRIVATE / "workflows").glob("*-L1-*.json")
            for model in (read(path).get("model_usage") or {})
        )
    )
    summary["claude_usage"] = dict(
        Counter(
            {
                field: sum(
                    c["usage"].get(field, 0)
                    for r in observations
                    if r["condition"] == "L1"
                    for c in r["calls"]
                )
                for field in (
                    "input_tokens",
                    "output_tokens",
                    "cache_creation_input_tokens",
                    "cache_read_input_tokens",
                )
            }
        )
    )
    summary["claude_usage"]["cache_1h_input_tokens"] = sum(
        c["usage"].get("cache_creation", {}).get("ephemeral_1h_input_tokens", 0)
        for r in observations
        if r["condition"] == "L1"
        for c in r["calls"]
    )
    summary["interrupted"] = {
        "workflows": sum(r["status"] == "incomplete" for r in observations),
        "first_repeat": sum(
            r["status"] == "incomplete" and r["repeat"] == 1 for r in observations
        ),
        "calls": sum(
            r["call_count"] for r in observations if r["status"] == "incomplete"
        ),
        "resent": 0,
    }
    summary["jev_calls"] = dict(
        Counter(read(path)["status"] for path in (PRIVATE / "jev").glob("*.json"))
    )
    summary["continuation_gold"] = dict(
        Counter(
            "continue" if item["gold"]["continue"] else "new"
            for item in inputs.values()
            if item["task"] == "continuation"
        )
    )
    summary["claude_record_folders"] = [
        p.name
        for p in (Path.home() / ".claude/projects").glob(
            "*experiment-382-judge-vs-llm*claude-work*"
        )
        if p.is_dir()
    ]
    unwrapped = rows(PRIVATE / "exploratory-processed.jsonl")
    summary["exploratory_unwrapped"] = {
        task: summarize([r for r in unwrapped if r["task"] == task])
        for task in ("constraint", "continuation")
    }
    summary["exploratory_unwrapped_comparisons"] = {
        task: paired(
            first(task + "/" + baseline),
            [r for r in unwrapped if r["task"] == task and r["repeat"] == 1],
        )
        for task, baseline in (("constraint", "J1"), ("continuation", "B1"))
    }
    summary["exploratory_jev_decimal"] = summarize(
        rows(PRIVATE / "exploratory-rounding.jsonl")
    )
    summary["exploratory_semantic_comparison"] = paired(
        [r for r in rows(PRIVATE / "exploratory-rounding.jsonl") if r["repeat"] == 1],
        [r for r in unwrapped if r["task"] == "constraint" and r["repeat"] == 1],
    )
    summary["tie_order_audit"] = tie_order_audit(
        [read(path) for path in sorted((PRIVATE / "workflows").glob("*.json"))], inputs
    )
    summary["publication"] = {
        "redaction_span_chars": 8,
        "source_files": len(read(PRIVATE / "privacy-sources.json")),
        "generated_rows": len(read(PUBLIC / "data/generated.json")),
        "redacted_rows": sum(
            r["text_redacted"] for r in read(PUBLIC / "data/generated.json")
        ),
        "raw_seal_sha256": hashlib.sha256(
            (PUBLIC / "data/RAW_SHA256SUMS").read_bytes()
        ).hexdigest(),
        "derived_manifest_sha256": hashlib.sha256(
            (PUBLIC / "data/SHA256SUMS").read_bytes()
        ).hexdigest(),
    }
    write(PUBLIC / "results/summary.json", summary)
    with (PUBLIC / "results/conditions.csv").open("w", newline="") as stream:
        writer = csv.writer(stream)
        writer.writerow(["condition", "metric", "k", "n", "value", "ci_low", "ci_high"])
        for key, c in conditions.items():
            for name, m in c["metrics"].items():
                writer.writerow(
                    [key, name, m["k"], m["n"], m["value"], *(m["ci"] or [None, None])]
                )
    print(
        json.dumps(
            {
                "calls": summary["calls"],
                "hypotheses": hs,
                "recommendation": summary["recommendation"],
            },
            ensure_ascii=False,
        )
    )


if __name__ == "__main__":
    setup()
    (process if sys.argv[1] == "process" else analyze)()
