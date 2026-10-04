"""집계 JSON에서 공개 보고서 수치를 렌더링한다."""

from __future__ import annotations

import hashlib

from storage import PUBLIC, read


def pct(value: float | None) -> str:
    return "측정 불가" if value is None else f"{100 * value:.1f}%"


def ci_text(ci: list | None, *, difference: bool = False) -> str:
    if ci is None:
        return "측정 불가"
    suffix = "%p" if difference else "%"
    return f"[{100 * ci[0]:.1f}, {100 * ci[1]:.1f}]{suffix}"


def rate(value: dict) -> str:
    return f"{value['k']}/{value['n']}, {pct(value['value'])} {ci_text(value['ci'])}"


def main() -> None:
    s = read(PUBLIC / "results/summary.json")
    sel, lab, h = s["selection"], s["labels"], s["h1"]
    recommended = ", ".join(
        f"{c} {t:.2f}" if t is not None else f"{c} 없음"
        for c, t in s["recommendations"].items()
    )
    verdict_h2 = "후보 존재" if s["recommendations"]["B"] is not None else "보류"
    candidate_a, candidate_b = [s["curves"][c][0] for c in ("A", "B")]
    lines = [
        "# 한국어 이어 가기 판단 정확도: 실험 결과",
        "",
        "## 요약",
        "",
        f"기준값 0.80으로 `keep_current`를 이진 분류한 대응 표본 {h['n']}턴에서 B−A 정확도 차이는 {100 * h['difference']['accuracy']:+.1f}%p, 세션 cluster bootstrap 95% 구간은 {ci_text(h['cluster']['accuracy']['ci'], difference=True)}였다. H1은 {h['verdict']}다. 탐색 권장 후보는 {recommended}이며 제품 기준값은 유지한다.",
        "",
        "## 방법",
        "",
        "| 항목 | 값 |",
        "|---|---|",
        f"| 설계 | [실험 설계](design.md), 봉인 커밋 `{s['run']['design_commit']}` |",
        f"| 실행 id | `{s['run']['run_id']}` |",
        "| 환경 | [env.json](env.json) |",
        f"| 표본 | 목표 660, 실제 {sel['selected']}턴, {len(sel['strata'])}개 프로젝트 층 |",
        "",
        "두 Codex 라벨러는 다른 지시로 서로의 답과 Jev 결과를 보지 않고 판정했다. A와 B는 같은 한국어 입력과 상태 복원 규칙을 썼다. B에만 직전 입력 전체·목표 발췌·assistant 진행 발췌를 추가했다. 첫 반복만 정확도 분석에 썼으며 세 번의 호출을 별도 표본으로 합산하지 않았다.",
        "",
        "## 설계와 다른 점",
        "",
        f"적격 모집단이 {sel['eligible']}턴이어서 목표보다 {660 - sel['eligible']}턴 적었다. 설계의 전수 사용 규칙을 적용했다. 수집 뒤 설계의 가설·라벨 지침·선택 기준은 바꾸지 않았다. 분석·보고서 코드는 수집 중 추가했으며 봉인된 계산 규칙을 구현했다.",
        "",
        "## 결과",
        "",
        "### 흐름",
        "",
        "| 단계 | 수 |",
        "|---|---|",
        f"| 원본 파일 | {sel['files']} |",
        f"| 추출한 사용자 입력 | {sel['counts']['human_turns']} |",
        f"| 제외: 직전 사용자 입력 없는 첫 턴 | {sel['counts']['first_turn']} |",
        f"| 제외: 직전 입력은 있으나 비한국어인 새 입력 | {sel['counts']['non_korean']} |",
        f"| 제외: UUID 중복 | {sel['counts'].get('duplicate_uuid', 0)} |",
        f"| 제외: 수집 중 변경 파일 | {sel['counts'].get('changed_files', 0)} |",
        f"| 별도 제외: 자동 주입·도구 결과 등의 user 행 | {sel['counts']['non_human_user_rows']} |",
        f"| 선택 | {sel['selected']} |",
        f"| 최초 라벨 실패가 있는 쌍·합의율 제외 | {lab['invalid_initial_pairs']} |",
        f"| 최종 continue | {lab['final'].get('continue', 0)} |",
        f"| 최종 new | {lab['final'].get('new', 0)} |",
        f"| 최종 uncertain·분석 제외 | {lab['final'].get('uncertain', 0)} |",
        f"| 명확 라벨 | {s['clear_n']} |",
        f"| 첫 응답 양쪽 유효 대응 표본 | {s['paired_n']} |",
        f"| 명확 라벨 중 대응 응답 누락 | {s['paired_excluded']} |",
        "",
        f"유효한 최초 라벨 쌍의 3분류 합의율은 {rate(lab['agreement'])}, Cohen kappa는 {lab['kappa']:.3f}였다. 불일치·모호 사례 {lab['adjudicated']}턴을 별도 호출로 재판정했다. 전체 선택 턴 중 유효 최초 라벨 쌍은 {pct(lab['valid_initial_pair_coverage']['value'])}다. 사람 확인용 경계 사례 {s['human_review_n']}개를 비공개에 저장했으며 아직 사람 검토를 받지 않았다.",
        "",
        "### 확인 분석",
        "",
        "| 가설 | 지표 | 결과 | 판정 |",
        "|---|---|---|---|",
        f"| H1 | 0.80에서 B−A 정확도 | {100 * h['difference']['accuracy']:+.1f}%p {ci_text(h['cluster']['accuracy']['ci'], difference=True)} | {h['verdict']} |",
        "",
        f"A만 정답인 쌍은 {h['a_only_correct']}개, B만 정답인 쌍은 {h['b_only_correct']}개였다. exact McNemar 양측 p={h['mcnemar_p']:.6g}였다. 프로젝트 안에서 세션을 재표집하고 A/B 대응을 유지한 구간이며 반복 호출의 변동을 독립 표본으로 세지 않았다.",
        "",
        "### 기준값 곡선",
        "",
        "각 값은 유효한 첫 응답과 최종 명확 라벨의 비가중 집계다. 잘못 이어 붙임은 정답 새 작업 중 이어 가기로 판정한 비율이다. 가상 묻기는 전체 선택 턴에서 0.30 이상 기준값 미만 또는 응답 실패인 비율이다. 현재 router의 낮은 확신으로 인한 실제 묻기는 0이며 충돌 후 provider 거절 확인은 이 기록으로 측정할 수 없다.",
        "",
        "| 조건 | 기준값 | 정밀도 | 재현율 | F1 | 잘못 이어 붙임 | 가상 묻기 |",
        "|---|---|---|---|---|---|---|",
    ]
    for condition, curve in s["curves"].items():
        for r in curve:
            lines.append(
                f"| {condition} | {r['threshold']:.2f} | {pct(r['precision']['value'])} | {pct(r['recall']['value'])} | {pct(r['f1'])} | {pct(r['false_join_rate']['value'])} | {pct(r['ask_rate_hypothetical']['value'])} |"
            )
    lines += [
        "",
        "각 기준값의 혼동행렬·분모·Wilson 구간·cluster 구간·A/B 차이는 [집계 JSON](results/summary.json), 점추정 표는 [CSV](results/tables/thresholds.csv)에 있다.",
        "",
        "| 조건 | 기준값 | 정밀도, Wilson 95% | 재현율, Wilson 95% | 정밀도 cluster 95% | 재현율 cluster 95% |",
        "|---|---|---|---|---|---|",
    ]
    for condition, curve in s["curves"].items():
        selected_thresholds = sorted({0.8, s["recommendations"][condition]} - {None})
        for threshold in selected_thresholds:
            r = next(r for r in curve if r["threshold"] == threshold)
            lines.append(
                f"| {condition} | {threshold:.2f} | {rate(r['precision'])} | {rate(r['recall'])} | {ci_text(r['cluster']['precision']['ci'])} | {ci_text(r['cluster']['recall']['ci'])} |"
            )
    lines += [
        "",
        f"H2의 B 판정은 {verdict_h2}다. 권장 후보는 {recommended}다. 정밀도 두 구간 하한이 80% 이상이고 정밀도·재현율의 명목·cluster 구간 반폭이 모두 7%p 이하인 최소 기준값만 선택했다. 같은 표본으로 선택한 탐색 후보이며 독립 검증된 기본값은 아니다.",
        "",
        "| 지표 | 0.80에서 B−A | 대응 cluster 95% |",
        "|---|---|---|",
    ]
    for name in ("accuracy", "precision", "recall", "f1", "false_join_rate"):
        value = h["difference"][name]
        lines.append(
            f"| {name} | {100 * value:+.1f}%p | {ci_text(h['cluster'][name]['ci'], difference=True)} |"
        )
    lines += [
        "",
        "### 반복과 요청 크기",
        "",
        "| 조건 | 0.80 분류 3회 일치 | 불완전 반복 집합 | 평균 확률 표준편차 |",
        "|---|---|---|---|",
    ]
    for condition, curve in s["curves"].items():
        r = next(r for r in curve if r["threshold"] == 0.8)
        lines.append(
            f"| {condition} | {rate(r['repeat_agreement'])} | {r['repeat_incomplete']} | {r['mean_probability_stddev']:.4f} |"
        )
    lines += [
        "",
        "| 조건 | min | p50 | p90 | p95 | p99 | max |",
        "|---|---|---|---|---|---|---|",
    ]
    for condition, entry in s["request_sizes"].items():
        lines.append(
            "| "
            + condition
            + " | "
            + " | ".join(f"{v:,.0f}" for v in entry["quantiles"].values())
            + " |"
        )
    lines += [
        "",
        "크기 단위는 UTF-8 바이트다. 큰 요청을 잘라 성공 표본으로 바꾸지 않았다.",
        "",
        "| 조건 | 상태 | 횟수 |",
        "|---|---|---|",
    ]
    for condition, entry in s["request_sizes"].items():
        for status, count in entry["status"].items():
            lines.append(f"| {condition} | {status} | {count} |")
    lines += [
        "",
        f"실제 예약·호출 수는 Jev {s['calls'].get('jev', 0):,}회, Codex {s['calls'].get('codex', 0):,}회였다. Jev 관측 행은 {s['receipts']:,}/{s['expected_receipts']:,}개였다. 전송 전 oversize는 관측 실패로 세되 외부 호출 수에는 넣지 않았다.",
        "",
        "### 탐색 분석",
        "",
        "| 조건 | 현재 engine 재생 정확도 | 정밀도 | 재현율 | 잘못 이어 붙임 |",
        "|---|---|---|---|---|",
    ]
    for condition, r in s["engine_replay"].items():
        lines.append(
            f"| {condition} | {rate(r['accuracy'])} | {pct(r['precision']['value'])} | {pct(r['recall']['value'])} | {pct(r['false_join_rate']['value'])} |"
        )
    lines += [
        "",
        "idle에서 현재 engine은 0.30 미만만 새 작업으로 처리한다. running에서는 관계·전송 선택지와 확신도로 정한다. 위 재생은 응답 실패 때의 유지 대체 규칙까지 포함하며, 위의 상단 기준값 곡선과 다른 지표다.",
        "",
        "| 조건 | 최초 합의 사례만의 0.80 정확도 |",
        "|---|---|",
    ]
    for condition, r in s["initial_agreement_sensitivity"].items():
        lines.append(f"| {condition} | {rate(r['accuracy'])} |")
    lines += [
        "",
        "프로젝트·활동 상태·입력 길이별 성능과 관계 선택지 집계는 집계 JSON에 있다.",
        "",
        "## 논의",
        "",
        "### 해석",
        "",
        "B는 같은 입력에 직전 작업 정보를 추가한 비교다. 결과는 작업 정보 제공의 효과를 측정하며 실제 Saturn 엔진에 상태 수집 기능을 구현한 검증은 아니다. 목표와 진행 내용은 고정 길이 발췌이므로 전체 대화 이해의 효과와도 구분한다. 기준값 선택은 독립 표본에서 재검증해야 한다.",
        "",
        f"명확 라벨 중 이어 가기가 {lab['final']['continue']}/{s['clear_n']}턴을 차지한다. 이 불균형 때문에 정밀도 하한 80% 조건만으로 새 작업 오접합을 제어할 수 없다. 사전 규칙의 0.50 후보는 새 작업 {lab['final']['new']}턴 중 A {candidate_a['fp']}턴, B {candidate_b['fp']}턴을 잘못 이어 붙였다. 운영 기준값 채택은 보류한다. 이는 기준값 선택 규칙을 사후 변경한 결과가 아니라, 선택된 후보의 오접합 위험에 대한 해석이다.",
        "",
        f"현재 engine 규칙을 재생한 정확도 차이는 {100 * s['engine_comparison']['accuracy_difference']:+.1f}%p, 대응 cluster 95% 구간은 {ci_text(s['engine_comparison']['cluster']['accuracy']['ci'], difference=True)}다. 이 차이를 0.80 이진 분류의 차이와 구분해야 한다. 현재 idle 대체 규칙을 유지하면 상단 기준값만 바꿔도 처리 결과는 달라지지 않는다.",
        "",
        "### 타당성 위협",
        "",
        "| 종류 | 위협 | 이 실험에서 |",
        "|---|---|---|",
        f"| 내적 | 모델 합의가 사람 정답은 아님 | 최초 유효 쌍 {lab['agreement']['n']}턴, 최초 라벨 실패 포함 쌍 {lab['invalid_initial_pairs']}턴, 재판정 분리 |",
        f"| 구성 | 발췌가 장기 목표·진행을 생략 | 목표 잘림 {s['context']['goal_truncated']}턴, 진행 잘림 {s['context']['progress_truncated']}턴 |",
        f"| 구성 | 실행 상태 복원·이전 처리 방식 가정 | stop_reason 누락 {s['context']['missing_stop_reason']}턴, assistant 없음 {s['context']['no_assistant']}턴, queue 고정 |",
        f"| 외적 | 한 사용자의 프로젝트 편중 | Saturn {s['context']['saturn_n']}턴, 전체 {len(sel['strata'])}개 프로젝트·{s['context']['sessions']}개 세션 |",
        f"| 통계 | 상관·희소 층·동일 표본 선택 | 세션 하나뿐인 층 {s['context']['single_session_strata']}개, 선택 후보는 탐색 |",
        "| 내적 | 큰 요청 실패가 조건 차이에 영향 | 실패 수·요청 크기·완전 대응 표본 분리 |",
        "",
        "### 한계",
        "",
        "프로젝트 층화 전수는 보존 기록에 대한 기술 통계다. Wilson 구간은 독립 턴 가정의 명목 구간이며 세션 bootstrap도 같은 사용자·프로젝트 바깥의 일반화 불확실성을 모두 반영하지 못한다. 실제 작업 상태, provider 끼워 넣기 성공, 사용자 확인과 이후 만족도는 측정하지 않았다.",
        "",
        "## 재현",
        "",
        "```sh",
        "cd docs/experiments/continuation-judgment-korean",
        "./run.sh verify",
        "./run.sh analyze",
        "python3 scripts/report.py",
        "```",
        "",
        "분석·검증은 외부 호출 없이 저장한 비공개 자료를 읽는다. 공개 파일에는 원문·개별 라벨·응답을 포함하지 않는다. [데이터 안내](data/README.md)에 필드와 재현 경계를 기록했다.",
        "",
        "| 파일 | SHA-256 |",
        "|---|---|",
        f"| data/SHA256SUMS | `{hashlib.sha256((PUBLIC / 'data/SHA256SUMS').read_bytes()).hexdigest()}` |",
        "",
        "## 결론",
        "",
        "| 가설 | 판정 | 반영한 문서 |",
        "|---|---|---|",
        f"| H1 | {h['verdict']} | [router](../../design/router.md), [입력 처리](../../design/input-handling.md) |",
        f"| H2 | {verdict_h2}, 탐색 후보 {recommended} | [router](../../design/router.md) |",
        "",
    ]
    (PUBLIC / "report.md").write_text("\n".join(lines))


if __name__ == "__main__":
    main()
