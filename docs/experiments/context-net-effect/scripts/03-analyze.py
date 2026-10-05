"""집계와 사전 지정 판정. 같은 입력이면 같은 바이트를 낸다.

사용: python3 03-analyze.py [--run formal|pilot] [--check]
"""

from __future__ import annotations

import argparse
import json
import math
import random
import sys
from pathlib import Path

import plan

HERE = Path(__file__).resolve().parent
EXP = HERE.parent
TABLES = EXP / "results" / "tables"
RESULTS = EXP / "results"
Z = 1.959963984540054
BOOT_RUNS = 5000
BOOT_SEED = 7007
PAIRS = [("rrf", "none"), ("rrf", "provider"), ("jev", "none"), ("jev", "provider"), ("jev", "rrf")]
CANDIDATES = ("rrf", "jev")
COMPARATORS = ("none", "provider")


def wilson(k: int, n: int) -> tuple[float, float]:
    if n == 0:
        return (0.0, 1.0)
    p = k / n
    d = 1 + Z * Z / n
    c = (p + Z * Z / (2 * n)) / d
    h = Z * math.sqrt(p * (1 - p) / n + Z * Z / (4 * n * n)) / d
    return max(0.0, c - h), min(1.0, c + h)


def newcombe_paired(a: int, b: int, c: int, d: int) -> tuple[float, float, float]:
    """대응 비율 차이 (X-Y)의 Newcombe 방법 10 구간. a 둘 다 성공, b X만, c Y만, d 둘 다 실패."""
    n = a + b + c + d
    if n == 0:
        return 0.0, -1.0, 1.0
    p1, p2 = (a + b) / n, (a + c) / n
    l1, u1 = wilson(a + b, n)
    l2, u2 = wilson(a + c, n)
    big = (a + b) * (c + d) * (a + c) * (b + d)
    if big == 0:
        phi = 0.0
    else:
        cc = a * d - b * c
        dd = cc - n / 2 if cc > n / 2 else cc + n / 2 if cc < -n / 2 else 0.0
        phi = dd / math.sqrt(big)
    diff = p1 - p2
    lo = diff - math.sqrt(max((p1 - l1) ** 2 + (u2 - p2) ** 2 - 2 * phi * (p1 - l1) * (u2 - p2), 0))
    hi = diff + math.sqrt(max((u1 - p1) ** 2 + (p2 - l2) ** 2 - 2 * phi * (u1 - p1) * (p2 - l2), 0))
    return diff, lo, hi


def boot_mean_diff(pairs: list[tuple[float, float]], seed_label: str) -> tuple[float, float, float]:
    """(X, Y) 값 쌍의 평균 차이와 시드 단위 복원 추출 percentile 95% 구간."""
    if not pairs:
        return 0.0, 0.0, 0.0
    diffs = [x - y for x, y in pairs]
    rng = random.Random(f"{BOOT_SEED}-{seed_label}")
    means = []
    for _ in range(BOOT_RUNS):
        means.append(sum(diffs[rng.randrange(len(diffs))] for _ in diffs) / len(diffs))
    means.sort()
    return sum(diffs) / len(diffs), means[int(0.025 * BOOT_RUNS)], means[int(0.975 * BOOT_RUNS) - 1]


def r(x: float | None, digits: int = 4) -> float | None:
    return None if x is None else round(x, digits)


