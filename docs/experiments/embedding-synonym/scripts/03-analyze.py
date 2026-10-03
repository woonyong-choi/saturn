"""분석용 표로 가설 판정, 탐색 표, 그림 원본을 만든다.

입력: data/processed/proposals.csv, data/processed/rankings.csv, data/processed/cost.csv
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
EXACT_BELOW = 25
MODELS = ("e5", "minilm")
MODEL_LABEL = {"e5": "multilingual-e5-small", "minilm": "paraphrase-multilingual-MiniLM-L12-v2"}
CHART_LABEL = {"e5": "multilingual-e5-small", "minilm": "MiniLM-L12-v2"}  # 차트 이름 칸에 들어가는 길이
FOLD_PRECISION = 0.85
PRECISION_TARGET = 0.80
RECALL_TARGET = 0.50
MIN_GAIN = 0.10
NI_MARGIN = 0.02
BUDGET = {"install_bytes": 200_000_000, "rss_increase_bytes": 200_000_000, "latency_p95_ms": 20.0}


# 통계


def pct(x):
    return round(100 * x, 1)


def sig(x, digits=3):
    return float(f"{x:.{digits}g}")


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
    return {"k": k, "n": n, "pct": pct(k / n) if n else None, "ci_pct": [pct(lo), pct(hi)],
            "raw": [k / n if n else 0.0, lo, hi]}


def public(prop):
    return {k: v for k, v in prop.items() if k != "raw"}


def binom_two_sided(k, n, p0):
    if n == 0:
        return 1.0
    probs = [math.comb(n, i) * p0 ** i * (1 - p0) ** (n - i) for i in range(n + 1)]
    observed = probs[k]
    return min(1.0, sum(p for p in probs if p <= observed * (1 + 1e-7)))


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


def paired(xs, ys):
    a = b = c = d = 0
    for x, y in zip(xs, ys):
        a += x and y
        b += x and not y
        c += y and not x
        d += not x and not y
    diff, lower, upper = paired_diff_ci(a, b, c, d)
    n = a + b + c + d
    return {"n": n, "baseline": proportion(a + b, n), "candidate": proportion(a + c, n),
            "table": {"both": a, "baseline_only": b, "candidate_only": c, "neither": d},
            "diff_pct": pct(diff), "diff_ci_pct": [pct(lower), pct(upper)],
            "diff_raw": [diff, lower, upper], "mcnemar": mcnemar(b, c)}


def describe(values):
    xs = sorted(values)
    n = len(xs)
    mean = sum(xs) / n
    sd = math.sqrt(sum((x - mean) ** 2 for x in xs) / (n - 1)) if n > 1 else 0.0

    def q(p):
        pos = (n - 1) * p
        lo, hi = math.floor(pos), math.ceil(pos)
        return xs[lo] + (xs[hi] - xs[lo]) * (pos - lo)

    q1, q3 = q(0.25), q(0.75)
    iqr = q3 - q1
    mild = sum(1 for x in xs if (q3 + 1.5 * iqr < x <= q3 + 3 * iqr) or (q1 - 3 * iqr <= x < q1 - 1.5 * iqr))
    severe = sum(1 for x in xs if x > q3 + 3 * iqr or x < q1 - 3 * iqr)
    return {"n": n, "mean": sig(mean), "sd": sig(sd), "median": sig(q(0.5)), "p5": sig(q(0.05)),
            "p95": sig(q(0.95)), "outliers_mild": mild, "outliers_severe": severe}


# 데이터


def read_csv(name):
    with open(os.path.join(PROCESSED_DIR, name), encoding="utf-8") as f:
        return list(csv.DictReader(f))


def write_csv(name, rows):
    with open(os.path.join(RESULTS_DIR, "tables", name), "w", encoding="utf-8", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=list(rows[0]), lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


# (a) 짝 찾기


def fold_threshold(rows):
    """다른 접힘에서 접힘 정밀도 85% 이상을 지키며 재현율이 가장 큰 기준값. 같으면 큰 값."""
    gold_n = sum(1 for r in rows if r["gold_label"] == "same")
    best = (-1, None)
    for tau in sorted({float(r["top1_score"]) for r in rows}, reverse=True):
        proposed = [r for r in rows if float(r["top1_score"]) >= tau]
        correct = sum(1 for r in proposed if r["top1_label"] == "same")
        hit = sum(1 for r in proposed if r["top1_label"] == "same" and r["gold_label"] == "same")
        if proposed and correct / len(proposed) >= FOLD_PRECISION and gold_n and hit > best[0]:
            best = (hit, tau)
    return best[1]


def pair_metrics(rows):
    """접힘마다 다른 접힘에서 고른 기준값을 적용해 모은 정밀도와 재현율."""
    out = {"thresholds": {}}
    proposed = correct = hit = 0
    gold_n = sum(1 for r in rows if r["gold_label"] == "same")
    decided = []
    for fold in ("0", "1"):
        tau = fold_threshold([r for r in rows if r["fold"] != fold])
        out["thresholds"][fold] = None if tau is None else round(tau, 6)
        for r in rows:
            if r["fold"] != fold:
                continue
            take = tau is not None and float(r["top1_score"]) >= tau
            decided.append((r, take))
            if take:
                proposed += 1
                correct += r["top1_label"] == "same"
                hit += r["top1_label"] == "same" and r["gold_label"] == "same"
    out["precision"] = proportion(correct, proposed)
    out["recall"] = proportion(hit, gold_n)
    out["decided"] = decided
    return out


def target_verdict(prop, target, p_holm):
    if prop["n"] == 0:
        return "기각"
    _, lo, hi = prop["raw"]
    if lo >= target and p_holm < ALPHA:
        return "채택"
    if hi < target and p_holm < ALPHA:
        return "기각"
    return "보류"


# (b) 후보 순위


def hit10(value):
    return value != "" and int(value) <= 10


def ranking_compare(rows, sets, model, only_same=False):
    picked = [r for r in rows if r["model"] == model and r["set"] in sets
              and (not only_same or r["pair_gold_label"] == "same")]
    picked.sort(key=lambda r: r["rank_query_id"])
    return paired([hit10(r["base_gold_rank"]) for r in picked],
                  [hit10(r["rrf_gold_rank"]) for r in picked])


# 비용


def cost_metrics(rows, model):
    mine = [r for r in rows if r["model"] == model]
    install = next(int(r["value"]) for r in mine if r["measure"] == "install_bytes")
    install_detail = json.loads(next(r["detail"] for r in mine if r["measure"] == "install_bytes"))
    rss = next(int(r["value"]) for r in mine if r["measure"] == "rss_increase_bytes")
    rss_detail = json.loads(next(r["detail"] for r in mine if r["measure"] == "rss_increase_bytes"))
    latencies = [int(r["value"]) / 1e6 for r in mine if r["measure"] == "latency_ns"]
    latency = describe(latencies)
    passed = (install <= BUDGET["install_bytes"] and rss <= BUDGET["rss_increase_bytes"]
              and latency["p95"] <= BUDGET["latency_p95_ms"])
    return {
        "install_mb": round(install / 1e6, 1),
        "install_detail_mb": {k: round(v / 1e6, 1) for k, v in install_detail.items()},
        "rss_increase_mb": round(rss / 1e6, 1),
        "rss_detail_mb": {k: round(v / 1e6, 1) for k, v in rss_detail.items() if k != "load_ns"},
        "load_s": sig(rss_detail["load_ns"] / 1e9),
        "latency_ms": latency,
        "verdict": "채택" if passed else "기각",
    }


# 확인 분석


def strip(result):
    out = dict(result)
    out.pop("diff_raw", None)
    out.pop("raw", None)
    for key in ("baseline", "candidate", "precision", "recall"):
        if key in out and isinstance(out[key], dict):
            out[key] = {k: v for k, v in out[key].items() if k != "raw"}
    return out


def confirmatory(proposals, rankings, cost):
    hyps = []
    pair = {m: pair_metrics([r for r in proposals if r["model"] == m]) for m in MODELS}
    for hid, metric, model, target in (("H1", "precision", "e5", PRECISION_TARGET),
                                       ("H2", "precision", "minilm", PRECISION_TARGET),
                                       ("H3", "recall", "e5", RECALL_TARGET),
                                       ("H4", "recall", "minilm", RECALL_TARGET)):
        prop = pair[model][metric]
        hyps.append({"id": hid, "kind": metric, "model": model, "target_pct": pct(target),
                     "thresholds": pair[model]["thresholds"], "result": prop,
                     "p": binom_two_sided(prop["k"], prop["n"], target)})
    for hid, model in (("H5", "e5"), ("H6", "minilm")):
        r = ranking_compare(rankings, ("synonym",), model, only_same=True)
        hyps.append({"id": hid, "kind": "gain", "model": model, "min_gain_pct": pct(MIN_GAIN),
                     "result": r, "p": r["mcnemar"]["p"]})
    for hid, model in (("H7", "e5"), ("H8", "minilm")):
        r = ranking_compare(rankings, ("lexical-ko", "lexical-en"), model)
        hyps.append({"id": hid, "kind": "guard", "model": model, "margin_pct": pct(NI_MARGIN),
                     "result": r, "p": None})
    for hid, model in (("H9", "e5"), ("H10", "minilm")):
        hyps.append({"id": hid, "kind": "cost", "model": model, "result": cost[model], "p": None})

    tested = [h for h in hyps if h["p"] is not None]
    for h, p in zip(tested, holm([h["p"] for h in tested])):
        h["p_holm"] = p
    for h in hyps:
        h.setdefault("p_holm", None)
        if h["kind"] in ("precision", "recall"):
            h["verdict"] = target_verdict(h["result"], h["target_pct"] / 100, h["p_holm"])
        elif h["kind"] == "gain":
            diff, lo, hi = h["result"]["diff_raw"]
            if h["p_holm"] < ALPHA and lo > 0 and diff >= MIN_GAIN:
                h["verdict"] = "채택"
            elif hi < MIN_GAIN:
                h["verdict"] = "기각"
            else:
                h["verdict"] = "보류"
        elif h["kind"] == "guard":
            h["verdict"] = "채택" if h["result"]["diff_raw"][1] > -NI_MARGIN else "기각"
        else:
            h["verdict"] = h["result"]["verdict"]
        h["p"] = None if h["p"] is None else sig(h["p"], 4)
        h["p_holm"] = None if h["p_holm"] is None else sig(h["p_holm"], 4)
        h["result"] = strip(h["result"])
    return hyps, pair


def decision(hyps):
    v = {(h["kind"], h["model"]): h["verdict"] for h in hyps}
    channel = [m for m in MODELS if v[("gain", m)] == v[("guard", m)] == v[("cost", m)] == "채택"]
    catalog = [m for m in MODELS
               if v[("precision", m)] == v[("recall", m)] == v[("cost", m)] == "채택"]
    accuracy_only = [m for m in MODELS if v[("cost", m)] != "채택" and (
        v[("gain", m)] == v[("guard", m)] == "채택" or v[("precision", m)] == v[("recall", m)] == "채택")]
    if channel:
        return {"option": "embedding-channel", "models": channel, "accuracy_only_models": accuracy_only}
    if catalog:
        return {"option": "embedding-catalog", "models": catalog, "accuracy_only_models": accuracy_only}
    return {"option": "judge-only", "models": [], "accuracy_only_models": accuracy_only}


# 탐색 분석


def exploratory(proposals, rankings, pair):
    out = {}
    gold_same = defaultdict(dict)
    for r in proposals:
        if r["gold_label"] == "same":
            gold_same[r["model"]][r["query_id"]] = int(r["gold_rank"])
    out["gold_rank"] = {}
    for m in MODELS:
        ranks = list(gold_same[m].values())
        out["gold_rank"][m] = {f"hit_at_{k}": public(proportion(sum(1 for x in ranks if x <= k), len(ranks)))
                               for k in (1, 5, 10)}
    labels = defaultdict(int)
    for r in proposals:
        if r["model"] == "e5":
            labels[r["gold_label"]] += 1
    out["gold_labels"] = dict(sorted(labels.items()))
    out["top1_labels"] = {m: dict(sorted(
        {lab: sum(1 for r in proposals if r["model"] == m and r["top1_label"] == lab)
         for lab in ("same", "different", "unclear")}.items())) for m in MODELS}
    # 기준값 없이 1위 제안 전부의 정밀도와 두 모델 비교
    by = {m: {r["query_id"]: r for r in proposals if r["model"] == m} for m in MODELS}
    ids = sorted(by["e5"])
    out["top1_all_precision"] = {m: public(proportion(
        sum(1 for q in ids if by[m][q]["top1_label"] == "same"), len(ids))) for m in MODELS}
    cmp = paired([by["minilm"][q]["top1_label"] == "same" for q in ids],
                 [by["e5"][q]["top1_label"] == "same" for q in ids])
    cmp["mcnemar"]["p"] = sig(cmp["mcnemar"]["p"], 4)
    out["e5_minus_minilm_top1"] = strip(cmp)
    # unclear를 뺀 정밀도
    out["precision_without_unclear"] = {}
    for m in MODELS:
        taken = [r for r, take in pair[m]["decided"] if take and r["top1_label"] != "unclear"]
        out["precision_without_unclear"][m] = public(proportion(
            sum(1 for r in taken if r["top1_label"] == "same"), len(taken)))
    # 정의 종류별 1위 정확도
    kinds = defaultdict(lambda: [0, 0])
    for r in proposals:
        if r["model"] == "e5" and r["gold_label"] == "same":
            kinds[r["def_kind"]][0] += r["top1_label"] == "same"
            kinds[r["def_kind"]][1] += 1
    out["e5_top1_by_kind"] = {k: {"k": v[0], "n": v[1]} for k, v in sorted(kinds.items())}
    # 순위: 질의 묶음별, 임베딩 단독
    sets = []
    for m in MODELS:
        for s in ("synonym", "lexical-ko", "lexical-en"):
            picked = [r for r in rankings if r["model"] == m and r["set"] == s
                      and (s != "synonym" or r["pair_gold_label"] == "same")]
            n = len(picked)
            row = {"model": m, "set": s, "n": n}
            for col in ("base", "embedding", "rrf"):
                p = proportion(sum(1 for r in picked if hit10(r[f"{col}_gold_rank"])), n)
                row[f"{col}_pct"], row[f"{col}_lo"], row[f"{col}_hi"] = p["pct"], *p["ci_pct"]
            sets.append(row)
    out["recall_by_set"] = sets
    return out


# 그림


def write_bar_chart(name: str, title: str, subtitle: str, hypotheses: list[dict]) -> None:
    """모델마다 단어 기반 순위와 임베딩을 더한 순위의 비율을 막대와 신뢰구간으로 쓴다."""
    header = f"""chart bar
