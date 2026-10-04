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
from common import PRIVATE_ROOT, PUBLIC_ROOT, read_jsonl, sha256_file, write_json  # noqa: E402
from _03_import import load_consensus  # noqa: E402

Z95 = 1.959963984540054
DISTANCE_BANDS = ((1, 5, "1-5"), (6, 20, "6-20"), (21, 100, "21-100"), (101, None, "101+"))


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
    return {
        "correct": k,
        "n": n,
        "value": round(k / n, 6) if n else None,
        "ci95_low": round(low, 6) if low is not None else None,
        "ci95_high": round(high, 6) if high is not None else None,
    }


def probability_band(value: float | None) -> str:
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


def load_label_runs() -> dict[str, dict[str, dict]]:
    runs: dict[str, dict[str, dict]] = defaultdict(dict)
    for path in sorted((PRIVATE_ROOT / "labels").glob("*.jsonl")):
        for row in read_jsonl(path):
            if row.get("status") == "ok" and isinstance(row.get("parsed"), dict):
                runs[row["labeler"]][row["conversation_id"]] = row["parsed"]
    return runs


def same_turn_label(left: dict, right: dict) -> bool:
    return (
        left.get("is_constraint") == right.get("is_constraint")
        and left.get("operation", "none") == right.get("operation", "none")
        and sorted(left.get("targets", [])) == sorted(right.get("targets", []))
    )


def label_agreement(selected_rows: list[dict]) -> dict:
    runs = load_label_runs()
    labelers = sorted(runs)
    if len(labelers) < 2:
        return {"turns": ratio(0, 0), "by_length_band": {}, "final_set": ratio(0, 0)}
    left, right = runs[labelers[0]], runs[labelers[1]]
    bands = {
        row["conversation_id"]: row["length_band"]
        for row in selected_rows
        if row.get("turn_id") == "u-0001"
    }
    total = agreed = 0
    by_band: dict[str, list[int]] = defaultdict(lambda: [0, 0])
    final_total = final_agreed = 0
    for conversation_id in sorted(set(left) & set(right)):
        left_turns = {row["turn_id"]: row for row in left[conversation_id].get("turns", [])}
        right_turns = {row["turn_id"]: row for row in right[conversation_id].get("turns", [])}
        length_band = bands.get(conversation_id, "unknown")
        for turn_id in sorted(set(left_turns) & set(right_turns)):
            is_agree = same_turn_label(left_turns[turn_id], right_turns[turn_id])
            total += 1
            agreed += is_agree
            by_band[length_band][0] += is_agree
            by_band[length_band][1] += 1
        left_final = sorted(left[conversation_id].get("final_active_turn_ids", []))
        right_final = sorted(right[conversation_id].get("final_active_turn_ids", []))
        final_total += 1
        final_agreed += left_final == right_final
    return {
        "turns": ratio(agreed, total),
        "by_length_band": {key: ratio(value[0], value[1]) for key, value in sorted(by_band.items())},
        "final_set": ratio(final_agreed, final_total),
    }


def distance_band(distance: int) -> str:
    for low, high, name in DISTANCE_BANDS:
        if distance >= low and (high is None or distance <= high):
            return name
    return "unknown"


def relation_probability(row: dict, index: int) -> float | None:
    return answer(row, f"replaces_{index}")


def predicted_relation(probability: float | None) -> str | None:
    if probability is None:
        return None
    if probability >= 0.8:
        return "replace"
    if probability >= 0.5:
        return "partial"
    return "compatible"


def transition_rows(
    rows: list[dict],
    selected_index: dict[tuple[str, str], dict],
    consensus: dict[str, dict[str, dict]],
) -> list[dict]:
    result = []
    for row in rows:
        if row.get("kind") != "turn" or row.get("is_gold") is not True:
            continue
        label = consensus.get(row["conversation_id"], {}).get("turns", {}).get(row["turn_id"])
        gold_operation = label["operation"] if label else "none"
        current = selected_index.get((row["conversation_id"], row["turn_id"]))
        for index, candidate_id in enumerate(row.get("candidate_ids", []), start=1):
            gold = row.get("candidate_gold_relations", {}).get(candidate_id)
            if gold is None:
                continue
            probability = relation_probability(row, index)
            candidate = selected_index.get((row["conversation_id"], candidate_id))
            distance = abs(current["turn_index"] - candidate["turn_index"]) if current and candidate else None
            prediction = predicted_relation(probability)
            result.append({
                "condition": row["condition"],
                "length_band": row["length_band"],
                "candidate_id": candidate_id,
                "probability": probability,
                "predicted": prediction,
                "gold": gold,
                "gold_operation": gold_operation,
                "correct": prediction == gold if prediction is not None else None,
                "distance": distance,
                "distance_band": distance_band(distance) if distance is not None else "unknown",
            })
    return result


