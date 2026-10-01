"""Compute precision, hypothesis verdicts, and token estimates for the term catalog experiment.

Inputs: data/processed/*.csv, data/raw/{labels-public,readme}-{run}.jsonl, and the private
co-occurrence candidates, samples, labels, and collect log.
Outputs: results/summary.json, results/tables/*.csv, results/figures/*.vl.json.
"""

import csv
import json
import math
import os
import random
import statistics
from collections import defaultdict
from pathlib import Path

EXPERIMENT_DIR = Path(__file__).resolve().parent.parent
RAW_DIR = EXPERIMENT_DIR / "data" / "raw"
PROCESSED_DIR = EXPERIMENT_DIR / "data" / "processed"
RESULTS_DIR = EXPERIMENT_DIR / "results"
PRIVATE_DIR = Path(
    os.environ.get(
        "TERM_CATALOG_PRIVATE_DIR",
        "~/workspace/woon/.local/orchestration/saturn-experiments/term-catalog-precision/raw",
    )
).expanduser()
SEED = 127
Z95 = 1.959963984540054
ALPHA = 0.05
BATCH_SIZE = 20
BATCH_COUNT = 200
EVIDENCE_CHARS = 160
QUEUE_SCORE = 2.0
QUEUE_EVIDENCE = 3
WEIGHT_PAREN = 3
WEIGHT_COMMENT = 2
HANGUL_FACTORS = (0.6, 1.0, 1.6)


# Statistics ---------------------------------------------------------------

def wilson(k, n):
    if n == 0:
        return None, None
    p = k / n
    denom = 1 + Z95 ** 2 / n
    center = (p + Z95 ** 2 / (2 * n)) / denom
    half = Z95 * math.sqrt(p * (1 - p) / n + Z95 ** 2 / (4 * n * n)) / denom
    return max(0.0, center - half), min(1.0, center + half)


def binom_tail_ge(k, n, p0):
    return sum(math.comb(n, i) * p0 ** i * (1 - p0) ** (n - i) for i in range(k, n + 1))


def binom_tail_le(k, n, p0):
    return sum(math.comb(n, i) * p0 ** i * (1 - p0) ** (n - i) for i in range(0, k + 1))


def normal_cdf(x):
    return 0.5 * (1 + math.erf(x / math.sqrt(2)))


def newcombe(k1, n1, k2, n2):
    p1, p2 = k1 / n1, k2 / n2
    l1, u1 = wilson(k1, n1)
    l2, u2 = wilson(k2, n2)
    diff = p1 - p2
    lower = diff - math.sqrt((p1 - l1) ** 2 + (u2 - p2) ** 2)
    upper = diff + math.sqrt((u1 - p1) ** 2 + (p2 - l2) ** 2)
    return diff, lower, upper


def z_test(k1, n1, k2, n2):
    """One-sided p values (greater, less) for p1 - p2 with a pooled z test."""
    pooled = (k1 + k2) / (n1 + n2)
    se = math.sqrt(pooled * (1 - pooled) * (1 / n1 + 1 / n2))
    if se == 0:
        return 1.0, 1.0
    z = (k1 / n1 - k2 / n2) / se
    return 1 - normal_cdf(z), normal_cdf(z)


def holm(p_values):
    order = sorted(p_values, key=lambda key: p_values[key])
    adjusted, running = {}, 0.0
    for rank, key in enumerate(order):
        running = max(running, min(1.0, (len(order) - rank) * p_values[key]))
        adjusted[key] = running
    return adjusted


def describe(values):
    ordered = sorted(values)

    def pct(q):
        position = (len(ordered) - 1) * q
        low, high = math.floor(position), math.ceil(position)
        return ordered[low] + (ordered[high] - ordered[low]) * (position - low)

    return {"mean": statistics.fmean(ordered), "sd": statistics.stdev(ordered) if len(ordered) > 1 else 0.0,
            "median": pct(0.5), "p5": pct(0.05), "p95": pct(0.95), "min": ordered[0], "max": ordered[-1],
            "n": len(ordered)}


# Inputs -------------------------------------------------------------------

def latest_run_id():
    runs = sorted(p.name[len("paren-"):-len(".jsonl")] for p in RAW_DIR.glob("paren-*.jsonl"))
    return runs[-1]


def read_csv(path):
    with path.open(encoding="utf-8") as handle:
        return list(csv.DictReader(handle))


