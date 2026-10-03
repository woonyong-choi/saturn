"""정규화된 private 행에서 사전 등록 지표와 공개 보고서를 만든다."""

from __future__ import annotations

import csv
import json
import math
import statistics
import sys
from collections import defaultdict
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import PRIVATE_ROOT, PUBLIC_ROOT, read_jsonl, write_json  # noqa: E402
from _03_import import load_consensus  # noqa: E402

Z95 = 1.959963984540054


def wilson(k: int, n: int) -> tuple[float | None, float | None]:
    if not n:
        return None, None
    p = k / n
    denominator = 1 + Z95 * Z95 / n
    center = (p + Z95 * Z95 / (2 * n)) / denominator
    half = Z95 * math.sqrt(p * (1 - p) / n + Z95 * Z95 / (4 * n * n)) / denominator
    return center - half, center + half


def ratio(k: int, n: int) -> dict:
    low, high = wilson(k, n)
    return {"correct": k, "n": n, "value": round(k / n, 6) if n else None,
            "ci95_low": round(low, 6) if low is not None else None,
            "ci95_high": round(high, 6) if high is not None else None}


def band(value: float | None) -> str:
    if value is None or value < 0.5:
        return "<0.5"
    if value < 0.7:
        return "0.5-0.7"
    if value < 0.8:
        return "0.7-0.8"
    return ">=0.8"


def answer(row: dict, question_id: str) -> float | None:
    value = ((row.get("answers") or {}).get(question_id) or {}).get("noul")
    return float(value) if isinstance(value, (int, float)) and 0 <= value <= 1 else None


