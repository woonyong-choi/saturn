"""공개 집계만 읽어 실험 보고서의 표와 판정을 만든다."""

from __future__ import annotations

from runtime import PUBLIC, read


def percent(value: float | None) -> str:
    return "측정 불가" if value is None else f"{100 * value:.1f}%"


def metric(value: dict) -> str:
    if value["value"] is None:
        return "측정 불가"
    lo, hi = value["ci"]
    return f"{value['k']}/{value['n']}, {percent(value['value'])} [{100 * lo:.1f}, {100 * hi:.1f}]"


def dollars(value: float | None) -> str:
    return "미상" if value is None else f"{value:.4g}"


def seconds(value: float | None) -> str:
    return "미상" if value is None else f"{value / 1000:.2f}"


def interval(values: list) -> str:
    return (
        f"[{100 * values[0]:.1f}, {100 * values[1]:.1f}]%p"
        if values[0] is not None
        else "측정 불가"
    )


def main() -> None:
    s = read(PUBLIC / "results/summary.json")
    c = s["conditions"]
    flow = s["flow"]
    recommendation = s["recommendation"] or "없음"
    candidate = s["point_candidate"] or "없음"
    lines = [
        "# 제약 해제·예외 판단의 Jev와 저렴한 LLM 비교: 실험 결과",
        "",
        "## 요약",
        "",
        f"L1의 요청 재현율은 {metric(c['constraint/L1']['metrics']['request_recall'])}, J1은 {metric(c['constraint/J1']['metrics']['request_recall'])}였다. 사전 등록한 점추정 통과 후보는 {candidate}, 검정과 군집 구간까지 통과한 권장 조건은 {recommendation}이다. 제품 설계와 기본값은 변경하지 않았다.",
        "",
        "## 방법",
        "",
        "| 항목 | 값 |",
        "|---|---|",
        f"| 설계 | [사전 등록](design.md), 봉인 커밋 `{s['run']['design_commit']}` |",
        f"| 실행 id | `{s['run']['run_id']}` |",
        "| 환경 | [env.json](env.json), 실제 응답 모델은 아래 표 |",
        f"| 표본 | 제약 {flow['constraint_selected']}개, 이어 가기 {flow['continuation_selected']}개 |",
        "| 비교 | 동일 입력마다 세 반복, 첫 반복만 성능 분모. J2는 별도 탐색 부분집합 |",
        "| 정답 | 독립 두 모델 합의와 범위 의미 재판정. 사람 검증 아님 |",
        "",
        "범위는 실제 예외 입력 전체를 분모로 했다. 게이트에서 요청을 놓치거나 출력 계약이 틀리면 대상·종류·종합 정확도에서도 오답으로 셌다. 종합 정확도는 요청 종류·대상·예외 종류·범위 의미가 모두 맞아야 성공이다. 실제 문장 비해제 입력은 합성 활성 목록에 놓인 스트레스 표본이었다.",
        "",
        "## 설계와 다른 점",
        "",
        "| 변경 | 시점 | 이유 | 결론에 미친 영향 |",
        "|---|---|---|---|",
        "| 분석·검증·보고 스크립트 추가 | 라벨 수집 중 | 저장 응답의 집계와 재현 검사 | 평가 질문·경계 유지 |",
        "| 분석 모듈 이름 변경 | 이어 가기 추출 중, 평가 전 | Python 표준 statistics 모듈과 이름 충돌 | 저장 라벨 재사용, 모델 재호출 없음 |",
        "| 이어 가기 불완전 라벨 묶음 제외 처리 보완 | 독립 라벨 중, 평가 전 | 응답에서 입력 ID 누락 | 사전 제외 규칙 적용, 해당 묶음 전체 제외 |",
        "| 비정상 JSON 외피 처리 보완 | 평가 전 | 객체가 아닌 외피의 예외 처리 누락 | 형식 오류로 집계 |",
        "| 단일 JSON 코드 블록 추출 탐색 추가 | 초기 평가 응답 확인 뒤 | Haiku가 JSON을 Markdown 포장으로 반환 | 확인 분석·권장 판정에 사용 금지, 저장 답의 의미 판단만 별도 분석 |",
        "",
        "앞 실험의 비공개 worktree 자료가 남아 있지 않은 점은 봉인 전에 확인해 설계에 기록했다. 해제 합성문은 공개 문장과 남아 있는 원제약의 ID를 연결했다. 이어 가기 표본은 같은 추출기로 같은 시기 원천에서 다시 뽑았으므로 앞 표본과 정확히 같은 부분집합임을 입증하지 못했다. 앞 실험 수치와 이번 수치를 대응 비교하지 않았다.",
        "",
        "## 결과",
        "",
        "### 흐름",
        "",
        "| 단계 | 수 |",
        "|---|---|",
        f"| 기존 합성문 복원 | {flow['restored']} |",
        f"| 새 예외 문장 생성 | {flow['generated']} |",
        f"| 실제 비해제 후보 | {flow['real_candidates']} |",
        f"| 실제 비해제 합의·선택 | {flow['real_accepted']}·{flow['real_selected']} |",
        f"| 라벨·중복·형식 제외 | {flow['exclusions']} |",
        f"| 범위 의미 불일치 제외 | {flow['scope_excluded']} |",
        f"| 최종 제약 표본 | {flow['constraint_selected']} |",
        f"| 최종 이어 가기 표본 | {flow['continuation_selected']} |",
        "",
        "| 정답 종류 | 수 | 실제 분모 검정력 |",
        "|---|---|---|",
    ]
    for reason, n in s["exclusion_reasons"].items():
        lines.insert(
            lines.index("| 정답 종류 | 수 | 실제 분모 검정력 |") - 1,
            f"| 제외 사유: {reason} | {n} |",
        )
    for reason, n in s["continuation_flow"]["excluded"].items():
        lines.insert(
            lines.index("| 정답 종류 | 수 | 실제 분모 검정력 |") - 1,
            f"| 이어 가기 제외: {reason} | {n} |",
        )
    for kind, n in flow["kinds"].items():
        p = s["observed_power"]["kinds"].get(kind)
        lines.append(f"| {kind} | {n} | {percent(p['power']) if p else '해당 없음'} |")
    lines += [
        "",
        "종류별 검정력은 사전 가정 p0=0.90·p1=0.99에서 실제 분모로 다시 계산한 명목값이다. 군집 상관과 다중 비교 보정으로 실제 검정력은 더 낮아질 수 있다.",
        "",
        "| 조건 | ok | invalid | 그 밖 실패·미완료 |",
        "|---|---|---|---|",
    ]
    for key, v in c.items():
        statuses = v["statuses"]
        lines.append(
            f"| {key} | {statuses.get('ok', 0)} | {statuses.get('invalid', 0)} | {sum(n for k, n in statuses.items() if k not in ('ok', 'invalid'))} |"
        )
    lines += [
        "",
        "### 확인 분석",
        "",
        "| 조건 | 종합 정확도 | 잘못된 영구 해제 | 요청 정밀도 | 요청 재현율 | 종류 정확도 |",
        "|---|---|---|---|---|---|",
    ]
    for key in ("constraint/J1", "constraint/L1", "constraint/J2"):
        m = c[key]["metrics"]
        lines.append(
            "| "
            + key
            + " | "
            + " | ".join(
                metric(m[name])
                for name in (
                    "accuracy",
                    "false_permanent",
                    "request_precision",
                    "request_recall",
                    "kind_accuracy",
                )
            )
            + " |"
        )
    lines += [
        "",
        "괄호 구간은 명목 Wilson 95%다. J2 행은 탐색 부분집합이며 J1·L1 전체 표본과 직접 순위를 비교하지 않는다. 위험률 분자는 잘못된 영구 종류와 잘못된 대상의 영구 해제를 모두 포함한다.",
        "",
        "| 조건 | 비영구 정답에서 영구 해제 | 영구 해제 출력 중 오류 | 대상 정확도 | 범위 의미 일치 | 범위 문자열 일치 |",
        "|---|---|---|---|---|---|",
    ]
    for key in ("constraint/J1", "constraint/L1", "constraint/J2"):
        m = c[key]["metrics"]
        lines.append(
            "| "
            + key
            + " | "
            + " | ".join(
                metric(m[name])
                for name in (
                    "false_permanent_nonpermanent",
                    "false_permanent_precision_error",
                    "target_accuracy",
                    "scope_semantic",
                    "scope_exact",
                )
            )
            + " |"
        )
    lines += ["", "| 가설 | 원 p | Holm p | 판정 |", "|---|---|---|---|"]
    for h, v in s["hypotheses"].items():
        lines.append(f"| {h} | {v['p']:.3g} | {v['holm_p']:.3g} | {v['verdict']} |")
    lines += [
        "",
        "| 조건 | 위험률 군집 구간 | 재현율 군집 구간 | 종류 정확도 군집 구간 |",
        "|---|---|---|---|",
    ]
    for key in ("constraint/J1", "constraint/L1"):
        ci = c[key]["cluster_intervals"]
        lines.append(
            "| "
            + key
            + " | "
            + " | ".join(
                interval(ci[n]["ci"]).replace("%p", "%")
                for n in ("false_permanent", "request_recall", "kind_accuracy")
            )
            + " |"
        )
    lines += [
        "",
        "### 지연·비용·반복",
        "",
        "| 조건 | 단계 지연 합 중앙값·95분위(초) | 평균 USD/워크플로우 | 평균 USD/호출 | 세 반복 JSON 일치 | 형식 오류 |",
        "|---|---|---|---|---|---|",
    ]
    for key, v in c.items():
        lines.append(
            f"| {key} | {seconds(v['latency_ms']['median'])}·{seconds(v['latency_ms']['p95'])} | {dollars(v['cost_usd']['mean'])} | {dollars(v['per_call_cost_usd']['mean'])} | {metric(v['repeat_agreement'])} | {metric(v['metrics']['format_error'])} |"
        )
    lines += [
        "",
        "지연은 실제 HTTP·CLI 실행 시간을 단계별로 합했다. 작업 대기열 시간은 뺐고 CLI 시작 시간은 포함했다. API만의 응답 시간 비교가 아니다. 비용은 전체 반복의 usage를 [설계의 공식 단가](design.md#분석)로 환산한 API 상당액이며, Claude 구독의 실제 추가 청구액을 뜻하지 않는다. 비용 미상 조건은 최소 비용 순위로 권장하지 않는다.",
        "",
        "| 조건 | 호출 수 | 알려진 비용 합(USD) | 비용 미상 호출 |",
        "|---|---|---|---|",
    ]
    for key, v in c.items():
        lines.append(
            f"| {key} | {v['call_count']} | {dollars(v['known_total_cost_usd'])} | {v['unknown_cost_calls']} |"
        )
    lines += [
        "",
        "### 이어 가기",
        "",
        "| 조건 | 정확도 | 재현율 | 새 작업 오접합 |",
        "|---|---|---|---|",
    ]
    for key in ("continuation/B1", "continuation/L1"):
        m = c[key]["metrics"]
        lines.append(
            "| "
            + key
            + " | "
            + " | ".join(metric(m[n]) for n in ("accuracy", "recall", "false_join"))
            + " |"
        )
    pair = s["comparisons"]["continuation/L1-minus-B1"]
    lines += [
        "",
        f"L1−B1 정확도 차이는 {100 * pair['difference']:.1f}%p, 대응 bootstrap 구간 {interval(pair['ci'])}, 세션 군집 구간 {interval(pair['cluster_ci'])}였다. L1만 정답 {pair['right_only']}개, B1만 정답 {pair['left_only']}개였고 정확 McNemar p={pair['p']:.3g}였다. 작은 차이와 희귀 오접합을 확정할 표본은 아니다.",
        "",
        "### 탐색 분석",
        "",
        "| 비교 | 대응 n | 뒤 조건만 정답 | 앞 조건만 정답 | 정확도 차이 | 대응 구간 |",
        "|---|---|---|---|---|---|",
    ]
    for key, v in s["comparisons"].items():
        if key.startswith("constraint"):
            lines.append(
                f"| {key} | {v['n']} | {v['right_only']} | {v['left_only']} | {100 * v['difference']:.1f}%p | {interval(v['ci'])} |"
            )
    m = s["exploratory_j1_threshold_080"]
    lines += [
        "",
        f"J1을 0.80으로 다시 계산하면 재현율은 {metric(m['request_recall'])}, 종합 정확도는 {metric(m['accuracy'])}였다. 이 재계산으로 확인 기준값을 바꾸지 않았다.",
        "",
        "| 조건·출처 | 종합 정확도 | 위험률 | 재현율 |",
        "|---|---|---|---|",
    ]
    for key, v in s["strata"].items():
        if key.startswith("constraint/") and "/source/" in key:
            m = v["metrics"]
            lines.append(
                "| "
                + key
                + " | "
                + " | ".join(
                    metric(m[n])
                    for n in ("accuracy", "false_permanent", "request_recall")
                )
                + " |"
            )
    lines += [
        "",
        "### JSON 코드 블록 추출 후 탐색",
        "",
        "초기 L1 응답에서 Markdown 포장이 관찰돼 수집 뒤 분석을 추가했다. 단일 JSON 코드 블록만 추출하고 블록 밖 설명을 무시했다. 키·값·종류·대상·범위는 수정하지 않았다. JSON과 출력 계약 검증을 다시 통과한 답만 채점했다. 추가 평가 호출은 없으며 원래 L1 형식 오류와 확인 가설은 그대로다.",
        "",
        "| 과제 | 종합 정확도 | 재현율 | 종류 정확도 | 위험률 또는 오접합 |",
        "|---|---|---|---|---|",
    ]
    for task, v in s["exploratory_unwrapped"].items():
        m = v["metrics"]
        recall = "request_recall" if task == "constraint" else "recall"
        risk = "false_permanent" if task == "constraint" else "false_join"
        kind = metric(m["kind_accuracy"]) if task == "constraint" else "해당 없음"
        lines.append(
            f"| {task}/L1 단일 블록 | {metric(m['accuracy'])} | {metric(m[recall])} | {kind} | {metric(m[risk])} |"
        )
    lines += [
        "",
        "이 탐색 수치는 Haiku의 의미 판단과 형식 실패를 구분하는 데만 사용한다. 네이티브 JSON Schema 옵션이나 포장 제거기를 포함한 새 워크플로우가 사전 기준을 통과했다고 주장하지 않는다.",
    ]
    lines += [
        "",
        "## 논의",
        "",
        "### 해석",
        "",
        f"사전 등록한 전체 입력 기준 위험률·요청 재현율·종류 정확도의 점추정 후보는 {candidate}다. 검정·군집 구간까지 고려한 권장은 {recommendation}이다. 종류만 맞아도 범위가 틀리면 종합 판단은 실패하며, 이 기준은 별도 단계 정확도와 함께 읽어야 한다. 실제 범위 예외의 만료·복구·실행은 측정하지 않았다.",
        "",
        "### 타당성 위협",
        "",
        "| 종류 | 위협 | 이 실험에서 |",
        "|---|---|---|",
        "| 내적 | 두 모델 합의로 쉬운 표본이 남음 | 불일치·모호함·형식 오류 제외, 사람 검증 미수행 |",
        "| 구성 | Jev 원문 후보 선택의 범위 제한 | 의미·문자열 일치와 종합 정확도 분리 |",
        "| 구성 | 지금은·잠깐의 만료 시점 부재 | 원문 보존만 평가, 자동 만료 지원 주장 제외 |",
        "| 외적 | 실제 제약 요청 양성 부재 | 실제 비해제와 합성 출처 분리 |",
        "| 재현 | 앞 비공개 자료 부재 | 해제 합성문 연결, 이어 가기 재추출 차이 명시 |",
        "| 통계 | 종류별 분모·원천 상관 | 실제 검정력·군집 구간·Holm 보정 적용 |",
        "| 시간 | CLI·캐시·동시 실행 영향 | 실제 모델·사용량·세 반복·지연 분포 보존 |",
        "",
        "### 한계",
        "",
        "- L2 생략: 독립 라벨·범위 판정과 L1 전체 반복을 먼저 수행한 시간 제약",
        "- 부분 영구 예외와 기한 예외를 scoped 한 종류로 합친 계약",
        "- J2는 탐색 부분집합이며 독립 확인 표본의 재검증 필요",
        "- 두 과제의 합의 표본에서 얻은 값이며 제품 전체 판단의 품질·비용으로 일반화 제외",
        "",
        "## 재현",
        "",
        "```sh",
        "./docs/experiments/constraint-exception-judge/run.sh verify",
        "./docs/experiments/constraint-exception-judge/run.sh analyze",
        "```",
        "",
        "분석은 [summary.json](results/summary.json)·[conditions.csv](results/conditions.csv)를 다시 만든다. 입력·응답의 해시는 [SHA256SUMS](data/SHA256SUMS)에 있고 비공개 원자료가 있어야 다시 계산할 수 있다.",
        "",
        "| 호출 종류 | 실제 예약 수 |",
        "|---|---|",
    ]
    for key, n in s["calls"].items():
        lines.append(f"| {key} | {n} |")
    lines += [
        "",
        "실패·미완료 예약도 호출 수에 포함했다. Jev는 재전송하지 않았고 CLI의 내부 네트워크 동작은 프로세스 호출 수와 구별된다.",
        "",
        "| 실제 Claude 모델 | 응답 수 |",
        "|---|---|",
    ]
    for key, n in s["claude_models"].items():
        lines.append(f"| {key} | {n} |")
    lines += [
        "",
        "정리 대상 Claude project 기록 폴더는 다음과 같다. 기록 파일의 원문은 공개하지 않았다.",
        "",
    ]
    for folder in s["claude_record_folders"]:
        lines.append(f"- `~/.claude/projects/{folder}/`")
    if not s["claude_record_folders"]:
        lines.append("해당 작업 경로의 project 기록 폴더는 발견되지 않았다.")
    lines += ["", "## 결론", "", "| 가설 | 판정 | 반영한 문서 |", "|---|---|---|"]
    for h, v in s["hypotheses"].items():
        lines.append(f"| {h} | {v['verdict']} | 없음 |")
    lines += [
        "",
        "사용자 지시에 따라 docs/design을 변경하지 않았다. PR은 보고서와 재현 스크립트만 포함하며 머지와 이슈 댓글은 수행하지 않았다.",
        "",
    ]
    (PUBLIC / "report.md").write_text("\n".join(lines))


if __name__ == "__main__":
    main()
