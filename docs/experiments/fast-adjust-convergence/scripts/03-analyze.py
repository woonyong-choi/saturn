"""processed/의 표로 가설을 판정하고 results/에 요약, 표, 그림 원본을 쓴다."""

from __future__ import annotations

import argparse
import csv
import json
import logging
import math
import sys
from collections import defaultdict
from pathlib import Path

import numpy as np

logger = logging.getLogger(__name__)

Z95 = 1.959963984540054
FAMILY_ALPHA = 0.05
TARGET_WRONG_RATE = 0.05
EQUIVALENCE = (0.04, 0.06)
WINDOW_RANGE_LIMIT = 0.02
REACH_BAND = 0.01
REACH_LAST_BLOCK = 75
REACH_SHARE = 0.8
LATE_FIRST_BLOCK = 51
BLOCK = 100

BEHAVIOR = ("behavior-3", "behavior-5", "behavior-10")
ASKED = ("asked-3", "asked-5", "asked-10")
SHIFT = "behavior-shift"
LABELS = {
    "behavior-3": "3%",
    "behavior-5": "5%",
    "behavior-10": "10%",
    "asked-3": "3%",
    "asked-5": "5%",
    "asked-10": "10%",
}
SIGNAL_LABELS = {"behavior": "행동 신호", "asked": "물은 판단만"}


