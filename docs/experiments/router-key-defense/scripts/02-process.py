"""raw/ 의 층별 최신 실행을 읽어 data/processed/trials.csv 로 합친다."""
import csv
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RAW = ROOT / "data" / "raw"
OUT = ROOT / "data" / "processed" / "trials.csv"
FIELDS = ["run_id", "trial_id", "layer", "condition", "ts_utc", "outcome", "exit_code", "bash_exit", "rep", "extra"]
KNOWN = set(FIELDS) - {"extra"}


def latest_by_layer():
    files = {}
    for path in sorted(RAW.glob("*.jsonl")):
        layer, _, run = path.stem.partition("-2")
        files[layer] = path  # 이름 순서가 실행 시각 순서
    return files


def main():
    OUT.parent.mkdir(parents=True, exist_ok=True)
    rows = []
    for path in latest_by_layer().values():
        for line in path.read_text(encoding="utf-8").splitlines():
            row = json.loads(line)
            extra = {k: v for k, v in row.items() if k not in KNOWN}
            row["extra"] = json.dumps(extra, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
            rows.append(row)
    with OUT.open("w", encoding="utf-8", newline="") as f:
        writer = csv.DictWriter(f, fieldnames=FIELDS, lineterminator="\n")
        writer.writeheader()
        for row in rows:
            writer.writerow({k: row.get(k) for k in FIELDS})


main()