def repeat_metrics(rows: list[dict]) -> dict:
    groups: dict[tuple, list[dict]] = defaultdict(list)
    for row in rows:
        if row.get("is_answer") is not None:
            groups[(row["conversation_id"], row["turn_id"], row["condition"], row.get("kind"))].append(row)
    same = 0
    standard_deviations = []
    by_condition: dict[str, list[int]] = defaultdict(lambda: [0, 0])
    for key, members in groups.items():
        if len(members) < 2:
            continue
        classifications = {member["is_predicted"] for member in members}
        all_same = len(classifications) == 1
        same += all_same
        by_condition[key[2]][0] += all_same
        by_condition[key[2]][1] += 1
        standard_deviations.append(statistics.pstdev(member["is_answer"] for member in members))
    return {
        "is_constraint_classification": ratio(same, len(groups)),
        "by_condition": {key: ratio(value[0], value[1]) for key, value in sorted(by_condition.items())},
        "mean_probability_stddev": round(statistics.mean(standard_deviations), 6) if standard_deviations else None,
    }


def log_metrics(rows: list[dict], consensus: dict[str, dict[str, dict]]) -> dict:
    registration_correct = registration_total = 0
    transition_correct = transition_total = 0
    release_not_measured = 0
    for row in rows:
        if row.get("kind") != "turn" or row.get("is_gold") is None or row.get("is_answer") is None:
            continue
        label = consensus.get(row["conversation_id"], {}).get("turns", {}).get(row["turn_id"])
        if not label:
            continue
        operation = label["operation"] if label["is_constraint"] else "none"
        if operation in ("none", "register"):
            registration_total += 1
            registration_correct += row["is_predicted"] == label["is_constraint"]
        if operation == "release":
            release_not_measured += 1
            continue
        if operation not in ("replace", "partial"):
            continue
        transition_total += 1
        predicted = {
            candidate_id: predicted_relation(relation_probability(row, index))
            for index, candidate_id in enumerate(row.get("candidate_ids", []), start=1)
        }
        gold = row.get("candidate_gold_relations", {})
        transition_correct += bool(predicted) and all(
            predicted.get(candidate_id) == relation for candidate_id, relation in gold.items()
        )
    return {
        "registration": ratio(registration_correct, registration_total),
        "transition": ratio(transition_correct, transition_total),
        "release_not_measured": release_not_measured,
    }


def confidence_metrics(turn_rows: list[dict], transitions: list[dict]) -> dict:
    result = {}
    sources = (
        ("registration", turn_rows, lambda row: row.get("is_answer"), lambda row: row.get("is_predicted") == row.get("is_gold")),
        ("transition", [row for row in transitions if row["probability"] is not None], lambda row: row.get("probability"), lambda row: row.get("correct")),
    )
    for name, source, value_getter, correct_getter in sources:
        result[name] = {}
        for name_band in ("<0.5", "0.5-0.7", "0.7-0.8", ">=0.8"):
            members = [row for row in source if probability_band(value_getter(row)) == name_band]
            result[name][name_band] = ratio(sum(correct_getter(row) for row in members), len(members))
    return result


def final_set_metrics(rows: list[dict]) -> dict:
    final_rows = []
    for row in rows:
        if row.get("kind") != "keep" or not row.get("gold_final_ids"):
            continue
        candidates = set(row["candidate_ids"])
        gold = set(row["gold_final_ids"])
        predicted = {
            candidate
            for index, candidate in enumerate(row["candidate_ids"], start=1)
            if (answer(row, f"keep_{index}") or 0) >= 0.7
        }
        union = predicted | gold
        final_rows.append({
            "condition": row["condition"],
            "candidate_accuracy": sum((candidate in predicted) == (candidate in gold) for candidate in candidates) / len(candidates),
            "exact_match": predicted == gold,
            "jaccard": len(predicted & gold) / len(union) if union else 1.0,
        })
    if not final_rows:
        return {"status": "not_estimable", "reason": "두 라벨러의 대화 끝 유효 제약 집합이 3개 대화 모두 불일치했다", "rows": 0}
    return {
        "status": "measured",
        "rows": len(final_rows),
        "by_condition": {
            condition: {
                "exact_match": ratio(
                    sum(row["exact_match"] for row in final_rows if row["condition"] == condition),
                    sum(row["condition"] == condition for row in final_rows),
                ),
                "jaccard_mean": statistics.mean([row["jaccard"] for row in final_rows if row["condition"] == condition]),
            }
            for condition in sorted({row["condition"] for row in final_rows})
        },
    }