def analyze(rows: list[dict]) -> dict:
    out: dict = {"plan_version": plan.PLAN_VERSION, "providers": {}}
    for provider in sorted({x["provider"] for x in rows}):
        prows = [x for x in rows if x["provider"] == provider]
        seeds = sorted({x["seed"] for x in prows})
        by = {(x["seed"], x["arm"]): x for x in prows}
        block: dict = {"seeds": len(seeds), "arms": {}, "pairs": {}}
        for arm in plan.ARMS:
            arows = [by[(s, arm)] for s in seeds if (s, arm) in by]
            usable = [x for x in arows if x.get("usable")]
            entry = {"trials": len(arows), "usable": len(usable), "unusable": len(arows) - len(usable)}
            for key in ("f1", "f2", "f3", "composite"):
                k = sum(x[key] for x in usable)
                lo, hi = wilson(k, len(arows))  # 분모는 모든 trial. 못 쓴 trial은 실패로 센다
                entry[key] = {"k": k, "n": len(arows), "rate": r(k / len(arows) if arows else None), "wilson": [r(lo), r(hi)]}
            for key in ("provider_cost", "router_tokens", "total_cost", "window_seconds", "tool_calls", "metrics_reruns", "incident_reruns", "cache_write_tokens", "cache_read_tokens", "input_tokens", "output_tokens"):
                vals = [x[key] for x in usable]
                entry[key] = {"mean": r(sum(vals) / len(vals), 2) if vals else None, "sum": sum(vals)}
            if arm in CANDIDATES:
                entry["restarts"] = sum(x["restarts"] for x in usable)
                entry["packet_tokens_mean"] = r(sum(x["packet_tokens"] or 0 for x in usable) / max(len(usable), 1), 1)
                entry["incident_forms"] = {f: sum(x["incident_form"] == f for x in usable) for f in sorted({x["incident_form"] for x in usable})}
                entry["metrics_forms"] = {f: sum(x["metrics_form"] == f for x in usable) for f in sorted({x["metrics_form"] for x in usable})}
            if arm == "jev":
                n = len(usable)
                entry["applied_rate"] = {"k": sum(x["jev_applied"] for x in usable), "n": n}
                entry["compact_ok"] = {"k": sum(x["compact_ok"] for x in usable), "n": sum(x["compact_judgments"] for x in usable)}
            block["arms"][arm] = entry
        for x_arm, y_arm in PAIRS:
            both = [s for s in seeds if (s, x_arm) in by and (s, y_arm) in by]
            pair: dict = {"projects": len(both)}
            for key in ("f1", "f2", "f3", "composite"):
                a = b = c = d = 0
                for s in both:
                    xv = by[(s, x_arm)].get(key, 0) if by[(s, x_arm)].get("usable") else 0
                    yv = by[(s, y_arm)].get(key, 0) if by[(s, y_arm)].get("usable") else 0
                    if xv and yv:
                        a += 1
                    elif xv:
                        b += 1
                    elif yv:
                        c += 1
                    else:
                        d += 1
                diff, lo, hi = newcombe_paired(a, b, c, d)
                pair[key] = {"both": a, "x_only": b, "y_only": c, "neither": d, "diff": r(diff), "ci95": [r(lo), r(hi)]}
            for key in ("total_cost", "provider_cost", "provider_cost_out4", "provider_cost_out8", "window_seconds", "cache_write_tokens"):
                vals = [(by[(s, x_arm)][key], by[(s, y_arm)][key]) for s in both if by[(s, x_arm)].get("usable") and by[(s, y_arm)].get("usable")]
                mean, lo, hi = boot_mean_diff(vals, f"{provider}-{x_arm}-{y_arm}-{key}")
                pair[key] = {"mean_diff": r(mean, 1), "ci95": [r(lo, 1), r(hi, 1)], "pairs": len(vals)}
            block["pairs"][f"{x_arm}-vs-{y_arm}"] = pair
        out["providers"][provider] = block
    out["verdicts"] = verdicts(out)
    return out


def verdicts(result: dict) -> dict:
    """design.md 판정 절의 규칙. 품질 허용폭 0, 비용과 시간은 구간 상한이 0 아래."""
    consistency = RESULTS / "compact-consistency.json"
    consistent = json.loads(consistency.read_text())["meets_design_threshold"] if consistency.exists() else None
    out = {}
    for provider, block in result["providers"].items():
        pairs = block["pairs"]
        for x_arm in CANDIDATES:
            vs_provider = pairs.get(f"{x_arm}-vs-provider")
            if not vs_provider:
                continue
            vs_none = pairs.get(f"{x_arm}-vs-none")
            h1 = vs_provider["composite"]["ci95"][0] >= 0
            h3 = True if vs_none is None else vs_none["composite"]["ci95"][0] >= 0
            h2 = vs_provider["total_cost"]["ci95"][1] < 0 and vs_provider["window_seconds"]["ci95"][1] < 0
            checks = {"H1_quality_vs_provider": h1, "H3_quality_vs_none": h3, "H2_cost_time_vs_provider": h2}
            if x_arm == "jev":
                vs_rrf = pairs["jev-vs-rrf"]
                checks["H4_vs_rrf"] = vs_rrf["composite"]["ci95"][0] >= 0 and vs_rrf["total_cost"]["ci95"][1] < 0
                checks["H5_compact_consistency"] = consistent
                applied = block["arms"]["jev"]["applied_rate"]
                checks["H6_applied_rate"] = wilson(applied["k"], applied["n"])[0] >= 0.90
            if all(v is True for v in checks.values()):
                verdict = "채택"
            elif vs_provider["composite"]["ci95"][1] < 0 or vs_provider["total_cost"]["ci95"][0] > 0:
                verdict = "기각"
            else:
                verdict = "보류"
            out[f"{provider}:{x_arm}"] = {"checks": checks, "verdict": verdict}
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--run", default="formal")
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()
    rows = json.loads((TABLES / f"{args.run}-trials.json").read_text())
    text = json.dumps(analyze(rows), ensure_ascii=False, sort_keys=True, indent=1) + "\n"
    target = RESULTS / f"{args.run}-summary.json"
    if args.check:
        same = target.exists() and target.read_text() == text
        print("analyze byte-identical" if same else "analyze MISMATCH")
        return 0 if same else 1
    target.write_text(text)
    print(f"summary -> {target}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
