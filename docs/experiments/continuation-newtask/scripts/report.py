"""공개 집계에서 결과 문서와 데이터 안내를 생성한다."""

from __future__ import annotations

from runtime import PUBLIC, read, sha


def pct(value: float | None) -> str:
    return "해당 없음" if value is None else f"{100 * value:.1f}%"


def ci(value: list | None) -> str:
    return (
        "미확인" if value is None else f"[{100 * value[0]:.1f}, {100 * value[1]:.1f}]%"
    )


def format_counts(value: dict) -> str:
    return "; ".join(f"{key} {n}" for key, n in value.items()) or "없음"


def ratio(value: dict) -> str:
    return f"{value['k']}/{value['n']} ({pct(value['value'])})"


def curve_table(curves: list[dict]) -> list[str]:
    lines = [
        "| 조건 | 기준값 | 오접합 [95%] | 재현율 [95%] | 정밀도 [95%] | 가상 묻기 [95%] |",
        "|---|---|---|---|---|---|",
    ]
    for r in curves:
        cells = [
            f"{pct(r[k]['value'])} {ci(r[k]['ci'])}"
            if r[k]["value"] is not None
            else "해당 없음"
            for k in ("false_join_rate", "recall", "precision", "ask_rate")
        ]
        lines.append(
            f"| {r['condition']} | {r['threshold']:.2f} | " + " | ".join(cells) + " |"
        )
    return lines


def overview(s: dict) -> list[str]:
    new = [r for r in s["curves"] if r["scope"] == "new"]
    b1, b2 = [
        next(r for r in new if r["condition"] == b and r["threshold"] == 0.50)
        for b in ("B1", "B2")
    ]
    lab, power = s["labels"], s["power"]
    verdicts = "·".join(h["decision"] for h in s["hypotheses"])
    lines = [
        "# 새 작업 표본을 늘린 이어 가기 판단 확인: 실험 결과",
        "",
        "## 요약",
        "",
        f"새 작업 {power['observed_new']}턴을 확보했다. B1 0.50은 오접합 {ratio(b1['false_join_rate'])}, Wilson 95% 구간 {ci(b1['false_join_rate']['ci'])}, 재현율 {ratio(b1['recall'])}이다. B2 0.50은 오접합 {ratio(b2['false_join_rate'])}, 구간 {ci(b2['false_join_rate']['ci'])}, 재현율 {ratio(b2['recall'])}이다. H1·H2는 각각 {verdicts}다. 제품 설계와 기본값은 변경하지 않았다.",
        "",
        "가설 판정은 후보 보강을 포함한 선택 표본의 수정 분석에 한정한다. 새 작업 목표는 미달했고 무작위 대조의 B1 오접합 상한도 기준보다 높아 실제 입력 분포의 통과 확정은 보류한다.",
        "",
        "## 방법",
        "",
        "| 항목 | 값 |",
        "|---|---|",
        f"| 설계 | [design.md](design.md), 봉인 커밋 `{s['run']['design_commit']}` |",
        f"| 실행 id | `{s['run']['run_id']}` |",
        "| 환경 | [env.json](env.json) |",
        f"| 표본 | 전체 계획 최대 2,000턴, 수집 {s['selection']['selected']}턴, 감사 뒤 분석 {lab['selected']}턴. 새 작업 목표 {power['required_new']}턴, 실제 {power['observed_new']}턴 |",
        "| 비교 | B1·B2의 질문·state·응답 검증·결합 규칙은 앞 실험 코드 그대로 사용했다. 각 조건 한 번이며 반복을 합산하지 않았다 |",
        "| 구간 | 기준값 곡선 표의 대괄호는 Wilson 양측 95%다. 확인 분석에는 Bonferroni exact 경계와 출처 내 세션 cluster 95%를 함께 사용했다 |",
        "",
        "## 설계와 다른 점",
        "",
        "수집 뒤 질문·가설·기준값·거르기·봉인 표본은 바꾸지 않았다. 분석·보고·검증 스크립트는 봉인 뒤 추가했다. 기존 수집기에는 비용 주석만 보완했으며 실행 본문은 바꾸지 않았다. 설계 표의 종결 표현만 교정했으며 가설·수치·방법은 봉인본과 같다. 구조 감사에서 사용자 역할에 저장된 내부 이벤트 6종이 최초 제외 목록에서 빠진 오류를 발견했다. heartbeat·codex_internal_context·codex_delegation·realtime_delegation·recommended_plugins·external_codex_apps_open_page로 시작하는 현재 또는 직전 입력 쌍을 사후 제외했다. 라벨·확률을 기준으로 제외하지 않았고 봉인 표본·응답을 보존했다. 추가 Jev 호출이나 대체 표본은 넣지 않았다. 따라서 확인 표는 사용자 입력으로 제한한 수정 분석이며 최초 추출기의 오류 없는 독립 재현으로 해석할 수 없다. Codex만 남고 Claude 새 표본이 없는 경우와 과거 비공개 B1·B2 응답 없이 공개 집계를 합산하는 제한은 수집 전에 설계에 고정했다.",
        "",
        "## 결과",
        "",
        "### 흐름",
        "",
        "| 단계 | 수 |",
        "|---|---|",
        f"| 원본 파일 | {s['selection']['files']} |",
        f"| 최초 추출기 적격 (사후 감사 전) | {s['selection']['population']} |",
        f"| 선택 | {s['selection']['selected']} |",
        f"| 사후 내부 이벤트·직전 입력 제외 | {s['extraction_audit']['excluded']} |",
        f"| 감사 뒤 분석 | {lab['selected']} |",
    ]
    return lines