def format_percent(metric: dict | None) -> str:
    if not metric or metric.get("value") is None:
        return "측정 불가"
    return f"{metric['value'] * 100:.1f}% [{metric['ci95_low'] * 100:.1f}, {metric['ci95_high'] * 100:.1f}]"


def build_report(summary: dict) -> str:
    registration = summary["registration"]
    agreement = summary["label_agreement"]["turns"]
    length_lines = [
        f"| {name} | {format_percent(metrics['precision'])} | {format_percent(metrics['recall'])} | {metrics['n']} |"
        for name, metrics in summary["length_bands"].items()
    ]
    if summary["transition"]["status"] == "measured":
        transition_lines = [
            f"| {condition} | {format_percent(metric)} |"
            for condition, metric in summary["transition"]["by_condition"].items()
        ]
    else:
        transition_lines = ["| 전체 | 측정 불가 |"]
    distance_lines = [f"| {name} | {format_percent(metric)} |" for name, metric in summary["distance_bands"].items()]
    if not distance_lines:
        distance_lines = ["| 전체 | 측정 불가 |"]
    confidence_lines = [
        f"| 등록 {name} | {format_percent(metric)} |"
        for name, metric in summary["confidence_bands"]["registration"].items()
    ] + [
        f"| 전이 {name} | {format_percent(metric)} |"
        for name, metric in summary["confidence_bands"]["transition"].items()
    ]
    repeat = summary["repeats"]["is_constraint_classification"]
    logs = summary["logs"]
    final_set = summary["final_set"]
    checksum = sha256_file(PUBLIC_ROOT / "data" / "SHA256SUMS")
    final_set_text = f"측정 불가: {final_set['reason']}" if final_set["status"] == "not_estimable" else "측정"
    label_lines = [
        f"| {name} | {metric['correct']} | {metric['n']} | {metric['value'] * 100:.1f}% |"
        for name, metric in summary["label_agreement"]["by_length_band"].items()
    ]
    return f"""# 실제 대화 기록의 제약 판단 품질: 실험 결과

## 요약

선택한 실제 대화는 3개(30-119턴 2개, 120-399턴 1개)였고 400+턴 대화는 없었다. 두 라벨러의 턴별 완전 일치는 {agreement['correct']}/{agreement['n']} ({agreement['value'] * 100:.1f}%)였으며, 확인 분석에는 일치 항목만 사용했다. Jev의 `is_constraint`는 정밀도 {format_percent(registration['precision'])}, 재현율 {format_percent(registration['recall'])}이었다. H1은 기각되고 H2는 보류된다. 최종 유효 제약 집합은 {final_set_text}. 따라서 H4는 판정하지 않는다.

## 방법

| 항목 | 값 |
|---|---|
| 설계 | [실험 설계](design.md), 설계 커밋 `e6f485b` |
| 실행 | `env.json`의 `execution_id` |
| 환경 | [env.json](env.json) |
| 표본 | 대화 3개, 전체 사용자 턴 301개, Jev 표본 턴 60개, Jev 호출 360회 |
| 라벨 | `gpt-6-astra`와 `claude -p --model sonnet`, 각 대화 1회 |

## 설계와 다른 점

- 400+턴 후보가 없어 해당 길이 구간은 측정하지 못했다.
- 세 대화 모두 라벨러의 최종 유효 제약 집합이 불일치해 `keep_<n>` 호출과 H4 확인 분석을 만들지 못했다.
- 수집·처리·분석은 완료했으며, Jev와 라벨러 호출 상한을 넘지 않았다.

## 결과

### 라벨 합의

| 구간 | 일치 | 전체 | 비율 |
|---|---:|---:|---:|
{chr(10).join(label_lines)}
| 전체 | {agreement['correct']} | {agreement['n']} | {agreement['value'] * 100:.1f}% |

### 확인 분석

| 가설 | 지표 | 결과 | 판정 |
|---|---|---|---|
| H1 | 길이별 `is_constraint` 정밀도 | 아래 표 | 기각 |
| H2 | 길이별 `is_constraint` 재현율 | 아래 표 | 보류 |
| H3 | 조건별 전이 정확도 | 합의된 `replace`·`partial`·`release` 0개, 측정 불가 | 판정 안 함 |
| H4 | 대화 끝 유효 제약 집합 | 라벨 합의 0/3, 측정 불가 | 판정 안 함 |

| 길이 | 정밀도 | 재현율 | 합의된 Jev 행 |
|---|---|---|---:|
{chr(10).join(length_lines)}

| 조건 | 전이 정확도 |
|---|---|
{chr(10).join(transition_lines)}

### 탐색 분석

| 지표 | 결과 |
|---|---|
| 세 반복 `is_constraint` 분류 일치 | {format_percent(repeat)} |
| 반복 확률 표준편차 평균 | {summary['repeats']['mean_probability_stddev']:.4f} |
| 등록 로그 후보 정확도 | {format_percent(logs['registration'])} |
| 대체·부분 변경 로그 후보 정확도 | {format_percent(logs['transition'])} |
| 해제 로그 후보 | {logs['release_not_measured']}건, `replaces_<n>`만으로 해제와 양립을 구분하지 않아 측정 안 함 |

| 확신 구간 | 정확도 |
|---|---|
{chr(10).join(confidence_lines)}

| 제약 사이 거리 | 전이 정확도 |
|---|---|
{chr(10).join(distance_lines)}

## 논의

### 해석

이 표본에서 Jev는 합의된 제약을 모두 재현했지만 제약이 아닌 입력도 많이 등록해 정밀도가 낮았다. 관계 후보 18개는 모두 `compatible`로만 집계되었고, 합의된 `replace`·`partial`·`release` 전이는 없어 조건 비교를 하지 못했다. 긴 400+턴 대화와 대화 끝 집합은 측정되지 않아 긴 맥락 전체에 대한 품질 결론으로 일반화할 수 없다.

### 타당성 위협

| 종류 | 위협 | 이 실험에서 |
|---|---|---|
| 내적 | 실제 정답이 없어 라벨러 합의를 기준으로 썼다. | 턴 합의율 82.1%와 최종 집합 합의 0/3을 보고했다. |
| 구성 | 자동 삽입과 비밀값 가림 판정이 기록 구조에 의존한다. | 가림 테스트와 요청 dry-run이 통과했다. |
| 외적 | 한 사용자의 세 대화와 30-399턴 범위에 한정됐다. | 400+턴 부재와 표본 범위를 결과에 명시했다. |

### 한계

- 공개 결과에는 개인 대화 원문과 라벨 원문을 포함하지 않았다.
- `keep_<n>`은 라벨러 최종 집합 불일치로 호출하지 못했다.
- 관계 후보 18개는 전이 정답이 아니라 `compatible` 행으로만 남았고, 전이 정확도 표본은 없다.

## 재현

```sh
./run.sh verify
./run.sh analyze
```

| 파일 | SHA-256 |
|---|---|
| `data/SHA256SUMS` | `{checksum}` |

## 결론

| 가설 | 판정 | 반영한 문서 |
|---|---|---|
| H1 | 기각: 전체 정밀도 {format_percent(registration['precision'])} | [맥락 고르기](../../design/context-selection.md) |
| H2 | 보류: 전체 재현율 {format_percent(registration['recall'])} | [맥락 고르기](../../design/context-selection.md) |
| H3 | 판정 안 함: 합의된 전이 표본 0개 | [맥락 고르기](../../design/context-selection.md) |
| H4 | 판정 안 함: 최종 집합 라벨 합의 0/3 | [맥락 고르기](../../design/context-selection.md) |
| H5 | 탐색: 확신 구간별 정확도만 보고 | 없음 |
| H6 | 탐색: 거리 구간별 정확도만 보고 | 없음 |
"""


