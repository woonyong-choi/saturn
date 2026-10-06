"""원자료를 재집계하며 실패·결측·진단 조건을 분리한다."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import importlib.util
import json
import math
import random
from types import ModuleType
import sys

import collect

EXP = collect.EXP
BASE = collect.BASE
sys.path.insert(0, str(BASE))


def load_module(name: str, filename: str) -> ModuleType:
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
    inputs = sorted(store["inputs"], key=lambda r: r["id"])
    expected_lines = [
        line.strip()
        for turn in raw["turns"]
        for line in turn["text"].splitlines()
        if line.strip()
    ]
    row["submitted_turns"] = len(raw["turns"])
    row["stored_inputs"] = len(inputs)
    row["input_texts_match"] = [i["text"] for i in inputs] == expected_lines
    row["protocol_valid"] = (
        len(inputs) == len(raw["turns"]) and row["input_texts_match"]
    )
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
    compact_inputs = {i["id"] for i in inputs if i["text"] == "/compact"}
    compact_runs = {
        r["id"] for r in store["runs_meta"] if r["input_id"] in compact_inputs
    }
    compact_usage = PROCESS.window_usage(store, compact_runs)
    compact_reported = sum(sum(v.values()) for v in compact_usage.values())
    row["native_compaction_reported_tokens"] = (
        compact_reported if compact_runs else None
    )
    row["native_compaction_usage_unresolved"] = (
        bool(compact_runs) and compact_reported == 0
    )
    missing |= row["native_compaction_usage_unresolved"]
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
    # 줄 단위 CLI 접수 때문에 위치 대응이 달라진 진단도 실제 질문 ID로 재채점한다.
    f2_turn = next((t for t in turns if t["label"] == "f2"), None)
    f2_ids = {i["id"] for i in inputs if f2_turn and i["text"] == f2_turn["text"]}
    f2_runs = {r["id"] for r in store["runs_meta"] if r["input_id"] in f2_ids}
    f2_text = "".join(
        e["body"]["Text"]["text"]
        for e in PROCESS.event_bodies(store)
        if e["run_id"] in f2_runs and "Text" in e["body"]
    )
    row["f2"] = int(raw["facts"]["ticket"] in f2_text)
    row["composite"] = int(row["f1"] and row["f2"] and row["f3"])
    row["composite_unadjusted"] = row["composite"]
    if not row["intervention_observed"] or not raw["complete"]:
        row["composite"] = 0
    row["reported_weighted_cost"] = row["total_cost"]
    row["reported_raw_token_total"] = row["raw_token_total"]
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


def binomial_interval(k: int, n: int, tail: float = 0.0125) -> tuple[float, float]:
    """두 불일치 비율의 네 꼬리에 Bonferroni를 적용할 정확 구간이다."""

    def cdf(p: float, stop: int) -> float:
        return sum(math.comb(n, i) * p**i * (1 - p) ** (n - i) for i in range(stop + 1))

    def solve(stop: int, target: float) -> float:
        lo, hi = 0.0, 1.0
        for _ in range(70):
            mid = (lo + hi) / 2
            if cdf(mid, stop) > target:
                lo = mid
            else:
                hi = mid
        return (lo + hi) / 2

    return (
        0.0 if k == 0 else solve(k - 1, 1 - tail),
        1.0 if k == n else solve(k, tail),
    )


def paired_interval(counts: list[int]) -> tuple[float, float, float]:
    n = sum(counts)
    if n == 0:
        return 0.0, -1.0, 1.0
    b, c = counts[1:3]
    bl, bu = binomial_interval(b, n)
    cl, cu = binomial_interval(c, n)
    return (b - c) / n, bl - cu, bu - cl


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
    quality = paired_interval(counts)
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
        out["decision"] = "실행 규약 위반: 진단 비교 불가"
    return out


def summarize(rows: list[dict], followup: bool = False) -> dict:
    out = {
        "planned": 16 if followup else 40,
        "collected": len(rows),
        "providers": {},
        "product_adoption": False,
    }
    for provider in collect.PROVIDERS:
        selected = [r for r in rows if r["provider"] == provider]
        arms = {}
        for arm in ("rrf_lookup", "jev_lookup") if followup else collect.ARMS:
            group = [r for r in selected if r["arm"] == arm]
            k = sum(r.get("composite", 0) for r in group)
            entry = {
                "collected": len(group),
                "success": k,
                "f1_success": sum(r.get("f1", 0) for r in group),
                "f2_success": sum(r.get("f2", 0) for r in group),
                "f3_success": sum(r.get("f3", 0) for r in group),
                "header_success": sum(r.get("header_ok", 0) for r in group),
                "planned": len(collect.SEEDS),
                "wilson95": list(STATS.wilson(k, len(collect.SEEDS))),
                "incomplete": sum(not r["complete"] for r in group),
                "protocol_invalid": sum(
                    not r.get("protocol_valid", False) for r in group
                ),
                "native_compaction_usage_unresolved": sum(
                    r.get("native_compaction_usage_unresolved", False) for r in group
                ),
                "missing_usage": sum(r["missing_usage"] for r in group),
                "no_intervention": sum(
                    not r.get("intervention_observed", False) for r in group
                ),
            }
            for field in (
                "total_cost",
                "reported_weighted_cost",
                "reported_raw_token_total",
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
        if followup:
            pairs = [("jev_lookup", "rrf_lookup")]
        out["providers"][provider] = {
            "arms": arms,
            "comparisons": [comparison(selected, x, y) for x, y in pairs],
        }
    return out


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--verify", action="store_true")
    parser.add_argument("--followup", action="store_true")
    args = parser.parse_args()
    folder = "followup" if args.followup else "raw"
    prefix = "followup-" if args.followup else ""
    files = sorted((EXP / "data" / folder).glob("*.json.gz"))
    records = [json.load(gzip.open(f, "rt")) for f in files]
    formal = [r for r in records if r["seed"] in collect.SEEDS]
    identities = [r["trial"] for r in formal]
    if len(identities) != len(set(identities)):
        raise ValueError("duplicate trials")
    if args.verify:
        expected = {
            f"{prefix}{p}-{seed}-{arm}"
            for p in collect.PROVIDERS
            for seed in collect.SEEDS
            for arm in (("rrf_lookup", "jev_lookup") if args.followup else collect.ARMS)
        }
        if set(identities) != expected:
            raise ValueError("formal collection incomplete or unexpected trial present")
    rows = [trial_row(r) for r in formal]
    summary = summarize(rows, args.followup)
    statuses = {}
    for raw in formal:
        for turn in raw["turns"]:
            statuses[turn["status"]] = statuses.get(turn["status"], 0) + 1
    requests = [
        (raw, judgment)
        for raw in formal
        for judgment in raw.get("store", {}).get("judgments", [])
        if "compact" in judgment["question_sets"] and judgment.get("sent")
    ]
    summary["audit"] = {
        "source_commits": sorted({r["source_commit"] for r in formal}),
        "started_unix": min((r["started_unix"] for r in formal), default=None),
        "ended_unix": max((r["ended_unix"] for r in formal), default=None),
        "submitted_turns": sum(len(r["turns"]) for r in formal),
        "stored_inputs": sum(len(r.get("store", {}).get("inputs", [])) for r in formal),
        "planned_input_limit": 240 if args.followup else 640,
        "turn_status_counts": statuses,
        "protocol_invalid": [
            r["trial"] for r in rows if not r.get("protocol_valid", False)
        ],
        "compact_requests": len(requests),
        "compact_requests_without_corrected_header": sum(
            raw["facts"]["header_new"] not in judgment["sent"]
            for raw, judgment in requests
        ),
        "f3_retries_disagreement": [
            r["trial"]
            for r in formal
            if r["grades"].get("f3") and not r["grades"]["f3"]["retries_ok"]
        ],
        "nonliteral_grade_values": [
            r["trial"]
            for r in formal
            if any(
                value is None
                for g in r["grades"].values()
                for value in g.get("values", {}).values()
            )
        ],
        "engines_remaining": [r["trial"] for r in formal if r.get("engines_remaining")],
    }
    results = {prefix + "trials.json": rows, prefix + "summary.json": summary}
    hashes = "".join(
        hashlib.sha256(f.read_bytes()).hexdigest() + "  " + folder + "/" + f.name + "\n"
        for f in files
    )
    targets = {
        EXP / "results" / name: json.dumps(
            body, ensure_ascii=False, sort_keys=True, indent=2
        )
        + "\n"
        for name, body in results.items()
    }
    targets[EXP / "data" / (prefix + "SHA256SUMS")] = hashes
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
