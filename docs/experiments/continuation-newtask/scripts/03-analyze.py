"""새 표본과 과거 공개 집계를 구분해 분포·성능·불확실성을 재생성한다."""

from __future__ import annotations

import csv
import json
from collections import Counter

from audit import audit
from power import calculate
from runtime import (
    MODELS,
    PRIVATE,
    PUBLIC,
    SEED,
    analysis,
    metrics,
    read,
    rows,
    sha,
    write,
)

NAMES = ("false_join_rate", "recall", "precision", "ask_rate", "operational_recall")


def labels() -> tuple[list[dict], dict]:
    lanes = []
    for model in (*MODELS, "adjudicated"):
        entries = []
        for path in sorted((PRIVATE / "labels").glob(model + "-*.json")):
            entries.extend(read(path))
        lanes.append({r["id"]: r for r in entries})
    cases = []
    for c in read(PRIVATE / "sample.json"):
        first = [
            idx.get(c["id"], {"label": "uncertain", "reason": "missing"})
            for idx in lanes[:2]
        ]
        format_failure = any(
            r.get("reason") == "format failure or incomplete call" for r in first
        )
        missing = any(c["id"] not in lane for lane in lanes[:2]) or format_failure
        agree = not missing and first[0]["label"] == first[1]["label"]
        label = (
            first[0]
            if agree and first[0]["label"] != "uncertain"
            else lanes[2].get(c["id"], {"label": "uncertain"})
        )
        cases.append(
            {
                **c,
                "label": label["label"],
                "initial_agreement": agree,
                "initial_labels": [r["label"] for r in first],
                "label_missing": missing,
                "initial_format_failure": format_failure,
                "adjudicated": c["id"] in lanes[2],
            }
        )
    write(PRIVATE / "final-labels.json", cases)
    return cases, {
        model: len(lane) for model, lane in zip((*MODELS, "adjudicated"), lanes)
    }


def label_stats(cases: list[dict]) -> dict:
    n = len(cases)
    valid = [c for c in cases if not c["label_missing"]]
    a, b = [Counter(c["initial_labels"][i] for c in valid) for i in (0, 1)]
    agrees = sum(c["initial_agreement"] for c in valid)
    expected = (
        sum(a[k] * b[k] for k in ("new", "continue", "uncertain")) / len(valid) ** 2
        if valid
        else None
    )
    kappa = (
        (agrees / len(valid) - expected) / (1 - expected)
        if valid and expected != 1
        else None
    )
    return {
        "selected": n,
        "labels": dict(Counter(c["label"] for c in cases)),
        "new_rate": metrics.ratio(sum(c["label"] == "new" for c in cases), n),
        "continue_rate": metrics.ratio(sum(c["label"] == "continue" for c in cases), n),
        "uncertain_rate": metrics.ratio(
            sum(c["label"] == "uncertain" for c in cases), n
        ),
        "agreement": metrics.ratio(agrees, len(valid)),
        "agreement_all_selected": metrics.ratio(agrees, n),
        "initial_format_failure": sum(c["initial_format_failure"] for c in cases),
        "kappa": kappa,
        "missing_initial": n - len(valid),
        "adjudicated": sum(c["adjudicated"] for c in cases),
        "sessions": len({c["session"] for c in cases}),
    }


def cluster_cases(cases: list[dict]) -> list[dict]:
    return [{**c, "project": c["source"]} for c in cases]


def curve(
    cases: list[dict], records: dict, condition: str, t: float, draws: list[Counter]
) -> dict:
    predictions = {
        c["id"]: analysis.decision(records.get((c["id"], condition)), t) == "continue"
        for c in cases
        if records.get((c["id"], condition), {}).get("status") == "ok"
    }
    clear = [c for c in cases if c["label"] != "uncertain"]
    pairs = [
        (c["label"] == "continue", predictions[c["id"]])
        for c in clear
        if c["id"] in predictions
    ]
    out = metrics.measure(metrics.confusion(pairs))
    out.update(
        condition=condition,
        threshold=t,
        valid_clear=len(pairs),
        missing_clear=len(clear) - len(pairs),
    )
    out["ask_rate"] = metrics.ratio(
        sum(
            analysis.decision(records.get((c["id"], condition)), t) == "ask"
            for c in cases
        ),
        len(cases),
    )
    out["operational_recall"] = metrics.ratio(
        out["tp"], sum(c["label"] == "continue" for c in clear)
    )
    out["cluster"] = metrics.bootstrap(clear, predictions, draws)
    if t == 0.50:
        for name in ("false_join_rate", "recall"):
            r = out[name]
            r["exact_bounds"] = analysis.exact(r["k"], r["n"])
    return out


