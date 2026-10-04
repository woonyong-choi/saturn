"""집계 JSON에서 보고서의 표와 판정을 만든다."""

from __future__ import annotations

from support import PUBLIC, read, sha


def pct(value: float | None) -> str:
    return "해당 없음" if value is None else f"{100 * value:.1f}%"


def span(values: list | None) -> str:
    return (
        "해당 없음"
        if values is None
        else f"[{100 * values[0]:.1f}, {100 * values[1]:.1f}]%"
    )


def count(value: dict) -> str:
    return f"{value['k']}/{value['n']} ({pct(value['value'])})"


def curves_section(s: dict, lines: list[str]) -> None:
    lines += [
        "",
        "exact는 각 방향의 단측 98.75% Clopper–Pearson 경계다. 두 조건의 오접합 상한·재현율 하한에 Bonferroni를 적용했다. cluster는 프로젝트 안에서 세션을 재표집한 명목 95% 구간이다. 채택에는 두 방식의 오접합 상한과 재현율 하한이 모두 기준을 만족해야 한다. 점추정 통과만으로 채택하지 않았다.",
        "",
        "### 기준값 곡선",
        "",
        "| 조건 | 기준값 | 오접합 | 재현율 | 정밀도 | 가상 묻기 | 운영 재현율 |",
        "|---|---|---|---|---|---|---|",
    ]
    for r in s["curves"]:
        lines.append(
            f"| {r['condition']} | {r['threshold']:.2f} | {pct(r['false_join_rate']['value'])} | {pct(r['recall']['value'])} | {pct(r['precision']['value'])} | {pct(r['ask_rate']['value'])} | {pct(r['operational_recall']['value'])} |"
        )
    lines += [
        "",
        "각 곡선의 분자·분모, Wilson 구간, exact 경계와 cluster 구간은 [summary.json](results/summary.json)에 있다. [CSV](results/tables/thresholds.csv)는 같은 집계다. 가상 묻기는 제품에 구현된 동작이 아니며 B2의 질문 불일치도 포함한다.",
        "",
        "### B0와 대응 비교",
        "",
        "| 조건 | 기준값 | 같은 턴 | B0만 정답 / 조건만 정답 | 오접합 차이 / cluster 구간 | 재현율 차이 / cluster 구간 |",
        "|---|---|---|---|---|---|",
    ]
    for r in s["paired"]:
        if r["threshold"] != 0.8:
            continue
        lines.append(
            f"| {r['condition']} | {r['threshold']:.2f} | {r['n']} | {r['base_correct_only']} / {r['condition_correct_only']} | {100 * r['delta']['false_join_rate']:+.1f}%p / {span(r['cluster']['false_join_rate']['ci']).replace('%', '%p')} | {100 * r['delta']['recall']:+.1f}%p / {span(r['cluster']['recall']['ci']).replace('%', '%p')} |"
        )


def comparison_section(s: dict, lines: list[str]) -> None:
    lines += [
        "",
        "모든 기준값의 대응 차이와 불일치 수는 summary에 있다. 같은 입력·세션을 유지했으며 다른 시점의 B0 응답을 재사용한 시간 차이는 제거하지 못했다.",
        "",
        "### 두 질문 결합과 반복 안정성",
        "",
        "| B2 기준값 | 기존 질문만 오접합 / 재현율 | 결합 오접합 / 재현율 | 결합 묻기 |",
        "|---|---|---|---|",
    ]
    for solo in s["B2_keep_only"]:
        r = next(
            r
            for r in s["curves"]
            if r["condition"] == "B2" and r["threshold"] == solo["threshold"]
        )
        lines.append(
            f"| {r['threshold']:.2f} | {pct(solo['false_join_rate']['value'])} / {pct(solo['recall']['value'])} | {pct(r['false_join_rate']['value'])} / {pct(r['recall']['value'])} | {pct(r['ask_rate']['value'])} |"
        )
    lines += ["", "| 조건 | 0.80 두 반복 이진 일치 | 불완전 묶음 |", "|---|---|---|"]
    for r in s["repeat_agreement"]:
        if r["threshold"] == 0.8:
            lines.append(
                f"| {r['condition']} | {count(r['agreement'])} | {r['incomplete']} |"
            )


