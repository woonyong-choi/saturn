"""기존 B와 새 응답을 대응시켜 곡선과 불확실성을 재생성한다."""

from __future__ import annotations

import csv
import math
from collections import Counter

from support import (
    BASE,
    CONDITIONS,
    PRIVATE,
    PUBLIC,
    SEED,
    load,
    read,
    rows,
    sha,
    write,
)

metrics = load("baseline_metrics", "metrics.py")


def cdf(k: int, n: int, p: float) -> float:
    if k < 0:
        return 0.0
    if k >= n or p == 0:
        return 1.0
    if p == 1:
        return 0.0
    return min(
        1.0,
        sum(
            math.exp(
                math.lgamma(n + 1)
                - math.lgamma(i + 1)
                - math.lgamma(n - i + 1)
                + i * math.log(p)
                + (n - i) * math.log1p(-p)
            )
            for i in range(k + 1)
        ),
    )


def exact(k: int, n: int, alpha: float = 0.0125) -> list[float] | None:
    if not n:
        return None
    bounds = []
    for lower in (True, False):
        if (lower and k == 0) or (not lower and k == n):
            bounds.append(0.0 if lower else 1.0)
            continue
        lo, hi = 0.0, 1.0
        for _ in range(55):
            mid = (lo + hi) / 2
            if lower:
                if cdf(n - k, n, 1 - mid) < alpha:
                    lo = mid
                else:
                    hi = mid
            elif cdf(k, n, mid) > alpha:
                lo = mid
            else:
                hi = mid
        bounds.append((lo + hi) / 2)
    return bounds


def decision(row: dict | None, t: float, *, solo: bool = False) -> str:
    if not row or row["status"] != "ok":
        return "ask"
    p = row["answers"]["keep_current"]
    q = row["answers"].get("is_new_task") if not solo else None
    if p >= t and (q is None or q < round(1 - t, 2)):
        return "continue"
    if p < 0.30 and (q is None or q >= 0.70):
        return "new"
    return "ask"


def curve(
    cases: list[dict],
    records: dict,
    condition: str,
    t: float,
    draws: list[Counter],
    *,
    solo: bool = False,
) -> dict:
    valid = {
        c["id"]: decision(records.get((c["id"], condition, 1)), t, solo=solo)
        == "continue"
        for c in cases
        if records.get((c["id"], condition, 1), {}).get("status") == "ok"
    }
    clear = [c for c in cases if c["label"] != "uncertain"]
    pairs = [
        (c["label"] == "continue", valid[c["id"]]) for c in clear if c["id"] in valid
    ]
    result = metrics.measure(metrics.confusion(pairs))
    result.update(
        condition=condition,
        threshold=t,
        valid_clear=len(pairs),
        missing_clear=len(clear) - len(pairs),
        ask_rate=metrics.ratio(
            sum(
                decision(records.get((c["id"], condition, 1)), t, solo=solo) == "ask"
                for c in cases
            ),
            len(cases),
        ),
    )
    result["operational_recall"] = metrics.ratio(
        result["tp"], sum(c["label"] == "continue" for c in clear)
    )
    for name in ("false_join_rate", "recall", "precision"):
        ratio = result[name]
        ratio["exact_bounds"] = exact(ratio["k"], ratio["n"])
    result["cluster"] = metrics.bootstrap(clear, valid, draws)
    return result


def extension_labels() -> tuple[list[dict], dict]:
    cases = read(PRIVATE / "sample.json")
    models = ("gpt-6-astra", "gpt-5.6-luna", "adjudicated")
    lanes = [{r["id"]: r for r in read(PRIVATE / f"labels-{m}.json")} for m in models]
    output, agreements = [], 0
    for c in cases:
        a, b = [
            lane.get(c["id"], {"label": "uncertain", "reason": "missing"})
            for lane in lanes[:2]
        ]
        agree = a["label"] == b["label"]
        agreements += agree
        final = (
            a
            if agree and a["label"] != "uncertain"
            else lanes[2].get(c["id"], {"label": "uncertain"})
        )
        output.append({**c, "label": final["label"], "initial_agreement": agree})
    return output, {
        "selected": len(cases),
        "initial_agreements": agreements,
        "adjudicated": len(lanes[2]),
        "final": dict(Counter(c["label"] for c in output)),
    }