def paired(cases: list[dict], records: dict, t: float, draws: list[Counter]) -> dict:
    clear = [
        c
        for c in cases
        if c["label"] != "uncertain"
        and all(
            records.get((c["id"], b), {}).get("status") == "ok" for b in ("B1", "B2")
        )
    ]
    a, b = [
        {
            c["id"]: analysis.decision(records[c["id"], name], t) == "continue"
            for c in clear
        }
        for name in ("B1", "B2")
    ]
    measured = [
        metrics.measure(
            metrics.confusion(
                [(c["label"] == "continue", pred[c["id"]]) for c in clear]
            )
        )
        for pred in (a, b)
    ]
    return {
        "threshold": t,
        "n": len(clear),
        "B1_continue_only": sum(a[c["id"]] and not b[c["id"]] for c in clear),
        "B2_continue_only": sum(b[c["id"]] and not a[c["id"]] for c in clear),
        "delta": {
            name: measured[1][name]["value"] - measured[0][name]["value"]
            if all(m[name]["value"] is not None for m in measured)
            else None
            for name in NAMES[:3]
        },
        "cluster": metrics.bootstrap_delta(clear, a, b, draws),
    }


def combined(curves: list[dict], old: dict) -> list[dict]:
    prior = {(r["condition"], r["threshold"]): r for r in old["curves"]}
    output = []
    for row in curves:
        a = prior[row["condition"], row["threshold"]]
        result = metrics.measure([a[k] + row[k] for k in ("tp", "fp", "fn", "tn")])
        result.update(
            scope="combined",
            condition=row["condition"],
            threshold=row["threshold"],
            cluster=None,
        )
        for name in ("ask_rate", "operational_recall"):
            result[name] = metrics.ratio(
                a[name]["k"] + row[name]["k"], a[name]["n"] + row[name]["n"]
            )
        result["valid_clear"] = a["valid_clear"] + row["valid_clear"]
        result["missing_clear"] = a["missing_clear"] + row["missing_clear"]
        output.append(result)
    return output


def distribution_delta(
    cases: list[dict], left: str, right: str, draws: list[Counter]
) -> dict:
    by_session = {}
    for c in cases:
        cell = by_session.setdefault(c["session"], [0, 0, 0, 0])
        key = c["arm"] if left == "enriched" else str(c["prefilter"])
        if key not in (left, right):
            continue
        offset = 0 if key == left else 2
        cell[offset] += c["label"] == "new"
        cell[offset + 1] += 1
    values = []
    for draw in draws:
        v = [sum(by_session[s][i] * n for s, n in draw.items()) for i in range(4)]
        if v[1] and v[3]:
            values.append(v[0] / v[1] - v[2] / v[3])
    total = [sum(c[i] for c in by_session.values()) for i in range(4)]
    return {
        "left": left,
        "right": right,
        "new_rate_delta": total[0] / total[1] - total[2] / total[3]
        if total[1] and total[3]
        else None,
        "ci": metrics.interval(values),
        "valid_draws": len(values),
    }


# cost: io one CSV write; time O(n); vars: n = curve rows; basis: estimate
def save_tables(curves: list[dict]) -> None:
    path = PUBLIC / "results/tables/thresholds.csv"
    path.parent.mkdir(parents=True, exist_ok=True)
    fields = ["scope", "condition", "threshold", "tp", "fp", "fn", "tn"]
    for name in NAMES:
        fields.extend(name + suffix for suffix in ("_k", "_n", "", "_lower", "_upper"))
    with path.open("w", newline="") as out:
        writer = csv.DictWriter(out, fieldnames=fields)
        writer.writeheader()
        for row in curves:
            flat = {k: row[k] for k in fields[:7]}
            for name in NAMES:
                r = row[name]
                flat.update(
                    {
                        name + "_k": r["k"],
                        name + "_n": r["n"],
                        name: r["value"],
                        name + "_lower": r["ci"][0] if r["ci"] else None,
                        name + "_upper": r["ci"][1] if r["ci"] else None,
                    }
                )
            writer.writerow(flat)


