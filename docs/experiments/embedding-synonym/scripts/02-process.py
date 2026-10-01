"""가장 최근 실행의 raw 파일과 라벨을 합쳐 분석용 표를 만든다.

입력: data/raw/{pairs,proposals,rankings,label-sheet,cost}-{실행 id}.jsonl, data/raw/labels-{실행 id}.csv
출력: data/processed/proposals.csv, data/processed/rankings.csv, data/processed/cost.csv
"""

import csv
import json
import os
import sys

EXP_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RAW_DIR = os.path.join(EXP_DIR, "data", "raw")
PROCESSED_DIR = os.path.join(EXP_DIR, "data", "processed")


def read_jsonl(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f]


def latest_run_id():
    ids = sorted(f[len("pairs-"):-len(".jsonl")] for f in os.listdir(RAW_DIR)
                 if f.startswith("pairs-"))
    if not ids:
        sys.exit("data/raw/에 pairs 파일이 없다")
    return ids[-1]


def write_csv(name, header, rows):
    os.makedirs(PROCESSED_DIR, exist_ok=True)
    with open(os.path.join(PROCESSED_DIR, name), "w", encoding="utf-8", newline="") as f:
        writer = csv.writer(f, lineterminator="\n")
        writer.writerow(header)
        for row in rows:
            writer.writerow(["" if v is None else v for v in row])


def main():
    run_id = latest_run_id()
    raw = lambda name: read_jsonl(os.path.join(RAW_DIR, f"{name}-{run_id}.jsonl"))  # noqa: E731
    pairs = {q["query_id"]: q for q in raw("pairs")}
    sheet = {r["row_id"]: (r["query_id"], r["identifier"]) for r in raw("label-sheet")}
    with open(os.path.join(RAW_DIR, f"labels-{run_id}.csv"), encoding="utf-8", newline="") as f:
        labels = {sheet[r["row_id"]]: r["label"] for r in csv.DictReader(f)}

    proposal_rows = []
    for p in sorted(raw("proposals"), key=lambda r: (r["condition"], r["query_id"])):
        q = pairs[p["query_id"]]
        top1 = p["top_identifiers"][0]
        proposal_rows.append([
            p["query_id"], q["fold"], q["kind"], p["condition"], top1, p["top_scores"][0],
            labels[(p["query_id"], top1)], q["identifier"], labels[(p["query_id"], q["identifier"])],
            p["gold_rank"], p["gold_score"],
        ])
    write_csv("proposals.csv",
              ["query_id", "fold", "def_kind", "model", "top1_identifier", "top1_score", "top1_label",
               "gold_identifier", "gold_label", "gold_rank", "gold_score"], proposal_rows)

    gold_label = {qid: labels[(qid, q["identifier"])] for qid, q in pairs.items()}
    queries = {q["rank_query_id"]: q for q in raw("rank-queries")}
    ranking_rows = []
    for r in sorted(raw("rankings"), key=lambda r: (r["condition"], r["rank_query_id"])):
        q = queries[r["rank_query_id"]]
        pair_label = gold_label[q["pair_query_id"]] if q["pair_query_id"] else None
        ranking_rows.append([
            r["rank_query_id"], r["set"], r["condition"], pair_label, len(q["gold_doc_ids"]),
            r["base_gold_rank"], r["embedding_gold_rank"], r["rrf_gold_rank"],
        ])
    write_csv("rankings.csv",
              ["rank_query_id", "set", "model", "pair_gold_label", "n_gold", "base_gold_rank",
               "embedding_gold_rank", "rrf_gold_rank"], ranking_rows)

    cost_rows = []
    for c in raw("cost"):
        detail = c["detail"]
        cost_rows.append([c["condition"], c["measure"], c["value"], detail.get("round"),
                          detail.get("sentence"), json.dumps(detail, ensure_ascii=False, sort_keys=True)
                          if c["measure"] != "latency_ns" else None])
    cost_rows.sort(key=lambda r: (r[0], r[1], -1 if r[3] is None else r[3], -1 if r[4] is None else r[4]))
    write_csv("cost.csv", ["model", "measure", "value", "round", "sentence", "detail"], cost_rows)
    print(f"processed {run_id}: proposals {len(proposal_rows)}, rankings {len(ranking_rows)}, "
          f"cost {len(cost_rows)}")


if __name__ == "__main__":
    main()