def paired(
    cases: list[dict], records: dict, condition: str, t: float, draws: list[Counter]
) -> dict:
    clear = [
        c
        for c in cases
        if c["label"] != "uncertain"
        and all(
            records.get((c["id"], b, 1), {}).get("status") == "ok"
            for b in ("B0", condition)
        )
    ]
    a, b = [
        {c["id"]: decision(records[c["id"], name, 1], t) == "continue" for c in clear}
        for name in ("B0", condition)
    ]
    measures = [
        metrics.measure(
            metrics.confusion(
                [(c["label"] == "continue", pred[c["id"]]) for c in clear]
            )
        )
        for pred in (a, b)
    ]
    base_only = sum(
        a[c["id"]] == (c["label"] == "continue")
        and b[c["id"]] != (c["label"] == "continue")
        for c in clear
    )
    condition_only = sum(
        b[c["id"]] == (c["label"] == "continue")
        and a[c["id"]] != (c["label"] == "continue")
        for c in clear
    )
    return {
        "condition": condition,
        "threshold": t,
        "n": len(clear),
        "base_correct_only": base_only,
        "condition_correct_only": condition_only,
        "delta": {
            name: measures[1][name]["value"] - measures[0][name]["value"]
            if all(m[name]["value"] is not None for m in measures)
            else None
            for name in ("false_join_rate", "recall", "precision")
        },
        "cluster": metrics.bootstrap_delta(clear, a, b, draws),
    }


def hypothesis(row: dict) -> str:
    f, r = row["false_join_rate"]["exact_bounds"], row["recall"]["exact_bounds"]
    fc, rc = [
        row["cluster"].get(name, {}).get("ci") for name in ("false_join_rate", "recall")
    ]
    if any(v is None for v in (f, r, fc, rc)):
        return "보류"
    if max(f[1], fc[1]) <= 0.05 and min(r[0], rc[0]) >= 0.60:
        return "채택"
    if min(f[0], fc[0]) > 0.05 or max(r[1], rc[1]) < 0.60:
        return "기각"
    return "보류"


def request_and_repeat_stats(
    cases: list[dict], records: dict, original: list[dict], current: list[dict]
) -> tuple[dict, list[dict]]:
    conditions = ("B0", *CONDITIONS)
    sizes = {}
    repeats = []
    for b in conditions:
        receipts = (
            original if b == "B0" else [r for r in current if r["condition"] == b]
        )
        values = [
            r["request_bytes"] for r in receipts if r.get("request_bytes") is not None
        ]
        sizes[b] = {
            **{
                name: metrics.quantile(values, p)
                for name, p in (
                    ("min", 0),
                    ("p50", 0.5),
                    ("p90", 0.9),
                    ("p95", 0.95),
                    ("p99", 0.99),
                    ("max", 1),
                )
            },
            "statuses": dict(Counter(r["status"] for r in receipts)),
            "http_statuses": dict(Counter(str(r.get("http_status")) for r in receipts)),
            "models": sorted({r.get("model") or "missing" for r in receipts}),
            "n": len(receipts),
        }
        for t in metrics.THRESHOLDS:
            complete = [
                c
                for c in cases
                if all(
                    records.get((c["id"], b, rep), {}).get("status") == "ok"
                    for rep in (1, 2)
                )
            ]
            agrees = sum(
                (decision(records[c["id"], b, 1], t) == "continue")
                == (decision(records[c["id"], b, 2], t) == "continue")
                for c in complete
            )
            repeats.append(
                {
                    "condition": b,
                    "threshold": t,
                    "agreement": metrics.ratio(agrees, len(complete)),
                    "incomplete": len(cases) - len(complete),
                }
            )
    return sizes, repeats


def save_summary(summary: dict) -> None:
    write(PUBLIC / "results/summary.json", summary)
    fields = [
        "condition",
        "threshold",
        "tp",
        "fp",
        "fn",
        "tn",
        "false_join_rate",
        "recall",
        "precision",
        "ask_rate",
        "operational_recall",
    ]
    with (PUBLIC / "results/tables/thresholds.csv").open("w", newline="") as out:
        writer = csv.DictWriter(out, fieldnames=fields)
        writer.writeheader()
        for r in summary["curves"]:
            writer.writerow(
                {k: r[k]["value"] if isinstance(r[k], dict) else r[k] for k in fields}
            )
    files = [
        p
        for p in PRIVATE.rglob("*")
        if p.is_file()
        and "runtime" not in p.relative_to(PRIVATE).parts
        and p.name != "budget.lock"
    ]
    manifest = [
        f"{sha(p)}  continuation-misjoin/{p.relative_to(PRIVATE)}"
        for p in sorted(files)
    ]
    manifest.extend(
        f"{v}  continuation-ko/{k}"
        for k, v in sorted(summary["run"]["base_hashes"].items())
    )
    (PUBLIC / "data/SHA256SUMS").write_text("\n".join(manifest) + "\n")
    print(
        {
            "hypotheses": summary["hypotheses"],
            "candidate": summary["candidate"],
            "calls": summary["calls"],
        }
    )


