"""질의별 결과 표로 가설 판정, 탐색 표, 그림 원본을 만든다.

입력: data/processed/outcomes.csv, data/processed/index.csv
출력: results/summary.json, results/tables/*.csv, results/figures/*.muto와 *.json(mutoscope 차트 입력)
"""

import csv
import json
import math
import os
from collections import defaultdict

EXP_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROCESSED_DIR = os.path.join(EXP_DIR, "data", "processed")
RESULTS_DIR = os.path.join(EXP_DIR, "results")
Z = 1.959963984540054
ALPHA = 0.05
MIN_GAIN = 0.03
NI_MARGIN = 0.02
EXACT_BELOW = 25

HYPOTHESES = [
    {"id": "H1", "lang": "ko", "variant": "typo", "metric": "hit_at_10",
     "baseline": "base", "candidate": "ko-jamo3", "kind": "superiority"},
    {"id": "H2", "lang": "ko", "variant": "clean", "metric": "hit_at_1",
     "baseline": "base", "candidate": "ko-jamo3", "kind": "non-inferiority"},
    {"id": "H3", "lang": "en", "variant": "typo", "metric": "hit_at_10",
     "baseline": "base", "candidate": "en-char4", "kind": "superiority"},
    {"id": "H4", "lang": "en", "variant": "clean", "metric": "hit_at_1",
     "baseline": "base", "candidate": "en-char4", "kind": "non-inferiority"},
]
EXPLORATORY_PAIRS = {
    "ko": ["ko-jamo3", "ko-jamo2", "ko-jamo4", "ko-union"],
    "en": ["en-char4", "en-union"],
}
LANG_LABEL = {"ko": "한글 질의", "en": "영문 질의"}


# 통계


def pct(x):
    return round(100 * x, 1)


def wilson(k, n):
    if n == 0:
        return (0.0, 1.0)
    p = k / n
    denom = 1 + Z * Z / n
    center = (p + Z * Z / (2 * n)) / denom
    half = Z * math.sqrt(p * (1 - p) / n + Z * Z / (4 * n * n)) / denom
    return (max(0.0, center - half), min(1.0, center + half))


def proportion(k, n):
    lo, hi = wilson(k, n)
    return {"k": k, "n": n, "pct": pct(k / n), "ci_pct": [pct(lo), pct(hi)]}


def paired_diff_ci(a, b, c, d):
    """Newcombe(1998) 방법 10. 차이 = 후보 성공률 - 기준 성공률. b: 기준만 성공, c: 후보만 성공."""
    n = a + b + c + d
    p1, p2 = (a + b) / n, (a + c) / n
    l1, u1 = wilson(a + b, n)
    l2, u2 = wilson(a + c, n)
    denom = (a + b) * (c + d) * (a + c) * (b + d)
    phi = (a * d - b * c) / math.sqrt(denom) if denom else 0.0
    diff = p2 - p1
    lower = diff - math.sqrt(max(0.0, (p2 - l2) ** 2 - 2 * phi * (p2 - l2) * (u1 - p1) + (u1 - p1) ** 2))
    upper = diff + math.sqrt(max(0.0, (u2 - p2) ** 2 - 2 * phi * (u2 - p2) * (p1 - l1) + (p1 - l1) ** 2))
    return diff, max(-1.0, lower), min(1.0, upper)


def mcnemar(b, c):
    m = b + c
    if m == 0:
        return {"method": "해당 없음", "statistic": None, "p": 1.0}
    if m < EXACT_BELOW:
        tail = sum(math.comb(m, i) for i in range(min(b, c) + 1)) / 2 ** m
        return {"method": "정확 이항", "statistic": None, "p": min(1.0, 2 * tail)}
    stat = (abs(b - c) - 1) ** 2 / m
    return {"method": "McNemar 연속성 보정", "statistic": round(stat, 3),
            "p": math.erfc(math.sqrt(stat / 2))}


def holm(pvalues):
    order = sorted(range(len(pvalues)), key=lambda i: pvalues[i])
    adjusted, running = [0.0] * len(pvalues), 0.0
    for rank, i in enumerate(order):
        running = max(running, min(1.0, (len(pvalues) - rank) * pvalues[i]))
        adjusted[i] = running
    return adjusted


def sig(p):
    return float(f"{p:.4g}")


# 데이터