def append_flow(s: dict, lines: list[str]) -> None:
    lab = s["labels"]
    for reason, count in s["extraction_audit"]["reasons"].items():
        lines.append(f"| 사후 제외 사유 {reason} (중복 가능) | {count} |")
    for source, counts in s["selection"]["counts"].items():
        for key, n in counts.items():
            lines.append(f"| {source} 추출 {key} | {n} |")
    for label, n in lab["labels"].items():
        lines.append(f"| 최종 {label} | {n} |")
    lines.extend(
        [
            f"| 최초 라벨 누락 | {lab['missing_initial']} |",
            f"| 재판정 사례 | {lab['adjudicated']} |",
            "",
            "| 출처 | 감사 뒤 분석 | new | continue | uncertain | 세션 |",
            "|---|---|---|---|---|---|",
        ]
    )
    for source, v in s["source_counts"].items():
        lines.append(
            f"| {source} | {v['selected']} | {v['labels'].get('new', 0)} | {v['labels'].get('continue', 0)} | {v['labels'].get('uncertain', 0)} | {v['sessions']} |"
        )
    lines.extend(
        [
            "",
            f"양쪽 최초 라벨이 유효한 사례의 3분류 합의율은 {ratio(lab['agreement'])}, Wilson 95% {ci(lab['agreement']['ci'])}, Cohen kappa는 {lab['kappa']:.4f}였다. 최초 라벨 누락·형식 실패 {lab['missing_initial']}턴은 이 분모에서 제외했다. 실패를 불합의로 둔 전체 분석 표본 분모의 합의율은 {ratio(lab['agreement_all_selected'])}였다. 두 모델의 최초 답을 보존하고 불일치·uncertain을 같은 지침으로 재판정했다. 이는 사람 승인 정답이 아니다.",
            "",
            f"라벨 호출 상태: {format_counts(s['label_calls']['statuses'])}. 수리 호출 {s['label_calls']['repairs']}회, 영수증 없는 예약 {s['label_calls']['reserved_without_receipt']}회였다.",
            "",
            "| 조건 | 유효·실패 상태 | 누락 | 요청 바이트 p50 / p95 / 최대 |",
            "|---|---|---|---|",
        ]
    )
    for b, r in s["request_sizes"].items():
        lines.append(
            f"| {b} | {format_counts(r['statuses'])} | {r['missing']} | {r['p50']:,.1f} / {r['p95']:,.1f} / {r['max']:,} |"
        )
    lines.extend(
        [
            "",
            f"요청 상태 표는 사후 제외 전 전체 수집 기준이다. 내부 이벤트 쌍에 쓴 {s['excluded_judge_calls']}회는 성능 분모에서 제외했다.",
            "",
            "uncertain은 오접합·재현율·정밀도 분모에서 제외하고 묻기에는 포함했다. 응답 실패는 묻기로 처리했다. 실패 포함 운영 재현율과 모든 분자·분모는 [summary.json](results/summary.json)과 [CSV](results/tables/thresholds.csv)에 있다.",
            "",
            "### 확인 분석",
            "",
            "| 가설 | 오접합 exact / cluster | 재현율 exact / cluster | 점추정 기준 | 오접합 Wilson 상한 ≤5% | 판정 |",
            "|---|---|---|---|---|---|",
        ]
    )


