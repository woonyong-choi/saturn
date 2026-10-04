"""집계 JSON의 수치로 연구 보고서와 실험 목록을 갱신한다."""

from __future__ import annotations

from runtime import PUBLIC, read


def percent(value: float | None) -> str:
    return "측정 불가" if value is None else f"{100 * value:.1f}%"


def counts(values: dict) -> str:
    return ", ".join(f"{k} {v}" for k, v in values.items())


def bounds(values: list | None, scale: float = 100) -> str:
    if values is None:
        return "측정 불가"
    if scale == 1:
        return f"[{values[0]:.3g}, {values[1]:.3g}]"
    return f"[{values[0] * scale:.1f}, {values[1] * scale:.1f}]"


def measured(value: dict) -> str:
    return (
        f"{value['k']}/{value['n']} · {percent(value['value'])} {bounds(value['ci'])}"
    )


def table(headers: list[str], rows: list[list]) -> str:
    return "\n".join(
        ["| " + " | ".join(headers) + " |", "|" + "---|" * len(headers)]
        + ["| " + " | ".join(map(str, row)) + " |" for row in rows]
    )


def audit_section(s: dict) -> str:
    a = s["audit"]
    transitions = [[k.replace("->", " → "), v] for k, v in a["transitions"].items()]
    groups = [[k, measured(v)] for k, v in a["by_kind"].items()]
    return f"""## 앞 실험 점검 결과

### 정답 신뢰도

기존 표본은 100건이며 sol이 판단 가능으로 확정한 90건을 재판정했다. 나머지 10건을 확정 정답에 포함하지 않았다. 정의는 앞 실험과 같은 DEFINITION을 사용했고, Astra에는 기존 판정·모델 점수 없이 같은 앞뒤 자료를 주었다. 판정 절차와 근거 메시지 위치를 요구하는 지시만 바꿨다.

일치율은 {measured(a["agreement_rate"])}, Cohen κ는 {a["kappa"]:.3f}다. 기존 제약 {
        a["old_positive"]
    }건 중 유지된 것은 {measured(a["old_positive_retained"])}다. 새 양성은 {
        a["new_positive"]
    }건, 새 판단 불가는 {
        a["new_uncertain"]
    }건이다. 전체 일치율은 음성이 많은 분포에 영향을 받으며 양성의 신뢰도를 대신하지 못한다.

{table(["이전 sol → 새 Astra", "건수"], transitions)}

{table(["입력 형태 추정", "일치"], groups)}

### 사람이 읽을 수 있는 근거 대조

불일치 사례의 목표 입력·뒤 사용자 요청·두 판정 이유를 직접 대조했다. 원문과 모델 응답은 공개하지 않고 아래에 판단이 갈린 구조를 정리했다. 이는 근거 검토이며 사용자가 새 정답을 판정한 결과는 아니다. 어느 쪽 라벨도 덮어쓰지 않았다.

{
        table(
            ["Astra 분류", "건수", "근거에서 갈린 부분"],
            [
                [
                    "작업 한정",
                    a["disagreement_categories"].get("task_local", 0),
                    "같은 산출물의 수정·축약·윤문이나 완료 조건까지의 하위 과업을 서로 다른 작업으로 셌는지",
                ],
                [
                    "사후 정보 부족",
                    a["disagreement_categories"].get("insufficient_context", 0),
                    "재사용 가능한 문장이지만 뒤 맥락에는 같은 산출물만 있고 지속 적용 의도는 드러나지 않는지",
                ],
                [
                    "붙여넣기 채택",
                    a["disagreement_categories"].get("pasted_adoption", 0),
                    "새 작업마다 같은 운영 지침이 다시 주어졌을 때 기존 규칙의 지속인지 매번 새 채택인지",
                ],
                [
                    "작업 목표",
                    a["disagreement_categories"].get("task_goal", 0),
                    "현재 질문·산출물 요구를 뒤의 구현 진행을 보고 지속적인 작업 방식 규칙으로 바꿔 해석했는지",
                ],
                [
                    "지속 규칙",
                    a["disagreement_categories"].get("persistent", 0),
                    "과제 지시 안의 조건부 언어 규칙을 과제 종료 이후에도 유효하다고 해석했는지",
                ],
            ],
        )
    }

일부 이전 이유는 최신 입력에 없던 규칙을 뒤 진행에서 끌어왔다. 반대로 새 판정도 명시적인 채택과 단순 재주입의 경계에서 하나의 해석을 고른다. 따라서 두 판정의 불일치는 이전 sol의 오류율과 같지 않으며, 기존 정답을 확정 정답으로 쓰기 어렵다는 근거다. 확대 실험에서는 두 모델의 독립 합의와 다른 지시의 재판정으로 이 불확실성을 남겼다.

### Jev 질문 맥락

원자료의 Jev 요청 {a["question_audit"]["requests"]}개에서 질문 종류는 {
        a["question_audit"]["unique_questions"]
    }개였다. 질문은 “단일 요청을 넘어 적용되는 작업 방식 규칙”을 묻는 문장이었고, 작업 하나가 여러 요청에 걸칠 수 있다는 경계는 없었다. state의 정의 필드는 없었고 `previous_context`와 `latest_user_input`만 있었다. LLM에는 현재 작업 종료까지만 적용되는 지시를 제외하는 상세 DEFINITION이 별도로 있었다. 이 비교는 질문 정의가 같지 않았다.

현재 [core 질문](../../../saturn-terminal/core/src/routers/constraint.rs)은 의문형으로 구현돼 있다. 앞 실험 시점의 구현 부재와 현재 상태를 구분해야 한다. 현재 문장도 작업 하나짜리 제외를 명시하지 않는다. [router](../../design/router.md)는 등록·묻기 기준을 설명하며, 실행 질문의 직접 근거는 core다. 이번 current 조건은 현재 의문형을 사용했으므로 앞 실험의 선언형 요청을 완전히 재현한 것은 아니다.
"""


