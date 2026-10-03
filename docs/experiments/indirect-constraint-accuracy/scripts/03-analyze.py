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
CONDITIONS = ("baseline", "flag", "flag_text", "oracle_flag", "flag_hint")
PAIR_EXPECTED = {"replaces": "replace", "partial": "possible", "compatible": "none"}


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


def mcnemar_p(b: int, c: int) -> float | None:
    """H0: 후보가 맞고 현행이 틀린 수 b가 반대 c보다 많지 않다. b+c < 25면 정확 이항, 아니면 McNemar 정규 근사."""
    total = b + c
    if total == 0:
        return None
    if total < 25:
        return sum(math.comb(total, i) for i in range(b, total + 1)) / 2**total
    return 0.5 * math.erfc(((b - c) / math.sqrt(total)) / math.sqrt(2))


def newcombe_paired(n11: int, b: int, c: int, n00: int) -> tuple[float, float, float]:
    """짝지은 두 비율 차이(후보 - 현행)와 Newcombe 방법 10의 95% 신뢰구간."""
    n = n11 + b + c + n00
    p1, p2 = (n11 + b) / n, (n11 + c) / n
    l1, u1 = wilson(n11 + b, n)
    l2, u2 = wilson(n11 + c, n)
    margins = (n11 + b) * (c + n00) * (n11 + c) * (b + n00)
    phi = (n11 * n00 - b * c) / math.sqrt(margins) if margins else 0.0
    diff = p1 - p2
    low = diff - math.sqrt((p1 - l1) ** 2 - 2 * phi * (p1 - l1) * (u2 - p2) + (u2 - p2) ** 2)
    high = diff + math.sqrt((u1 - p1) ** 2 - 2 * phi * (u1 - p1) * (p2 - l2) + (p2 - l2) ** 2)
    return diff, low, high


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


def percentile(values: list[float], q: float) -> float:
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


def write_table(name: str, rows: list[dict]) -> None:
    with (RESULTS / "tables" / f"{name}.csv").open("w", encoding="utf-8", newline="") as file:
        writer = csv.DictWriter(file, fieldnames=list(rows[0]), lineterminator="\r\n")
        writer.writeheader()
        writer.writerows([{key: "" if value is None else value for key, value in row.items()} for row in rows])


def correct(row: dict) -> bool:
    return (row["predicted"] == "1") == (row["label"] == "1")


def paired(inputs: list[dict], candidate: str, reference: str, subset) -> dict:
    """같은 입력의 두 조건 정오를 2x2로 센다. b는 후보만 맞은 수, c는 현행만 맞은 수다."""
    by_item: dict[str, dict[str, bool]] = {}
    for row in inputs:
        if subset(row) and row["condition"] in (candidate, reference):
            by_item.setdefault(row["item_id"], {})[row["condition"]] = correct(row)
    pairs = [(v[candidate], v[reference]) for v in by_item.values() if candidate in v and reference in v]
    n11 = sum(a and b for a, b in pairs)
    b = sum(a and not o for a, o in pairs)
    c = sum(o and not a for a, o in pairs)
    n00 = sum(not a and not o for a, o in pairs)
    diff, low, high = newcombe_paired(n11, b, c, n00) if pairs else (None, None, None)
    return {
        "candidate": candidate, "reference": reference, "n": len(pairs), "both_correct": n11, "b": b, "c": c,
        "both_wrong": n00, "diff": r(diff), "ci95_low": r(low), "ci95_high": r(high), "p_one_sided": r(mcnemar_p(b, c)),
    }