def read_csv(path):
    with open(path, encoding="utf-8") as f:
        return list(csv.DictReader(f))


def outcomes_by(rows):
    table = defaultdict(dict)
    for r in rows:
        table[(r["lang"], r["variant"], r["condition"])][r["query_id"]] = r
    return table


def compare(table, lang, variant, metric, baseline, candidate):
    base = table[(lang, variant, baseline)]
    cand = table[(lang, variant, candidate)]
    a = b = c = d = 0
    for qid, row in base.items():
        x, y = int(row[metric]), int(cand[qid][metric])
        a += x and y
        b += x and not y
        c += y and not x
        d += not x and not y
    n = a + b + c + d
    diff, lower, upper = paired_diff_ci(a, b, c, d)
    return {
        "n": n,
        "baseline_result": proportion(a + b, n),
        "candidate_result": proportion(a + c, n),
        "table": {"both": a, "baseline_only": b, "candidate_only": c, "neither": d},
        "diff_pct": pct(diff), "diff_ci_pct": [pct(lower), pct(upper)],
        "diff_raw": [diff, lower, upper],
        "odds_ratio": round(c / b, 3) if b else None,
        "mcnemar": mcnemar(b, c),
    }


def verdict(result):
    diff, lower, upper = result["diff_raw"]
    if result["kind"] == "non-inferiority":
        return "채택" if lower > -NI_MARGIN else "기각"
    if result["p_holm"] < ALPHA and lower > 0 and diff >= MIN_GAIN:
        return "채택"
    if upper < MIN_GAIN:
        return "기각"
    return "보류"


def confirmatory(table):
    results = []
    for h in HYPOTHESES:
        result = compare(table, h["lang"], h["variant"], h["metric"], h["baseline"], h["candidate"])
        results.append({**h, **result})
    superiority = [r for r in results if r["kind"] == "superiority"]
    for r, p in zip(superiority, holm([r["mcnemar"]["p"] for r in superiority])):
        r["p_holm"] = p
    for r in results:
        r.setdefault("p_holm", None)
        r["verdict"] = verdict(r)
        r["mcnemar"]["p"] = sig(r["mcnemar"]["p"])
        r["p_holm"] = None if r["p_holm"] is None else sig(r["p_holm"])
        del r["diff_raw"]
    return results


def condition_rows(table):
    rows = []
    for (lang, variant, condition), queries in sorted(table.items()):
        n = len(queries)
        hit10 = sum(int(r["hit_at_10"]) for r in queries.values())
        hit1 = sum(int(r["hit_at_1"]) for r in queries.values())
        rr = sum(1 / int(r["gold_rank"]) for r in queries.values()
                 if r["gold_rank"] and int(r["gold_rank"]) <= 10)
        r10, r1 = proportion(hit10, n), proportion(hit1, n)
        rows.append({
            "lang": lang, "variant": variant, "condition": condition, "n": n,
            "recall_at_10_pct": r10["pct"], "recall_at_10_lo": r10["ci_pct"][0],
            "recall_at_10_hi": r10["ci_pct"][1],
            "precision_at_1_pct": r1["pct"], "precision_at_1_lo": r1["ci_pct"][0],
            "precision_at_1_hi": r1["ci_pct"][1],
            "mrr_at_10": round(rr / n, 3),
        })
    return rows


def typo_type_rows(rows):
    groups = defaultdict(list)
    for r in rows:
        if r["variant"] == "typo":
            groups[(r["lang"], r["typo_type"], r["condition"])].append(int(r["hit_at_10"]))
    out = []
    for (lang, typo, condition), hits in sorted(groups.items()):
        p = proportion(sum(hits), len(hits))
        out.append({"lang": lang, "typo_type": typo, "condition": condition, "n": len(hits),
                    "recall_at_10_pct": p["pct"], "recall_at_10_lo": p["ci_pct"][0],
                    "recall_at_10_hi": p["ci_pct"][1]})
    return out