def result_section(s: dict) -> str:
    curves = s["curves"]
    mainrows = [
        [
            r["condition"],
            r["threshold"],
            measured(r["precision"]),
            measured(r["recall"]),
            r["unresolved_auto"],
        ]
        for r in curves
        if r["condition"] != "astra" and r["threshold"] in (0.8, 0.9)
    ]
    astra = next(
        r for r in curves if r["condition"] == "astra" and r["threshold"] == 0.5
    )
    contrasts = []
    for name in ("H1", "H3"):
        for r in s["hypotheses"][name]["comparisons"]:
            contrasts.append(
                [
                    name,
                    r["threshold"],
                    percent(r["value"]).replace("%", "%p"),
                    bounds(r["ci"]),
                    bounds(r["individual_ci"]),
                    f"{r['permutation_p']:.4g}"
                    if r["permutation_p"] is not None
                    else "측정 불가",
                    r["undefined_cluster"],
                ]
            )
    policies = [
        [
            r["condition"],
            r["lower"],
            measured(r["precision"]),
            measured(r["combined_recall"]),
            measured(r["ask_rate"]),
            "통과" if r["point_pass"] else "미달",
        ]
        for r in s["policies"]
        if r["threshold"] == 0.9
    ]
    repeatrows = [
        [c, t, measured(s["repeats"][c][str(t)])]
        for c in ("current", "scope", "scope_gate")
        for t in (0.5, 0.8, 0.9)
    ]
    performance_rows = []
    for c, r in s["hypotheses"]["H4"]["conditions"].items():
        performance_rows.append(
            [
                c,
                f"{r['median_s']:.3g}",
                f"{r['p95_s']:.3g}",
                bounds(r["p95_ci"], 1),
                f"{r['mean_cost_usd']:.4g}",
                f"{r['cost_ratio']:.4g}",
                bounds(r["cost_ratio_ci"], 1),
            ]
        )
    groups = [
        [
            c,
            k,
            measured(v["precision"]),
            measured(v["combined_recall"]),
            measured(v["ask_rate"]),
        ]
        for c, values in s["groups"].items()
        if c != "astra"
        for k, v in values.items()
    ]
    curve_rows = [
        [
            r["condition"],
            f"{r['threshold']:.2f}",
            percent(r["precision"]["value"]),
            percent(r["recall"]["value"]),
            percent(r["ask_rate"]["value"]),
            percent(r["combined_recall"]["value"]),
        ]
        for r in curves
        if r["condition"] != "astra"
    ]
    return f"""## 결과

### 흐름과 정답

{table(["항목", "수"], [["표본", s["sampling"]["selected"]], ["신규 입력", s["sampling"]["new"]], ["초기 sol·Astra 일치", measured(s["gold"]["initial_agreement"])], ["세 번째 판정 대상", s["gold"]["third"]], ["정답 제약", s["gold"]["labels"].get("constraint", 0)], ["정답 아님", s["gold"]["labels"].get("not_constraint", 0)], ["제외: 판단 불가", s["gold"]["labels"].get("uncertain", 0)], ["호출 실패", sum(v for k, v in s["calls"]["statuses"].items() if k != "ok")], ["미완료 예약", s["calls"]["incomplete"]]])}

초기 두 판정 중 유효한 쌍에서 라벨 일치는 {measured(s["gold"]["valid_pair_agreement"])}였고 의미 불일치는 {s["gold"]["semantic_disagreements"]}건이었다. 모델별 무효 판정 수는 {counts(s["gold"]["invalid_judgments"])}, 무효 묶음 수는 {counts(s["gold"]["invalid_batches"])}다. Sol의 해당 응답은 요청보다 많은 ID 항목을 반환했다. 프로세스 성공을 출력 계약 성공으로 세지 않았고 사전 규칙대로 묶음 전체를 제외했다. 일부 답을 골라 살리는 사후 수리는 하지 않았다.

근거 위치의 범위를 대조했을 때 잘못된 참조는 {s["evidence_quality"]["invalid_references"]}개, 영향 입력은 {s["evidence_quality"]["affected_inputs"]}건이고 최종 라벨별 수는 {counts(s["evidence_quality"]["affected_labels"])}다. 원응답의 위치를 추정해 고치지 않았다. 사전 파서는 evidence 목록의 구조를 검사했지만 위치 범위까지 강제하지 않았으므로 해당 라벨은 원래 분석에 남아 있다. 이는 합의 정답의 추가 한계다.

Sol 응답이 무효여서 최초 Astra와 다른 지시의 Astra 두 표로만 정리된 입력은 {s["gold"]["astra_only_resolved"]}건이다. 두 번의 독립 실행이지만 두 모델 간 합의와 같지는 않다. 이 부분은 참고 Astra 성능을 해석할 때 특히 조심해야 한다.

조건별 유효성 실패 관측 수는 {counts(s["invalid_by_condition"])}다. Jev는 세 반복 전체, Astra는 한 반복 기준이다. 형식·응답 모델 실패도 같은 수에 포함했다.

검정력 계산의 필요 양성은 {s["power"]["positive_required"]}건이고 실제 양성은 {s["power"]["actual_positive"]}건이다. 필요한 양성 수를 채웠더라도 정밀도의 예측 양성 수와 프로젝트 상관에 대한 검정력까지 확보한 것은 아니다.

### 확인 분석

{table(["가설", "판정"], [[name, r["verdict"]] for name, r in s["hypotheses"].items()])}

{table(["조건", "자동 기준", "자동 정밀도·95% 구간", "자동 재현율·95% 구간", "자동 후보 중 정답 미해소"], mainrows)}

{table(["비교", "자동 기준", "정밀도 차이", "군집 99.375% 구간", "입력 99.375% 구간", "군집 순열 p", "정밀도 미정 재표집 수"], contrasts)}

차이의 단위는 %p다. 프로젝트 군집 구간이 주 분석이며 입력 구간은 민감도 분석이다. 예측 양성 분모가 0인 재표집에서는 정밀도 차이를 만들지 않았다. 미정 재표집 비율이 설계 기준을 넘으면 유의해 보이는 점추정만으로 채택하지 않았다. 순열 검정은 프로젝트 단위 조건 교환이라 가능한 배정 수가 적다.

기준 0.9의 프로젝트 군집 95% 구간은 다음과 같다. 앞 표의 Wilson 구간과 같은 의미의 독립 표본 구간이 아니다.

{table(["조건", "정밀도 군집 구간", "재현율 군집 구간"], [[c, bounds(s["cluster_ci95"][c]["0.9"]["precision"]), bounds(s["cluster_ci95"][c]["0.9"]["recall"])] for c in ("current", "scope", "scope_gate")])}

### 자동 0.9와 묻기 하한

{table(["조건", "묻기 하한", "자동 정밀도", "결합 재현율", "묻기 비율", "점추정 기준"], policies)}

이 표의 구간은 설명용 95% Wilson 구간이다. H2 판정에는 정책과 지표를 함께 보정한 동시 구간을 썼으며 [집계 JSON](results/summary.json)의 policies.simultaneous에 있다. 묻기의 분모에는 정답 미해소 입력도 포함한다. 결합 재현율은 사용자가 묻기에 언제나 정답으로 답한다는 가정이다.

### 기준값 곡선

{table(["조건", "자동 기준", "정밀도", "자동 재현율", "묻기 비율", "결합 재현율"], curve_rows)}

곡선의 묻기 하한은 자동 기준과 0.7 중 작은 값이다. 0.7 이하에서는 묻기 구간이 없으며 결합 재현율은 자동 재현율과 같다. 다른 묻기 하한 조합은 [정책 표](results/tables/policies.csv), Astra를 포함한 전체 곡선은 [기준값 표](results/tables/thresholds.csv)에 있다. 곡선의 개별 점은 탐색 결과다.

### 반복 일치와 참고 분류기

{table(["조건", "자동 기준", "세 반복 행동 완전 일치"], repeatrows)}

자동·묻기·미등록 세 행동의 완전 일치이며 실패는 불일치다. 같은 입력의 반복을 독립 표본으로 늘려 세지 않았다. Astra 앞 맥락 분류의 기준 0.5 정확도는 {measured(astra["accuracy"])}, 정밀도는 {measured(astra["precision"])}, 재현율은 {measured(astra["recall"])}다. 정답 판정에 같은 모델 계열이 참여했으므로 객관적 정확도 상한으로 단정할 수 없다.

### 지연과 비용

{table(["조건", "중앙값 초", "p95 초", "p95 보정 구간 초", "평균 USD/입력", "현재 대비 비용비", "비용비 보정 구간"], performance_rows)}

지연에는 클라이언트 HTTP 처리와 네트워크가 포함되고 로컬 대기열 시간은 제외된다. 평균보다 느린 입력을 숨기지 않도록 p95로 1초 기준을 판정했다. 두 질문은 하나의 HTTP 요청으로 동시에 전달했다. Astra는 최대 네 입력 묶음이므로 요청 지연과 입력당 상각 지연을 구분했으며 개별 실시간 호출과 같지 않다. 비용은 실제 usage에 공식 단가를 적용한 API 상당액이다.

{table(["종류", "호출 예약 수", "알려진 비용 USD"], [[k, v, f"{s['calls']['cost_by_kind'][k]:.4g}"] for k, v in s["calls"]["reserved"].items()])}

단계별 호출 수는 다음과 같다: {counts(s["calls"]["by_phase"])}. 비용 미상은 {s["calls"]["cost_unknown"]}회, 저장 응답은 {s["calls"]["saved"]}개다. 호출 상한에는 실패와 미완료도 포함했다. CLI 내부 네트워크 재시도는 프로세스 호출 수와 별도로 세지 못했다.

### 신규 표본과 입력 형태

{table(["조건", "부분집합", "0.9 정밀도", "0.7 이상 결합 재현율", "묻기 비율"], groups)}

with_sol_vote는 유효한 Sol 판정이 있는 입력만 남긴 추가 탐색이다. 두 Astra 판정만으로 정리된 입력의 영향을 확인하기 위해 함께 표시했다.

신규 표본은 이전 표본과 다른 입력이지만 같은 사용자·프로젝트를 공유한다. 붙여넣기 추정은 실제 작성자 인증이 아니고 표본이 적다. 집단 간 차이를 작성자의 인과 효과로 해석하지 않는다. 프로젝트 군집 정확도·정밀도·재현율 구간은 집계 JSON의 cluster_ci95에 있다.
"""