# cost: time O(g*t*r*s); vars: g = groups, t = thresholds, r = draws, s = sessions; basis: estimate
def measure_groups(cases: list[dict], records: dict) -> tuple[dict, dict, list, list]:
    groups = {
        "new": cases,
        "random": [c for c in cases if c["arm"] == "random"],
        "enriched": [c for c in cases if c["arm"] == "enriched"],
        "random_pass": [c for c in cases if c["arm"] == "random" and c["prefilter"]],
        "random_fail": [
            c for c in cases if c["arm"] == "random" and not c["prefilter"]
        ],
        "initial_agreement": [
            c for c in cases if c["initial_agreement"] and not c["label_missing"]
        ],
    }
    for source in ("Claude", "Codex"):
        groups[source] = [c for c in cases if c["source"] == source]
    curves, stats, paired_rows = [], {}, []
    for name, group in groups.items():
        stats[name] = label_stats(group)
        if not group:
            continue
        draws = metrics.cluster_counts(cluster_cases(group), SEED)
        for b in ("B1", "B2"):
            for t in metrics.THRESHOLDS:
                curves.append({"scope": name, **curve(group, records, b, t, draws)})
        if name == "new":
            paired_rows = [paired(group, records, t, draws) for t in metrics.THRESHOLDS]
        print(name, len(group), flush=True)
    return groups, stats, curves, paired_rows


def confirm_hypotheses(primary: list[dict]) -> list[dict]:
    confirm = []
    for r in primary:
        if r["threshold"] != 0.50:
            continue
        f, recall = r["false_join_rate"], r["recall"]
        confirm.append(
            {
                "condition": r["condition"],
                "threshold": 0.50,
                "decision": analysis.hypothesis(r),
                "point_pass": f["value"] is not None
                and recall["value"] is not None
                and f["value"] <= 0.05
                and recall["value"] >= 0.60,
                "wilson_upper_pass": f["ci"] is not None and f["ci"][1] <= 0.05,
                "false_join_exact": f["exact_bounds"],
                "recall_exact": recall["exact_bounds"],
                "false_join_cluster": r["cluster"].get("false_join_rate", {}).get("ci"),
                "recall_cluster": r["cluster"].get("recall", {}).get("ci"),
            }
        )
    return confirm


def request_sizes(records_list: list[dict], cases: list[dict]) -> dict:
    sizes = {}
    for b in ("B1", "B2"):
        records_b = [r for r in records_list if r["condition"] == b]
        lengths = [
            r["request_bytes"] for r in records_b if r.get("request_bytes") is not None
        ]
        sizes[b] = {
            "statuses": dict(Counter(r["status"] for r in records_b)),
            "errors": dict(
                Counter(
                    r.get("error", "none") for r in records_b if r["status"] != "ok"
                )
            ),
            "models": sorted({r.get("model") or "missing" for r in records_b}),
            "missing": len(cases) - len(records_b),
            "min": metrics.quantile(lengths, 0),
            "p50": metrics.quantile(lengths, 0.5),
            "p95": metrics.quantile(lengths, 0.95),
            "max": metrics.quantile(lengths, 1),
        }
    return sizes


def label_call_stats() -> dict:
    statuses = Counter()
    models = Counter()
    for path in sorted((PRIVATE / "codex").glob("*/receipt.json")):
        receipt = read(path)
        models[receipt["model"]] += 1
        if receipt.get("returncode") != 0:
            statuses["process_failure"] += 1
            continue
        text = receipt.get("stdout", "")
        try:
            parsed = json.loads(text[text.find("{") : text.rfind("}") + 1])["labels"]
            expected = json.loads(
                (path.parent / "prompt.txt").read_text().split("평가 데이터:\n")[-1]
            )
            status = (
                "ok"
                if [r.get("id") for r in parsed] == [r["id"] for r in expected]
                else "coverage_mismatch"
            )
        except (ValueError, KeyError, TypeError):
            status = "invalid_json"
        statuses[status] += 1
    calls = [r for r in rows(PRIVATE / "calls.jsonl") if r["kind"] == "codex"]
    return {
        "statuses": dict(statuses),
        "models": dict(models),
        "repairs": sum(r["trial_id"].endswith("-repair") for r in calls),
        "reserved_without_receipt": len(calls) - sum(statuses.values()),
    }


