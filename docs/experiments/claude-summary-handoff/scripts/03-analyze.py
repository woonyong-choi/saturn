"""data/processed/의 표로 확인 분석과 탐색 분석을 하고 results/에 쓴다.

사용: python3 scripts/03-analyze.py (실험 폴더에서 실행)
"""
import csv
import json
import math
import random
import sys
from pathlib import Path

EXPERIMENT_DIR = Path(__file__).resolve().parent.parent
PROCESSED_DIR = EXPERIMENT_DIR / "data" / "processed"
RESULTS_DIR = EXPERIMENT_DIR / "results"
Z = 1.959963984540054
H1_THRESHOLD = 0.90
H2_MARGIN = -0.05
BOOTSTRAP_REPS = 2000
SEED = 122


def read_csv(name: str) -> list[dict]:
    path = PROCESSED_DIR / name
    with path.open(encoding="utf-8", newline="") as source:
        return list(csv.DictReader(source))


def wilson(k: int, n: int) -> list[float | None]:
    if n == 0:
        return [None, None]
    p = k / n
    center = (p + Z * Z / (2 * n)) / (1 + Z * Z / n)
    half = Z * math.sqrt(p * (1 - p) / n + Z * Z / (4 * n * n)) / (1 + Z * Z / n)
    return [max(0.0, center - half), min(1.0, center + half)]


def binomial_tail_at_least(k: int, n: int, p: float) -> float:
    return sum(math.comb(n, i) * p ** i * (1 - p) ** (n - i) for i in range(k, n + 1))


def mcnemar(b: int, c: int) -> dict:
    if b + c == 0:
        return {"method": "exact_binomial", "statistic": None, "p": 1.0}
    if b + c < 25:
        tail = sum(math.comb(b + c, i) for i in range(0, min(b, c) + 1)) / 2 ** (b + c)
        return {"method": "exact_binomial", "statistic": None, "p": min(1.0, 2 * tail)}
    statistic = (b - c) ** 2 / (b + c)
    return {"method": "mcnemar_chi2", "statistic": statistic, "p": math.erfc(math.sqrt(statistic / 2))}


def newcombe_paired(a: int, b: int, c: int, d: int) -> list[float | None]:
    n = a + b + c + d
    if n == 0:
        return [None, None]
    p1, p2 = (a + b) / n, (a + c) / n
    l1, u1 = wilson(a + b, n)
    l2, u2 = wilson(a + c, n)
    denominator = (a + b) * (c + d) * (a + c) * (b + d)
    phi = (a * d - b * c) / math.sqrt(denominator) if denominator else 0.0
    diff = p1 - p2
    lower = math.sqrt(max(0.0, (p1 - l1) ** 2 - 2 * phi * (p1 - l1) * (u2 - p2) + (u2 - p2) ** 2))
    upper = math.sqrt(max(0.0, (u1 - p1) ** 2 - 2 * phi * (u1 - p1) * (p2 - l2) + (p2 - l2) ** 2))
    return [diff - lower, diff + upper]


def holm(p_values: dict[str, float]) -> dict[str, float]:
    ordered = sorted(p_values.items(), key=lambda item: item[1])
    adjusted, running = {}, 0.0
    for rank, (name, p) in enumerate(ordered):
        running = max(running, min(1.0, (len(ordered) - rank) * p))
        adjusted[name] = running
    return adjusted


def paired_table(rows: list[dict]) -> dict:
    a = sum(1 for r in rows if r["correct_claude_summary"] == "true" and r["correct_saturn_packet"] == "true")
    b = sum(1 for r in rows if r["correct_claude_summary"] == "true" and r["correct_saturn_packet"] == "false")
    c = sum(1 for r in rows if r["correct_claude_summary"] == "false" and r["correct_saturn_packet"] == "true")
    d = sum(1 for r in rows if r["correct_claude_summary"] == "false" and r["correct_saturn_packet"] == "false")
    n = a + b + c + d
    return {
        "n": n, "a": a, "b": b, "c": c, "d": d,
        "accuracy_claude_summary": (a + b) / n if n else None,
        "accuracy_claude_summary_ci95": wilson(a + b, n),
        "accuracy_saturn_packet": (a + c) / n if n else None,
        "accuracy_saturn_packet_ci95": wilson(a + c, n),
        "accuracy_diff": (b - c) / n if n else None,
        "accuracy_diff_ci95": newcombe_paired(a, b, c, d),
        "test": mcnemar(b, c),
    }


def judge_h1(ci: list[float | None]) -> str:
    if ci[0] is None:
        return "보류"
    if ci[0] >= H1_THRESHOLD:
        return "채택"
    return "기각" if ci[1] < H1_THRESHOLD else "보류"


def judge_h2(ci: list[float | None]) -> str:
    if ci[0] is None:
        return "보류"
    if ci[0] > H2_MARGIN:
        return "채택"
    return "기각" if ci[1] < 0 else "보류"