def read_jsonl(path):
    with path.open(encoding="utf-8") as handle:
        return [json.loads(line) for line in handle]


def write_csv(path, header, rows):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8", newline="") as out:
        writer = csv.writer(out, lineterminator="\n")
        writer.writerow(header)
        writer.writerows(rows)


def load_labels(run_id):
    labels = {}
    for path in (RAW_DIR / f"labels-public-{run_id}.jsonl", PRIVATE_DIR / f"labels-cooc-{run_id}.jsonl"):
        for row in read_jsonl(path):
            labels[(row["ko"], row["en"])] = row["label"]
    return labels


# Measures -----------------------------------------------------------------

def precision(keys, labels):
    labeled = [labels.get(key) for key in keys]
    present = [label for label in labeled if label]
    k = sum(label == "same" for label in present)
    unclear = sum(label == "unclear" for label in present)
    n = len(present)
    low, high = wilson(k, n)
    clear_n = n - unclear
    clear_low, clear_high = wilson(k, clear_n)
    return {"k": k, "n": n, "unlabeled": len(labeled) - n, "unclear": unclear,
            "precision": k / n if n else None, "ci_low": low, "ci_high": high,
            "precision_without_unclear": k / clear_n if clear_n else None,
            "ci_low_without_unclear": clear_low, "ci_high_without_unclear": clear_high}


def one_sample_test(stat, p0):
    if stat["n"] == 0:
        return "hold", 1.0
    p_greater = binom_tail_ge(stat["k"], stat["n"], p0)
    p_less = binom_tail_le(stat["k"], stat["n"], p0)
    if stat["ci_low"] >= p0:
        return "accept", p_greater
    if stat["ci_high"] < p0:
        return "reject", p_less
    return "hold", min(p_greater, p_less)


def difference(first, second):
    if first["n"] == 0 or second["n"] == 0:
        return None
    diff, low, high = newcombe(first["k"], first["n"], second["k"], second["n"])
    p_greater, p_less = z_test(first["k"], first["n"], second["k"], second["n"])
    return {"diff": diff, "ci_low": low, "ci_high": high, "p_greater": p_greater, "p_less": p_less,
            "k1": first["k"], "n1": first["n"], "k2": second["k"], "n2": second["n"]}


def difference_test(diff):
    if diff is None:
        return "hold", 1.0
    if diff["ci_low"] > 0:
        return "accept", diff["p_greater"]
    if diff["ci_high"] < 0:
        return "reject", diff["p_less"]
    return "hold", min(diff["p_greater"], diff["p_less"])


def estimate_tokens(text, hangul_factor):
    ascii_chars = sum(1 for ch in text if ord(ch) < 128)
    hangul = sum(1 for ch in text if "가" <= ch <= "힣")
    other = len(text) - ascii_chars - hangul
    return math.ceil(ascii_chars / 4 + hangul * hangul_factor + other)


def request_body(project, pairs):
    state_pairs, questions = [], []
    for index, pair in enumerate(pairs):
        evidence = [text[:EVIDENCE_CHARS] for text in pair["evidence"][:2] if text]
        state_pairs.append({"a": pair["ko"], "b": pair["en"], "evidence": evidence})
        questions.append({"id": f"same_{index}", "type": "noul",
                          "question": f"이 프로젝트에서 `pairs[{index}].a`와 `pairs[{index}].b`는 같은 대상을 가리킨다."})
    body = {"model": "jev", "question_set": "term@1.0",
            "state": {"project": project, "pairs": state_pairs}, "questions": questions}
    return json.dumps(body, ensure_ascii=False, separators=(",", ":"))


def token_comparison(pool, project):
    rng = random.Random(SEED)
    batches = []
    for _ in range(BATCH_COUNT):
        if len(pool) >= BATCH_SIZE:
            batches.append(rng.sample(pool, BATCH_SIZE))
        else:
            batches.append(rng.choices(pool, k=BATCH_SIZE))
    result = {}
    for factor in HANGUL_FACTORS:
        batch_tokens, single_tokens, reduction, saved = [], [], [], []
        for batch in batches:
            together = estimate_tokens(request_body(project, batch), factor)
            apart = sum(estimate_tokens(request_body(project, [pair]), factor) for pair in batch)
            batch_tokens.append(together)
            single_tokens.append(apart)
            reduction.append(1 - together / apart)
            saved.append(apart - together)
        result[str(factor)] = {"batch_tokens": describe(batch_tokens), "single_tokens": describe(single_tokens),
                               "token_reduction": describe(reduction), "tokens_saved": describe(saved),
                               "batch_tokens_values": batch_tokens, "reduction_values": reduction}
    return result