def collection_section(s: dict, lines: list[str]) -> None:
    lines += [
        "",
        "두 질문의 답은 독립 측정이 아니다. 같은 B2 응답 안의 단독·결합 비교는 결합 규칙의 효과를 보여 주지만 B0와의 차이에는 추가 질문이 기존 확률을 바꾼 효과도 섞인다. 두 번째 반복은 안정성 확인에만 사용했다.",
        "",
        "### 요청 크기와 호출 수",
        "",
        "| 조건 | 요청 수 | 최소 | p50 | p90 | p95 | p99 | 최대 바이트 |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for b, v in s["request_sizes"].items():
        lines.append(
            "| "
            + " | ".join(
                [b, str(v["n"])]
                + [f"{v[k]:.0f}" for k in ("min", "p50", "p90", "p95", "p99", "max")]
            )
            + " |"
        )
    lines += [
        "",
        f"새 호출은 Jev {s['calls']['jev']}/{s['calls']['limits']['jev']}회, Codex 라벨러 {s['calls']['codex']}/{s['calls']['limits']['codex']}회였다. B0 {s['calls']['reused_B0_first_two']}응답은 역사적 재사용이며 이번 호출 수에 포함하지 않았다. 실제 모델명은 조건별 summary에 있다. 요청 상한은 전송 전에 검사했다.",
        "",
        "### 최초 합의와 추가 표본",
        "",
        "| 조건 | 최초 합의 표본의 0.80 오접합 | 재현율 | 명확 표본 |",
        "|---|---|---|---|",
    ]
    for r in s["initial_agreement_sensitivity"]:
        if r["threshold"] == 0.8:
            lines.append(
                f"| {r['condition']} | {count(r['false_join_rate'])} | {count(r['recall'])} | {r['valid_clear']} |"
            )


def discussion_section(s: dict, lines: list[str], choice: str) -> None:
    lines += [
        "",
        "추가 표본은 기존 표본과 섞지 않았다. 두 모델이 continue로 일치한 한 건이므로 새 작업 오접합의 근거를 늘리지 못했다. 조건별 전체 곡선은 `extension_curves`에 있으며 new 분모가 없어 오접합은 null이다.",
        "",
        "## 논의",
        "",
        "### 해석",
        "",
        f"사전 선택 규칙에 따른 탐색 권장 조건·기준값은 {choice}이다. 후보는 오접합 점추정 5% 이하와 재현율 60% 이상인 조합 중 재현율을 먼저 최대화한 결과다. 이 순위는 같은 자료에서 고른 후보이며 제품 기본값의 확정 근거가 아니다.",
        "",
        "독립적으로 완료할 목표·산출물을 새 작업으로 명시한 B1 질문과 낮춘 기준값을 함께 사용하는 방법이 이번 표본에서 가장 높은 재현율의 후보였다. B1의 0.80을 그대로 유지하면 재현율이 부족하므로 질문 변경과 기준값을 함께 검증해야 한다. 오접합을 줄이는 효과는 재현율 손실과 묻기 증가를 함께 봐야 한다. B2가 오접합을 낮추더라도 두 확률이 모두 강하게 일치해야 하므로 이어 가기 누락이 늘 수 있다. B3는 길이와 순서를 함께 바꿔 어느 변경이 효과를 만들었는지 분리하지 못한다. 현재 router는 중간 확신을 유지하는 대체 규칙도 있으므로 이 곡선의 기준값만 바꿔 제품 동작이 같아진다고 해석할 수 없다.",
        "",
        "### 타당성 위협",
        "",
        "| 종류 | 위협 | 이 실험에서 |",
        "|---|---|---|",
        "| 내적 | 기존 오접합을 보고 같은 자료로 평가 | 개발 표본 재사용, 새 질문 수집 전에 설계 봉인 |",
        "| 내적 | 모델 라벨과 실제 사용자 의도의 차이 | 기존 라벨 고정, 최초 합의 민감도 별도 집계, 사람 승인 정답 아님 |",
        "| 구성 | 복원 state와 제품 행동의 차이 | 가상 묻기만 계산, 설계·구현 변경 없음 |",
        "| 구성 | B3 두 변경과 B2 두 효과 혼합 | B3 개별 효과 주장 제외, B2 같은 응답의 단독 질문 곡선 병기 |",
        "| 통계 | new 희소·군집·기준값 선택 | exact·cluster 구간과 검정력 부족 보고, 곡선 선택은 탐색 |",
        "| 외적 | 한 사용자·프로젝트 편중과 역사적 B0 | 다른 사용자 일반화 없음, B0 재호출 없이 시간 차이 유지 |",
        "",
        "### 한계",
        "",
        f"새 작업 {s['power']['available_base_new']}건은 사전 검정력 계산의 {s['power']['required_new']}건보다 {s['power']['shortfall']}건 부족하다. 독립 이항 가정에서도 부족하며 세션 상관은 요구 수를 더 늘릴 수 있다. 같은 추출 규칙의 추가 적격을 전수 사용했지만 new는 늘지 않았다. 기존 출현 비율로는 약 {s['power']['required_extra_turns_at_old_prevalence']}턴의 추가 자료가 필요하다. 복제·합성 자료나 라벨 재작성으로 분모를 채우지 않았다.",
        "",
        "## 재현",
        "",
        "원문과 응답은 지정 worktree의 Git 제외 `.local/experiments/continuation-misjoin/`에 있다. 기존 자료는 main의 `.local/experiments/continuation-ko/`에서 읽는다. 공개 집계에는 원문·응답·세션 식별자를 넣지 않았다.",
        "",
        "```sh",
        "./docs/experiments/continuation-misjoin/run.sh analyze",
        "./docs/experiments/continuation-misjoin/run.sh verify",
        "```",
        "",
        "| 파일 | SHA-256 |",
        "|---|---|",
        f"| `data/SHA256SUMS` | `{sha(PUBLIC / 'data/SHA256SUMS')}` |",
        "",
        "## 결론",
        "",
        "| 가설 | 판정 | 반영한 문서 |",
        "|---|---|---|",
        f"| H1 B1 0.80 | {s['hypotheses']['B1']} | 없음 |",
        f"| H2 B2 0.80 | {s['hypotheses']['B2']} | 없음 |",
        "",
        f"탐색 후보는 {choice}이며 운영 채택은 보류한다. 사용자 지시에 따라 결과는 이 보고서에만 남겼다.",
    ]


def opening_section(s: dict, summary: str) -> list[str]:
    lines = [
        "# 이어 가기 판단의 새 작업 오접합 줄이기: 실험 결과",
        "",
        "## 요약",
        "",
        summary,
        "",
        "## 방법",
        "",
        "| 항목 | 값 |",
        "|---|---|",
        f"| 설계 | [실험 설계](design.md), 봉인 커밋 `{s['run']['design_commit']}` |",
        f"| 실행 id | `{s['run']['run_id']}` |",
        "| 환경 | [env.json](env.json) |",
        f"| 표본 | 기존 {s['flow']['base_selected']}턴, 추가 {s['flow']['extension']['selected']}턴 |",
        "| 기준 조건 | B0의 첫 두 반복을 원본 파일에서 읽기만 재사용, 재호출 없음 |",
        "",
        "B1은 같은 주제·파일·이전 결과 참고와 같은 목표의 완성·교정을 구분하도록 질문을 바꿨다. B2는 기존 이어 가기 질문과 새 작업 질문을 같은 요청으로 물었다. `p>=t`와 `q<1-t`가 함께 성립할 때만 이어 가기로 보냈다. B3는 목표·진행 발췌를 줄이고 필드 순서를 바꾼 탐색 조건이었다. 첫 반복만 주 분석에 사용했다.",
        "",
        "## 설계와 다른 점",
        "",
        "가설·문장·기준값·결합 규칙·표본 선택은 수집 뒤 바꾸지 않았다. 분석·검증·보고 스크립트는 수집 중 추가했다. 실행 래퍼는 analyze에서 감사 집계와 보고서까지 재생성하도록 보완했다. 봉인 수집기의 중복 제외 수 누락은 당시 파일 해시와 바이트 접두를 확인한 읽기 전용 감사로 복원했다. 원본 응답과 수집기는 변경하지 않았다. 준비 스캔 완료 전에 실행한 라벨러 두 번과 judge 한 번은 표본 파일이 없어 외부 호출 전에 종료됐다. 준비 완료 뒤 다시 실행했으며 호출 장부에는 실제 예약만 셌다. 사전 등록한 추가 표본 전수 규칙을 적용했지만 추가 적격은 한 건뿐이었다. 필요한 새 작업 수를 채우지 못한 것은 아래 검정력 제한으로 남겼다.",
        "",
        "## 결과",
        "",
        "### 흐름",
        "",
        "| 단계 | 수 |",
        "|---|---|",
        *[f"| 추가 추출 감사 {k} | {v} |" for k, v in s["extraction_audit"].items()],
        *[
            f"| 추가 추출 {k} | {v} |"
            for k, v in s["flow"]["extraction_counts"].items()
        ],
        f"| 기존 선택 | {s['flow']['base_selected']} |",
        *[f"| 기존 {k} | {v} |" for k, v in s["flow"]["base_labels"].items()],
        f"| 프로젝트 / 세션 | {s['flow']['base_projects']} / {s['flow']['base_sessions']} |",
        f"| 추가 적격 / 선택 | {s['flow']['extension_eligible']} / {s['flow']['extension']['selected']} |",
        f"| 추가 최초 합의 / 재판정 | {s['flow']['extension']['initial_agreements']} / {s['flow']['extension']['adjudicated']} |",
        *[f"| 추가 {k} | {v} |" for k, v in s["flow"]["extension"]["final"].items()],
        "",
        "| 조건 | 유효 명확 라벨 | 명확 라벨 중 응답 실패·누락 | 전체 수집 응답 상태 |",
        "|---|---|---|---|",
    ]
    for b in ("B0", "B1", "B2", "B3"):
        r = next(r for r in s["curves"] if r["condition"] == b)
        lines.append(
            f"| {b} | {r['valid_clear']} | {r['missing_clear']} | {s['request_sizes'][b]['statuses']} |"
        )
    lines += [
        "",
        "uncertain은 정밀도·재현율·오접합 분모에서 제외했고 묻기에는 포함했다. 실패는 정상 새 작업 정답으로 세지 않았다. 전체 반복의 HTTP 상태·실패는 summary에 보존했다.",
        "",
        "### 확인 분석",
        "",
        "| 가설 | 0.80 오접합 | exact 경계 / cluster 구간 | 재현율 | exact 경계 / cluster 구간 | 판정 |",
        "|---|---|---|---|---|---|",
    ]
    for h, b in (("H1", "B1"), ("H2", "B2")):
        r = next(
            r for r in s["curves"] if r["condition"] == b and r["threshold"] == 0.8
        )
        f, rc = r["false_join_rate"], r["recall"]
        lines.append(
            f"| {h} {b} | {count(f)} | {span(f['exact_bounds'])} / {span(r['cluster']['false_join_rate']['ci'])} | {count(rc)} | {span(rc['exact_bounds'])} / {span(r['cluster']['recall']['ci'])} | {s['hypotheses'][b]} |"
        )
    return lines


def main() -> None:
    s = read(PUBLIC / "results/summary.json")
    candidate = s["candidate"]
    selected = next(
        (
            r
            for r in s["curves"]
            if candidate
            and r["condition"] == candidate["condition"]
            and r["threshold"] == candidate["threshold"]
        ),
        None,
    )
    choice = (
        f"{candidate['condition']} {candidate['threshold']:.2f}"
        if candidate
        else "없음"
    )
    summary = f"탐색 후보는 {choice}이다. "
    if selected:
        summary += f"오접합 {count(selected['false_join_rate'])}, 재현율 {count(selected['recall'])}, 정밀도 {count(selected['precision'])}이다. 오접합의 Wilson 95% 구간은 {span(selected['false_join_rate']['ci'])}, 세션 cluster 구간은 {span(selected['cluster']['false_join_rate']['ci'])}다. "
    summary += f"고정 0.80의 H1(B1)은 {s['hypotheses']['B1']}, H2(B2)는 {s['hypotheses']['B2']}이다. 개발 표본 재사용과 새 작업 표본 부족으로 운영 채택은 보류한다. 제품 설계와 기본값은 변경하지 않았다."
    lines = opening_section(s, summary)
    curves_section(s, lines)
    comparison_section(s, lines)
    collection_section(s, lines)
    discussion_section(s, lines, choice)
    text = "\n".join(lines) + "\n"
    while "\n\n\n" in text:
        text = text.replace("\n\n\n", "\n\n")
    (PUBLIC / "report.md").write_text(text)


if __name__ == "__main__":
    main()