def append_confirmation(s: dict, lines: list[str]) -> None:
    power = s["power"]
    new = [r for r in s["curves"] if r["scope"] == "new"]
    for i, h in enumerate(s["hypotheses"], 1):
        lines.append(
            f"| H{i} {h['condition']} 0.50 | {ci(h['false_join_exact'])} / {ci(h['false_join_cluster'])} | {ci(h['recall_exact'])} / {ci(h['recall_cluster'])} | {'통과' if h['point_pass'] else '미달'} | {'통과' if h['wilson_upper_pass'] else '미달'} | {h['decision']} |"
        )
    lines.extend(
        [
            "",
            "exact는 각 방향 단측 98.75% Clopper–Pearson 경계다. 두 조건의 오접합 상한·재현율 하한 네 경계를 보정했다. 채택에는 exact와 cluster 두 방식이 모두 오접합 상한 5% 이하·재현율 하한 60% 이상이어야 한다. 다른 기준값은 탐색이며 이 표의 판정을 대체하지 않는다.",
            "",
            f"검정력 재계산은 new {power['required_new']}턴, 임계 오접합 {power['critical_k']}건, 대립 p=0.02에서 power={power['power']:.4f}였다. 새 자료의 new는 {power['observed_new']}턴으로 부족분은 {power['shortfall']}턴이다. 기존 72턴을 더하면 {power['combined_new']}턴이지만 과거 자료는 질문 개발에 사용됐으므로 독립 검증 수로 세지 않았다.",
            "",
            f"합산해도 목표 대비 {power['combined_shortfall']}턴 부족했다. 유효 new 분모는 B1 {power['valid_new_by_condition']['B1']}턴, B2 {power['valid_new_by_condition']['B2']}턴이다.",
            "",
            "### 새 표본 기준값 곡선",
            "",
        ]
    )
    lines.extend(curve_table(new))
    lines.extend(
        [
            "",
            "### 앞 실험과 합친 기준값 곡선",
            "",
            "앞 실험 608턴의 첫 반복 집계에 새 표본을 더했다. 같은 턴을 반복해 표본 수를 늘리지 않았다. 과거 추가 1턴은 합산에서 제외했다. 과거 비공개 응답이 없어 합산 cluster 구간과 개별 대응 추정은 하지 않았다. 표의 Wilson 구간은 세션 상관을 반영하지 못하는 기술 통계다.",
            "",
        ]
    )
    lines.extend(curve_table([r for r in s["curves"] if r["scope"] == "combined"]))


