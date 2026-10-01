"""가장 최근 실행의 raw 순위를 질의별 결과 표로 바꾼다.

입력: data/raw/rankings-{실행 id}.jsonl, data/raw/index-{실행 id}.jsonl
출력: data/processed/outcomes.csv, data/processed/index.csv
"""

import csv
import json
import os
import sys

TOP_K = 10
EXP_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
RAW_DIR = os.path.join(EXP_DIR, "data", "raw")
PROCESSED_DIR = os.path.join(EXP_DIR, "data", "processed")

OUTCOME_FIELDS = ["run_id", "query_id", "lang", "variant", "typo_type", "condition",
                  "n_gold", "gold_rank", "hit_at_10", "hit_at_1", "n_query_tokens", "n_retrieved"]
INDEX_FIELDS = ["run_id", "condition", "vocab_size", "postings", "total_tokens"]


def latest_run_id():
    ids = sorted(f[len("rankings-"):-len(".jsonl")] for f in os.listdir(RAW_DIR)
                 if f.startswith("rankings-"))
    if not ids:
        sys.exit("data/raw/에 rankings 파일이 없다")
    return ids[-1]


def read_jsonl(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f]


def outcome(row):
    rank = row["gold_rank"]
    return {
        "run_id": row["run_id"], "query_id": row["query_id"], "lang": row["lang"],
        "variant": row["variant"], "typo_type": row["typo_type"] or "",
        "condition": row["condition"], "n_gold": len(row["gold_doc_ids"]),
        "gold_rank": "" if rank is None else rank,
        "hit_at_10": int(rank is not None and rank <= TOP_K),
        "hit_at_1": int(rank == 1),
        "n_query_tokens": row["n_query_tokens"], "n_retrieved": row["n_retrieved"],
    }


def write_csv(path, fields, rows):
    with open(path, "w", encoding="utf-8", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=fields, lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def main():
    run_id = latest_run_id()
    rankings = read_jsonl(os.path.join(RAW_DIR, f"rankings-{run_id}.jsonl"))
    index = read_jsonl(os.path.join(RAW_DIR, f"index-{run_id}.jsonl"))
    os.makedirs(PROCESSED_DIR, exist_ok=True)
    rows = sorted((outcome(r) for r in rankings),
                  key=lambda r: (r["condition"], r["query_id"], r["variant"]))
    write_csv(os.path.join(PROCESSED_DIR, "outcomes.csv"), OUTCOME_FIELDS, rows)
    write_csv(os.path.join(PROCESSED_DIR, "index.csv"), INDEX_FIELDS,
              [{k: r[k] for k in INDEX_FIELDS} for r in index])
    print(f"run_id {run_id}: outcomes {len(rows)}")


if __name__ == "__main__":
    main()