def bootstrap_diff(rows: list[dict]) -> list[float | None]:
    by_scenario: dict[tuple[str, str], list[dict]] = {}
    for r in rows:
        by_scenario.setdefault((r["run_id"], r["scenario_id"]), []).append(r)
    clusters = [by_scenario[key] for key in sorted(by_scenario)]
    if not clusters:
        return [None, None]
    rng = random.Random(SEED)
    diffs = []
    for _ in range(BOOTSTRAP_REPS):
        sample = [r for _ in clusters for r in rng.choice(clusters)]
        diffs.append(paired_table(sample)["accuracy_diff"])
    diffs.sort()
    return [diffs[int(0.025 * BOOTSTRAP_REPS)], diffs[int(0.975 * BOOTSTRAP_REPS) - 1]]


def distribution(values: list[int]) -> dict:
    if not values:
        return {"n": 0}
    ordered = sorted(values)
    mean = sum(ordered) / len(ordered)
    sd = math.sqrt(sum((v - mean) ** 2 for v in ordered) / (len(ordered) - 1)) if len(ordered) > 1 else 0.0

    def quantile(q: float) -> float:
        return ordered[min(len(ordered) - 1, int(q * len(ordered)))]

    return {"n": len(ordered), "mean": mean, "sd": sd, "median": quantile(0.5), "p5": quantile(0.05), "p95": quantile(0.95)}


def write_accuracy_chart(table: dict) -> None:
    """같은 질문에서 Claude 요약과 Saturn 패킷의 정답률과 95% Wilson 신뢰구간을 덤벨로 쓴다."""
    header = f"""chart dumbbell
title "전환 뒤 질문 정답률"
subtitle "n={table['n']}. 오차 막대는 95% Wilson 신뢰구간"
x "정답률(%)"
decimals 1

series saturn "Saturn 패킷" role=main
series claude "Claude 요약" role=compare
"""
    row = {"label": "전환 뒤 질문"}
    for key, field in (("claude", "accuracy_claude_summary"), ("saturn", "accuracy_saturn_packet")):
        row[key] = round(table[field] * 100, 8)
        row[f"{key}.low"], row[f"{key}.high"] = (round(v * 100, 8) for v in table[f"{field}_ci95"])
    figures = RESULTS_DIR / "figures"
    figures.mkdir(parents=True, exist_ok=True)
    (figures / "h2-accuracy.muto").write_text(f'{header}data "h2-accuracy.json"\n', encoding="utf-8")
    (figures / "h2-accuracy.json").write_text(json.dumps([row], ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def write_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def main() -> int:
    try:
        compactions = read_csv("compactions.csv")
        scenarios = read_csv("scenarios.csv")
        answers = read_csv("answers.csv")
    except FileNotFoundError as missing:
        print(f"분석 불가: {missing.filename}이 없다", file=sys.stderr)
        return 1
    k = sum(1 for r in compactions if r["read_success"] == "true")
    n = len(compactions)
    h1_ci = wilson(k, n)
    h1_p = binomial_tail_at_least(k, n, H1_THRESHOLD) if n else 1.0
    kept = [r for r in answers if not r["excluded"]]
    table = paired_table(kept)
    adjusted = holm({"H1": h1_p, "H2": table["test"]["p"]})
    exclusions: dict[str, int] = {}
    for r in answers:
        if r["excluded"]:
            exclusions[r["excluded"]] = exclusions.get(r["excluded"], 0) + 1
    summary = {
        "flow": {
            "scenarios": len(scenarios),
            "scenarios_failed": sum(1 for r in scenarios if r["error"]),
            "scenarios_without_compaction": sum(1 for r in scenarios if r["compaction_count"] == "0"),
            "question_pairs": len(answers),
            "question_pairs_excluded": exclusions,
            "question_pairs_analyzed": len(kept),
        },
        "H1": {
            "k": k, "n": n, "read_success": k / n if n else None, "ci95": h1_ci,
            "p_one_sided_vs_90": h1_p, "p_holm": adjusted["H1"], "verdict": judge_h1(h1_ci),
        },
        "H2": {**table, "p_holm": adjusted["H2"], "verdict": judge_h2(table["accuracy_diff_ci95"])},
        "exploratory": {
            "by_question_kind": {
                kind: paired_table([r for r in kept if r["question_kind"] == kind])
                for kind in sorted({r["question_kind"] for r in kept})
            },
            "scenario_bootstrap_diff_ci95": bootstrap_diff(kept),
            "without_summary_fallback": paired_table([r for r in kept if r["summary_fallback"] == "false"]),
            "packet_chars_claude_summary": distribution([int(r["packet_chars_claude_summary"]) for r in kept if r["packet_chars_claude_summary"]]),
            "packet_chars_saturn_packet": distribution([int(r["packet_chars_saturn_packet"]) for r in kept if r["packet_chars_saturn_packet"]]),
            "summary_chars": distribution([int(r["summary_chars"]) for r in compactions if r["read_success"] == "true"]),
        },
    }
    write_json(RESULTS_DIR / "summary.json", summary)
    tables = RESULTS_DIR / "tables"
    tables.mkdir(parents=True, exist_ok=True)
    with (tables / "h2-contingency.csv").open("w", encoding="utf-8", newline="") as out:
        writer = csv.writer(out, lineterminator="\n")
        writer.writerow(["claude_summary", "saturn_packet_correct", "saturn_packet_wrong"])
        writer.writerow(["correct", table["a"], table["b"]])
        writer.writerow(["wrong", table["c"], table["d"]])
    if table["n"]:
        write_accuracy_chart(table)
    return 0


if __name__ == "__main__":
    sys.exit(main())