def append_distribution(s: dict, lines: list[str]) -> None:
    lines.extend(
        [
            "",
            "### 거르기 편향과 비례 표본",
            "",
            "| 집합 | 선택 | new | continue | uncertain | 최초 합의율 |",
            "|---|---|---|---|---|---|",
        ]
    )
    for name in ("random", "enriched", "random_pass", "random_fail"):
        v = s["groups"][name]
        lines.append(
            f"| {name} | {v['selected']} | {ratio(v['new_rate'])} | {ratio(v['continue_rate'])} | {ratio(v['uncertain_rate'])} | {pct(v['agreement']['value'])} |"
        )
    lines.extend(["", "| 비교 | new 비율 차이 | 세션 bootstrap 95% |", "|---|---|---|"])
    for d in s["distribution"]:
        lines.append(
            f"| {d['left']} − {d['right']} | {pct(d['new_rate_delta'])}p | {ci(d['ci'])}p |"
        )
    lines.extend(
        [
            "",
            "random은 최초 적격 전체에서 거르기와 관계없이 뽑은 출처 비례 표본에 동일한 사후 내부 이벤트 제외를 적용한 집합이다. enriched는 그 표본을 제외한 후보 보강이며, random_pass·random_fail은 무작위 대조 안의 거르기 통과 여부다. 비율 차이 구간은 같은 세션의 두 집합을 함께 재표집했다. 보강 자료는 원래 정답 분포를 유지하지 않으므로 전체·합산 정밀도는 그 혼합에 조건부다. 원래 적격 모집단의 정밀도는 아래 random 표로 판단한다.",
            "",
            "#### 거르지 않은 무작위 대조",
            "",
        ]
    )
    lines.extend(curve_table([r for r in s["curves"] if r["scope"] == "random"]))
    lines.extend(
        [
            "",
            "#### 후보 보강과 최초 합의 민감도",
            "",
            "| 집합 | 조건 | 0.50 오접합 [95%] | 재현율 [95%] | 정밀도 [95%] |",
            "|---|---|---|---|---|",
        ]
    )
    for r in s["curves"]:
        if (
            r["scope"]
            not in ("enriched", "initial_agreement", "random_pass", "random_fail")
            or r["threshold"] != 0.50
        ):
            continue
        cells = [
            f"{ratio(r[k])} {ci(r[k]['ci'])}"
            for k in ("false_join_rate", "recall", "precision")
        ]
        lines.append(f"| {r['scope']} | {r['condition']} | " + " | ".join(cells) + " |")


