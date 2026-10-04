"""집계 JSON으로 결과 보고서를 생성한다."""

from __future__ import annotations

from runtime import PUBLIC, read


def percent(value: float | None) -> str:
    return "측정 불가" if value is None else f"{value * 100:.1f}%"


def bounds(value: list | None) -> str:
    return (
        "측정 불가"
        if value is None
        else f"[{value[0] * 100:.1f}, {value[1] * 100:.1f}]"
    )


def proportion(value: dict) -> str:
    return f"{value['k']}/{value['n']} ({percent(value['rate'])})"


def difference(value: float | None) -> str:
    return "측정 불가" if value is None else f"{value * 100:+.1f}"


def number(value: float | None) -> str:
    return "측정 불가" if value is None else f"{value:.2f}"


def opening(summary: dict) -> list[str]:
    conditions = summary["conditions"]
    jev = conditions["jev-1"]
    gold = summary["adjudication"]
    sampling = summary["sampling"]
    verdict = summary["verdict"]
    lines = [
        "# 작업별 모델 선택: 실험 결과",
        "",
        "## 요약",
        "",
        f"Jev의 대체 후 허용 적중률은 {proportion(jev['effective'])}, 매뉴얼 sol은 {proportion(jev['manual'])}였다. 대응 차이는 {difference(jev['paired']['delta'])}%p, 프로젝트 군집 95% 구간은 {bounds(jev['paired']['cluster95'])}%p였다. provider 전환 적중은 {proportion(jev['switch_hit'])}였다. 이번 판정은 ‘{verdict['recommendation']}’다. 제품 설계 문서는 사용자 지시에 따라 변경하지 않았다.",
        "",
        "## 방법",
        "",
        "| 항목 | 값 |",
        "|---|---|",
        f"| 설계 | [실험 설계](design.md), 봉인 커밋 `{summary['design_commit']}` |",
        "| 환경 | [env.json](env.json), [후보 확인](models.json) |",
        f"| 표본 | 설계 {summary['planned_n']}개, 추출 {sampling['selected']}개, 분석 {gold['analyzed']}개 |",
        "| 정답 | gpt-6-sol 독립 2회, 불일치 3회째, 최선·허용 집합 전체 다수결 |",
        "| 질의 | 초기 engine state와 route 질문 재생, 참고 모델별 1회·Jev 3회 |",
        "",
        "## 설계와 다른 점",
        "",
        "| 변경 | 시점 | 이유 | 결론에 미친 영향 |",
        "|---|---|---|---|",
        "| JSON 예시의 noul 키와 정답 통로 오류 중단 처리 정정 | 표본 추출 중, 정답·선택 호출 전 | 독립 검토에서 실제 engine 계약과 차이 발견 | 정답·선택 응답을 보기 전 정정, 가설·기준·표본 규칙 변경 없음 |",
        "| Codex subagent·exec와 자동 실행 작업 폴더 제외 | 최초 정답 일부 예약 뒤, 응답 내용 열람 전 | 출처 검증에서 자동 실행 표본 발견 | 최초 표본 폐기 후 같은 시드로 재추출, 이전 호출도 예산에 포함 |",
        "| CLI 진단 error 이벤트의 도구 실행 오분류 수정 | 정답 수집 중, 선택 질의 전 | 정상 JSON 응답도 도구 사용으로 제외하던 판독 오류 발견 | 저장 응답을 수정된 판독기로 재처리, 추가 호출·미완료 예약 보존 |",
        "| 함수 타입과 Python 호환 주석 보완 | 정답·선택 호출 전 | 정적 검증 규칙 적용 | 의미 변경 없음 |",
        "",
        "## 결과",
        "",
        "### 흐름",
        "",
        "| 단계 | 수 |",
        "|---|---|",
        f"| 사용자 입력 추출을 수행한 세션 파일 | {sampling['files']} |",
        f"| 추출한 사용자 입력 | {sampling['extracted_users']} |",
        f"| 적격 첫 입력 | {sampling['eligible']} |",
        f"| 선정 | {sampling['selected']} |",
    ]
    lines += [
        f"| 표본 제외: {key} | {value} |" for key, value in sampling["excluded"].items()
    ]
    lines += [
        f"| 정답 제외: {key} | {value} |" for key, value in gold["exclusions"].items()
    ]
    lines += [
        f"| 분석 | {gold['analyzed']} |",
        f"| 유효하지 않은 라벨 응답 | {gold['invalid_ballots']} |",
        "",
        f"프로젝트는 {sampling['projects']}개였다. 기록별 선정 수는 `{sampling['by_source']}`, 작업 종류별 선정 수는 `{sampling['by_category']}`였다.",
        "",
        f"최초 표본에서 자동 실행 출처 {summary['rejected_source_samples']}개를 발견해 응답 내용을 보지 않고 재추출했다. 그전에 예약한 정답 호출 {summary['pre_audit_reserved_calls']}회도 전체 예산에 포함했다. 새 표본·프롬프트와 같은 기존 응답 {summary['source_audit']['reusable_gold_receipts']}건만 재사용했다.",
        "",
        "### 확인 분석",
        "",
        "| 가설 | 지표 | 값 | 95% 구간 | 판정 |",
        "|---|---|---|---|---|",
        f"| H1 | 오토−매뉴얼 | {difference(jev['paired']['delta'])}%p | 군집 {bounds(jev['paired']['cluster95'])}%p | {verdict['H1']} |",
        f"| H2 | provider 전환 적중 | {proportion(jev['switch_hit'])} | Wilson {bounds(jev['switch_hit']['wilson95'])}%, 군집 {bounds(jev['switch_hit']['cluster95'])}% | {verdict['H2']} |",
        "",
        f"대응 불일치는 오토만 적중 {jev['paired']['auto_only']}개, 매뉴얼만 적중 {jev['paired']['manual_only']}개였다. 정확 McNemar p값은 {jev['paired']['mcnemar_exact_p']:.6g}였다. 이 p값은 두 방식의 주변 적중률이 같다는 귀무가설의 검정이며 비열등 판정에는 사전 지정 군집 구간을 사용했다.",
        "",
        "모든 표본에서 두 방식의 적중 여부가 같으면 재표집 차이도 항상 0이므로 군집 구간이 [0, 0]으로 퇴화한다. 이때 H1 채택은 사전 등록 계산 규칙의 결과이며, 모집단에서 차이가 없거나 비열등성이 정밀하게 입증됐다는 뜻은 아니다. 이번 오토 기본값 권고에는 작은 전환 표본과 이 구간의 한계를 함께 반영했다.",
        "",
        "### 탐색 분석",
        "",
        "| 선택 모델 | 최선 일치 | 원래 선택 허용 | 대체 후 허용 | 매뉴얼 대비 | 전환 비율 | 전환 적중 |",
        "|---|---|---|---|---|---|---|",
    ]
    return lines