# Figures ------------------------------------------------------------------

def precision_figure(rows):
    n_text = ", ".join(f"{r['source']} n={r['n']}" for r in rows)
    return {
        "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
        "title": {"text": "출처별 짝 후보 정밀도", "subtitle": f"{n_text}. 오차 막대는 95% Wilson 신뢰구간, 기준선은 80%(H1, H2)와 50%(H3)"},
        "width": 420,
        "height": 220,
        "layer": [
            {"data": {"values": rows}, "mark": "bar",
             "encoding": {"x": {"field": "source", "type": "nominal", "title": "출처", "sort": None},
                          "y": {"field": "precision", "type": "quantitative", "title": "정밀도(%)",
                                "scale": {"domain": [0, 100]}}}},
            {"data": {"values": rows}, "mark": "errorbar",
             "encoding": {"x": {"field": "source", "type": "nominal", "sort": None},
                          "y": {"field": "ci_low", "type": "quantitative", "title": "정밀도(%)"},
                          "y2": {"field": "ci_high"}}},
            {"data": {"values": [{"threshold": 80}, {"threshold": 50}]}, "mark": "rule",
             "encoding": {"y": {"field": "threshold", "type": "quantitative"}}},
        ],
    }


def token_figure(values):
    rows = [{"batch": "짝 20개 묶음", "tokens": v} for v in values]
    return {
        "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
        "title": {"text": "짝 20개 묶음 요청의 추정 입력 토큰", "subtitle": f"n={len(values)}. 기준선은 가설 범위 2,000과 3,000"},
        "width": 420,
        "height": 220,
        "layer": [
            {"data": {"values": rows}, "mark": "boxplot",
             "encoding": {"x": {"field": "batch", "type": "nominal", "title": None},
                          "y": {"field": "tokens", "type": "quantitative", "title": "입력 토큰(토큰)",
                                "scale": {"zero": True}}}},
            {"data": {"values": [{"threshold": 2000}, {"threshold": 3000}]}, "mark": "rule",
             "encoding": {"y": {"field": "threshold", "type": "quantitative"}}},
        ],
    }


# Main ---------------------------------------------------------------------

def rounded(value):
    if isinstance(value, float):
        return float(f"{value:.6g}")
    if isinstance(value, dict):
        return {k: rounded(v) for k, v in value.items()}
    if isinstance(value, list):
        return [rounded(v) for v in value]
    return value