def main() -> int:
    if not (PROCESSED / "inputs.csv").exists():
        print("data/processed/가 없다: ./run.sh process를 먼저 실행한다", file=sys.stderr)
        return 2
    inputs = read_csv(PROCESSED / "inputs.csv")
    pairs = read_csv(PROCESSED / "pairs.csv")
    registrations = read_csv(PROCESSED / "registrations.csv")
    indirect = lambda row: row["indirect"] == "1"
    direct = lambda row: row["indirect"] == "0"

    def accuracy_counts(condition: str, subset) -> tuple[int, int]:
        rows = [x for x in inputs if x["condition"] == condition and subset(x)]
        return sum(correct(x) for x in rows), len(rows)

    counts = {
        "H1": accuracy_counts("baseline", indirect),
        "H2": (
            sum(x["band"] == "replace" and x["label"] == "replaces" for x in pairs),
            sum(x["band"] == "replace" for x in pairs),
        ),
        "H3": (
            sum(x["band"] == "none" and x["label"] == "compatible" for x in pairs),
            sum(x["band"] == "none" for x in pairs),
        ),
        "H4": (
            sum(x["category"] == "indirect-replace" and x["band"] == "replace" for x in pairs),
            sum(x["category"] == "indirect-replace" for x in pairs),
        ),
        "H7": accuracy_counts("flag", direct),
        "H8": accuracy_counts("flag_text", direct),
    }
    criteria = {"H1": 0.80, "H2": 0.90, "H3": 0.80, "H4": 0.70, "H7": 0.90, "H8": 0.90}
    metrics = {
        "H1": "현행 간접 지시 입력 정확도",
        "H2": "대체 구간 정밀도",
        "H3": "기록 없음 구간 정확도",
        "H4": "간접 지시 대체 쌍 재현율",
        "H7": "flag 간접 아닌 입력 정확도",
        "H8": "flag_text 간접 아닌 입력 정확도",
    }
    comparisons = {
        "H5": paired(inputs, "flag", "baseline", indirect),
        "H6": paired(inputs, "flag_text", "baseline", indirect),
    }
    raw_p = {name: score_test_p(k, n, criteria[name]) for name, (k, n) in counts.items()}
    raw_p.update({name: row["p_one_sided"] for name, row in comparisons.items()})
    adjusted = holm(raw_p)

    hypotheses = []
    for name in ("H1", "H2", "H3", "H4", "H5", "H6", "H7", "H8"):
        if name in counts:
            k, n = counts[name]
            low, high = wilson(k, n)
            ok = adjusted[name] is not None and adjusted[name] < ALPHA and low > criteria[name]
            rejected = high is not None and high < criteria[name]
            row = {"hypothesis": name, "metric": metrics[name], "k": k, "n": n, "value": r(k / n) if n else None,
                   "ci95_low": r(low), "ci95_high": r(high), "criterion": criteria[name]}
        else:
            c = comparisons[name]
            ok = adjusted[name] is not None and adjusted[name] < ALPHA and c["ci95_low"] > 0
            rejected = c["ci95_high"] is not None and c["ci95_high"] <= 0
            row = {"hypothesis": name,
                   "metric": f"{c['candidate']} 정확도 − baseline 정확도(간접 지시 입력)", "k": c["b"], "n": c["n"],
                   "value": c["diff"], "ci95_low": c["ci95_low"], "ci95_high": c["ci95_high"], "criterion": 0.0}
        row.update({"p_one_sided": r(raw_p[name]), "p_holm": r(adjusted[name]),
                    "verdict": "채택" if ok else "기각" if rejected else "보류"})
        hypotheses.append(row)
    verdicts = {h["hypothesis"]: h["verdict"] for h in hypotheses}

    # 조건별 정확도
    arms = []
    for condition in CONDITIONS:
        for scope, subset in (("indirect", indirect), ("direct", direct)):
            k, n = accuracy_counts(condition, subset)
            low, high = wilson(k, n)
            arms.append({"condition": condition, "scope": scope, "n": n, "correct": k, "accuracy": r(k / n),
                         "ci95_low": r(low), "ci95_high": r(high)})
    categories = []
    for condition in CONDITIONS:
        for category in sorted({x["category"] for x in inputs}):
            rows = [x for x in inputs if x["condition"] == condition and x["category"] == category]
            k = sum(correct(x) for x in rows)
            low, high = wilson(k, len(rows))
            categories.append({"condition": condition, "category": category, "n": len(rows), "correct": k,
                               "accuracy": r(k / len(rows)), "ci95_low": r(low), "ci95_high": r(high)})
    # 정밀도와 재현율(전체 350건)
    precision_recall = []
    for condition in CONDITIONS:
        rows = [x for x in inputs if x["condition"] == condition]
        predicted = [x for x in rows if x["predicted"] == "1"]
        positives = [x for x in rows if x["label"] == "1"]
        tp = sum(x["label"] == "1" for x in predicted)
        precision_recall.append({"condition": condition, "tp": tp, "predicted": len(predicted), "positives": len(positives),
                                 "precision": r(tp / len(predicted)) if predicted else None, "recall": r(tp / len(positives))})
    # 같은 입력 쌍 비교(탐색 포함)
    paired_rows = []
    for candidate in ("flag", "flag_text", "oracle_flag", "flag_hint"):
        for scope, subset in (("indirect", indirect), ("direct", direct)):
            paired_rows.append({"scope": scope, **paired(inputs, candidate, "baseline", subset)})
    # 등록 판정 정확도
    reg_k = sum((x["registered"] == "1") == (x["label"] == "1") for x in registrations)
    reg_low, reg_high = wilson(reg_k, len(registrations))
    registration = {
        "n": len(registrations), "correct": reg_k, "accuracy": r(reg_k / len(registrations)),
        "ci95_low": r(reg_low), "ci95_high": r(reg_high),
        "false_registered": sum(x["registered"] == "1" and x["label"] == "0" for x in registrations),
        "missed": sum(x["registered"] == "0" and x["label"] == "1" for x in registrations),
        "statuses": {s: sum(x["status"] == s for x in registrations) for s in sorted({x["status"] for x in registrations})},
    }
    # 쌍 구간
    bands = []
    for band_name, correct_label in (("replace", "replaces"), ("possible", "partial"), ("none", "compatible")):
        members = [x for x in pairs if x["band"] == band_name]
        k = sum(x["label"] == correct_label for x in members)
        low, high = wilson(k, len(members))
        bands.append({"band": band_name, "n": len(members), "correct": k, "accuracy": r(k / len(members)) if members else None,
                      "ci95_low": r(low), "ci95_high": r(high),
                      "replaces": sum(x["label"] == "replaces" for x in members),
                      "partial": sum(x["label"] == "partial" for x in members),
                      "compatible": sum(x["label"] == "compatible" for x in members)})
    pair_categories = []
    for category in sorted({x["category"] for x in pairs}):
        members = [x for x in pairs if x["category"] == category]
        k = sum(x["band"] == PAIR_EXPECTED[x["label"]] for x in members)
        low, high = wilson(k, len(members))
        pair_categories.append({
            "category": category, "n": len(members), "correct": k, "accuracy": r(k / len(members)),
            "ci95_low": r(low), "ci95_high": r(high),
            "replace": sum(x["band"] == "replace" for x in members),
            "possible": sum(x["band"] == "possible" for x in members),
            "none": sum(x["band"] == "none" for x in members)})
    # 121 항목과 새 항목의 기준 조건 정확도(항목 출처별)
    origins = []
    for origin in ("121", "new"):
        k, n = accuracy_counts("baseline", lambda x, o=origin: indirect(x) and x["origin"] == o)
        low, high = wilson(k, n)
        origins.append({"origin": origin, "n": n, "correct": k, "accuracy": r(k / n), "ci95_low": r(low), "ci95_high": r(high)})

    statuses = {}
    for name, rows in (("inputs", inputs), ("pairs", pairs)):
        statuses[name] = {s: sum(x["status"] == s for x in rows) for s in sorted({x["status"] for x in rows})}
    requests = len(inputs) + len(pairs) + len(registrations)

    # 채택 규칙: 후보의 개선 가설과 간접 아닌 입력 가설이 모두 채택이고 간접 지시 정확도 하한이 80%를 넘는다.
    candidates = []
    for name, condition, improve, harm in (("flag", "flag", "H5", "H7"), ("flag_text", "flag_text", "H6", "H8")):
        arm = next(a for a in arms if a["condition"] == condition and a["scope"] == "indirect")
        passed = verdicts[improve] == "채택" and verdicts[harm] == "채택" and arm["ci95_low"] > 0.8
        candidates.append({"candidate": name, "improve": improve, "harm": harm, "indirect_accuracy": arm["accuracy"],
                           "indirect_ci95_low": arm["ci95_low"], "passed": passed})
    passing = [c for c in candidates if c["passed"]]
    chosen = max(passing, key=lambda c: (c["indirect_accuracy"], c["candidate"] == "flag"))["candidate"] if passing else None

    summary = {
        "hypotheses": hypotheses,
        "adoption": {"candidates": candidates, "chosen": chosen},
        "arms": arms,
        "categories": categories,
        "precision_recall": precision_recall,
        "paired": paired_rows,
        "registration": registration,
        "replaces_bands": bands,
        "pair_categories": pair_categories,
        "baseline_indirect_by_origin": origins,
        "statuses": statuses,
        "judge_requests": requests,
        "latency_ms": {
            "inputs": latency_summary(inputs), "pairs": latency_summary(pairs),
        },
    }
    (RESULTS / "tables").mkdir(parents=True, exist_ok=True)
    (RESULTS / "figures").mkdir(parents=True, exist_ok=True)
    (RESULTS / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    for name, rows in (("hypotheses", hypotheses), ("arms", arms), ("categories", categories),
                       ("precision-recall", precision_recall), ("paired", paired_rows), ("replaces-bands", bands),
                       ("pair-categories", pair_categories)):
        write_table(name, rows)

    write_hypothesis_chart([h for h in hypotheses if h["hypothesis"] in counts])
    write_chart(
        "arm-accuracy",
        """chart bar
title "간접 지시 입력의 조건별 정확도"
subtitle "n=200. 오차 막대는 95% Wilson 신뢰구간"
x "정확도(%)"
decimals 1

series accuracy "정확도" role=main
""",
        [{"label": a["condition"], "accuracy": percent(a["accuracy"]), "accuracy.low": percent(a["ci95_low"]),
          "accuracy.high": percent(a["ci95_high"])} for a in arms if a["scope"] == "indirect"],
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
