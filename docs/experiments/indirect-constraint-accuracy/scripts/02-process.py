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
INPUT_CONDITIONS = ("baseline", "flag", "flag_text", "oracle_flag", "flag_hint")


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
    inputs = read_jsonl(EVAL / "inputs.jsonl")
    PROCESSED.mkdir(parents=True, exist_ok=True)

    registration_rows = []
    for text in sorted({x["previous"] for x in inputs}):
        trial = trials.get(("registration", text))
        answer = trial["answer"] if trial else None
        # 앞 입력의 정답: 간접 제약 입력의 앞 입력만 제약이고 나머지는 요청이다.
        owners = [x for x in inputs if x["previous"] == text]
        label = int(any(x["category"] == "indirect-constraint" for x in owners))
        registration_rows.append([
            text, trial["trial_id"] if trial else None, trial["status"] if trial else "missing", answer,
            int(answer is not None and answer >= CONSTRAINT_THRESHOLD), label,
        ])
    write_csv(
        PROCESSED / "registrations.csv",
        ["previous_text", "trial_id", "status", "answer", "registered", "label"],
        registration_rows,
    )

    input_rows = []
    for item in inputs:
        for condition in INPUT_CONDITIONS:
            trial = trials.get((condition, item["id"]))
            answer = trial["answer"] if trial else None
            predicted = answer is not None and answer >= CONSTRAINT_THRESHOLD
            input_rows.append([
                item["id"], trial["trial_id"] if trial else None, condition, item["category"], int(item["indirect"]),
                item["origin"], int(item["label"]), trial["status"] if trial else "missing", answer, int(predicted),
                trial["latency_ms"] if trial else None,
            ])
    write_csv(
        PROCESSED / "inputs.csv",
        ["item_id", "trial_id", "condition", "category", "indirect", "origin", "label", "status", "answer",
         "predicted", "latency_ms"],
        input_rows,
    )

    pair_rows = []
    for item in read_jsonl(EVAL / "pairs.jsonl"):
        trial = trials.get(("replaces", item["id"]))
        answer = trial["answer"] if trial else None
        pair_rows.append([
            item["id"], trial["trial_id"] if trial else None, item["category"], int(item["indirect"]),
            item["origin"], item["label"], trial["status"] if trial else "missing", answer, band(answer),
            trial["latency_ms"] if trial else None,
        ])
    write_csv(
        PROCESSED / "pairs.csv",
        ["item_id", "trial_id", "category", "indirect", "origin", "label", "status", "answer", "band", "latency_ms"],
        pair_rows,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