def main():
    run_id = latest_run_id()
    labels = load_labels(run_id)
    paren = {(r["ko"], r["en"]): r for r in read_csv(PROCESSED_DIR / "pairs-paren.csv")}
    comment = {(r["ko"], r["en"]): r for r in read_csv(PROCESSED_DIR / "pairs-comment.csv")}
    extended = {(r["ko"], r["en"]): r for r in read_csv(PROCESSED_DIR / "pairs-comment-extended.csv")}
    strata = defaultdict(list)
    for row in read_csv(PROCESSED_DIR / "samples.csv"):
        strata[row["stratum"]].append((row["ko"], row["en"]))
    cooc_candidates = {(r["project"], r["ko"], r["en"]): r
                       for r in read_csv(PRIVATE_DIR / f"cooc-candidates-{run_id}.csv")}
    cooc_sample = [(r["project"], r["ko"], r["en"]) for r in read_csv(PRIVATE_DIR / f"samples-cooc-{run_id}.csv")]
    collect_log = json.loads((PRIVATE_DIR / f"collect-log-{run_id}.json").read_text(encoding="utf-8"))
    turns = sum(1 for _ in (PRIVATE_DIR / f"cooc-{run_id}.jsonl").open(encoding="utf-8"))
    occurrences = {name: sum(1 for _ in (RAW_DIR / f"{name}-{run_id}.jsonl").open(encoding="utf-8"))
                   for name in ("paren", "comment")}

    stats = {
        "paren": precision(strata["paren_random"], labels),
        "comment": precision(strata["comment_random"], labels),
        "cooc": precision([key[1:] for key in cooc_sample], labels),
    }

    dice_sorted = sorted(cooc_sample, key=lambda key: float(cooc_candidates[key]["dice"]))
    dice_median = statistics.median(float(cooc_candidates[key]["dice"]) for key in cooc_sample) if cooc_sample else None
    upper = [key[1:] for key in cooc_sample if float(cooc_candidates[key]["dice"]) >= dice_median]
    lower = [key[1:] for key in cooc_sample if float(cooc_candidates[key]["dice"]) < dice_median]
    stats["cooc_dice_upper"] = precision(upper, labels)
    stats["cooc_dice_lower"] = precision(lower, labels)
    low_evidence = [key for key in strata["paren_random"] if int(paren[key]["evidence"]) < QUEUE_EVIDENCE]
    stats["paren_ev3"] = precision(strata["paren_ev3"], labels)
    stats["paren_ev12"] = precision(low_evidence, labels)

    tests, verdicts, details = {}, {}, {}
    for name, key, p0 in (("H1", "paren", 0.8), ("H2", "comment", 0.8), ("H3", "cooc", 0.5)):
        verdicts[name], tests[name] = one_sample_test(stats[key], p0)
    details["H4"] = difference(stats["cooc_dice_upper"], stats["cooc_dice_lower"])
    verdicts["H4"], tests["H4"] = difference_test(details["H4"])
    details["H5"] = difference(stats["paren_ev3"], stats["paren_ev12"])
    verdicts["H5"], tests["H5"] = difference_test(details["H5"])
    d1 = difference(stats["paren"], stats["comment"])
    d2 = difference(stats["comment"], stats["cooc"])
    details["H6"] = {"paren_minus_comment": d1, "comment_minus_cooc": d2}
    if d1 is None or d2 is None:
        verdicts["H6"], tests["H6"] = "hold", 1.0
    elif d2["ci_low"] > 0 and d1["ci_high"] >= 0:
        verdicts["H6"], tests["H6"] = "accept", d2["p_greater"]
    elif d1["ci_high"] < 0 or d2["ci_high"] < 0:
        candidates = [d["p_less"] for d in (d1, d2) if d["ci_high"] < 0]
        verdicts["H6"], tests["H6"] = "reject", min(candidates)
    else:
        verdicts["H6"], tests["H6"] = "hold", min(d2["p_greater"], d2["p_less"])
    adjusted = holm(tests)
    final = {}
    for name in tests:
        final[name] = verdicts[name] if verdicts[name] != "hold" and adjusted[name] < ALPHA else "hold"

    project = read_jsonl(RAW_DIR / f"readme-{run_id}.jsonl")[0]["text"]
    combined = defaultdict(lambda: {"paren": 0, "comment": 0, "evidence": []})
    for key, row in paren.items():
        combined[key]["paren"] = int(row["evidence"])
        combined[key]["evidence"] += [row["example_1"], row["example_2"]]
    for key, row in comment.items():
        combined[key]["comment"] = int(row["evidence"])
        combined[key]["evidence"] += [row["example_1"], row["example_2"]]
    queued = sorted(key for key, c in combined.items()
                    if WEIGHT_PAREN * c["paren"] + WEIGHT_COMMENT * c["comment"] >= QUEUE_SCORE
                    and c["paren"] + c["comment"] >= QUEUE_EVIDENCE)
    pool = [{"ko": ko, "en": en, "evidence": [t for t in combined[(ko, en)]["evidence"] if t]} for ko, en in queued]
    tokens = token_comparison(pool, project)
    primary = tokens["1.0"]
    batch_median = primary["batch_tokens"]["median"]
    reduction_median = primary["token_reduction"]["median"]
    final["H7"] = "accept" if 2000 <= batch_median <= 3000 else "reject"
    final["H8"] = "accept" if reduction_median >= 0.30 else "reject"
    final["H9"] = "hold"

    pattern_stats = {}
    for pattern in ("ko_paren_en", "en_paren_ko", "code_adjacent"):
        keys = [key for key in strata["paren_random"] if int(paren[key][pattern]) > 0]
        pattern_stats[pattern] = precision(keys, labels)
    dice_bins = {}
    if cooc_sample:
        quarter = max(1, len(dice_sorted) // 4)
        for index in range(4):
            chunk = dice_sorted[index * quarter:(index + 1) * quarter if index < 3 else len(dice_sorted)]
            dice_bins[f"q{index + 1}"] = precision([key[1:] for key in chunk], labels)
            dice_bins[f"q{index + 1}"]["dice_min"] = float(cooc_candidates[chunk[0]]["dice"])
            dice_bins[f"q{index + 1}"]["dice_max"] = float(cooc_candidates[chunk[-1]]["dice"])
    same_paren = {key for key in paren if labels.get(key) == "same"}
    same_comment = {key for key in comment if labels.get(key) == "same"}
    explicit = [key for key, row in paren.items() if int(row["ko_paren_en"]) + int(row["en_paren_ko"]) > 0]
    exploratory = {
        "paren_patterns": pattern_stats,
        "paren_explicit_population": {"pairs": len(explicit),
                                      "labeled": precision([k for k in explicit if labels.get(k)], labels)},
        "comment_extended": precision(strata["comment_extended"], labels),
        "comment_extended_pairs": len(extended),
        "comment_extended_only_pairs": len(set(extended) - set(comment)),
        "cooc_dice_quartiles": dice_bins,
        "overlap": {"paren_pairs": len(paren), "comment_pairs": len(comment),
                    "both_sources": len(set(paren) & set(comment)),
                    "same_labeled_paren": len(same_paren), "same_labeled_comment": len(same_comment),
                    "same_labeled_both": len(same_paren & same_comment)},
        "queued": {"pairs": len(queued), "precision": precision(queued, labels),
                   "cooc_only_can_queue": False, "dice_max": 1.0},
        "tokens_sensitivity": {f: {k: v for k, v in t.items() if not k.endswith("_values")}
                               for f, t in tokens.items()},
    }

    flow = {
        "paren_occurrences": occurrences["paren"], "paren_pairs": len(paren),
        "comment_occurrences": occurrences["comment"], "comment_pairs": len(comment),
        "cooc_files": collect_log["files"], "cooc_unreadable_files": collect_log["unreadable_files"],
        "cooc_bad_lines": collect_log["bad_lines"], "cooc_turns": turns,
        "cooc_candidates": len(cooc_candidates),
        "sampled": {"paren_random": len(strata["paren_random"]), "paren_ev3": len(strata["paren_ev3"]),
                    "comment_random": len(strata["comment_random"]),
                    "comment_extended": len(strata["comment_extended"]), "cooc_random": len(cooc_sample)},
        "label_rows": len(labels),
        "label_rows_unlabeled": sum(1 for label in labels.values() if not label),
        "token_pool_pairs": len(pool),
    }

    summary = rounded({
        "run_id": run_id,
        "flow": flow,
        "precision": stats,
        "dice_median": dice_median,
        "differences": details,
        "tests": {"p_one_sided": tests, "p_holm": adjusted, "ci_verdict": verdicts},
        "verdicts": final,
        "tokens": {"requests_single": BATCH_SIZE, "requests_batch": 1,
                   "batch_tokens": primary["batch_tokens"], "single_tokens": primary["single_tokens"],
                   "token_reduction": primary["token_reduction"], "tokens_saved": primary["tokens_saved"]},
        "exploratory": exploratory,
    })
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    (RESULTS_DIR / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n",
                                              encoding="utf-8")
    write_csv(RESULTS_DIR / "tables" / "precision.csv",
              ["group", "k", "n", "precision", "ci_low", "ci_high", "unclear"],
              [[g, s["k"], s["n"], summary["precision"][g]["precision"], summary["precision"][g]["ci_low"],
                summary["precision"][g]["ci_high"], s["unclear"]] for g, s in stats.items()])
    write_csv(RESULTS_DIR / "tables" / "tokens.csv",
              ["hangul_factor", "batch_median", "single_median", "reduction_median"],
              [[f, t["batch_tokens"]["median"], t["single_tokens"]["median"], round(t["token_reduction"]["median"], 6)]
               for f, t in tokens.items()])
    figure_rows = [{"source": s, "n": stats[s]["n"],
                    "precision": round(100 * (stats[s]["precision"] or 0), 1),
                    "ci_low": round(100 * (stats[s]["ci_low"] or 0), 1),
                    "ci_high": round(100 * (stats[s]["ci_high"] or 0), 1)} for s in ("paren", "comment", "cooc")]
    figures = RESULTS_DIR / "figures"
    figures.mkdir(parents=True, exist_ok=True)
    (figures / "precision-by-source.vl.json").write_text(
        json.dumps(precision_figure(figure_rows), ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    (figures / "batch-tokens.vl.json").write_text(
        json.dumps(token_figure(primary["batch_tokens_values"]), ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8")
    print(json.dumps(summary["verdicts"], ensure_ascii=False))


if __name__ == "__main__":
    main()
