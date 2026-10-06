"""원자료를 재집계하며 실패·결측·진단 조건을 분리한다."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import importlib.util
import json
import random
import sys
from pathlib import Path

import collect

EXP = collect.EXP
BASE = collect.BASE
sys.path.insert(0, str(BASE))


def load_module(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, BASE / filename)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PROCESS = load_module("previous_process", "02-process.py")
STATS = load_module("previous_stats", "03-analyze.py")
FIELDS = ("input_tokens", "cache_read_tokens", "cache_write_tokens", "output_tokens")


def trial_row(raw: dict) -> dict:
    store = raw.get("store", {})
    identity = {k: raw[k] for k in ("trial", "provider", "seed", "arm", "complete")}
    if not store.get("runs_meta"):
        return dict(identity, usable=False, composite=0, missing_usage=True)
    row = PROCESS.process_trial(raw)
    if not row.get("usable"):
        return dict(row, composite=0, missing_usage=True)
    turns = raw["turns"]
    boundary = next(t for t in turns if t["label"] == "boundary")
    boundary_input = sorted(store["inputs"], key=lambda r: r["id"])[
        turns.index(boundary)
    ]["id"]
    b_runs = [r for r in store["runs_meta"] if r["input_id"] == boundary_input]
    cutoff = max(r["ended_at"] or 0 for r in b_runs)
    win_runs = {r["id"] for r in store["runs_meta"] if r["started_at"] >= cutoff - 1}
    window_rows = [r for r in store["usage"] if r["run_id"] in win_runs]
    missing = any(r.get(k) is None for r in window_rows for k in FIELDS)
    executed = {r["id"] for r in store["runs_meta"] if r["id"] in win_runs}
    missing |= bool(executed - {r["run_id"] for r in window_rows})
    judgments = [j for j in store["judgments"] if j["started_at"] >= cutoff - 1]
    missing |= any(
        j.get("input_tokens") is None or j.get("output_tokens") is None
        for j in judgments
    )
    row["missing_usage"] = bool(missing)
    row["window_seconds"] = round(raw["ended_unix"] - cutoff / 1000, 3)
    row["raw_token_total"] = sum(row[k] for k in FIELDS) + row["router_tokens"]
    row["actual_models"] = sorted(
        {r["model"] for r in store["usage"] if r.get("model")}
    )
    row["lookup_count"] = len(store["evidence_lookups"])
    row["lookup_results"] = store["evidence_lookups"]
    contexts = []
    for event in store["events"]:
        body = json.loads(event["body"])
        if "ContextSize" in body and event["run_id"] in {r["id"] for r in b_runs}:
            contexts.append(body["ContextSize"])
    row["boundary_context"] = contexts
    row["intervention_observed"] = raw["arm"] == "provider" or row["restarts"] > 0
    row["composite_unadjusted"] = row["composite"]
    if not row["intervention_observed"] or not raw["complete"]:
        row["composite"] = 0
    if missing:
        row["total_cost"] = None
        row["raw_token_total"] = None
    row["diagnostic_only"] = raw["arm"] == "rescue"
    row["engines_remaining"] = raw.get("engines_remaining", [])
    return row


def bootstrap(differences: list[float]) -> dict:
    if not differences:
        return {"n": 0, "mean": None, "ci95": [None, None]}
    rng = random.Random(58701)
    n = len(differences)
    samples = sorted(
        sum(rng.choice(differences) for _ in range(n)) / n for _ in range(10000)
    )
    return {"n": n, "mean": sum(differences) / n, "ci95": [samples[249], samples[9749]]}


def comparison(rows: list[dict], x: str, y: str) -> dict:
    by = {(r["seed"], r["arm"]): r for r in rows}
    counts = [0, 0, 0, 0]
    metrics = {k: [] for k in ("total_cost", "raw_token_total", "window_seconds")}
    for seed in collect.SEEDS:
        left, right = by.get((seed, x)), by.get((seed, y))
        a, b = (
            bool(left and left.get("composite")),
            bool(right and right.get("composite")),
        )
        counts[0 if a and b else 1 if a else 2 if b else 3] += 1
        for key, values in metrics.items():
            if (
                left
                and right
                and left.get(key) is not None
                and right.get(key) is not None
            ):
                values.append(left[key] - right[key])
    quality = STATS.newcombe_paired(*counts)
    out = {
        "x": x,
        "y": y,
        "quality_counts": counts,
        "quality_diff": quality[0],
        "quality_ci95": list(quality[1:]),
        "metrics": {k: bootstrap(v) for k, v in metrics.items()},
    }
    cost = out["metrics"]["total_cost"]
    out["decision"] = "보류"
    if quality[2] < 0 or (cost["n"] == len(collect.SEEDS) and cost["ci95"][0] > 0):
        out["decision"] = "기각"
    if sum((seed, x) in by and (seed, y) in by for seed in collect.SEEDS) != len(
        collect.SEEDS
    ):
        out["decision"] = "보류"
    if x == "rescue":
        out["decision"] = "진단 전용"
    return out


def summarize(rows: list[dict]) -> dict:
    out = {
        "planned": 40,
        "collected": len(rows),
        "providers": {},
        "product_adoption": False,
    }
    for provider in collect.PROVIDERS:
        selected = [r for r in rows if r["provider"] == provider]
        arms = {}
        for arm in collect.ARMS:
            group = [r for r in selected if r["arm"] == arm]
            k = sum(r.get("composite", 0) for r in group)
            entry = {
                "collected": len(group),
                "success": k,
                "planned": len(collect.SEEDS),
                "wilson95": list(STATS.wilson(k, len(collect.SEEDS))),
                "incomplete": sum(not r["complete"] for r in group),
                "missing_usage": sum(r["missing_usage"] for r in group),
                "no_intervention": sum(
                    not r.get("intervention_observed", False) for r in group
                ),
            }
            for field in (
                "total_cost",
                "raw_token_total",
                "window_seconds",
                "lookup_count",
                *FIELDS,
                "router_tokens",
            ):
                values = [r[field] for r in group if r.get(field) is not None]
                entry[field] = {
                    "n": len(values),
                    "sum": sum(values),
                    "mean": sum(values) / len(values) if values else None,
                }
            arms[arm] = entry
        pairs = [
            ("rrf_lookup", "provider"),
            ("jev_lookup", "provider"),
            ("rrf_lookup", "rrf"),
            ("jev_lookup", "rrf_lookup"),
            ("rescue", "rrf"),
        ]
        out["providers"][provider] = {
            "arms": arms,
            "comparisons": [comparison(selected, x, y) for x, y in pairs],
        }
    return out


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    files = sorted((EXP / "data/raw").glob("*.json.gz"))
    records = [json.load(gzip.open(f, "rt")) for f in files]
    formal = [r for r in records if r["seed"] in collect.SEEDS]
    identities = [r["trial"] for r in formal]
    if len(identities) != len(set(identities)):
        raise ValueError("duplicate trials")
    rows = [trial_row(r) for r in formal]
    results = {"trials.json": rows, "summary.json": summarize(rows)}
    hashes = "".join(
        hashlib.sha256(f.read_bytes()).hexdigest() + "  raw/" + f.name + "\n"
        for f in files
    )
    targets = {
        EXP / "results" / name: json.dumps(
            body, ensure_ascii=False, sort_keys=True, indent=2
        )
        + "\n"
        for name, body in results.items()
    }
    targets[EXP / "data/SHA256SUMS"] = hashes
    for path, text in targets.items():
        if args.verify:
            if not path.exists() or path.read_text() != text:
                raise ValueError("hash or analysis mismatch: " + path.name)
        else:
            path.write_text(text)
    print(
        "verified" if args.verify else "analyzed",
        len(rows),
        "formal trials; pilot",
        len(records) - len(formal),
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