def study_metadata(cases: list[dict], extra_stats: dict, original: list[dict]) -> dict:
    calls = Counter(r["kind"] for r in rows(PRIVATE / "calls.jsonl"))
    return {
        "run": read(PRIVATE / "run.json"),
        "flow": {
            "base_selected": len(cases),
            "base_labels": dict(Counter(c["label"] for c in cases)),
            "base_projects": len({c["project"] for c in cases}),
            "base_sessions": len({c["session"] for c in cases}),
            "extension": extra_stats,
            "extension_eligible": read(PRIVATE / "extension-census.json")[
                "eligible_extra"
            ],
            "extraction_counts": read(PRIVATE / "extension-census.json")["counts"],
        },
        "calls": {
            "jev": calls["jev"],
            "codex": calls["codex"],
            "reused_B0_first_two": len(original),
            "limits": {"jev": 4000, "codex": 300},
        },
        "power": {
            "alpha": 0.0125,
            "null_rate": 0.05,
            "alternative_rate": 0.02,
            "required_new": 363,
            "critical_k": 9,
            "power": cdf(9, 363, 0.02),
            "available_base_new": 72,
            "shortfall": 291,
            "required_extra_turns_at_old_prevalence": 2458,
        },
        "extraction_audit": read(PRIVATE / "extraction-audit.json"),
    }


def main() -> None:
    cases = read(BASE / "final-labels.json")
    original = [
        r
        for r in rows(BASE / "jev.jsonl")
        if r["condition"] == "B" and r["repeat"] <= 2
    ]
    current = rows(PRIVATE / "jev.jsonl")
    records = {(r["id"], "B0", r["repeat"]): r for r in original}
    records.update({(r["id"], r["condition"], r["repeat"]): r for r in current})
    draws = metrics.cluster_counts(cases, SEED)
    conditions = ("B0", *CONDITIONS)
    curves = [
        curve(cases, records, b, t, draws)
        for b in conditions
        for t in metrics.THRESHOLDS
    ]
    candidates = [
        r
        for r in curves
        if r["false_join_rate"]["value"] is not None
        and r["false_join_rate"]["value"] <= 0.05
        and r["recall"]["value"] >= 0.60
    ]
    candidates.sort(
        key=lambda r: (
            -r["recall"]["value"],
            r["false_join_rate"]["value"],
            r["ask_rate"]["value"],
            r["condition"],
            r["threshold"],
        )
    )
    extra, extra_stats = extension_labels()
    extra_draws = metrics.cluster_counts(extra, SEED) if extra else []
    sizes, repeats = request_and_repeat_stats(cases, records, original, current)
    agreed = [c for c in cases if c["initial_agreement"] and c["label"] != "uncertain"]
    agreed_draws = metrics.cluster_counts(agreed, SEED)
    summary = {
        **study_metadata(cases, extra_stats, original),
        "curves": curves,
        "paired": [
            paired(cases, records, b, t, draws)
            for b in CONDITIONS
            for t in metrics.THRESHOLDS
        ],
        "hypotheses": {
            b: hypothesis(
                next(r for r in curves if r["condition"] == b and r["threshold"] == 0.8)
            )
            for b in ("B1", "B2")
        },
        "candidate": {
            "condition": candidates[0]["condition"],
            "threshold": candidates[0]["threshold"],
        }
        if candidates
        else None,
        "point_candidates": [
            {"condition": r["condition"], "threshold": r["threshold"]}
            for r in candidates
        ],
        "request_sizes": sizes,
        "repeat_agreement": repeats,
        "initial_agreement_sensitivity": [
            curve(agreed, records, b, t, agreed_draws)
            for b in conditions
            for t in metrics.THRESHOLDS
        ],
        "B2_keep_only": [
            curve(cases, records, "B2", t, draws, solo=True) for t in metrics.THRESHOLDS
        ],
        "extension_curves": [
            curve(extra, records, b, t, extra_draws)
            for b in CONDITIONS
            for t in metrics.THRESHOLDS
        ]
        if extra
        else [],
    }
    save_summary(summary)


if __name__ == "__main__":
    main()
