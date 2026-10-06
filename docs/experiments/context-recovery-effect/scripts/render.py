"""집계 JSON의 수치만 사용해 진단 보고서 표를 만든다."""

from __future__ import annotations

import json
from pathlib import Path

EXP = Path(__file__).resolve().parents[1]


def main() -> None:
    summary = json.loads((EXP / "results/summary.json").read_text())
    rows = json.loads((EXP / "results/trials.json").read_text())
    text = f"""# 맥락 복구 순효과: 실험 결과

## 요약

사전 등록한 {summary["planned"]}회 중 {summary["collected"]}회의 원자료를 수집했다. 입력 규약 위반은 {len(summary["audit"]["protocol_invalid"])}회였다. 실제 접수는 {summary["audit"]["stored_inputs"]}개로 계획 상한 {summary["audit"]["planned_input_limit"]}개를 넘었다. 이 표본은 실패 원인을 구분하는 진단이며 제품 채택을 입증하지 않는다. 원문 조회와 Jev의 실제 조건별 성과는 아래 표와 [집계](results/summary.json)에 있다.

## 방법

| 항목 | 값 |
|---|---|
| 설계 | [실험 설계](design.md), 수집 전 최종 기준 fe0e5b2 |
| 실행 환경 | [env.json](env.json), 제품 기준 main 7413264 |
| 표본 | 신규 시드 101~104, 두 provider, 다섯 조건 |
| 원자료 | 로컬 보존, 공개 파일별 해시는 [SHA256SUMS](data/SHA256SUMS) |

## 설계와 다른 점

사전 설계의 조건·시드·판정 기준은 formal 수집 뒤 변경하지 않았다. 예비 실행은 별도 보존하고 formal에서 제외했다. 분석기에 전체 40회 수집 확인 게이트를 추가했다. 수집 도중 rescue의 여러 줄 입력이 CLI에서 별도 입력으로 접수되는 문제가 발견됐다. F2 채점은 순서 대신 원래 질문과 일치하는 입력 ID로 수정했다. 원자료와 원래 실행은 그대로 보존하고 rescue는 의도한 단일 입력 보강 비교에서 무효로 표시한다. 가설과 채택 기준은 바꾸지 않았다. 사전 설계 문서의 금지 낱말만 같은 뜻으로 교정했다. 기본 압축 호출의 사용량이 0 또는 이전 누적값으로 보고되는 경우가 확인돼 그 값은 압축 비용 미계측으로 분류했다. 관측 가중 비용은 따로 남기고 전체 비용 판정은 보류했다.

## 결과

### 흐름

| provider | 조건 | 수집 | 전체 성공 [Wilson 95%] | 미완료 | 미개입 | 사용량 결측 | 입력 규약 위반 |
|---|---|---:|---:|---:|---:|---:|---:|
"""
    for provider, block in summary["providers"].items():
        for arm, entry in block["arms"].items():
            text += f"| {provider} | {arm} | {entry['collected']} | {entry['success']}/{entry['planned']} [{entry['wilson95'][0] * 100:.1f}, {entry['wilson95'][1] * 100:.1f}]% | {entry['incomplete']} | {entry['no_intervention']} | {entry['missing_usage']} | {entry['protocol_invalid']} |\n"
    text += "\n| provider | 조건 | F1 | F2 | F3 | 최종 헤더 보존 |\n|---|---|---:|---:|---:|---:|\n"
    for provider, block in summary["providers"].items():
        for arm, entry in block["arms"].items():
            n = entry["planned"]
            text += f"| {provider} | {arm} | {entry['f1_success']}/{n} | {entry['f2_success']}/{n} | {entry['f3_success']}/{n} | {entry['header_success']}/{n} |\n"
    text += "\n### 확인 분석\n\n가중 토큰은 실제 토큰 개수나 청구 금액이 아니다. cache read 0.1, Claude cache write 1.25, 출력 5, router 원시 토큰 1의 가정이다. 미완료 실행은 종료까지 발생한 비용이며 작업을 끝낸 비용으로 해석하지 않는다. 기본 압축 작업의 사용량이 계측되지 않은 경우 아래 관측 합계에는 그 비용이 빠져 있다. 전체 비용 비교 표에서는 결측으로 표시한다.\n\n| provider | 조건 | 보고된 가중 비용 평균 | 보고된 원시 토큰 합 평균 | 경계 뒤 초 평균 | 조회 합계 |\n|---|---|---:|---:|---:|---:|\n"

    def mean(entry: dict, name: str) -> str:
        value = entry[name]["mean"]
        return "결측" if value is None else f"{value:,.1f}"

    for provider, block in summary["providers"].items():
        for arm, entry in block["arms"].items():
            text += f"| {provider} | {arm} | {mean(entry, 'reported_weighted_cost')} | {mean(entry, 'reported_raw_token_total')} | {mean(entry, 'window_seconds')} | {entry['lookup_count']['sum']} |\n"
    text += "\n| provider | 비교 | 품질 차이 [95% 구간] | X만/Y만 성공 | 가중 비용 차이 [95% 구간] | 판정 |\n|---|---|---|---|---|---|\n"
    for provider, block in summary["providers"].items():
        for pair in block["comparisons"]:
            quality = [pair["quality_diff"], *pair["quality_ci95"]]
            cost = pair["metrics"]["total_cost"]
            cost_text = (
                "결측"
                if cost["mean"] is None
                else f"{cost['mean']:,.1f} [{cost['ci95'][0]:,.1f}, {cost['ci95'][1]:,.1f}]"
            )
            text += f"| {provider} | {pair['x']} − {pair['y']} | {quality[0] * 100:.1f}%p [{quality[1] * 100:.1f}, {quality[2] * 100:.1f}] | {pair['quality_counts'][1]}/{pair['quality_counts'][2]} | {cost_text} | {pair['decision']} |\n"
    failed = [r for r in rows if not r["complete"]]
    text += "\n### 실패한 실행\n\n"
    for row in failed:
        text += f"- {row['trial']}: {row.get('turn_statuses', '시작 실패')}\n"
    if not failed:
        text += "미완료 실행은 없었다.\n"
    text += """
## 논의

### 해석

rescue의 여러 줄 보강은 실제로 여러 입력과 실행으로 나뉘었다. 표의 값은 발생한 동작을 기록한 것이며 사전 설계한 단일 입력 보강의 효과로 해석하지 않는다. 이 조건은 원인 진단 비교에도 사용할 수 없다.

조회 설정을 켠 것과 실제로 조회해서 근거를 되찾은 것을 구분한다. 조회 횟수와 개별 결과는 trials.json에 있다. 본문의 성공률은 중단까지 포함한 실제 작업 완료율이며 모델의 기억 능력만의 점수가 아니다.

### 타당성 위협

| 종류 | 위협 | 이 실험에서 |
|---|---|---|
| 내적 | 모델 초기 응답과 서버·cache 상태가 다르다. | 입력과 시작 저장소는 같고 순서를 섞었지만 동일한 provider 내부 상태를 복제한 실험은 아니다. |
| 구성 | 원시 토큰과 가격이 다르다. | 비용은 가중 지표로 한정하며 실제 요금 절감을 주장하지 않는다. |
| 구성 | 승인 대기·중단은 품질과 비용에 동시에 영향을 준다. | 실행 실패를 보존하며 중단된 작업의 낮은 비용을 효율 개선으로 해석하지 않는다. |
| 외적 | 한 종류의 합성 서비스와 네 시드다. | 실제 프로젝트와 다른 모델·기본 압축 시점에 일반화하지 않는다. |

### 한계

- 같은 session 내부의 과거 기록 편집은 측정하지 않았다.
- 품질 저하가 없음을 일반적으로 입증할 표본 크기가 아니다.
- 원문 조회를 켠 결과만으로 Jev 선별 일관성 개선을 입증하지 않는다.
- 품질 구간은 두 불일치 비율에 Clopper–Pearson 정확 이항 구간을 구하고 Bonferroni 경계를 적용한 보수적인 구간이다. 이항 구간의 정의는 [SciPy 공식 문서](https://docs.scipy.org/doc/scipy/reference/generated/scipy.stats._result_classes.BinomTestResult.proportion_ci.html)를 따른다. 작은 표본에서 구간이 넓다는 사실을 숨기지 않는다.

## 재현

```sh
./run.sh analyze
./run.sh verify
```

원자료가 없는 환경에서는 verify를 통과했다고 표시하지 않는다. 기존 context-net-effect의 검증은 기록된 Python 3.13에서 재집계·분석·반복 판단 모두 바이트 단위로 일치했다. Python 3.9에서는 소수점 말단과 반올림 차이가 있어 같은 바이트가 아니었다.

## 결론

이 진단으로 제품 자동 적용을 승인하지 않는다. 조건별 기각·보류와 실제 실패 경로를 보존하고, 후속 설계는 원문 손실·조회 실패·추가 비용을 구분한 근거에 한정한다. provider 기본 압축을 대체한다는 목표의 달성 여부와 진단 조건의 성공을 동일하게 표시하지 않는다.
"""
    (EXP / "report.md").write_text(text)


if __name__ == "__main__":
    main()
