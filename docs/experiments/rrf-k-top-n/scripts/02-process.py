"""data/raw/의 후보별 수치에서 채널 순위와 흐름 수를 만든다."""
import csv
import json
import sys
from collections import defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent.parent
RAW_DIR = HERE / "data" / "raw"
OUT_DIR = HERE / "data" / "processed"
KEEP_THRESHOLD = 0.5


def channel_rank(rows, value):
    """값이 큰 순, 같으면 기록 번호가 큰 순. 값이 0 이하인 후보는 순위가 없다."""
    ranked = sorted((r for r in rows if value(r) > 0), key=lambda r: (-value(r), -r["record_no"]))
    return {r["record_no"]: i for i, r in enumerate(ranked, start=1)}


def main():
    files = sorted(RAW_DIR.glob("*.jsonl"))
    if not files:
        print("02-process: data/raw/에 수집 파일이 없다", file=sys.stderr)
        return 2
    sets = defaultdict(list)
    replicas = defaultdict(list)
    for path in files:
        for line in path.open(encoding="utf-8"):
            row = json.loads(line)
            target = sets if row["repeat"] == 1 else replicas
            target[(row["run_id"], row["trial_id"])].append(row)
    flow = defaultdict(int)
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    with (OUT_DIR / "channel-ranks.csv").open("w", encoding="utf-8", newline="") as out:
        writer = csv.writer(out, lineterminator="\n")
        writer.writerow(["run_id", "trial_id", "session_id", "candidate_id", "record_no", "kept",
                         "has_paths", "rank_file", "rank_word", "rank_recent"])
        for key in sorted(sets):
            rows = sorted(sets[key], key=lambda r: r["record_no"])
            flow["collected"] += 1
            status = rows[0]["status"]
            if status != "ok":
                flow["excluded_" + status] += 1
                continue
            flow["analyzed"] += 1
            file_rank = channel_rank(rows, lambda r: r["file_overlap"])
            word_rank = channel_rank(rows, lambda r: r["bm25"])
            recent_rank = channel_rank(rows, lambda r: r["record_no"])
            for r in rows:
                n = r["record_no"]
                kept = max(r["p_call"], r["p_result"]) >= KEEP_THRESHOLD
                writer.writerow([r["run_id"], r["trial_id"], r["session_id"], r["candidate_id"], n,
                                 int(kept), int(r["has_paths"]), file_rank.get(n, ""),
                                 word_rank.get(n, ""), recent_rank[n]])
    with (OUT_DIR / "replicas.csv").open("w", encoding="utf-8", newline="") as out:
        writer = csv.writer(out, lineterminator="\n")
        writer.writerow(["trial_id", "candidate_id", "kept_first", "kept_second"])
        for key in sorted(replicas):
            second = {r["candidate_id"]: r for r in replicas[key]}
            for r in sorted(sets[key], key=lambda r: r["record_no"]):
                other = second.get(r["candidate_id"])
                if r["status"] != "ok" or other is None or other["status"] != "ok":
                    continue
                writer.writerow([r["trial_id"], r["candidate_id"],
                                 int(max(r["p_call"], r["p_result"]) >= KEEP_THRESHOLD),
                                 int(max(other["p_call"], other["p_result"]) >= KEEP_THRESHOLD)])
    with (OUT_DIR / "flow.csv").open("w", encoding="utf-8", newline="") as out:
        writer = csv.writer(out, lineterminator="\n")
        writer.writerow(["stage", "sets"])
        for stage in sorted(flow):
            writer.writerow([stage, flow[stage]])
    return 0


if __name__ == "__main__":
    sys.exit(main())
