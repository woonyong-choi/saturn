"""수집 후 발견한 확률 합계 경계 오류를 원응답 변경 없이 재집계한다."""

from __future__ import annotations

import importlib.util
import json
import sys
from collections import Counter
from decimal import Decimal
from pathlib import Path

sys.dont_write_bytecode = True
PUBLIC = Path(__file__).resolve().parent
PRIVATE = (
    PUBLIC.parents[2] / ".local/experiments/constraint-registration-generalization"
)
SPEC = importlib.util.spec_from_file_location(
    "collector", PUBLIC / "scripts/02-collect.py"
)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("collector unavailable")
COLLECTOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(COLLECTOR)


def recovered_probabilities(record: dict, questions: dict) -> dict | None:
    if record["status"] == "ok":
        return record["probabilities"]
    if record["status"] != "schema_error":
        return None
    response = record.get("response", {})
    answers = response.get("answers", {})
    if response.get("model") != "jev-1.13.0" or set(answers) != set(questions):
        return None
    result = {}
    for arm in ("baseline", "korean"):
        answer = answers[arm]
        value = answer.get("noul")
        if answer.get("type") != "noul" or not COLLECTOR.valid_probability(value):
            return None
        result[arm] = value
    choice = answers["choice"]
    values = choice.get("probabilities", {})
    if choice.get("type") != "choice" or set(values) != set(
        questions["choice"]["criteria"]
    ):
        return None
    if choice.get("choice") not in values or not all(
        COLLECTOR.valid_probability(v) for v in values.values()
    ):
        return None
    exact_gap = abs(sum(Decimal(str(v)) for v in values.values()) - Decimal(1))
    float_gap = abs(sum(values.values()) - 1)
    if exact_gap > Decimal("0.01") or float_gap <= 0.01:
        return None
    result["choice"] = values["persistent"]
    return result


def main() -> None:
    questions = json.loads((PUBLIC / "questions.json").read_text())
    records = [
        json.loads(p.read_text()) for p in sorted((PRIVATE / "raw").glob("*.json"))
    ]
    accepted, recovered, sums = [], [], Counter()
    for record in records:
        probabilities = recovered_probabilities(record, questions)
        if probabilities is None:
            continue
        accepted.append(probabilities)
        if record["status"] != "ok":
            recovered.append(record["sample_id"])
            values = record["response"]["answers"]["choice"]["probabilities"].values()
            sums[str(sum(Decimal(str(v)) for v in values))] += 1
    summary = {
        "analysis": "post-hoc validation correction, not a change to model thresholds",
        "raw_records": len(records),
        "strict_valid": sum(r["status"] == "ok" for r in records),
        "recovered": len(recovered),
        "decimal_sums": dict(sums),
        "corrected_valid": len(accepted),
        "remaining_failures": len(records) - len(accepted),
        "raw_responses_modified": False,
        "repeated_api_calls": 0,
        "corrected_arms": [
            {
                "arm": arm,
                "auto_threshold": auto,
                "ask_threshold": ask,
                "auto_count": sum(p[arm] >= auto for p in accepted),
                "ask_count": sum(ask <= p[arm] < auto for p in accepted),
            }
            for arm, auto, ask in (
                ("baseline", 0.9, 0.7),
                ("korean", 0.77, 0.43),
                ("choice", 0.9, 0.7),
            )
        ],
    }
    (PUBLIC / "results/validation-audit.json").write_text(
        json.dumps(summary, indent=2) + "\n"
    )
    (PRIVATE / "recovered-validation-ids.json").write_text(json.dumps(recovered) + "\n")
    report = PUBLIC / "report.md"
    if report.exists():
        text = report.read_text()
        marker = "확률 합계의 부동소수점 경계 오류"
        if marker not in text:
            paragraph = (
                f"{marker}를 수집 중 확인했다. 확률 합계가 십진수로 0.99인 응답도 "
                "이진 부동소수점 차이는 0.010000000000000009여서 기존 검증기가 거부했다. "
                f"사후 재집계에서 {len(recovered)}건을 복구해 유효 응답은 {len(accepted):,}건이다. "
                "이는 정답 여부와 무관한 검증기 수정이다. 원래 실패 표시는 유지하고 원응답 수정이나 재호출은 하지 않았다. "
                "[검증기 재집계](results/validation-audit.json)에 보정한 후보 수를 별도로 기록했다. "
                "H4 표는 사전에 고정한 검증기로 얻은 결과다.\n\n"
                "앞선 [400건 탐색](pilot.md)은 새 4,000건의 성능 근거로 합치지 않았다. "
                "프로젝트 단위는 경로 식별자이며 별도 worktree가 같은 논리 프로젝트에 속할 수 있다."
            )
            text = text.replace("\n## 결과\n", "\n" + paragraph + "\n\n## 결과\n", 1)
            text = text.replace("유효 응답은 ", "최초 검증의 유효 응답은 ", 1)
            text = text.replace(
                "독립 정답이 없어 90% 정밀도 달성 여부는 미평가다.",
                f"검증기 경계 오류를 보정한 유효 응답은 {len(accepted):,}건이다. "
                "독립 정답이 없어 90% 정밀도 달성 여부는 미평가다.",
                1,
            )
            korean = next(
                arm for arm in summary["corrected_arms"] if arm["arm"] == "korean"
            )
            text = text.replace(
                "\n### 지연과 비용\n",
                f"\n검증기 오류 보정 후 한국어 질문의 자동 후보는 {korean['auto_count']}건, "
                f"확인 후보는 {korean['ask_count']}건이다. 이 개수는 정답 수가 아니다. "
                "4,000건을 호출해도 사전 H1의 자동 예측 양성 최소 100건 조건을 채우지 못했다.\n"
                "\n### 지연과 비용\n",
                1,
            )
            report.write_text(text)
    print(json.dumps(summary), flush=True)


if __name__ == "__main__":
    main()
