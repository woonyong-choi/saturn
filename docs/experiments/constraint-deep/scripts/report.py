"""집계 JSON의 수치만 사용해 공개 결과 문서를 작성한다."""

from __future__ import annotations

from storage import PUBLIC


def percent(value: float | None) -> str:
    return "측정 불가" if value is None else f"{value * 100:.1f}%"


def bounds(values: list[float | None], unit: str = "%") -> str:
    low, high = values
    if low is None or high is None:
        return "측정 불가"
    return f"[{low * 100:.1f}, {high * 100:.1f}]{unit}"


def interval(metric: dict) -> str:
    if metric["value"] is None:
        return f"측정 불가 (n={metric['n']})"
    low, high = metric["ci95"]
    return f"{metric['k']}/{metric['n']}, {percent(metric['value'])} [{low * 100:.1f}, {high * 100:.1f}]"


def write_report(summary: dict) -> None:
    agreement = summary["label_agreement"]
    base = next(row for row in summary["threshold_curve"] if row["threshold"] == 0.8)
    alternative = summary["alternative"]
    final = next(row for row in summary["final_sets"] if row["threshold"] == 0.8)
    recommended = summary["recommended_threshold"]
    recommendation = (
        f"탐색 후보 기준값은 {recommended:.2f}다"
        if recommended is not None
        else "정밀도 하한과 신뢰구간 폭 조건을 함께 충족하는 기준값이 없어 확정 권장값은 없다"
    )
    candidate = next(
        (row for row in summary["threshold_curve"] if row["threshold"] == recommended),
        None,
    )
    candidate_evidence = (
        f"후보 {recommended:.2f}의 정밀도는 {interval(candidate['precision'])}, "
        f"재현율은 {interval(candidate['recall'])}, F1은 {percent(candidate['f1'])}, "
        f"전체 입력 묻기 비율은 {interval(candidate['all_input_ask_rate'])}다. "
        f"프로젝트 bootstrap 정밀도 구간은 {bounds(candidate['cluster_bootstrap']['precision_ci95'])}, "
        f"재현율 구간은 {bounds(candidate['cluster_bootstrap']['recall_ci95'])}다. "
        "독립 턴을 가정한 선택 조건을 통과한 탐색 후보이며 확정 권장값은 아니다."
        if candidate
        else "선택 조건을 통과한 후보가 없다."
    )
    curve = "\n".join(
        f"| {row['threshold']:.2f} | {interval(row['precision'])} | {interval(row['recall'])} | "
        f"{percent(row['f1'])} | {interval(row['all_input_ask_rate'])} |"
        for row in summary["threshold_curve"]
    )
    sample = "\n".join(
        f"| {band} | {summary['selection']['conversations_by_band'].get(band, 0)} | {count} |"
        for band, count in summary["selection"]["turns_by_band"].items()
    )
    confidence = "\n".join(
        f"| {row['low']:.1f}~{row['high']:.1f} | {interval(row['precision'])} | "
        f"{'충족' if row['meets_half_width'] else '미충족'} | {row['worst_case_n_shortfall']} |"
        for row in summary["confidence_bands"]
    )
    hypotheses = "\n".join(
        f"| {name} | {row['metric']} | {interval(row)} | {row['p_value']:.6g} | "
        f"{row['holm_p_value']:.6g} | {row['decision']} |"
        for name, row in summary["hypotheses"].items()
    )
    strata = "\n".join(
        f"| {kind} | {name} | {interval(row['accuracy'])} | {interval(row['precision'])} |"
        for kind, values in summary["strata"].items()
        for name, row in values.items()
    )
    relations = summary["relations"]
    relation_table = "\n".join(
        f"| {name} | {interval(row['accuracy'])} | {interval(row['precision'])} | {interval(row['recall'])} |"
        for name, row in relations["by_operation"].items()
    )
    distance = "\n".join(
        f"| {band} | {interval(row)} |"
        for band, row in relations["by_distance"].items()
    )
    responses = "\n".join(
        f"| {row['kind']} | {row['status']} | {row['http_status']} | {row['calls']} | "
        f"{row['request_bytes_min']} / {row['request_bytes_median']} / {row['request_bytes_max']} |"
        for row in summary["response_diagnostics"]
    )
    bootstrap = base["cluster_bootstrap"]
    text = f"""# 실제 대화 기록 전체의 제약 판단: 실험 결과

## 요약

프로젝트 대화 {summary["selected_projects"]}개, 사용자 입력 {summary["selected_turns"]}턴을 측정했다. 최초 두 라벨러의 턴 전이 완전 일치는 {interval(agreement["turn_full"])}, 끝 집합 일치는 {interval(agreement["final_set"])}였다. 기준값 0.80에서 정밀도는 {interval(base["precision"])}, 재현율은 {interval(base["recall"])}다. {recommendation}. 현재 제품 기본값은 바꾸지 않는다.

## 방법

| 항목 | 값 |
|---|---|
| 설계 | [실험 설계](design.md), 봉인 커밋 `{summary["design_commit"]}` |
| 실행 시작 | `{summary["run_started"]}` |
| 환경 | [env.json](env.json) |
| 표본 | 현시점 파일 {summary["census"]["files"]}개, 추출 규칙을 통과한 사용자 턴 {summary["census"]["user_turns"]}개에서 적격 프로젝트 전수 |
| 라벨 | 공식 Codex exec의 gpt-6-astra·gpt-5.6-luna 독립 실행, 별도 지시의 astra 재판정 |
| 호출 | Jev {summary["calls"].get("jev", 0)}회, Codex {summary["calls"].get("codex", 0)}회. 실패와 수리 호출도 포함 |
| 분석 단위 | 서로 다른 턴. 동일 요청의 세 반복 중 첫 응답만 정확도 계산에 사용 |

## 설계와 다른 점

사용자가 제시한 이전 목록의 파일 수와 이번 실행 시점의 목록이 달랐다. 이번 목록을 고정한 뒤 해시가 바뀐 파일 {summary["exclusions"]["changed_files"]}개, 사용자 턴 {summary["exclusions"]["changed_file_turns"]}개를 사전 제외 규칙대로 제외했다. 지정 모델·질문·기준값 선택 규칙은 바꾸지 않았다. 분석 코드는 수집 중 추가했다.

수리 호출 뒤에도 끝 집합의 중복 또는 턴별 전이와의 불일치가 남은 묶음은 {agreement["final_set_invalid_chunks"]}개였다. 봉인 설계는 이 묶음을 유효 라벨로 쓰지 않도록 했으나, 이후 라벨링을 이어 가기 위해 필드와 대상 검사를 통과한 턴별 원판정은 보존하고 다음 묶음의 상태만 그 전이에서 재생했다. 원응답과 잘못 선언한 끝 집합은 고치지 않았다. 마지막 묶음의 끝 집합도 실패하면 그 프로젝트는 끝 집합 합의 분모에 넣지 않는다.

요청한 턴의 앞부분만 반환한 묶음도 {agreement["partial_output_chunks"]}개 있었다. 순서와 필드 검사를 통과한 접두부 {agreement["partial_output_retained_turns"]}턴만 원판정으로 보존하고, 누락 턴은 라벨을 만들어 채우지 않았다. 뒤 묶음 크기를 `min(10, 반환된 접두부 턴 수)`로 줄여 이어 갔다. 이 역시 사후 변경이다. 이하 출력 스키마 변경까지 포함한 사후 변경과 그 뒤 상태에 의존한 전체 {summary["exclusions"]["all_protocol_deviation_turns"]}턴 중 정답 미해소와 겹치지 않는 {summary["exclusions"]["confirmatory_protocol_deviation_turns"]}턴을 확인 검정에서 제외하고 탐색 곡선에만 포함했다. 확인 분석은 {summary["confirmatory_turns"]}턴이다.

수집 중 처리되지 않던 Python 시간 초과 예외를 보완했다. 시작만 기록된 요청은 미완료로 남겼고 재전송하지 않았다. 통계 판정 규칙과 호출 상한은 유지했다.

기존 가림 함수가 인식하지 못한 형식의 키가 비공개 파생 자료에 남아 있어 정확한 환경 키 문자열 가림을 추가했다. 원본 파일은 수정하지 않았고 파생 대화와 요청 기록의 해당 문자열만 가렸다. 노출 문자열 자체는 보고하지 않는다. 이 보완 중 라벨 호출 {summary["interrupted_label_calls"]}개를 중단했으며 원래 요청은 미완료로 남겼다. 이후 별도 trial ID로 출력 스키마를 강제하고 20턴 묶음으로 바꿔 {agreement["structured_output_chunks"]}묶음을 수집했다. 이 사후 변경 역시 확인 검정 제외와 탐색 분석에만 반영했다. 가림 수정의 파일별 횟수는 집계 JSON의 `redaction_repair`에 있다.

## 결과

### 흐름

| 단계 | 수 |
|---|---:|
| 전수 파일 | {summary["census"]["files"]} |
| 추출 사용자 턴 | {summary["census"]["user_turns"]} |
| 제외: 해시 변경 파일의 턴 | {summary["exclusions"]["changed_file_turns"]} |
| 제외: 프로젝트 내 같은 UUID | {summary["exclusions"]["duplicate_turns"]} |
| 제외: 10턴 미만 프로젝트 턴 | {summary["exclusions"]["short_project_turns"]} |
| 측정 대상 턴 | {summary["selected_turns"]} |
| 제외: 미해소 또는 누락 정답 | {summary["exclusions"]["ambiguous_or_missing_gold"]} |
| 주 정확도 분석 턴 | {base["n"]} |
| 사람 확인 대기 사례 | {summary["human_review_cases"]} |

| 프로젝트 길이 | 대화 | 턴 |
|---|---:|---:|
{sample}

Saturn 연결 대화는 {summary["saturn_turns"]}턴이었다. 성공·실패 상태별 Jev 호출 수는 `{summary["jev_statuses"]}`이고, 단계별 수는 `{summary["jev_by_kind"]}`다. Codex 통로별 수는 `{summary["label_calls_by_lane"]}`다.

| 단계 | 상태 | HTTP | 호출 | 요청 바이트 최소 / 중앙 / 최대 |
|---|---|---:|---:|---|
{responses}

HTTP 400 응답 본문은 비어 있어 실패 원인을 단정할 수 없다. 요청 크기는 저장한 JSON의 UTF-8 바이트이며 토큰 수가 아니다. 등록 실패는 행동 없음으로, 관계 실패는 전이 종류 불일치로 계산하고 끝 집합은 불완전으로 표시했다. 후보 쌍별 이진 지표에서는 행동 없음이 대체·해제 음성 예측이다.

### 라벨 합의

| 지표 | 일치율, 95% Wilson 구간 |
|---|---|
| 제약 여부 | {interval(agreement["turn_classification"])} |
| 제약 여부·전이·대상 | {interval(agreement["turn_full"])} |
| 대화 끝 집합 | {interval(agreement["final_set"])} |

끝 집합 검증에 실패해 합의율에서 제외한 프로젝트는 {agreement["final_set_excluded_projects"]}개다. 재판정한 턴은 {agreement["adjudicated_turns"]}개이고 미해소 표시가 있는 턴은 {agreement["unresolved_turns"]}개다. 재판정 결과는 모델 판단이며 사람 확인 완료를 뜻하지 않는다.

### 확인 분석

| 가설 | 지표 | 값, 95% Wilson 구간 | p | Holm p | 판정 |
|---|---|---|---:|---:|---|
{hypotheses}

### 탐색 분석

등록 기준값 곡선의 묻기 비율은 전체 측정 대상 입력 중 `0.50 ≤ p < 기준값`인 비율이다. 정답 미해소 입력도 사용자에게 묻는 부담에는 포함한다.

| 기준값 | 정밀도, 95% 구간 | 재현율, 95% 구간 | F1 | 전체 입력 묻기 비율, 95% 구간 |
|---:|---|---|---|---|
{curve}

확신 구간의 정밀도는 해당 구간 입력을 등록한다고 보았을 때 실제 제약인 비율이다. 분류 정확도와 다르다. 오른쪽 경계는 제외하고 마지막 구간만 1.0을 포함했다. 최악 비율에서 반폭 목표에 필요한 표본은 구간별 {summary["precision_target_minimum"]}개다. 관측 구간의 반폭 목표는 {sum(row["meets_half_width"] for row in summary["confidence_bands"])}/10 구간에서 충족했다.

| 확신 구간 | 양성 비율, 95% 구간 | 반폭 목표 | 최악 조건 표본 부족분 |
|---|---|---|---:|
{confidence}

독립 관측 가정의 검정력 계산은 p0={summary["power"]["null"]}, p1={summary["power"]["alternative"]}, α={summary["power"]["alpha"]}에서 n={summary["power"]["n"]}, 검정력 {percent(summary["power"]["power"])}였다. 구간 점유율과 프로젝트 내 상관 때문에 전체 턴 수를 이 최소 수와 단순 비교할 수 없다. 기준값 0.80의 프로젝트 bootstrap 정밀도 구간은 {bounds(bootstrap["precision_ci95"])}, 재현율 구간은 {bounds(bootstrap["recall_ci95"])}였다. 기준값별 구간은 [집계 JSON](results/summary.json)에 있다.

| 구분 | 층 | 정확도, 95% 구간 | 정밀도, 95% 구간 |
|---|---|---|---|
{strata}

프로젝트는 원본 디렉터리 대신 가명 ID로 표시했다. 언어 분류는 한글 포함 여부를 쓰므로 혼합 문장도 ko에 속한다.

| 관계 | 정확도, 95% 구간 | 정밀도, 95% 구간 | 재현율, 95% 구간 |
|---|---|---|---|
{relation_table}

관계 후보 쌍은 {relations["pairs"]}개였고 전이 종류 일치율은 {interval(relations["classification"])}였다. 정답 전이 대상 {relations["gold_transition_targets"]}개 중 후보에 없는 대상은 {relations["uncovered_gold_targets"]}개, 응답이 유효하지 않은 쌍은 {relations["invalid_pairs"]}개였다. 해제는 별도 `releases` 보조 질문의 결과다. 후보 밖 정답을 양립 정답으로 세지 않았다.

| 턴 거리 | 전이 종류 정확도, 95% 구간 |
|---|---|
{distance}

기준값 0.80의 끝 집합 잠정 exact match는 {interval(final["provisional_exact"])}, 평균 Jaccard는 {percent(final["provisional_mean_jaccard"])}였다. 후보 누락·미완료 관계가 있는 프로젝트는 {final["incomplete_projects"]}개, 미해소 라벨을 포함한 프로젝트는 {final["ambiguous_projects"]}개다. 완전하고 미해소가 없는 프로젝트만의 exact match는 {interval(final["complete_unambiguous_exact"])}다. 잠정 수치를 완전한 상태 추적 정확도로 해석하지 않는다.

세 반복의 등록 분류 일치는 {interval(summary["repeats"]["classification"])}, 확률 표준편차 평균은 {summary["repeats"]["mean_probability_sd"]:.4f}였다. 불완전한 세 반복 묶음은 {summary["repeats"]["incomplete_triplets"]}개다. 관계 종류의 세 반복 일치는 {interval(summary["relationship_repeats"]["classification"])}이고, 불완전한 관계 반복 묶음은 {summary["relationship_repeats"]["incomplete_triplets"]}개다.

질문 대안의 유효한 쌍은 {alternative["n"]}턴이다. 두 문장 모두 정답 {alternative["both_correct"]}, 기준만 정답 b={alternative["baseline_only"]}, 대안만 정답 c={alternative["alternative_only"]}, 모두 오답 {alternative["both_wrong"]}였다. 기준 정확도 {interval(alternative["baseline_accuracy"])}, 대안 정확도 {interval(alternative["alternative_accuracy"])}, 차이 {percent(alternative["accuracy_difference"])}p, paired bootstrap 구간 {bounds(alternative["difference_ci95"], "%p")}, {alternative["method"]} p={alternative["p_value"]:.6g}다. 같은 데이터에서 만든 탐색 비교이므로 새 기본 질문으로 채택하지 않는다.

## 논의

### 해석

{candidate_evidence}

대체·해제의 이진 정확도는 양립 음성이 많은 영향을 받는다. 정밀도와 양성 수를 함께 보아야 하며, 거리별 하락에는 관계 요청 실패도 섞여 있어 장거리 기억 성능만의 차이로 해석하지 않는다. 질문 대안은 동일 기준값에서 재현율 {interval(alternative["alternative_metrics"]["recall"])}여서 놓친 제약이 늘었다.

{recommendation}. 표본 전체를 세었어도 각 확신 구간과 실제 제약 양성의 수가 충분하다는 뜻은 아니다. 프로젝트 연결은 긴 입력 흐름을 만들지만 오래된 작업·동시 세션의 제약이 실제로 이어졌음을 증명하지 않는다. 이번 결과만으로 제약 칸 상한·문장 분할·합침·후보 상한을 확정하지 않는다.

### 타당성 위협

| 종류 | 위협 | 이 실험에서 |
|---|---|---|
| 내적 | 모델 합의는 사람 정답이 아니다. | 최초 합의와 재판정 미해소를 분리하고 사람 검토 목록을 남겼다. |
| 구성 | 세션 연결은 서로 다른 작업 규칙을 합친다. | 세션 경계와 범위를 주었으나 연결의 의미는 검증하지 못했다. |
| 구성 | 라벨러의 앞선 상태 오류가 뒤로 전파된다. | 독립 상태를 유지하고 전이와 끝 집합의 기계적 일관성을 검사했다. |
| 구성 | 사용자 입력만으로 지시 대상을 알 수 없을 수 있다. | 미해소 항목을 주 분석에서 제외했다. |
| 외적 | 한 사용자의 현재 보존 기록에 한정된다. | 과거 목록에서 사라진 파일과 바뀐 파일은 복원하지 않았다. |
| 통계 | 구간 희소성과 프로젝트 내 상관이 있다. | Wilson 명목 구간과 프로젝트 bootstrap을 구분하고 빈 구간을 보고했다. |

### 한계

- 반복 호출을 독립 표본으로 세지 않았다. 동일 사용자·프로젝트 내 상관은 남아 있다.
- 주제 전환만으로 해제하지 않는 지침과 턴 단위 최종 집합은 문장 단위 제품 동작과 다르다.
- 질문 대안과 기준값 선택에는 독립 검증 표본이 없다.
- 사람 검토는 대기 상태다. 원문·라벨·응답은 공개하지 않는다.

## 재현

```sh
cd docs/experiments/constraint-deep
./run.sh verify
./run.sh analyze
```

분석은 저장한 원자료만 읽으며 외부 호출을 하지 않는다. [데이터 안내](data/README.md)의 비공개 원자료가 필요하다.

| 파일 | SHA-256 |
|---|---|
| data/SHA256SUMS | `{summary["manifest_sha256"]}` |

## 결론

| 가설 | 판정 | 반영한 문서 |
|---|---|---|
| H1 | {summary["hypotheses"]["H1"]["decision"]}: 0.80 정밀도 | [제약](../../design/constraints.md) |
| H2 | {summary["hypotheses"]["H2"]["decision"]}: 0.80 재현율 | [제약](../../design/constraints.md) |
"""
    (PUBLIC / "report.md").write_text(text)
