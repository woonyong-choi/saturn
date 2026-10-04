#!/usr/bin/env python3
"""사람 확인 40건과 gpt-6-astra 정답 곡선을 다시 계산한다.

입력은 비공개 원자료 폴더(기본은 메인 저장소의 `.local/experiments/constraint-deep`)다.
원문은 읽지 않고 판정, 라벨, 확률만 쓴다. 출력은 results/summary.json이다.
"""
import glob
import json
import os
import subprocess
import sys

THRESHOLDS = [0.50, 0.60, 0.65, 0.70, 0.75, 0.80, 0.85, 0.90]
HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "results", "summary.json")


def source_dir():
    if len(sys.argv) > 1:
        return sys.argv[1]
    if os.environ.get("SATURN_DEEP_DIR"):
        return os.environ["SATURN_DEEP_DIR"]
    # worktree에는 .local이 없으므로 git 공통 폴더의 부모(메인 저장소)에서 찾는다.
    common = subprocess.check_output(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"], cwd=HERE, text=True).strip()
    return os.path.join(os.path.dirname(common), ".local", "experiments", "constraint-deep")


def wilson(k, n, z=1.96):
    if n == 0:
        return None
    p = k / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * ((p * (1 - p) / n + z * z / (4 * n * n)) ** 0.5) / d
    return [round(100 * (c - h), 1), round(100 * (c + h), 1)]


def rate(k, n):
    return {"k": k, "n": n, "pct": round(100 * k / n, 1) if n else None, "wilson95": wilson(k, n)}


def curve(pairs):
    """pairs: (gold bool, probability). 기준값 이상을 양성으로 본다."""
    positives = sum(1 for g, _ in pairs if g)
    rows = []
    for t in THRESHOLDS:
        pred = [(g, p) for g, p in pairs if p >= t]
        tp = sum(1 for g, _ in pred if g)
        rows.append({
            "threshold": t,
            "predicted": len(pred),
            "true_positive": tp,
            "gold_positive": positives,
            "precision": rate(tp, len(pred)),
            "recall": rate(tp, positives),
        })
    return rows


def human_check(base):
    verdicts = {r["case_id"]: r["verdict"] for r in
                json.load(open(os.path.join(base, "human-review-result.json")))["results"]}
    cases = {}
    for line in open(os.path.join(base, "human-review.jsonl")):
        d = json.loads(line)
        cases[f"{d['conversation_id']}:{d['turn']['turn_id']}"] = d
    sure = {c: v == "constraint" for c, v in verdicts.items() if v in ("constraint", "not_constraint")}
    counts = {"total": len(verdicts), "sure": len(sure),
              "unsure": sum(1 for v in verdicts.values() if v == "unsure")}
    agree = {}
    for name in ("gpt-6-astra", "gpt-5.6-luna", "adjudicated"):
        agree[name] = sum(1 for c, g in sure.items() if bool(cases[c]["labels"][name]["is_constraint"]) == g)
    pairs = [(g, float(cases[c]["probability"])) for c, g in sure.items()]
    return {"counts": counts, "labeler_agreement": agree, "sure_n": len(sure),
            "jev_curve": curve(pairs)}


def astra_curve(base):
    gold = {}
    for path in glob.glob(os.path.join(base, "labels", "gpt-6-astra", "*.jsonl")):
        conv = os.path.basename(path)[:-len(".jsonl")]
        for line in open(path):
            for t in json.loads(line)["turns"]:
                if not t.get("ambiguous"):
                    gold[(conv, t["turn_id"])] = bool(t["is_constraint"])
    prob = {}
    with open(os.path.join(base, "jev.jsonl")) as f:
        for line in f:
            if '"kind": "base"' not in line:
                continue
            d = json.loads(line)
            m = d["meta"]
            if m["kind"] == "base" and d["repeat"] == 1 and d["status"] == "ok":
                prob[(m["conversation_id"], m["turn_id"])] = float(d["probabilities"]["is_constraint"])
    pairs = [(g, prob[k]) for k, g in gold.items() if k in prob]
    return {"turns": len(pairs), "gold_positive": sum(1 for g, _ in pairs if g), "curve": curve(pairs)}


def main():
    base = source_dir()
    result = {"human_check": human_check(base), "astra_curve": astra_curve(base)}
    with open(OUT, "w") as f:
        json.dump(result, f, ensure_ascii=False, indent=1)
        f.write("\n")
    print("human check", result["human_check"]["counts"], result["human_check"]["labeler_agreement"])
    for r in result["human_check"]["jev_curve"]:
        print(f"  {r['threshold']:.2f} precision {r['true_positive']}/{r['predicted']} recall {r['true_positive']}/{r['gold_positive']}")
    c = result["astra_curve"]
    print("astra curve", c["turns"], "turns", c["gold_positive"], "positive")
    for r in c["curve"]:
        print(f"  {r['threshold']:.2f} precision {r['true_positive']}/{r['predicted']} recall {r['true_positive']}/{r['gold_positive']}")


if __name__ == "__main__":
    main()