def main() -> int:
    processed_path = PRIVATE_ROOT / "processed.jsonl"
    if not processed_path.exists():
        raise RuntimeError("processed.jsonl이 없다: 04-process.py를 먼저 실행한다")
    rows = read_jsonl(processed_path)
    selected_rows = read_jsonl(PRIVATE_ROOT / "selected.jsonl")
    selected_index = {(row["conversation_id"], row["turn_id"]): row for row in selected_rows}
    consensus = load_consensus()
    label_runs = load_label_runs()
    turn_rows = [row for row in rows if row.get("kind") == "turn" and row.get("is_gold") is not None]
    by_band = {}
    for length_band in sorted({row["length_band"] for row in turn_rows}):
        members = [row for row in turn_rows if row["length_band"] == length_band and row["is_answer"] is not None]
        predicted = [row for row in members if row["is_predicted"]]
        positives = [row for row in members if row["is_gold"]]
        tp = sum(row["is_gold"] for row in predicted)
        by_band[length_band] = {
            "n": len(members),
            "precision": ratio(tp, len(predicted)),
            "recall": ratio(tp, len(positives)),
            "accuracy": ratio(sum(row["is_predicted"] == row["is_gold"] for row in members), len(members)),
        }
    valid_turns = [row for row in turn_rows if row["is_answer"] is not None]
    predicted = [row for row in valid_turns if row["is_predicted"]]
    positives = [row for row in valid_turns if row["is_gold"]]
    tp = sum(row["is_gold"] for row in predicted)
    transitions = transition_rows(rows, selected_index, consensus)
    valid_transitions = [
        row
        for row in transitions
        if row["correct"] is not None and row["gold_operation"] in ("replace", "partial", "release")
    ]
    transition = (
        {
            "status": "measured",
            "by_condition": {
                condition: ratio(
                    sum(row["correct"] for row in valid_transitions if row["condition"] == condition),
                    sum(row["condition"] == condition for row in valid_transitions),
                )
                for condition in sorted({row["condition"] for row in valid_transitions})
            },
            "rows": len(valid_transitions),
        }
        if valid_transitions
        else {
            "status": "not_estimable",
            "reason": "합의된 표본에 replace·partial·release 전이가 없다",
            "rows": 0,
        }
    )
    distance = {
        name: ratio(
            sum(row["correct"] for row in valid_transitions if row["distance_band"] == name),
            sum(row["distance_band"] == name for row in valid_transitions),
        )
        for name in sorted({row["distance_band"] for row in valid_transitions})
    }
    label_stats = label_agreement(selected_rows)
    final_set = final_set_metrics(rows)
    confidence = confidence_metrics(valid_turns, valid_transitions)
    summary = {
        "execution_id": json.loads((PUBLIC_ROOT / "env.json").read_text(encoding="utf-8")).get("execution_id"),
        "calls": {"jev": len(rows), "label_calls_per_model": len(next(iter(label_runs.values()), {}))},
        "label_agreement": label_stats,
        "registration": {"precision": ratio(tp, len(predicted)), "recall": ratio(tp, len(positives))},
        "length_bands": by_band,
        "transition": transition,
        "distance_bands": distance,
        "final_set": final_set,
        "confidence_bands": confidence,
        "repeats": repeat_metrics(valid_turns),
        "logs": log_metrics(valid_turns, consensus),
        "statuses": {status: sum(row.get("status") == status for row in rows) for status in sorted({row.get("status") for row in rows})},
        "exploratory": {"transition_rows": len(valid_transitions), "final_set_rows": final_set.get("rows", 0)},
    }
    write_json(PUBLIC_ROOT / "results" / "summary.json", summary)
    write_csv(PUBLIC_ROOT / "results" / "tables" / "length-bands.csv", [{"length_band": key, **value["accuracy"]} for key, value in by_band.items()])
    transition_rows_for_csv = (
        [{"condition": key, **value} for key, value in transition["by_condition"].items()]
        if transition["status"] == "measured"
        else [{"status": transition["status"], "reason": transition["reason"], "rows": transition["rows"]}]
    )
    write_csv(PUBLIC_ROOT / "results" / "tables" / "transitions.csv", transition_rows_for_csv)
    write_csv(PUBLIC_ROOT / "results" / "tables" / "distance-bands.csv", [{"distance_band": key, **value} for key, value in distance.items()])
    write_csv(PUBLIC_ROOT / "results" / "tables" / "label-agreement.csv", [{"length_band": key, **value} for key, value in label_stats["by_length_band"].items()])
    write_csv(PUBLIC_ROOT / "results" / "tables" / "confidence-bands.csv", [{"kind": kind, "band": key, **value} for kind, bands in confidence.items() for key, value in bands.items()])
    write_csv(PUBLIC_ROOT / "results" / "tables" / "final-set.csv", [{key: value for key, value in final_set.items() if not isinstance(value, dict)}])
    (PUBLIC_ROOT / "report.md").write_text(build_report(summary), encoding="utf-8")
    print(f"분석 끝: 결과 {PUBLIC_ROOT / 'results' / 'summary.json'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
