"""채널 순위를 k마다 RRF로 합쳐 상위 N의 recall과 Wilson 구간을 계산한다."""
import csv
import json
import math
import random
import sys
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
PROCESSED = HERE / "data" / "processed"
RESULTS = HERE / "results"
K_VALUES = [10, 30, 60, 100]
N_VALUES = [5, 10, 20, 40]
CURVE_N = range(1, 61)
H1 = {"k": 60, "top_n": 10, "lower_bound": 0.90}
SELECT_LOWER = 0.95
SEED = 116
BOOTSTRAP = 2000
Z = 1.959964


def wilson(hits, n):
    if n == 0:
        return None, None
    p = hits / n
    center = (p + Z * Z / (2 * n)) / (1 + Z * Z / n)
    half = Z * math.sqrt(p * (1 - p) / n + Z * Z / (4 * n * n)) / (1 + Z * Z / n)
    return round(center - half, 4), round(center + half, 4)


CHANNELS = ("rank_file", "rank_word", "rank_recent")


def rrf_ranks(rows, k, channels=CHANNELS):
    """채널 RRF 점수 내림차순, 같으면 기록 번호가 큰 순."""
    def score(r):
        return sum(1 / (k + int(r[c])) for c in channels if r[c])
    ordered = sorted(rows, key=lambda r: (-score(r), -int(r["record_no"])))
    return {r["candidate_id"]: i for i, r in enumerate(ordered, start=1)}


def covered_counts(sets, k, n, channels=CHANNELS):
    """세트마다 (덮은 kept 수, kept 수)."""
    counts = {}
    for key, rows in sets.items():
        ranks = rrf_ranks(rows, k, channels)
        kept = [r for r in rows if r["kept"] == "1"]
        counts[key] = (sum(ranks[r["candidate_id"]] <= n for r in kept), len(kept))
    return counts


def cell(counts):
    hits = sum(c for c, _ in counts.values())
    total = sum(t for _, t in counts.values())
    low, high = wilson(hits, total)
    return {"covered": hits, "kept": total, "recall": round(hits / total, 4) if total else None,
            "ci95": [low, high]}


def session_bootstrap(sets, counts):
    by_session = defaultdict(list)
    for key, rows in sets.items():
        by_session[rows[0]["session_id"]].append(counts[key])
    sessions = sorted(by_session)
    rng = random.Random(SEED)
    values = []
    for _ in range(BOOTSTRAP):
        sample = [c for s in rng.choices(sessions, k=len(sessions)) for c in by_session[s]]
        total = sum(t for _, t in sample)
        if total:
            values.append(sum(c for c, _ in sample) / total)
    values.sort()
    return [round(values[int(0.025 * len(values))], 4), round(values[int(0.975 * len(values)) - 1], 4)]


def select(grid):
    passing = [g for g in grid if g["ci95"][0] is not None and g["ci95"][0] >= SELECT_LOWER]
    if not passing:
        return None
    best = min(passing, key=lambda g: (g["top_n"], -g["recall"], abs(g["k"] - 60)))
    return {"k": best["k"], "top_n": best["top_n"]}


def read_csv(name):
    with (PROCESSED / name).open(encoding="utf-8", newline="") as f:
        return list(csv.DictReader(f))


def main():
    rows = read_csv("channel-ranks.csv")
    if not rows:
        print("03-analyze: 분석할 세트가 없다", file=sys.stderr)
        return 2
    sets = defaultdict(list)
    for r in rows:
        sets[(r["run_id"], r["trial_id"])].append(r)
    grid, curve = [], []
    for k in K_VALUES:
        for n in N_VALUES:
            grid.append({"k": k, "top_n": n, **cell(covered_counts(sets, k, n))})
        for n in CURVE_N:
            curve.append({"k": k, "top_n": n, "recall": cell(covered_counts(sets, k, n))["recall"]})
    h1_counts = covered_counts(sets, H1["k"], H1["top_n"])
    h1 = {**H1, **cell(h1_counts)}
    low, high = h1["ci95"]
    h1["verdict"] = "채택" if low >= H1["lower_bound"] else "기각" if high < H1["lower_bound"] else "보류"
    replicas = read_csv("replicas.csv")
    agree = sum(r["kept_first"] == r["kept_second"] for r in replicas)
    channel_missing = {c: round(sum(not r[c] for r in rows) / len(rows), 4)
                       for c in ("rank_file", "rank_word")}
    per_set = sorted(round(c / t, 4) for c, t in h1_counts.values() if t)
    summary = {
        "flow": {r["stage"]: int(r["sets"]) for r in read_csv("flow.csv")},
        "candidates": len(rows),
        "sessions": len({r["session_id"] for r in rows}),
        "h1": h1,
        "grid": grid,
        "selected": select(grid),
        "exploratory": {
            "h1_session_bootstrap_ci95": session_bootstrap(sets, h1_counts),
            "h1_per_set_recall_median": per_set[len(per_set) // 2] if per_set else None,
            "replica_agreement": {"agree": agree, "n": len(replicas),
                                  "ci95": list(wilson(agree, len(replicas)))},
            "channel_missing_ratio": channel_missing,
            "single_channel_recall": {c: cell(covered_counts(sets, H1["k"], H1["top_n"], (c,)))
                                      for c in CHANNELS},
        },
    }
    (RESULTS / "tables").mkdir(parents=True, exist_ok=True)
    (RESULTS / "figures").mkdir(parents=True, exist_ok=True)
    (RESULTS / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True) + "\n")
    with (RESULTS / "tables" / "coverage.csv").open("w", encoding="utf-8", newline="") as out:
        writer = csv.writer(out, lineterminator="\n")
        writer.writerow(["k", "top_n", "covered", "kept", "recall", "ci95_low", "ci95_high"])
        for g in grid:
            writer.writerow([g["k"], g["top_n"], g["covered"], g["kept"], g["recall"], *g["ci95"]])
    figure = {
        "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
        "title": {"text": "RRF 상위 N이 judge가 남긴 항목을 덮는 비율", "subtitle": "k별, N=1~60"},
        "data": {"values": curve},
        "mark": {"type": "line", "point": False},
        "encoding": {
            "x": {"field": "top_n", "type": "quantitative", "title": "N"},
            "y": {"field": "recall", "type": "quantitative", "title": "recall", "axis": {"format": "%"}},
            "color": {"field": "k", "type": "nominal", "title": "k"},
        },
    }
    (RESULTS / "figures" / "coverage.vl.json").write_text(json.dumps(figure, ensure_ascii=False, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