def write_csv(path: Path, rows: list[dict]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fields = sorted({key for row in rows for key in row}) if rows else ["n"]
    with path.open("w", encoding="utf-8", newline="") as file:
        writer = csv.DictWriter(file, fieldnames=fields)
        writer.writeheader()
        writer.writerows(rows)


def main() -> int:
    processed_path = PRIVATE_ROOT / "processed.jsonl"
    if not processed_path.exists():
        raise RuntimeError("processed.jsonl이 없다: 04-process.py를 먼저 실행한다")
    rows = read_jsonl(processed_path)
    turn_rows = [row for row in rows if row.get("kind") == "turn" and row.get("is_gold") is not None]
    by_band = {}
    for length_band in sorted({row["length_band"] for row in turn_rows}):
        members = [row for row in turn_rows if row["length_band"] == length_band]
        predicted = [row for row in members if row["is_answer"] is not None and row["is_predicted"]]
        positives = [row for row in members if row["is_gold"]]
        tp = sum(row["is_gold"] for row in predicted)
        by_band[length_band] = {
            "n": len(members),
            "precision": ratio(tp, len(predicted)),
            "recall": ratio(tp, len(positives)),
            "accuracy": ratio(sum(row["is_predicted"] == row["is_gold"] for row in members), len(members)),
        }
    all_predicted = [row for row in turn_rows if row["is_answer"] is not None and row["is_predicted"]]
    all_positive = [row for row in turn_rows if row["is_gold"]]
    all_tp = sum(row["is_gold"] for row in all_predicted)
    registration = {"precision": ratio(all_tp, len(all_predicted)), "recall": ratio(all_tp, len(all_positive))}

    transition_rows = []
    for row in rows:
        if row.get("kind") != "turn" or row.get("is_gold") is not True:
            continue
        for index, candidate_id in enumerate(row.get("candidate_ids", []), start=1):
            gold = row.get("candidate_gold_relations", {}).get(candidate_id)
            if gold is None:
                continue
            probability = answer(row, f"replaces_{index}")
            predicted = "replace" if probability is not None and probability >= 0.8 else "partial" if probability is not None and probability >= 0.5 else "compatible"
            transition_rows.append({"condition": row["condition"], "length_band": row["length_band"], "candidate_id": candidate_id, "probability": probability, "predicted": predicted, "gold": gold, "correct": predicted == gold})
    transition = {}
    for condition in sorted({row["condition"] for row in transition_rows}):
        members = [row for row in transition_rows if row["condition"] == condition]
        transition[condition] = ratio(sum(row["correct"] for row in members), len(members))

    final_set_rows = []
    for row in rows:
        if row.get("kind") != "keep" or not row.get("gold_final_ids"):
            continue
        candidates = set(row["candidate_ids"])
        gold = set(row["gold_final_ids"])
        predicted = {candidate for index, candidate in enumerate(row["candidate_ids"], start=1) if (answer(row, f"keep_{index}") or 0) >= 0.7}
        union = predicted | gold
        final_set_rows.append({
            "condition": row["condition"],
            "candidate_accuracy": sum((candidate in predicted) == (candidate in gold) for candidate in candidates) / len(candidates) if candidates else None,
            "exact_match": predicted == gold,
            "jaccard": len(predicted & gold) / len(union) if union else 1.0,
        })
    final_set = {
        condition: {
            "candidate_accuracy": ratio(round(sum(row["candidate_accuracy"] for row in final_set_rows if row["condition"] == condition), 6), len([row for row in final_set_rows if row["condition"] == condition])) if [row for row in final_set_rows if row["condition"] == condition] else ratio(0, 0),
            "exact_match": ratio(sum(row["exact_match"] for row in final_set_rows if row["condition"] == condition), len([row for row in final_set_rows if row["condition"] == condition])),
            "jaccard_mean": statistics.mean([row["jaccard"] for row in final_set_rows if row["condition"] == condition]) if [row for row in final_set_rows if row["condition"] == condition] else None,
        }
        for condition in sorted({row["condition"] for row in final_set_rows})
    }

    confidence = {}
    for name, value_getter, correct_getter in (
        ("registration", lambda row: row.get("is_answer"), lambda row: row.get("is_predicted") == row.get("is_gold")),
        ("transition", lambda row: row.get("probability"), lambda row: row.get("correct")),
    ):
        confidence[name] = {}
        source = turn_rows if name == "registration" else transition_rows
        for name_band in ("<0.5", "0.5-0.7", "0.7-0.8", ">=0.8"):
            members = [row for row in source if band(value_getter(row)) == name_band]
            confidence[name][name_band] = ratio(sum(correct_getter(row) for row in members), len(members))

    statuses = {status: sum(row.get("status") == status for row in rows) for status in sorted({row.get("status") for row in rows})}
    summary = {
        "run_id": rows[0].get("trial_id") if rows else None,
        "calls": {"jev": len(rows), "label_models_max": 2},
        "label_agreement": {"conversations": len(load_consensus()), "turns": len(turn_rows)},
        "registration": registration,
        "length_bands": by_band,
        "transition": transition,
        "final_set": final_set,
        "confidence_bands": confidence,
        "statuses": statuses,
        "exploratory": {"transition_rows": len(transition_rows), "final_set_rows": len(final_set_rows)},
    }
    write_json(PUBLIC_ROOT / "results" / "summary.json", summary)
    write_csv(PUBLIC_ROOT / "results" / "tables" / "length-bands.csv", [{"length_band": key, **value["accuracy"]} for key, value in by_band.items()])
    write_csv(PUBLIC_ROOT / "results" / "tables" / "transitions.csv", [{"condition": key, **value} for key, value in transition.items()])
    write_csv(PUBLIC_ROOT / "results" / "tables" / "final-set.csv", [{"condition": key, **value} for key, value in final_set.items()])
    write_csv(PUBLIC_ROOT / "results" / "tables" / "confidence-bands.csv", [{"kind": kind, "band": key, **value} for kind, bands in confidence.items() for key, value in bands.items()])

    report = """# 실제 대화 기록의 제약 판단 품질: 실험 결과

## 요약

실제 수집 결과는 `results/summary.json`에 기록했다. 라벨 합의율과 길이별 등록 지표, 조건별 전이·보존 지표를 함께 보고한다. 설계의 확인 분석은 합의된 항목만 사용했고, 불일치 항목은 별도 상태로 남겼다.

## 방법

| 항목 | 값 |
|---|---|
| 설계 | [실험 설계](design.md) |
| 실행 id | `results/summary.json`의 `run_id` |
| 환경 | [env.json](env.json) |
| 표본 | 설계 최대 6개 대화·120개 턴, 실제 값은 `summary.json` |

## 설계와 다른 점

실행 뒤 확인한다. `summary.json`의 호출 수와 상태가 설계의 상한·제외 규칙과 다른 경우 이 절에 이유와 영향을 기록한다.

## 결과

### 흐름

| 단계 | 수 |
|---|---|
| 라벨 대화 | `summary.json`의 `label_agreement.conversations` |
| Jev 응답 | `summary.json`의 `calls.jev` |
| 전이 분석 행 | `summary.json`의 `exploratory.transition_rows` |
| 보존 분석 행 | `summary.json`의 `exploratory.final_set_rows` |

### 확인 분석

수치는 [summary.json](results/summary.json)과 [결과 표](results/tables/)에서 읽는다. H1~H4의 채택·기각·보류는 설계의 신뢰구간 기준으로 판정한다.

### 탐색 분석

반복 일치율, 확신 구간별 정확도, 제약 사이 거리별 정확도, 로그 후보 문구의 동작·식별자 누락을 보고한다.

## 논의

### 해석

실제 대화 표본에서 확인된 등록·전이·보존 품질을 설계 범위 안에서 해석한다. 합의 라벨과 표본 수가 제한되므로 전체 Claude Code 사용자 대화로 일반화하지 않는다.

### 타당성 위협

| 종류 | 위협 | 이 실험에서 |
|---|---|---|
| 내적 | 라벨러 불일치가 정답에 섞일 수 있다. | 불일치율과 제외 수를 함께 보고했다. |
| 구성 | 자동 삽입 제외와 후보 겹침 구현이 기록의 구조에 의존한다. | 가림·추출 테스트와 후보 목록을 private에 남겼다. |
| 외적 | 표본이 한 사용자의 기록에 한정된다. | 결과 적용 범위를 해당 표본으로 제한했다. |

### 한계

- 공개 결과에서 원문을 검증할 수 없다.
- Jev와 라벨러 응답은 실행마다 달라질 수 있다.
- `keep_<n>`은 측정 전 실험 질문이며 구현 계약이 아니다.

## 재현

```sh
./run.sh verify
./run.sh analyze
```

| 파일 | SHA-256 |
|---|---|
| `data/SHA256SUMS` | 실행 뒤 기록한다. |

## 결론

| 가설 | 판정 | 반영한 문서 |
|---|---|---|
| H1 | `summary.json`의 길이별 등록 지표로 판정 | [맥락 고르기](../../design/context-selection.md) |
| H2 | `summary.json`의 길이별 등록 지표로 판정 | [맥락 고르기](../../design/context-selection.md) |
| H3 | `summary.json`의 조건별 전이 지표로 판정 | [맥락 고르기](../../design/context-selection.md) |
| H4 | `summary.json`의 조건별 보존 지표로 판정 | [맥락 고르기](../../design/context-selection.md) |
| H5 | 탐색 분석 | 없음 |
| H6 | 탐색 분석 | 없음 |
"""
    (PUBLIC_ROOT / "report.md").write_text(report, encoding="utf-8")
    print(f"분석 끝: 결과 {PUBLIC_ROOT / 'results' / 'summary.json'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
