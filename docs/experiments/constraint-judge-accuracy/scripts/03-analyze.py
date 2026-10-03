"""data/processed/ 표로 가설 판정, 탐색 표, 그림 원본을 results/에 만든다.

사용: python3 scripts/03-analyze.py (실험 폴더에서 실행)
같은 입력이면 같은 바이트를 낸다.
"""
import csv
import json
import math
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PROCESSED = ROOT / "data" / "processed"
RESULTS = ROOT / "results"
Z95 = 1.959963984540054
ALPHA = 0.05
SWEEP = [round(0.5 + 0.05 * step, 2) for step in range(9)]


def percent(value: float) -> float:
    """비율을 백분율로 바꾼다. 이진 부동소수점 잔차는 버린다."""
    return round(value * 100, 8)


def write_chart(name: str, header: str, rows: list[dict]) -> None:
    """차트 입력 `{name}.muto`와 값 `{name}.json`을 mutoscope용으로 쓴다."""
    figures = RESULTS / "figures"
    figures.mkdir(parents=True, exist_ok=True)
    (figures / f"{name}.muto").write_text(f'{header}data "{name}.json"\n', encoding="utf-8")
    (figures / f"{name}.json").write_text(json.dumps(rows, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def write_hypothesis_chart(hypotheses: list[dict]) -> None:
    """가설마다 비율과 95% Wilson 신뢰구간을 막대로 쓴다. 기준은 행 이름과 점선에 적는다."""
    n_text = ", ".join(f"{h['hypothesis']} {h['n']}" for h in hypotheses)
    criteria = sorted({percent(h["criterion"]) for h in hypotheses})
    rules = "".join(f'rule {criterion:g} "기준 {criterion:g}%"\n' for criterion in criteria)
    header = f"""chart bar
title "가설별 비율"
subtitle "n={n_text}. 오차 막대는 95% Wilson 신뢰구간, 점선은 가설별 채택 기준"
x "비율(%)"
decimals 1

series proportion "측정 비율" role=main
{rules}"""
    rows = [{"label": f"{h['hypothesis']} (기준 {percent(h['criterion']):g}%)", "proportion": percent(h["value"]),
             "proportion.low": percent(h["ci95_low"]), "proportion.high": percent(h["ci95_high"])}
            for h in hypotheses]
    write_chart("hypotheses", header, rows)


def read_csv(path: Path) -> list[dict]:
    with path.open(encoding="utf-8", newline="") as file:
        return list(csv.DictReader(file))


def wilson(k: int, n: int) -> tuple[float | None, float | None]:
    if n == 0:
        return None, None
    p = k / n
    denominator = 1 + Z95**2 / n
    center = (p + Z95**2 / (2 * n)) / denominator
    half = Z95 * math.sqrt(p * (1 - p) / n + Z95**2 / (4 * n**2)) / denominator
    return center - half, center + half


def score_test_p(k: int, n: int, p0: float) -> float | None:
    """H0: 비율 ≤ p0 의 단측 점수 검정. Wilson 구간과 같은 통계량이다."""
    if n == 0:
        return None
    z = (k / n - p0) / math.sqrt(p0 * (1 - p0) / n)
    return 0.5 * math.erfc(z / math.sqrt(2))


def holm(p_values: dict[str, float | None]) -> dict[str, float | None]:
    known = sorted((p, name) for name, p in p_values.items() if p is not None)
    adjusted: dict[str, float | None] = {name: None for name in p_values}
    running = 0.0
    for rank, (p, name) in enumerate(known):
        running = max(running, min(1.0, (len(known) - rank) * p))
        adjusted[name] = running
    return adjusted


def r(value: float | None) -> float | None:
    return None if value is None else round(value, 6)


def answer(row: dict) -> float | None:
    return float(row["answer"]) if row["answer"] else None


def percentile(values: list[float], q: float) -> float:
    """선형 보간 분위수. statistics.quantiles의 inclusive 방식과 같다."""
    ordered = sorted(values)
    position = (len(ordered) - 1) * q
    lower = math.floor(position)
    upper = min(lower + 1, len(ordered) - 1)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def latency_summary(rows: list[dict]) -> dict:
    values = [float(x["latency_ms"]) for x in rows if x.get("latency_ms")]
    if not values:
        return {"n": 0}
    mean = sum(values) / len(values)
    sd = math.sqrt(sum((v - mean) ** 2 for v in values) / (len(values) - 1)) if len(values) > 1 else 0.0
    return {
        "n": len(values), "mean": r(mean), "sd": r(sd), "median": r(percentile(values, 0.5)),
        "p5": r(percentile(values, 0.05)), "p95": r(percentile(values, 0.95)), "max": r(max(values)),
    }


def main() -> int:
    if not (PROCESSED / "inputs.csv").exists():
        print("data/processed/가 없다: ./run.sh process를 먼저 실행한다", file=sys.stderr)
        return 2
    inputs = read_csv(PROCESSED / "inputs.csv")
    pairs = read_csv(PROCESSED / "pairs.csv")
    positive = lambda row: row["predicted"] == "1"
    labeled = lambda row: row["label"] == "1"

    counts = {
        "H1": (sum(positive(x) and labeled(x) for x in inputs), sum(positive(x) for x in inputs)),
        "H2": (sum(positive(x) and labeled(x) for x in inputs), sum(labeled(x) for x in inputs)),
        "H3": (
            sum(x["band"] == "replace" and x["label"] == "replaces" for x in pairs),
            sum(x["band"] == "replace" for x in pairs),
        ),
        "H4": (
            sum(x["band"] == "none" and x["label"] == "compatible" for x in pairs),
            sum(x["band"] == "none" for x in pairs),
        ),
        "H5": (
            sum(x["indirect"] == "1" and positive(x) == labeled(x) for x in inputs),
            sum(x["indirect"] == "1" for x in inputs),
        ),
        "H6": (
            sum(x["category"] == "indirect-replace" and x["band"] == "replace" for x in pairs),
            sum(x["category"] == "indirect-replace" for x in pairs),
        ),
    }
    criteria = {"H1": 0.80, "H2": 0.80, "H3": 0.90, "H4": 0.80, "H5": 0.80, "H6": 0.70}
    metrics = {
        "H1": "is_constraint 정밀도",
        "H2": "is_constraint 재현율",
        "H3": "대체 구간 정밀도",
        "H4": "기록 없음 구간 정확도",
        "H5": "간접 지시 입력 is_constraint 정확도",
        "H6": "간접 지시 대체 쌍 재현율",
    }
    raw_p = {name: score_test_p(k, n, criteria[name]) for name, (k, n) in counts.items()}
    adjusted = holm(raw_p)

    hypotheses = []
    for name, (k, n) in counts.items():
        low, high = wilson(k, n)
        if adjusted[name] is not None and adjusted[name] < ALPHA and low > criteria[name]:
            verdict = "채택"
        elif high is not None and high < criteria[name]:
            verdict = "기각"
        else:
            verdict = "보류"
        hypotheses.append({
            "hypothesis": name, "metric": metrics[name], "k": k, "n": n,
            "value": r(k / n) if n else None, "ci95_low": r(low), "ci95_high": r(high),
            "criterion": criteria[name], "p_one_sided": r(raw_p[name]), "p_holm": r(adjusted[name]),
            "verdict": verdict,
        })

    sweep = []
    for threshold in SWEEP:
        predicted = [x for x in inputs if answer(x) is not None and answer(x) >= threshold]
        tp = sum(labeled(x) for x in predicted)
        sweep.append({
            "threshold": threshold, "tp": tp, "predicted": len(predicted),
            "positives": sum(labeled(x) for x in inputs),
            "precision": r(tp / len(predicted)) if predicted else None,
            "recall": r(tp / sum(labeled(x) for x in inputs)),
        })

    bands = []
    for band_name, correct_label in (("replace", "replaces"), ("possible", "partial"), ("none", "compatible")):
        members = [x for x in pairs if x["band"] == band_name]
        k = sum(x["label"] == correct_label for x in members)
        low, high = wilson(k, len(members))
        bands.append({
            "band": band_name, "n": len(members), "correct": k,
            "accuracy": r(k / len(members)) if members else None, "ci95_low": r(low), "ci95_high": r(high),
            "replaces": sum(x["label"] == "replaces" for x in members),
            "partial": sum(x["label"] == "partial" for x in members),
            "compatible": sum(x["label"] == "compatible" for x in members),
        })

    categories = []
    for name, rows, correct in (
        ("inputs", inputs, lambda x: positive(x) == labeled(x)),
        ("pairs", pairs, lambda x: x["band"] == {"replaces": "replace", "partial": "possible", "compatible": "none"}[x["label"]]),
    ):
        for category in sorted({x["category"] for x in rows}):
            members = [x for x in rows if x["category"] == category]
            k = sum(correct(x) for x in members)
            low, high = wilson(k, len(members))
            categories.append({
                "set": name, "category": category, "n": len(members), "correct": k,
                "accuracy": r(k / len(members)), "ci95_low": r(low), "ci95_high": r(high),
            })

    statuses = {}
    for name, rows in (("inputs", inputs), ("pairs", pairs)):
        statuses[name] = {status: sum(x["status"] == status for x in rows) for status in sorted({x["status"] for x in rows})}

    category_bands = []
    for category in sorted({x["category"] for x in pairs}):
        members = [x for x in pairs if x["category"] == category]
        category_bands.append({
            "category": category, "n": len(members),
            "replace": sum(x["band"] == "replace" for x in members),
            "possible": sum(x["band"] == "possible" for x in members),
            "none": sum(x["band"] == "none" for x in members),
        })

    latency = {"all": latency_summary(inputs + pairs), "inputs": latency_summary(inputs), "pairs": latency_summary(pairs)}

    summary = {
        "hypotheses": hypotheses,
        "is_constraint_sweep": sweep,
        "replaces_bands": bands,
        "categories": categories,
        "pair_category_bands": category_bands,
        "statuses": statuses,
        "latency_ms": latency,
    }
    (RESULTS / "tables").mkdir(parents=True, exist_ok=True)
    (RESULTS / "figures").mkdir(parents=True, exist_ok=True)
    (RESULTS / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    for name, rows in (("hypotheses", hypotheses), ("is-constraint-sweep", sweep), ("replaces-bands", bands), ("categories", categories), ("pair-category-bands", category_bands)):
        with (RESULTS / "tables" / f"{name}.csv").open("w", encoding="utf-8", newline="") as file:
            writer = csv.DictWriter(file, fieldnames=list(rows[0]), lineterminator="\r\n")
            writer.writeheader()
            writer.writerows([{key: "" if value is None else value for key, value in row.items()} for row in rows])

    write_hypothesis_chart(hypotheses)
    return 0


if __name__ == "__main__":
    sys.exit(main())
