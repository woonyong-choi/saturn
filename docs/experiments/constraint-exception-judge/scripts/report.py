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
        f"Haiku의 저장 응답 {sum(s['claude_models'].values())}개는 모두 JSON을 코드 블록으로 감싸 반환했다. 사전 등록한 엄격 JSON 계약에서 형식 오류로 처리했으므로 L1 확인 결과는 의미 판단 능력의 추정이 아니다. 포장 추출과 Jev 수치 경계 보정은 아래 탐색 분석으로 분리했다.",
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
        "| 중단 예약의 재개 처리·미상 지연 보완 | 평가 재개 시 | 예약만 있고 응답 없는 회차 확인 | 해당 회차 실패 보존·재호출 금지, 지연 미상 유지 |",
        "| Jev 확률 합의 십진수 재검증 탐색 추가 | 분석 중 | 0.99 합을 이진 부동소수점 오차로 거절한 수집기 발견 | 원래 실패·확인 판정 보존, J1 저장 답만 별도 재계산 |",
        "| Jev 비용의 원응답 usage 복원 | 분석 중 | 확률 검증 실패 시 usage 필드도 누락 | 원응답의 실제 토큰 수로 비용 계산, 원파일 변경 없음 |",
        "| Claude 1시간 캐시 단가 반영 | 분석 중 | 사용량에 1시간 캐시 쓰기 관측 | 공식 단가로 환산해 CLI 총비용과 대조, 5분 단가 적용 제외 |",
        "| 동률 해석이 응답 키 순서를 사용 | 수집 중 구현, 분석 후 감사 | 설계는 선택지 삽입 순서지만 max가 서버 키 순서를 사용 | 영향 회차 수 보고, 원래 확인 결과 유지 |",
        "| 공개 생성문 가림·원자료 해시 봉인 보완 | 최종 검토 | 부분 원문과 분석 시 해시 재생성 문제 | 원천 목록 필수 검증, 원문 조각 가림, 불변 원자료 봉인 추가 |",
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
        f"| 중단 후 미완료 호출·워크플로우 | {s['interrupted']['calls']}·{s['interrupted']['workflows']} |",
        f"| 첫 반복에 포함된 미완료 | {s['interrupted']['first_repeat']} |",
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
    lines += [
        "",
        "| 조건·종류 | 종합 정확도 | 종류 정확도 | 대상 정확도 | 범위 의미 일치 |",
        "|---|---|---|---|---|",
    ]
    for key, value in s["strata"].items():
        if "/gold_kind/" not in key or key.endswith("/none"):
            continue
        m = value["metrics"]
        lines.append(
            "| "
            + key
            + " | "
            + " | ".join(
                metric(m[n])
                for n in (
                    "accuracy",
                    "kind_accuracy",
                    "target_accuracy",
                    "scope_semantic",
                )
            )
            + " |"
        )
    lines += [
        "",
        "| 실제 분모 | n | 사전 효과 가정에서의 검정력 |",
        "|---|---|---|",
    ]
    for name in ("recall", "false_permanent", "continuation"):
        p = s["observed_power"][name]
        lines.append(f"| {name} | {p['n']} | {percent(p['power'])} |")
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
        "미완료 호출은 비용·지연을 알 수 없어 해당 분포에서 제외했다. 실패는 정확도·재현율·세 반복 일치 분모에 남겼다. 위험률 0%라도 유효 출력 자체가 없으면 안전한 판단 모델이라는 근거가 아니다.",
        f"Claude 사용량에는 1시간 캐시 쓰기 {s['claude_usage']['cache_1h_input_tokens']}토큰이 있었다. 해당 토큰은 [공식 요금표](https://platform.claude.com/docs/en/about-claude/pricing)의 100만 토큰당 USD 2로 계산했다. 나머지 입력·출력·캐시 읽기·5분 쓰기는 봉인 단가를 유지했다.",
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
        f"정답은 continue {s['continuation_gold'].get('continue', 0)}개, new {s['continuation_gold'].get('new', 0)}개였다. 목표 표본보다 적은 실제 분모로 검정력을 다시 계산했다.",
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
    for task, pair in s["exploratory_unwrapped_comparisons"].items():
        lines += [
            "",
            f"{task}의 포장 추출 L1−Jev 정확도 차이는 {100 * pair['difference']:.1f}%p, 대응 구간 {interval(pair['ci'])}, 군집 구간 {interval(pair['cluster_ci'])}였다. L1만 정답 {pair['right_only']}개, Jev만 정답 {pair['left_only']}개, 보정 전 정확 McNemar p={pair['p']:.3g}였다. 사후 탐색이므로 확인 채택에 사용하지 않았다.",
        ]
    decimal = s["exploratory_jev_decimal"]
    m = decimal["metrics"]
    pair = s["exploratory_semantic_comparison"]
    lines += [
        "",
        "### Jev 확률 합 경계 보정 후 탐색",
        "",
        "수집기는 확률 합과 1의 차이가 0.01보다 큰지 float로 검사했다. 합이 0.99인 경계에서도 표현 오차 때문에 거절하는 경우가 있었다. 원응답을 Decimal로 다시 읽고 같은 선택지·유한 확률·합 허용 오차·최종 출력 계약을 검사했다. 저장된 J1 답만 재해석했으며 수집기·원응답·확인 분석은 바꾸지 않았다. J2는 앞 단계 실패 뒤 호출하지 않은 다음 단계가 있어 이 보정의 비교 대상에서 뺐다.",
        "",
        "| 조건 | 종합 정확도 | 잘못된 영구 해제 | 요청 재현율 | 종류 정확도 | 범위 의미 일치 |",
        "|---|---|---|---|---|---|",
        "| J1 십진수 경계 | "
        + " | ".join(
            metric(m[n])
            for n in (
                "accuracy",
                "false_permanent",
                "request_recall",
                "kind_accuracy",
                "scope_semantic",
            )
        )
        + " |",
        "",
        f"포장 추출 L1−경계 보정 J1의 정확도 차이는 {100 * pair['difference']:.1f}%p, 대응 구간 {interval(pair['ci'])}, 군집 구간 {interval(pair['cluster_ci'])}였다. L1만 정답 {pair['right_only']}개, J1만 정답 {pair['left_only']}개, 보정 전 정확 McNemar p={pair['p']:.3g}였다. 두 수리 모두 사후 탐색으로만 해석한다.",
        "",
        f"동률 감사에서는 J1·J2의 동률 질문 {s['tie_order_audit']['tied_questions']}개 중 {s['tie_order_audit']['different_choice_winners']}개의 최댓값 선택이 응답 키 순서와 설계 순서 사이에서 달랐다. 게이트·최종 계약까지 고려한 워크플로우 영향은 {s['tie_order_audit']['changed_workflows']}회, 첫 반복은 {s['tie_order_audit']['changed_first_repeat']}회였다. 이는 사전 등록 구현의 이탈이며 확인 응답을 사후 교체하지 않았다.",
    ]
    lines += [
        "",
        "## 논의",
        "",
        "### 해석",
        "",
        f"사전 등록한 전체 입력 기준 위험률·요청 재현율·종류 정확도의 점추정 후보는 {candidate}이다. 검정·군집 구간까지 고려한 권장 조건도 {recommendation}이다. 종류만 맞아도 범위가 틀리면 종합 판단은 실패하며, 이 기준은 별도 단계 정확도와 함께 읽어야 한다. 실제 범위 예외의 만료·복구·실행은 측정하지 않았다.",
        "",
        "수치 경계 보정 뒤에도 Jev는 위험률과 종류 정확도 기준을 함께 통과하지 못했고, 코드 블록 추출 뒤 Haiku도 재현율과 종류 정확도 기준을 통과하지 못했다. 따라서 이 결과로 Haiku로 교체할 근거는 없다. 이어 가기는 B1의 정확도·지연·비용이 관측상 유리했지만, 포장 추출 Haiku와의 정확 McNemar 검정은 유의하지 않았다. 새 작업의 희귀 오접합을 입증할 분모도 부족하다.",
        "",
        "### 타당성 위협",
        "",
        "| 종류 | 위협 | 이 실험에서 |",
        "|---|---|---|",
        "| 내적 | 두 모델 합의로 쉬운 표본에 치우칠 위험 | 불일치·모호함·형식 오류 제외, 사람 검증 미수행 |",
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
        "분석은 [summary.json](results/summary.json)·[conditions.csv](results/conditions.csv)를 다시 만든다. 수집·채점·검토가 끝난 뒤 [RAW_SHA256SUMS](data/RAW_SHA256SUMS)로 원자료를 봉인했다. 이후 분석은 먼저 이 불변 봉인을 검사하고, 파생 관측까지 포함한 [SHA256SUMS](data/SHA256SUMS)만 다시 만든다. 비공개 원자료가 있어야 재현할 수 있다.",
        "",
        f"공개 가림 원천 {s['publication']['source_files']}개는 앞 실험의 봉인 목록과 파일명·해시가 일치했다. 공개 생성문 {s['publication']['generated_rows']}개 중 {s['publication']['redacted_rows']}개에서 연속 {s['publication']['redaction_span_chars']}자 이상 원문 조각을 가렸다. 실제 사용자 입력은 공개하지 않았다.",
        "",
        "| 파일 | SHA-256 |",
        "|---|---|",
        f"| data/RAW_SHA256SUMS | `{s['publication']['raw_seal_sha256']}` |",
        f"| data/SHA256SUMS | `{s['publication']['derived_manifest_sha256']}` |",
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