def main() -> None:
    summary = read(PUBLIC / "results/summary.json")
    conditions = summary["conditions"]
    gold = summary["adjudication"]
    verdict = summary["verdict"]
    lines = opening(summary)
    for name, c in conditions.items():
        lines.append(
            f"| {name} | {proportion(c['best'])} | {proportion(c['raw'])} | {proportion(c['effective'])} | {difference(c['paired']['delta'])}%p | {proportion(c['switch'])} | {proportion(c['switch_hit'])} |"
        )
    lines += [
        "",
        "최선 일치와 원래 선택 허용은 확신 대체 전 답을 채점했다. 대체 후 허용은 형식·호출 실패도 기본 sol로 진행한 결과다. 반복은 독립 표본으로 합산하지 않았다. 예산 상한 등으로 예약하지 못한 질의는 오답에 넣지 않고 해당 모델의 분석 분모에서 제외했으며, 예약 뒤 응답이 없는 호출은 실패로 남겼다. 모든 비율 구간과 종류별 결과는 [집계](results/summary.json)에 있다.",
        "",
        "| 선택 모델 | 낮은 확신 | 전체 대체 | 형식·호출 실패 / 미실행 | 중앙 지연 | p90 지연 | 알려진 비용 합계 | 비용 미상 호출 |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for name, c in conditions.items():
        lines.append(
            f"| {name} | {proportion(c['low_confidence'])} | {proportion(c['fallback'])} | {c['invalid']} / {c['not_run']} | {number(c['latency']['median_s'])}초 | {number(c['latency']['p90_s'])}초 | USD {c['known_cost_usd']:.6f} | {c['unknown_cost_calls']} |"
        )
    lines += [
        "",
        "| 선택 모델 | 실패 원인 | 유효 응답 조건부 원래 선택 허용 |",
        "|---|---|---|",
    ]
    for name, c in conditions.items():
        lines.append(
            f"| {name} | `{c['invalid_reasons']}` | {proportion(c['success_conditional_raw'])} |"
        )
    lines += [
        "",
        "invalid_json_object는 JSON 전체를 해석할 수 없는 응답, invalid_choice_schema는 후보 키·확률 범위·합계 계약을 충족하지 못한 응답이다. 뒤에 붙은 문자 제거, 확률 재정규화, 추가 질의로 답을 고치지 않았다. 유효 응답 조건부 적중률은 형식 성공 표본에 한정되므로 전체 적중률과 직접 비교하지 않는다.",
        "",
        f"sol의 첫 두 판정 완전 일치는 {proportion(gold['first_complete_agreement'])}, 최선 후보 일치는 {proportion(gold['first_best_agreement'])}, 허용 집합 일치는 {proportion(gold['first_set_agreement'])}였다. 다수결에 세 번째 판정을 사용한 표본은 {gold['third_calls']}건이고, 중단 전 추가 호출을 포함한 세 번째 판정 예약은 {summary['reserved_gold_third_calls']}회였다. Jev 세 번 선택 완전 일치는 {proportion(summary['jev_three_way_agreement'])}였다.",
        "",
        f"사후 사용자 입력의 신호 분포는 `{gold['signal_counts']}`였다. 신호는 실제 실행 성공 판정이 아니다.",
        "",
        "| 고정 기본 후보 | 매뉴얼 허용 | 오토 허용 | 대응 차이 | 군집 95% 구간 |",
        "|---|---|---|---|---|",
    ]
    for name, c in summary["manual_sensitivity"].items():
        lines.append(
            f"| {name} | {proportion(c['manual'])} | {proportion(c['auto'])} | {difference(c['paired']['delta'])}%p | {bounds(c['paired']['cluster95'])}%p |"
        )
    lines += [
        "",
        "## 논의",
        "",
        "### 해석",
        "",
        f"{verdict['recommendation']}. 이 판정은 기본 sol과 초기 상태 재생에 한정한다. 후보별 고정 비교는 기본값에 따른 결과 민감도를 보여 주며 주 가설 판정과 구분한다. 원래 선택과 운영 대체 후 성능을 구별해야 낮은 확신으로 기본값에 머문 경우를 작업별 모델 선택의 이득으로 잘못 해석하지 않는다.",
        "",
        "### 타당성 위협",
        "",
        "| 종류 | 위협 | 이 실험에서 |",
        "|---|---|---|",
        "| 내적 | sol 정답 편향 | 자기 일치를 측정했으나 사람 검토·후보별 실제 실행 검증은 미수행 |",
        "| 구성 | 사후 근거 부족 | 사용자 후속 입력만 사용, assistant·tool의 실제 실행 결과는 근거에서 제외 |",
        "| 구성 | 초기 상태 제한 | 실행·보류·고정 없는 새 채팅만 재생. 중간 작업 전환과 관계 판단은 평가 범위 밖 |",
        "| 구성 | 품질·비용·속도 계열 가정 | 봉인한 지침에 따른 라벨이며 실제 작업 수행 비용·속도는 측정 범위 밖 |",
        "| 외적 | 개인 기록과 층 균형 | 한 사용자 기록의 층 균형 표본이므로 제품 전체 작업 분포로 일반화 제한 |",
        "| 통계 | 작은 군집과 전환 분모 | Wilson·군집 구간과 빈 분모를 함께 보고 |",
        "| 시간 | CLI·모델·동시 자원 경쟁 | 버전·usage·요청 지연 기록, 구독 실제 청구액과 분리 |",
        "",
        "### 한계",
        "",
        "이 실험은 후보를 골라 보낸 뒤 작업을 실행하는 A/B 실험이 아니다. 현재 모델 이름만으로 판단하는 질문의 적합성을 사후 라벨과 비교했다. 모델별 능력과 비용 설명을 추가한 질문, 후보를 실제로 실행한 결과, 사용자가 고른 다른 기본값은 독립 검증이 필요하다.",
        "",
        "## 재현",
        "",
        "```sh",
        "./run.sh verify",
        "./run.sh analyze",
        "```",
        "",
        f"호출 예약은 Codex {summary['calls'].get('codex', 0)}회, Claude {summary['calls'].get('claude', 0)}회, Jev {summary['calls'].get('jev', 0)}회였다. 응답이 남지 않은 예약은 {summary['incomplete_reserved_calls']}회였다. 후보 조회 RPC는 별도이며 Claude 확인 호출은 합계에 포함했다. 알려진 API 상당 비용 합은 USD {summary['all_known_cost_usd']:.6f}, 비용 미상 호출은 {summary['all_unknown_cost_calls']}회였다.",
        "",
        "원자료 접근과 재현 범위는 [데이터](data/README.md)에 있다. 원문·응답은 공개 저장소에 포함하지 않았다. 공개 해시는 [SHA256SUMS](data/SHA256SUMS)에 있다.",
        "",
        "## 결론",
        "",
        "| 가설 | 판정 | 반영한 문서 |",
        "|---|---|---|",
        f"| H1 | {verdict['H1']} | 없음 |",
        f"| H2 | {verdict['H2']} | 없음 |",
    ]
    text = "\n".join(lines) + "\n"
    (PUBLIC / "report.md").write_text(text)


if __name__ == "__main__":
    main()
