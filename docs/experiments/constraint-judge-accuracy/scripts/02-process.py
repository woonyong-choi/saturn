"""data/raw/의 답을 평가 세트 라벨과 이어 data/processed/에 표로 만든다.

사용: python3 scripts/02-process.py (실험 폴더에서 실행)
raw 파일이 여럿이면 실행 id가 가장 늦은 파일 하나만 쓴다.
"""
import csv
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EVAL = ROOT / "eval"
RAW = ROOT / "data" / "raw"
PROCESSED = ROOT / "data" / "processed"
CONSTRAINT_THRESHOLD = 0.7
REPLACE_THRESHOLD = 0.8
POSSIBLE_THRESHOLD = 0.5


def read_jsonl(path: Path) -> list[dict]:
    with path.open(encoding="utf-8") as file:
        return [json.loads(line) for line in file if line.strip()]


def band(answer: float | None) -> str:
    """판단이 없으면 대체 규칙대로 기록 없음 구간이다."""
    if answer is None or answer < POSSIBLE_THRESHOLD:
        return "none"
    if answer < REPLACE_THRESHOLD:
        return "possible"
    return "replace"


def write_csv(path: Path, header: list[str], rows: list[list]) -> None:
    with path.open("w", encoding="utf-8", newline="") as file:
        writer = csv.writer(file, lineterminator="\r\n")
        writer.writerow(header)
        for row in rows:
            writer.writerow(["" if value is None else value for value in row])


def main() -> int:
    raws = sorted(RAW.glob("*.jsonl"))
    if not raws:
        print("data/raw/에 수집 파일이 없다: ./run.sh collect를 먼저 실행한다", file=sys.stderr)
        return 2
    trials = {(row["condition"], row["item_id"]): row for row in read_jsonl(raws[-1])}
    PROCESSED.mkdir(parents=True, exist_ok=True)

    input_rows = []
    for item in read_jsonl(EVAL / "inputs.jsonl"):
        trial = trials.get(("is_constraint", item["id"]))
        answer = trial["answer"] if trial else None
        predicted = answer is not None and answer >= CONSTRAINT_THRESHOLD
        input_rows.append([
            item["id"], trial["trial_id"] if trial else None, item["category"], int(item["indirect"]),
            int(item["label"]), trial["status"] if trial else "missing", answer, int(predicted),
        ])
    write_csv(
        PROCESSED / "inputs.csv",
        ["item_id", "trial_id", "category", "indirect", "label", "status", "answer", "predicted"],
        input_rows,
    )

    pair_rows = []
    for item in read_jsonl(EVAL / "pairs.jsonl"):
        trial = trials.get(("replaces", item["id"]))
        answer = trial["answer"] if trial else None
        pair_rows.append([
            item["id"], trial["trial_id"] if trial else None, item["category"], int(item["indirect"]),
            item["label"], trial["status"] if trial else "missing", answer, band(answer),
        ])
    write_csv(
        PROCESSED / "pairs.csv",
        ["item_id", "trial_id", "category", "indirect", "label", "status", "answer", "band"],
        pair_rows,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