# cost: io O(f) private reads and hashes; vars: f = retained files; basis: estimate
def main() -> None:
    all_cases, lanes = labels()
    audited = audit()
    excluded_ids = {c["id"] for c in audited["excluded_cases"]}
    cases = [c for c in all_cases if c["id"] not in excluded_ids]
    source_counts = {
        s: label_stats([c for c in cases if c["source"] == s])
        for s in ("Claude", "Codex")
    }
    records_list = rows(PRIVATE / "jev.jsonl")
    records = {(r["id"], r["condition"]): r for r in records_list}
    groups, stats, curves, paired_rows = measure_groups(cases, records)
    primary = [r for r in curves if r["scope"] == "new"]
    old = read(PUBLIC.parent / "continuation-misjoin/results/summary.json")
    curves.extend(combined(primary, old))
    call_counts = Counter(r["kind"] for r in rows(PRIVATE / "calls.jsonl"))
    confirm = confirm_hypotheses(primary)
    power = calculate()
    new_n = sum(c["label"] == "new" for c in cases)
    power.update(
        observed_new=new_n,
        shortfall=max(0, power["required_new"] - new_n),
        combined_new=new_n + 72,
        combined_shortfall=max(0, power["required_new"] - new_n - 72),
        valid_new_by_condition={
            r["condition"]: r["false_join_rate"]["n"]
            for r in primary
            if r["threshold"] == 0.50
        },
    )
    sizes = request_sizes(records_list, all_cases)
    summary = {
        "run": read(PRIVATE / "run.json"),
        "analysis_config": {
            "seed": SEED,
            "bootstrap_replicates": 2000,
            "wilson_confidence": 0.95,
            "exact_alpha": 0.0125,
            "confirm_threshold": 0.50,
        },
        "selection": read(PRIVATE / "selection.json"),
        "labels": label_stats(cases),
        "all_labels": label_stats(all_cases),
        "excluded_judge_calls": sum(r["id"] in excluded_ids for r in records_list),
        "extraction_audit": {k: v for k, v in audited.items() if k != "excluded_cases"},
        "source_counts": source_counts,
        "lanes": lanes,
        "label_calls": label_call_stats(),
        "groups": stats,
        "power": power,
        "calls": dict(call_counts),
        "limits": {"jev": 4000, "codex": 500},
        "hypotheses": confirm,
        "curves": curves,
        "paired": paired_rows,
        "request_sizes": sizes,
        "distribution": [
            distribution_delta(
                cases,
                "enriched",
                "random",
                metrics.cluster_counts(cluster_cases(cases), SEED),
            ),
            distribution_delta(
                groups["random"],
                "True",
                "False",
                metrics.cluster_counts(cluster_cases(groups["random"]), SEED),
            ),
        ],
        "prior": {
            "selected": 608,
            "new": 72,
            "continue": 515,
            "uncertain": 21,
            "raw_available": False,
            "summary_sha256": sha(
                PUBLIC.parent / "continuation-misjoin/results/summary.json"
            ),
        },
    }
    write(PUBLIC / "results/summary.json", summary)
    save_tables(curves)
    paths = sorted(
        p
        for p in PRIVATE.rglob("*")
        if p.is_file()
        and "runtime" not in p.relative_to(PRIVATE).parts
        and p.name != "budget.lock"
    )
    (PUBLIC / "data/SHA256SUMS").write_text(
        "\n".join(
            f"{sha(p)}  continuation-newtask/{p.relative_to(PRIVATE)}" for p in paths
        )
        + "\n"
    )
    print({"calls": dict(call_counts), "new": new_n, "hypotheses": confirm})


if __name__ == "__main__":
    main()