title "{title}"
subtitle "{subtitle}"
x "재현율(%)"
decimals 1

series candidate "단어 기반 + 임베딩" role=main
series baseline "단어 기반" role=compare
"""
    rows = []
    for h in hypotheses:
        result = h["result"]
        row = {"label": CHART_LABEL[h["model"]]}
        for key, column in (("baseline", "baseline"), ("candidate", "candidate")):
            row[key] = result[column]["pct"]
            row[f"{key}.low"], row[f"{key}.high"] = result[column]["ci_pct"]
        rows.append(row)
    figures = os.path.join(RESULTS_DIR, "figures")
    with open(os.path.join(figures, f"{name}.muto"), "w", encoding="utf-8") as f:
        f.write(f'{header}data "{name}.json"\n')
    with open(os.path.join(figures, f"{name}.json"), "w", encoding="utf-8") as f:
        json.dump(rows, f, ensure_ascii=False, indent=2)
        f.write("\n")


def main():
    proposals = read_csv("proposals.csv")
    rankings = read_csv("rankings.csv")
    cost_rows = read_csv("cost.csv")
    cost = {m: cost_metrics(cost_rows, m) for m in MODELS}
    hyps, pair = confirmatory(proposals, rankings, cost)
    explore = exploratory(proposals, rankings, pair)
    summary = {
        "criteria": {"alpha": ALPHA, "fold_precision_pct": pct(FOLD_PRECISION),
                     "precision_target_pct": pct(PRECISION_TARGET),
                     "recall_target_pct": pct(RECALL_TARGET), "min_gain_pct": pct(MIN_GAIN),
                     "guard_margin_pct": pct(NI_MARGIN), "budget": BUDGET, "top_k": 10},
        "models": MODEL_LABEL,
        "counts": {
            "pair_queries": len({r["query_id"] for r in proposals}),
            "pair_queries_gold_same": len({r["query_id"] for r in proposals if r["gold_label"] == "same"}),
            "synonym_excluded_not_same": len({r["query_id"] for r in proposals if r["gold_label"] != "same"}),
            "label_rows": len({(r["query_id"], r["top1_identifier"]) for r in proposals}
                              | {(r["query_id"], r["gold_identifier"]) for r in proposals}),
            "rank_queries": {s: len({r["rank_query_id"] for r in rankings if r["set"] == s})
                             for s in ("synonym", "lexical-ko", "lexical-en")},
        },
        "hypotheses": hyps,
        "decision": decision(hyps),
        "exploratory": explore,
    }
    os.makedirs(os.path.join(RESULTS_DIR, "tables"), exist_ok=True)
    os.makedirs(os.path.join(RESULTS_DIR, "figures"), exist_ok=True)
    with open(os.path.join(RESULTS_DIR, "summary.json"), "w", encoding="utf-8") as f:
        json.dump(summary, f, ensure_ascii=False, indent=2)
        f.write("\n")
    write_csv("hypotheses.csv", [{
        "id": h["id"], "kind": h["kind"], "model": h["model"], "verdict": h["verdict"],
        "p": "" if h["p"] is None else h["p"], "p_holm": "" if h["p_holm"] is None else h["p_holm"],
    } for h in hyps])
    write_csv("recall-by-set.csv", explore["recall_by_set"])
    write_csv("cost.csv", [{"model": m, "install_mb": c["install_mb"], "rss_increase_mb": c["rss_increase_mb"],
                            "latency_median_ms": c["latency_ms"]["median"],
                            "latency_p95_ms": c["latency_ms"]["p95"], "verdict": c["verdict"]}
                           for m, c in cost.items()])

    n_syn = hyps[4]["result"]["n"]
    n_lex = hyps[6]["result"]["n"]
    write_bar_chart(
        "synonym-recall-at-10",
        "같은 뜻 질의의 상위 10개 재현율",
        f"n={n_syn}. 정답 짝이 same인 한국어 설명 질의, 정답은 주석을 뺀 코드 묶음",
        hyps[4:6])
    write_bar_chart(
        "lexical-recall-at-10",
        "단어가 겹치는 질의의 상위 10개 재현율",
        f"n={n_lex}. 한글 어절 질의 200개와 영문 식별자 질의 200개",
        hyps[6:8])
    for h in hyps:
        print(f"{h['id']} ({h['model']}, {h['kind']}): {h['verdict']}")
    print(f"decision: {summary['decision']}")


if __name__ == "__main__":
    main()