def main() -> int:
    logging.basicConfig(level=logging.INFO, format="%(levelname)s %(message)s")
    parser = argparse.ArgumentParser()
    parser.add_argument("--processed-dir", type=Path, required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()

    population = {row["condition"]: row for row in _read(args.processed_dir / "population.csv")}
    replicates = _read(args.processed_dir / "replicates.csv")
    blocks = _read(args.processed_dir / "blocks.csv")
    by_condition: dict[str, list[dict]] = defaultdict(list)
    for row in replicates:
        by_condition[row["condition"]].append(row)

    conditions = {
        condition: _describe_condition(rows, population[condition])
        for condition, rows in sorted(by_condition.items())
    }
    shift = _describe_shift(blocks, conditions["behavior-10"]["late_threshold_mean"]["mean"])
    checks = _checks(conditions, shift)
    hypotheses = _verdicts(checks)
    flow = _flow(by_condition)
    exploratory = {"window_range_by_restart": _window_by_restart(blocks)}

    summary = {
        "conditions": conditions,
        "exploratory": exploratory,
        "shift": shift,
        "checks": checks,
        "hypotheses": hypotheses,
        "flow": flow,
        "settings": {
            "equivalence": list(EQUIVALENCE),
            "family_alpha": FAMILY_ALPHA,
            "late_first_judgment": (LATE_FIRST_BLOCK - 1) * BLOCK + 1,
            "reach_band": REACH_BAND,
            "reach_last_judgment": REACH_LAST_BLOCK * BLOCK,
            "reach_share": REACH_SHARE,
            "target_wrong_rate": TARGET_WRONG_RATE,
            "window_range_limit": WINDOW_RANGE_LIMIT,
        },
    }

    tables = args.out_dir / "tables"
    figures = args.out_dir / "figures"
    tables.mkdir(parents=True, exist_ok=True)
    figures.mkdir(parents=True, exist_ok=True)
    with (args.out_dir / "summary.json").open("w", encoding="utf-8", newline="\n") as file:
        json.dump(summary, file, ensure_ascii=False, indent=2, sort_keys=True)
        file.write("\n")
    _write_checks(tables / "confirmatory.csv", checks)
    _write_conditions(tables / "conditions.csv", conditions)
    _write_wrong_rate_chart(figures, conditions)
    _write_window_chart(figures, conditions)
    _write_shift_chart(figures, shift)
    logger.info("hypotheses: %s", {key: value["verdict"] for key, value in hypotheses.items()})
    return 0


def _describe_condition(rows: list[dict], population: dict) -> dict:
    wrong_rows = [row for row in rows if row["late_wrong_rate"] != ""]
    late_acted = sum(int(row["late_acted"]) for row in wrong_rows)
    late_wrong = sum(int(row["late_acted_wrong"]) for row in wrong_rows)
    frozen = sum(int(row["frozen_final"]) for row in rows)
    wrong_signals = sum(int(row["late_wrong_signals"]) for row in rows)
    missed_signals = sum(int(row["late_missed_signals"]) for row in rows)
    return {
        "n": len(rows),
        "late_wrong_rate": _describe([float(row["late_wrong_rate"]) for row in wrong_rows]),
        "late_wrong_rate_pooled": _proportion(late_wrong, late_acted),
        "late_threshold_mean": _describe([float(row["late_threshold_mean"]) for row in rows]),
        "late_window_range": _describe([float(row["late_window_range"]) for row in rows]),
        "restarts": _describe([float(row["restarts"]) for row in rows]),
        "late_restarts": _describe([float(row["late_restarts"]) for row in rows]),
        "late_frozen_blocks": _describe([float(row["late_frozen_blocks"]) for row in rows]),
        "late_asks_per_judgment": _round(
            sum(int(row["late_asks"]) for row in rows) / (len(rows) * (100 - LATE_FIRST_BLOCK + 1) * BLOCK)
        ),
        "late_signals_per_judgment": _round(
            (wrong_signals + missed_signals) / (len(rows) * (100 - LATE_FIRST_BLOCK + 1) * BLOCK)
        ),
        "late_wrong_signal_share": _proportion(wrong_signals, wrong_signals + missed_signals),
        "frozen_final": _proportion(frozen, len(rows)),
        "population": {
            key: (float(value) if key not in ("condition", "signal_model") else value)
            for key, value in population.items()
        },
    }


def _describe_shift(blocks: list[dict], target: float) -> dict:
    series: dict[int, list[float]] = defaultdict(list)
    reach: dict[int, int | None] = {}
    for row in blocks:
        if row["condition"] != SHIFT:
            continue
        replicate = int(row["replicate"])
        block = int(row["block"])
        mean = float(row["threshold_mean"])
        series[block].append(mean)
        reach.setdefault(replicate, None)
        if block >= LATE_FIRST_BLOCK and reach[replicate] is None and abs(mean - target) <= REACH_BAND:
            reach[replicate] = block
    reached = sum(1 for block in reach.values() if block is not None and block <= REACH_LAST_BLOCK)
    reach_judgments = [
        (block - LATE_FIRST_BLOCK + 1) * BLOCK for block in reach.values() if block is not None
    ]
    return {
        "target_threshold": _round(target),
        "reached_within_limit": _proportion(reached, len(reach)),
        "reached_ever": _proportion(len(reach_judgments), len(reach)),
        "judgments_to_reach": _describe([float(value) for value in reach_judgments]),
        "mean_threshold_by_block": [
            {"block": block, "threshold": _round(float(np.mean(values)))}
            for block, values in sorted(series.items())
        ],
    }


def _window_by_restart(blocks: list[dict]) -> dict:
    """후반 100건 묶음의 기준값 폭을 급변 재시작이 있던 묶음과 없던 묶음으로 나눈다."""
    grouped: dict[str, dict[str, list[float]]] = defaultdict(lambda: defaultdict(list))
    for row in blocks:
        if int(row["block"]) < LATE_FIRST_BLOCK:
            continue
        key = "with_restart" if int(row["restarts"]) > 0 else "without_restart"
        width = float(row["threshold_max"]) - float(row["threshold_min"])
        grouped[row["condition"]][key].append(width)
    return {
        condition: {key: _describe(values) for key, values in sorted(groups.items())}
        for condition, groups in sorted(grouped.items())
    }


def _checks(conditions: dict, shift: dict) -> list[dict]:
    checks = []
    h1 = conditions["behavior-5"]["late_wrong_rate"]
    se = h1["se"]
    p = max(_upper_tail(h1["mean"], EQUIVALENCE[0], se), _lower_tail(h1["mean"], EQUIVALENCE[1], se))
    rejected = h1["ci95"][1] < EQUIVALENCE[0] or h1["ci95"][0] > EQUIVALENCE[1]
    checks.append(_check("H1", "behavior-5", "late_wrong_rate", h1, p, rejected))

    low = conditions["behavior-3"]["late_wrong_rate"]
    p = _upper_tail(low["mean"], 0.03, low["se"])
    checks.append(_check("H2", "behavior-3", "late_wrong_rate", low, p, low["ci95"][1] <= 0.03))
    high = conditions["behavior-10"]["late_wrong_rate"]
    p = _lower_tail(high["mean"], 0.10, high["se"])
    checks.append(_check("H2", "behavior-10", "late_wrong_rate", high, p, high["ci95"][0] >= 0.10))

    for condition in BEHAVIOR:
        window = conditions[condition]["late_window_range"]
        p = _lower_tail(window["mean"], WINDOW_RANGE_LIMIT, window["se"])
        rejected = window["ci95"][0] >= WINDOW_RANGE_LIMIT
        checks.append(_check("H3", condition, "late_window_range", window, p, rejected))

    reached = shift["reached_within_limit"]
    n = reached["n"]
    z = (reached["rate"] - REACH_SHARE) / math.sqrt(REACH_SHARE * (1 - REACH_SHARE) / n)
    p = _phi(-z)
    rejected = reached["wilson95"][1] < REACH_SHARE
    checks.append(
        {
            "hypothesis": "H4",
            "condition": SHIFT,
            "metric": "reached_within_limit",
            "value": reached["rate"],
            "ci95": reached["wilson95"],
            "n": n,
            "p": _significant(p),
            "rejected": rejected,
        }
    )
    _holm(checks)
    for check in checks:
        if check["p_holm"] <= FAMILY_ALPHA:
            check["verdict"] = "채택"
        elif check.pop("rejected"):
            check["verdict"] = "기각"
        else:
            check["verdict"] = "보류"
        check.pop("rejected", None)
    return checks


def _check(hypothesis: str, condition: str, metric: str, stats: dict, p: float, rejected: bool) -> dict:
    return {
        "hypothesis": hypothesis,
        "condition": condition,
        "metric": metric,
        "value": stats["mean"],
        "ci95": stats["ci95"],
        "n": stats["n"],
        "p": _significant(p),
        "rejected": rejected,
    }


def _holm(checks: list[dict]) -> None:
    order = sorted(range(len(checks)), key=lambda index: checks[index]["p"])
    running = 0.0
    total = len(checks)
    for rank, index in enumerate(order):
        adjusted = min(1.0, (total - rank) * checks[index]["p"])
        running = max(running, adjusted)
        checks[index]["p_holm"] = _significant(running)


def _verdicts(checks: list[dict]) -> dict:
    grouped: dict[str, list[str]] = defaultdict(list)
    for check in checks:
        grouped[check["hypothesis"]].append(check["verdict"])
    verdicts = {}
    for hypothesis, values in sorted(grouped.items()):
        if "기각" in values:
            verdict = "기각"
        elif all(value == "채택" for value in values):
            verdict = "채택"
        else:
            verdict = "보류"
        verdicts[hypothesis] = {"verdict": verdict, "conditions": values}
    return verdicts


def _flow(by_condition: dict[str, list[dict]]) -> dict:
    collected = sum(len(rows) for rows in by_condition.values())
    excluded = sum(1 for rows in by_condition.values() for row in rows if row["late_wrong_rate"] == "")
    return {
        "collected_replicates": collected,
        "excluded_no_late_action": excluded,
        "analyzed_replicates": collected - excluded,
    }


def _describe(values: list[float]) -> dict:
    if not values:
        return {"n": 0}
    array = np.asarray(values, dtype=float)
    n = len(array)
    sd = float(array.std(ddof=1)) if n > 1 else 0.0
    se = sd / math.sqrt(n)
    mean = float(array.mean())
    return {
        "n": n,
        "mean": _round(mean),
        "sd": _round(sd),
        "se": _round(se),
        "median": _round(float(np.median(array))),
        "p5": _round(float(np.percentile(array, 5))),
        "p95": _round(float(np.percentile(array, 95))),
        "ci95": [_round(mean - Z95 * se), _round(mean + Z95 * se)],
    }


def _proportion(k: int, n: int) -> dict:
    if n == 0:
        return {"k": 0, "n": 0}
    rate = k / n
    center = (rate + Z95**2 / (2 * n)) / (1 + Z95**2 / n)
    half = Z95 * math.sqrt(rate * (1 - rate) / n + Z95**2 / (4 * n * n)) / (1 + Z95**2 / n)
    return {"k": k, "n": n, "rate": _round(rate), "wilson95": [_round(center - half), _round(center + half)]}


def _upper_tail(mean: float, bound: float, se: float) -> float:
    """평균이 bound보다 크다는 단측 검정의 p값."""
    if se == 0:
        return 0.0 if mean > bound else 1.0
    return _phi(-(mean - bound) / se)


def _lower_tail(mean: float, bound: float, se: float) -> float:
    """평균이 bound보다 작다는 단측 검정의 p값."""
    if se == 0:
        return 0.0 if mean < bound else 1.0
    return _phi((mean - bound) / se)


def _phi(z: float) -> float:
    """표준 정규 누적 분포. 꼬리의 작은 p값을 잃지 않게 erfc로 계산한다."""
    return 0.5 * math.erfc(-z / math.sqrt(2.0))


def _significant(value: float) -> float:
    return float(f"{value:.6g}")


def _round(value: float) -> float:
    return round(value, 6)


def _write_chart(figures: Path, name: str, header: str, rows: list[dict]) -> None:
    """차트 입력 `{name}.muto`와 값 `{name}.json`을 mutoscope용으로 쓴다."""
    (figures / f"{name}.muto").write_text(f'{header}data "{name}.json"\n', encoding="utf-8", newline="\n")
    _write_json(figures / f"{name}.json", rows)


def _grouped_rows(conditions: dict, metric: str, scale: float) -> list[dict]:
    """시작 기준값마다 행동 신호와 물은 판단만 조건의 평균과 95% 신뢰구간을 한 행으로 묶는다."""
    rows = []
    for behavior, asked in zip(BEHAVIOR, ASKED, strict=True):
        row = {"label": f"시작 {LABELS[behavior]}"}
        for key, condition in (("behavior", behavior), ("asked", asked)):
            stats = conditions[condition][metric]
            row[key] = round(stats["mean"] * scale, 6)
            row[f"{key}.low"] = round(stats["ci95"][0] * scale, 6)
            row[f"{key}.high"] = round(stats["ci95"][1] * scale, 6)
        rows.append(row)
    return rows


def _write_wrong_rate_chart(figures: Path, conditions: dict) -> None:
    n = conditions["behavior-5"]["n"]
    header = f"""chart bar
title "후반 5,000건의 행동 중 틀림 비율"
subtitle "조건마다 복제 n={n}. 행은 시작 기준값의 틀림 비율, 점선은 목표 {TARGET_WRONG_RATE * 100:g}%"
x "틀림 비율(%)"
decimals 1

series behavior "{SIGNAL_LABELS['behavior']}" role=main
series asked "{SIGNAL_LABELS['asked']}" role=compare
rule {TARGET_WRONG_RATE * 100:g} "목표"
"""
    _write_chart(figures, "late-wrong-rate", header, _grouped_rows(conditions, "late_wrong_rate", 100))


def _write_window_chart(figures: Path, conditions: dict) -> None:
    n = conditions["behavior-5"]["n"]
    header = f"""chart bar
title "후반 5,000건의 100건 안 기준값 폭"
subtitle "조건마다 복제 n={n}. 행은 시작 기준값의 틀림 비율, 점선은 멈춤 폭 {WINDOW_RANGE_LIMIT:g}"
x "기준값 폭(최대 − 최소, 비율)"
decimals 3

series behavior "{SIGNAL_LABELS['behavior']}" role=main
series asked "{SIGNAL_LABELS['asked']}" role=compare
rule {WINDOW_RANGE_LIMIT:g} "멈춤 폭"
"""
    _write_chart(figures, "window-range", header, _grouped_rows(conditions, "late_window_range", 1))


def _write_shift_chart(figures: Path, shift: dict) -> None:
    n = shift["reached_within_limit"]["n"]
    header = f"""chart line
title "사용 방식 급변 전후의 평균 기준값"
subtitle "복제 n={n}, 5,000건에서 틀림 비율 3%→10%. 점선은 10% 조건 후반 평균"
x "판단 수(건)"
y "100건 평균 기준값(비율)"
zero off

series mean "평균 기준값" role=main
rule {shift["target_threshold"]:g} "목표"
"""
    rows = [{"x": row["block"] * BLOCK, "mean": row["threshold"]} for row in shift["mean_threshold_by_block"]]
    _write_chart(figures, "shift-threshold", header, rows)


def _read(path: Path) -> list[dict]:
    with path.open(encoding="utf-8", newline="") as file:
        return list(csv.DictReader(file))


def _write_checks(path: Path, checks: list[dict]) -> None:
    fields = ("hypothesis", "condition", "metric", "value", "ci95_low", "ci95_high", "n", "p", "p_holm", "verdict")
    with path.open("w", encoding="utf-8", newline="") as file:
        writer = csv.DictWriter(file, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        for check in checks:
            row = {key: check[key] for key in fields if key in check}
            row["ci95_low"], row["ci95_high"] = check["ci95"]
            writer.writerow(row)


def _write_conditions(path: Path, conditions: dict) -> None:
    fields = (
        "condition",
        "n",
        "late_wrong_rate_mean",
        "late_wrong_rate_ci95_low",
        "late_wrong_rate_ci95_high",
        "late_threshold_mean",
        "late_window_range_mean",
        "restarts_mean",
        "frozen_final_rate",
        "oracle_threshold_after",
        "balance_threshold_after",
    )
    with path.open("w", encoding="utf-8", newline="") as file:
        writer = csv.DictWriter(file, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        for condition, stats in conditions.items():
            writer.writerow(
                {
                    "condition": condition,
                    "n": stats["n"],
                    "late_wrong_rate_mean": stats["late_wrong_rate"]["mean"],
                    "late_wrong_rate_ci95_low": stats["late_wrong_rate"]["ci95"][0],
                    "late_wrong_rate_ci95_high": stats["late_wrong_rate"]["ci95"][1],
                    "late_threshold_mean": stats["late_threshold_mean"]["mean"],
                    "late_window_range_mean": stats["late_window_range"]["mean"],
                    "restarts_mean": stats["restarts"]["mean"],
                    "frozen_final_rate": stats["frozen_final"]["rate"],
                    "oracle_threshold_after": stats["population"]["oracle_threshold_after"],
                    "balance_threshold_after": stats["population"]["balance_threshold_after"],
                }
            )


def _write_json(path: Path, value: object) -> None:
    with path.open("w", encoding="utf-8", newline="\n") as file:
        json.dump(value, file, ensure_ascii=False, indent=2)
        file.write("\n")


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception:
        logger.exception("analyze failed")
        sys.exit(1)