def exploratory_pairs(table):
    out = []
    for lang, candidates in EXPLORATORY_PAIRS.items():
        for candidate in candidates:
            for variant, metric in (("typo", "hit_at_10"), ("clean", "hit_at_10"),
                                    ("typo", "hit_at_1"), ("clean", "hit_at_1")):
                r = compare(table, lang, variant, metric, "base", candidate)
                out.append({"lang": lang, "variant": variant, "metric": metric,
                            "candidate": candidate, "n": r["n"],
                            "baseline_pct": r["baseline_result"]["pct"],
                            "candidate_pct": r["candidate_result"]["pct"],
                            "diff_pct": r["diff_pct"], "diff_lo": r["diff_ci_pct"][0],
                            "diff_hi": r["diff_ci_pct"][1],
                            "baseline_only": r["table"]["baseline_only"],
                            "candidate_only": r["table"]["candidate_only"],
                            "p_unadjusted": sig(r["mcnemar"]["p"])})
    return out


# 출력


def write_csv(path, rows):
    with open(path, "w", encoding="utf-8", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=list(rows[0]), lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def write_dumbbell_chart(
    name: str, title: str, subtitle: str, axis_title: str, results: list[dict], metric: str
) -> None:
    """언어마다 기준 단위에서 후보 단위로 바뀐 비율과 신뢰구간을 덤벨로 쓴다."""
    header = f"""chart dumbbell
title "{title}"
subtitle "{subtitle}"
x "{axis_title}"
decimals 1

series candidate "후보" role=main
series baseline "기준" role=compare
"""
    rows = []
    for r in results:
        if r["metric"] != metric:
            continue
        row = {"label": LANG_LABEL[r["lang"]]}
        for key, result in (("baseline", r["baseline_result"]), ("candidate", r["candidate_result"])):
            row[key] = result["pct"]
            row[f"{key}.low"], row[f"{key}.high"] = result["ci_pct"]
        rows.append(row)
    figures = os.path.join(RESULTS_DIR, "figures")
    with open(os.path.join(figures, f"{name}.muto"), "w", encoding="utf-8") as f:
        f.write(f'{header}data "{name}.json"\n')
    with open(os.path.join(figures, f"{name}.json"), "w", encoding="utf-8") as f:
        json.dump(rows, f, ensure_ascii=False, indent=2)
        f.write("\n")


def main():
    rows = read_csv(os.path.join(PROCESSED_DIR, "outcomes.csv"))
    index_rows = read_csv(os.path.join(PROCESSED_DIR, "index.csv"))
    table = outcomes_by(rows)
    results = confirmatory(table)
    conditions = condition_rows(table)
    typos = typo_type_rows(rows)
    pairs = exploratory_pairs(table)
    index = [{k: (v if k in ("run_id", "condition") else int(v)) for k, v in r.items()}
             for r in index_rows]
    run_ids = sorted({r["run_id"] for r in rows})
    summary = {
        "run_id": run_ids[0] if len(run_ids) == 1 else run_ids,
        "criteria": {"alpha": ALPHA, "min_gain_pct": pct(MIN_GAIN),
                     "non_inferiority_margin_pct": pct(NI_MARGIN), "top_k": 10},
        "queries": {lang: len(table[(lang, "clean", "base")]) for lang in ("ko", "en")},
        "hypotheses": results,
        "exploratory": {"conditions": conditions, "typo_types": typos, "pairs": pairs,
                        "index": index},
    }
    os.makedirs(os.path.join(RESULTS_DIR, "tables"), exist_ok=True)
    os.makedirs(os.path.join(RESULTS_DIR, "figures"), exist_ok=True)
    with open(os.path.join(RESULTS_DIR, "summary.json"), "w", encoding="utf-8") as f:
        json.dump(summary, f, ensure_ascii=False, indent=2)
        f.write("\n")
    write_csv(os.path.join(RESULTS_DIR, "tables", "conditions.csv"), conditions)
    write_csv(os.path.join(RESULTS_DIR, "tables", "typo-types.csv"), typos)
    write_csv(os.path.join(RESULTS_DIR, "tables", "exploratory-pairs.csv"), pairs)
    write_csv(os.path.join(RESULTS_DIR, "tables", "index-size.csv"), index)
    n = summary["queries"]
    subtitle = f"n=한글 {n['ko']}, 영문 {n['en']}. 후보는 한글 자모 3개, 영문 글자 4개 단위"
    write_dumbbell_chart("recall-at-10", "오타 질의의 상위 10개 재현율", subtitle, "재현율(%)", results, "hit_at_10")
    write_dumbbell_chart("precision-at-1", "오타 없는 질의의 1위 정밀도", subtitle, "정밀도(%)", results, "hit_at_1")
    for r in results:
        print(f"{r['id']}: {r['verdict']}")


if __name__ == "__main__":
    main()