# cost: io saved experiment artifact reads and writes; basis: estimate
def main() -> None:
    s = read(PUBLIC / "results/summary.json")
    a = s["audit"]
    selected = s["recommendation"]
    choice = (
        "사전 등록한 후보 중 모든 운영 기준을 만족하는 질문·기준값 조합이 없어 권장값은 없다."
        if not selected
        else f"잠정 후보는 {selected['condition']}, 자동 {selected['threshold']}, 묻기 하한 {selected['lower']}다. 점추정 선택이며 동시 구간을 통과하지 못하면 운영 기본값 확정 근거로 쓰지 않는다."
    )
    text = (
        f"""# 작업 하나짜리 규칙을 거르는 제약 판단: 연구 보고서

## 요약

앞 실험 정답과 독립 Astra 재판정은 {
            measured(a["agreement_rate"])
        } 일치했다. 기존 양성의 유지율은 {
            measured(a["old_positive_retained"])
        }여서 이전 정답을 확정 기준으로 보기 어렵다. 확대 표본 {
            s["sampling"]["selected"]
        }건에서 가설 판정은 {
            ", ".join(k + " " + v["verdict"] for k, v in s["hypotheses"].items())
        }이다. {choice} 제품 질문과 기준값은 변경하지 않았다.

## 배경

[#382](https://github.com/woonyong-choi/saturn/issues/382)의 마지막 사용자 결정은 현재 작업 하나에만 적용되는 지시를 제약에서 빼는 것이다. 제약은 모델이 바뀌거나 맥락을 정리해도 다른 작업에 계속 적용할 규칙이다. 이번 연구는 이 정의의 정답 신뢰도와 Jev 질문의 정보 차이를 먼저 점검한 뒤, 질문 수정과 자동 기준 0.9가 운영 기준을 만족하는지 검증했다. 기본 모델의 자동 선택은 다루지 않았다.

### 앞선 실험의 판단 흐름

{
            table(
                ["실험", "확인한 것", "다음 판단으로 이어진 이유"],
                [
                    [
                        "[deep](../constraint-deep/report.md)",
                        "실제 대화의 등록 확률 곡선과 대체·해제 판단",
                        "등록 기준을 높이면 정밀도와 재현율이 맞바뀌었고 관계 판단과 정답 합의에 문제가 남았다.",
                    ],
                    [
                        "[relation](../constraint-relation/report.md)",
                        "후보를 줄인 대체·해제 재검증",
                        "요청 크기를 줄여도 암묵적 대체 판단의 정밀도가 부족해 사용자가 등록 기록을 보고 취소하는 흐름으로 옮겼다.",
                    ],
                    [
                        "[cancel-request](../constraint-cancel-request/report.md)",
                        "등록 기록 뒤 명시적 해제 요청",
                        "요청 식별과 대상 선택은 개선됐지만 일회성·조건부 예외를 영구 해제로 처리하는 문제가 남았다.",
                    ],
                    [
                        "[exception-judge](../constraint-exception-judge/report.md)",
                        "해제와 여러 예외 종류의 복합 판단",
                        "형식 처리의 사후 분석을 거쳐 Jev 사용 방향을 정했으나 예외 종류 오류와 사용자의 수정 경로가 남았다.",
                    ],
                    [
                        "[human-check](../constraint-human-check/report.md)",
                        "경계 사례의 실제 사용자 판정과 Astra 정답 곡선",
                        "라벨러 신뢰도 차이와 붙여넣기 맥락 부족이 드러났다. 기존 등록·묻기 기준의 근거는 그때의 제약 정의였다.",
                    ],
                    [
                        "[model-compare](../constraint-model-compare/report.md)",
                        "뒤 대화를 본 sol 정답과 여러 모델 비교",
                        "작업 한정 규칙을 제외하자 Jev 자동 정밀도가 낮아졌다. 정답·질문 정의 차이를 먼저 검증해야 했다.",
                    ],
                ],
            )
        }

이 흐름은 등록과 해제를 같은 문제로 묶어 해결했다는 뜻이 아니다. 이번 실험은 등록 후보의 지속 범위만 다룬다. 과거의 높은 등록 정밀도를 새로운 정의에 그대로 옮기지 않는다.

## 가설

{
            table(
                ["가설", "사전 예측"],
                [
                    [
                        "H1",
                        "작업 한정 제외 정의가 0.8과 0.9 정밀도를 각각 5%p 이상 높임",
                    ],
                    [
                        "H2",
                        "자동 0.9에서 정밀도 90% 이상·결합 재현율 70% 이상·묻기 20% 이하",
                    ],
                    [
                        "H3",
                        "작업 범위 보조 질문이 단일 범위 질문보다 두 기준값의 정밀도를 5%p 이상 높임",
                    ],
                    [
                        "H4",
                        "두 수정 조건 모두 p95 1초 미만·현재 대비 평균 비용 1.5배 이하",
                    ],
                ],
            )
        }

## 방법

{
            table(
                ["항목", "값"],
                [
                    [
                        "사전 등록",
                        "[design.md](design.md), 커밋 `"
                        + s["design_seal"]["commit"]
                        + "`",
                    ],
                    ["환경", "[env.json](env.json)"],
                    [
                        "표본",
                        str(s["sampling"]["selected"])
                        + "건, 신규 "
                        + str(s["sampling"]["new"])
                        + "건",
                    ],
                    [
                        "프로젝트·세션",
                        str(s["sampling"]["projects"])
                        + "개·"
                        + str(s["sampling"]["sessions"])
                        + "개",
                    ],
                    ["입력 형태", counts(s["sampling"]["by_kind"])],
                    [
                        "정답",
                        "sol·Astra 독립 판정, 불일치에 다른 지시의 Astra, 두 표 이상 일치만 채택",
                    ],
                    [
                        "평가",
                        "동일한 앞 맥락만 제공. 현재 질문·범위 명시·범위 명시와 작업 한정 보조 질문. Jev 세 반복",
                    ],
                    [
                        "분석",
                        "첫 반복의 성능, 프로젝트 군집 구간, 가설 및 정책의 다중 비교 보정",
                    ],
                ],
            )
        }

정답은 뒤 대화까지 보았고 평가기는 앞 대화만 보았다. 뒤 정보가 있어도 지속 의도를 가리지 못하면 판단 불가로 남겼다. 같은 요청의 세 반복은 일치도와 지연 분석에 사용했고 정답 표본 수에는 더하지 않았다. 자세한 분모·제외·통계 경계는 사전 등록에 고정했다.

## 설계와 다른 점

수집 중 분석·검증·보고 스크립트를 추가했다. 정의·평가 질문·표본 수·가설·판정 경계는 변경하지 않았다. 분석은 공개 집계만 생성하고 원응답과 봉인 정답을 덮어쓰지 않았다. 기존 100건의 재사용과 신규 300건의 별도 분석은 사전 등록한 방식이다. Sol 형식 실패를 확인한 뒤 유효한 Sol 판정이 있는 입력만의 민감도 분석을 추가했다. 이 부분은 탐색 분석이며 사전 가설 판정에는 사용하지 않았다.

"""
        + audit_section(s)
        + result_section(s)
        + f"""
## 의사 판단 기준과 결론

자동 정밀도 90% 이상, 자동과 묻기의 결합 재현율 70% 이상, 전체 입력의 묻기 비율 20% 이하, 자동 예측 양성 최소 10건을 동시에 요구했다. 통과 후보에서 현재 질문, 범위 단일 질문, 두 질문 순으로 가장 단순한 조건을 선택하고, 같은 조건에서는 더 낮은 자동 기준과 더 높은 묻기 하한을 택했다.

{choice} 가설의 통계 판정과 점추정 후보 선택은 구분한다.

0.9에서는 범위 명시의 정밀도가 현재 문장보다 높아졌지만, 자동 등록의 예측 양성과 실제 제약을 잡는 수가 모두 작았다. 묻기 하한을 사전 탐색 범위의 최저값으로 낮춰도 결합 재현율은 목표에 못 미쳤다. 보조 질문은 0.9 정밀도를 더 높이지 못했다. 단순히 문장을 자세히 쓰거나 기준값을 0.9로 높이는 방법으로 운영 기준을 충족했다고 볼 수 없다.

H1은 프로젝트 군집 구간이 넓어 개선 방향을 확정하지 못했다. H2는 점추정 목표를 모두 만족한 정책이 없었지만 다중 비교 구간까지 모든 정책의 실패를 확정하지 못해 보류다. H3는 0.9에서 정밀도 차이가 정의되지 않는 재표집 비율이 사전 한도를 넘어 보류다. 보류는 운영 승인이라는 뜻이 아니다. H4만 지연과 비용 구간으로 채택했다.

0.9를 지원하도록 설정 범위를 넓힐 수 있다는 구현 가능성과, 0.9를 기본값으로 채택할 성능 근거는 다른 판단이다.

검증한 질문 원문은 [protocol.py](scripts/protocol.py)의 SCOPE_QUESTION과 TASK_QUESTION에 있다. 단일 범위 질문은 “다른 작업에도 남아야 하는 작업 방식 규칙인가, 현재 작업에만 적용되는 지시는 제외한다”를 묻는다. 보조 질문은 “작업 방식 규칙 전부가 이번 작업에만 한정되는가”를 묻고, 하나라도 지속 규칙이면 필터로 버리지 않도록 정의했다. 추천 여부는 위 선택 결과를 따른다. 제품의 질문 문장·등록 기준·설계 문서는 이 PR에서 바꾸지 않았다.

## 한계

- Codex CLI는 저장소 아래 작업 디렉터리에서 실행했다. `--ignore-user-config`는 사용자 설정 파일을 제외하지만 프로젝트 지침 차단을 보장하지 않는다. 상위 경로에서 AGENTS.md 두 개를 확인했고 실행 명령에는 프로젝트 문서 주입을 끄는 설정이 없었다. ephemeral JSON 이벤트에 최종 시스템 지침이 없어 실제 주입 여부와 영향은 확인하지 못했다. 명시적으로 보낸 판정 프롬프트에서는 프로젝트명을 가렸지만, 전체 실행 맥락의 눈가림과 지시 동일성은 보장하지 못한다.
- 봉인한 수집 코드의 인증·모델 거절 중단은 대기 작업 취소와 재시작을 완전히 처리하지 못한다. 병렬 제출된 호출이 계속되거나 저장된 거절을 건너뛸 수 있다. 이번 실행의 거절은 0회여서 관측된 누락·초과 호출은 없었으나, 새 수집에는 영속 중단 상태와 제한된 작업 제출을 적용한 새 프로토콜이 필요하다. 봉인 코드와 원응답은 바꾸지 않았으며 현재 자료의 analyze·verify에는 추가 호출이 없다.
- 정답은 모델 합의다. 사람 정답을 대신하지 않으며 일치한 오해도 남을 수 있다.
- 한 사용자와 소수 프로젝트, 특히 한 프로젝트에 편중된 표본이다. 층화 표본의 정밀도를 전체 제품 발생률에 일반화하지 않는다.
- 붙여넣기 추정 표본이 적고 새 표본에서 해당 형태가 늘지 않았다. 자동 요약과 실제 사용자 채택의 구분도 추출 규칙으로 인증하지 못한다.
- 앞뒤 맥락 창 밖의 작업 변경과 제약 해제는 보이지 않는다. 사후 정답과 사전 판단 사이에는 정보 차이가 있다.
- 표본 수 계산은 앞 실험의 기존 양성 비율을 사용했다. 독립 점검에서 양성이 줄었으므로 계획 검정력을 확보했다고 단정할 수 없고 실제 양성 수와 구간으로 판단한다.
- 기존 표본을 재사용했고 같은 자료에서 기준값 후보를 골랐다. 독립 사용자·프로젝트의 확인 없이 확정 운영값으로 승격하지 않는다.
- 범위 질문은 작업 한정 제외 외에 모델 변경·맥락 정리·산출물 요구·붙여넣기 채택도 함께 명시했다. 개별 구절의 효과는 분리하지 못한다.
- 실제 engine의 전체 state, 다른 route 질문과의 상호작용, 문장 분리, 저장, 사용자 확인 UI는 측정하지 않았다.
- 서비스 부하·CLI 시작·캐시·모델 변경은 시간과 비용에 영향을 준다. 이번 관측을 상시 지연 보장으로 해석하지 않는다.

## 재현

```sh
./docs/experiments/constraint-scope/run.sh analyze
./docs/experiments/constraint-scope/run.sh verify
PYTHONPATH=docs/experiments/constraint-scope/scripts python3 -m unittest discover -s docs/experiments/constraint-scope/scripts/tests
```

verify는 키체인 값을 프로세스 환경으로 받은 상태에서 정확 키 잔존 여부를 검사한다. 원문·응답은 비공개 실험 저장소에만 있으며 공개 저장소에는 [데이터 설명](data/README.md), [해시](data/SHA256SUMS), 코드와 집계만 있다. analyze는 저장 원응답에서 정규화와 통계를 다시 만든다. 보고서의 측정 수치는 [summary.json](results/summary.json)에서 읽는다. 판정 근거·제한·사전 규칙은 본문과 설계 문서에서 함께 읽어야 한다.
"""
    )
    (PUBLIC / "report.md").write_text(text)
    index = PUBLIC.parent / "README.md"
    verdict = "; ".join(k + " " + v["verdict"] for k, v in s["hypotheses"].items())
    ending = (
        "권장 조합 없음"
        if not selected
        else f"잠정 {selected['condition']} {selected['threshold']}/{selected['lower']}"
    )
    new = f"| [constraint-scope](constraint-scope/report.md) | 작업 한정 지시 제외와 0.9 자동 등록 검증 | [제약](../design/constraints.md), [router](../design/router.md) | {verdict}: {ending} |"
    index.write_text(
        "\n".join(
            new if "[constraint-scope]" in line else line
            for line in index.read_text().splitlines()
        )
        + "\n"
    )


if __name__ == "__main__":
    main()