def append_discussion(s: dict, lines: list[str]) -> None:
    lines.extend(
        [
            "",
            "### 출처와 대응 차이",
            "",
            "새 표본은 모두 Codex다. 같은 시기·같은 추출 조건의 Claude 새 표본이 없으므로 출처 간 차이는 미확인이다. 기존 Claude와 이번 Codex의 차이는 출처뿐 아니라 시기·질문 개발·거르기·작업 분포가 섞여 있으므로 출처 효과로 해석하지 않는다.",
            "",
            "| 기준값 | B1만 continue / B2만 continue | 오접합 차이 B2−B1 [cluster 95%] | 재현율 차이 [cluster 95%] | 정밀도 차이 [cluster 95%] |",
            "|---|---|---|---|---|",
        ]
    )
    for r in s["paired"]:
        cells = [
            f"{pct(r['delta'][k])}p {ci(r['cluster'].get(k, {}).get('ci'))}p"
            if r["delta"][k] is not None
            else "해당 없음"
            for k in ("false_join_rate", "recall", "precision")
        ]
        lines.append(
            f"| {r['threshold']:.2f} | {r['B1_continue_only']} / {r['B2_continue_only']} | "
            + " | ".join(cells)
            + " |"
        )
    lines.extend(
        [
            "",
            "## 논의",
            "",
            "### 해석",
            "",
            "B1은 보강을 포함한 표본에서 exact·cluster 기준을 통과했지만, 무작위 대조에서는 오접합 구간이 넓었다. 보강 집합의 new 비율이 대조보다 높아 정답 분포가 달라졌고, 보강에서는 오접합도 낮게 관측됐다. 따라서 전체 집계의 통과를 원래 입력 분포로 일반화할 수 없다. 목표 미달은 검정력 제한이며, 관측 오접합 수가 적어 선택 표본의 구간 기준을 통과하는 것과 모순되지 않는다. B2는 오접합 점추정부터 기준을 넘었다.",
            "",
            "### 타당성 위협",
            "",
            "| 종류 | 위협 | 이 실험에서 |",
            "|---|---|---|",
            "| 내적 | 모델 합의와 사람 의도 차이 | 두 최초 라벨과 재판정을 보존했으며 사람 검토 정답은 얻지 않았다 |",
            "| 구성 | 자동 주입·요약·완료 상태 복원 | 최초 추출기가 놓친 내부 이벤트를 구조 감사 뒤 사후 제외했다. 장기 목표와 동시 작업 전체를 재현하지 못한다 |",
            "| 통계 | 후보 보강으로 쉬운 new에 치우침 | 대조/보강 및 대조 안 후보 여부의 분포와 성능을 분리했다 |",
            "| 통계 | 세션 상관·희소 new·기준값 선택 | 고정 0.50 확인과 탐색 곡선을 분리하고 exact·cluster를 병기했다 |",
            "| 외적 | 한 사용자·출처 불균형·과거 재사용 | Codex 새 자료만 확인했으며 과거 Claude 집계와 합산은 기술 통계다 |",
            "| 내적 | 요청 실패·과거 응답 부재 | 실패를 묻기에 넣고 과거 응답 부재로 계산할 수 없는 합산 군집 구간을 표시했다 |",
            "",
            "### 한계",
            "",
            "- 목표 표본 수를 채워도 실제 비율·군집·분포가 다르면 구간 통과를 보장하지 않는다.",
            "- 이어 가기와 묻기는 가상 판정이며 현재 engine의 운영 경로·실제 사용자 수정 비용을 측정하지 않았다.",
            "- 원문·응답은 비공개여서 공개 집계만으로 라벨의 의미를 독립 감사할 수 없다.",
            "",
            "## 재현",
            "",
            "```sh",
            "./docs/experiments/continuation-newtask/run.sh verify",
            "./docs/experiments/continuation-newtask/run.sh analyze",
            "```",
            "",
            "| 항목 | 값 |",
            "|---|---|",
            f"| Jev 호출 / 상한 | {s['calls'].get('jev', 0)} / {s['limits']['jev']} |",
            f"| Codex 호출 / 상한 | {s['calls'].get('codex', 0)} / {s['limits']['codex']} |",
            f"| data/SHA256SUMS SHA-256 | `{sha(PUBLIC / 'data/SHA256SUMS')}` |",
            "",
            "호출은 전송 직전 예약 장부 기준이며 실패·수리도 포함한다. 과거 응답 재사용은 새 호출로 세지 않았다. 원본·라벨·응답은 지정 worktree의 Git 제외 비공개 폴더에만 보관한다.",
            "",
            "## 결론",
            "",
            "| 가설 | 판정 | 반영한 문서 |",
            "|---|---|---|",
        ]
    )
    for i, h in enumerate(s["hypotheses"], 1):
        lines.append(f"| H{i} | {h['decision']} | 없음 |")
    lines.extend(
        [
            "",
            "사용자 지정에 따라 이 결과는 실험 문서에만 남겼다. `docs/design/`·제품 코드·기본값은 수정하지 않았다.",
            "",
        ]
    )


def update_index(s: dict) -> None:
    power = s["power"]
    b1 = next(
        r
        for r in s["curves"]
        if r["scope"] == "new" and r["condition"] == "B1" and r["threshold"] == 0.50
    )
    verdicts = "·".join(h["decision"] for h in s["hypotheses"])
    index = PUBLIC.parent / "README.md"
    rows = index.read_text().splitlines()
    for i, row in enumerate(rows):
        if "[continuation-newtask]" in row:
            rows[i] = (
                f"| [continuation-newtask](continuation-newtask/report.md) | 새 작업 표본을 늘린 B1·B2 오접합 확인 | [router](../design/router.md), [입력 처리](../design/input-handling.md) | H1·H2 {verdicts}: 새 작업 {power['observed_new']}턴, B1 0.50 오접합 {pct(b1['false_join_rate']['value'])} {ci(b1['false_join_rate']['ci'])}·재현율 {pct(b1['recall']['value'])} |"
            )
    index.write_text("\n".join(rows) + "\n")


def main() -> None:
    s = read(PUBLIC / "results/summary.json")
    lines = overview(s)
    append_flow(s, lines)
    append_confirmation(s, lines)
    append_distribution(s, lines)
    append_discussion(s, lines)
    (PUBLIC / "report.md").write_text("\n".join(lines))
    update_index(s)


if __name__ == "__main__":
    main()
