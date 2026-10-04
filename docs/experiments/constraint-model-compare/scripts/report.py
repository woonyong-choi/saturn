"""집계 JSON의 값으로만 보고서 표를 만든다."""

from __future__ import annotations

from runtime import PUBLIC, read


def percent(value: float | None) -> str:
    return "측정 불가" if value is None else f"{value * 100:.1f}%"


def measure(value: dict) -> str:
    estimate = percent(value["value"])
    interval = value.get("ci95")
    if interval:
        estimate += f" [{interval[0] * 100:.1f}, {interval[1] * 100:.1f}]"
    if "k" in value:
        estimate = f"{value['k']}/{value['n']} · " + estimate
    return estimate


def number(value: float | None, digits: int = 3) -> str:
    return "미상" if value is None else f"{value:.{digits}g}"


def mapping(value: dict) -> str:
    return ", ".join(f"{key}: {count}" for key, count in value.items()) or "없음"


def table(headers: list[str], rows: list[list[str]]) -> str:
    return "\n".join(
        [
            "| " + " | ".join(headers) + " |",
            "|" + "---|" * len(headers),
            *["| " + " | ".join(row) + " |" for row in rows],
        ]
    )


def model_table(models: dict) -> str:
    return table(
        [
            "모델",
            "정확도",
            "정밀도",
            "재현율",
            "F1",
            "형식 실패",
            "지연 중앙 초",
            "USD/호출",
        ],
        [
            [
                name,
                *[
                    measure(row[key])
                    for key in (
                        "accuracy",
                        "precision",
                        "recall",
                        "f1",
                        "format_failure",
                    )
                ],
                number(row["latency_median_s"]),
                number(row["cost_mean_usd"], 4),
            ]
            for name, row in models.items()
        ],
    )


def policy_table(summary: dict) -> str:
    rows = [("현재", summary["current_policy"])]
    if summary["selected"]["selected_policy"]:
        label = (
            "잠정 후보"
            if summary["selected"]["auto_candidate"] is not None
            else "자동 0.80 고정·묻기 탐색"
        )
        rows.append((label, summary["selected"]["selected_policy"]))
    return table(
        [
            "정책",
            "자동",
            "묻기 하한",
            "자동 정밀도",
            "묻기 비율",
            "묻기 양성 비율",
            "합친 재현율",
        ],
        [
            [
                name,
                str(row["auto"]),
                str(row["ask"]),
                *[
                    measure(row[key])
                    for key in (
                        "auto_precision",
                        "ask_rate",
                        "asked_precision",
                        "combined_recall",
                    )
                ],
            ]
            for name, row in rows
        ],
    )


def render(summary: dict) -> str:
    sample, gold = summary["sampling"], summary["gold"]
    selected = summary["selected"]
    curve = table(
        ["기준값", "정밀도", "재현율", "F1"],
        [
            [
                f"{r['threshold']:.2f}",
                measure(r["precision"]),
                measure(r["recall"]),
                measure(r["f1"]),
            ]
            for r in summary["jev_curve"]
        ],
    )
    groups = []
    for name in summary["models"]:
        human = summary["input_kinds"]["human_likely"][name]
        pasted = summary["input_kinds"]["ai_instruction_likely"][name]
        groups.append(
            [
                name,
                str(human["counts"]["n"]),
                measure(human["accuracy"]),
                measure(human["precision"]),
                measure(human["recall"]),
                str(pasted["counts"]["n"]),
                measure(pasted["accuracy"]),
                measure(pasted["precision"]),
                measure(pasted["recall"]),
            ]
        )
    group_table = table(
        [
            "모델",
            "직접 추정 n",
            "정확도",
            "정밀도",
            "재현율",
            "붙여넣기 추정 n",
            "정확도",
            "정밀도",
            "재현율",
        ],
        groups,
    )
    failures = table(
        ["모델", "상태별 수", "코드 블록 추출", "여러 줄 JSON", "비용 미상"],
        [
            [
                name,
                mapping(r["statuses"]),
                str(r["wrapped"]),
                str(r["multiline"]),
                str(r["cost_unknown"]),
            ]
            for name, r in summary["models"].items()
        ],
    )
    hypotheses = table(
        ["가설", "목표", "관측", "동시 97.5% 구간", "판정"],
        [
            [
                name,
                percent(r["target"]),
                measure(r["metric"]),
                str([round(v * 100, 1) for v in r["simultaneous_ci975"]])
                if r["simultaneous_ci975"]
                else "측정 불가",
                r["decision"],
            ]
            for name, r in summary["hypotheses"].items()
        ],
    )
    clusters = table(
        ["모델", "정확도", "정밀도", "재현율", "F1"],
        [
            [
                name,
                *[
                    "측정 불가"
                    if r["cluster_ci95"][key] is None
                    else f"[{r['cluster_ci95'][key][0] * 100:.1f}, {r['cluster_ci95'][key][1] * 100:.1f}]"
                    for key in ("accuracy", "precision", "recall", "f1")
                ],
            ]
            for name, r in summary["models"].items()
        ],
    )
    differences = table(
        ["모델−Jev", "n", "Jev만 정답", "모델만 정답", "정확도 차이·대응 95% 구간"],
        [
            [
                name,
                str(r["n"]),
                str(r.get("first_only")),
                str(r.get("second_only")),
                measure(r).replace("%", "%p"),
            ]
            for name, r in summary["paired_vs_jev"].items()
        ],
    )
    repeats = summary["jev_repeats"]
    repeat_table = table(
        ["기준", "세 반복 완전 일치"],
        [[key, measure(value)] for key, value in repeats["classification"].items()],
    )
    flow = [
        ["원기록 파일", str(sample["files"])],
        ["추출 사용자 입력", str(sample["extracted_users"])],
        *[["제외: " + key, str(value)] for key, value in sample["excluded"].items()],
        ["적격", str(sample["eligible"])],
        ["표본", str(sample["selected"])],
        ["판단 불가", str(gold["labels"].get("uncertain", 0))],
        ["분석", str(summary["models"]["jev"]["counts"]["n"])],
    ]
    choice = (
        "자동 등록 후보 없음"
        if selected["auto_candidate"] is None
        else f"자동 등록 잠정 후보 {selected['auto_candidate']:.2f}"
    )
    robustness = (
        "통과했다" if selected["precision_lower_bounds_pass"] else "통과하지 못했다"
    )
    ask_goal = "통과했다" if selected["ask_targets_pass"] else "통과하지 못했다"
    ask_note = (
        f"묻기 하한 탐색값은 {selected['selected_policy']['ask']:.2f}다. "
        if selected["selected_policy"]
        else "묻기 부담 조건에 맞는 하한도 없었다. "
    )
    if selected["auto_candidate"] is None:
        ask_note += "자동 등록 후보가 없어 묻기 탐색에서만 현재 자동 기준 0.80을 고정했다. 이 조합을 새 운영 기준으로 권장하지 않는다."
    baseline = (
        gold["labels"].get("not_constraint", 0)
        / summary["models"]["jev"]["counts"]["n"]
    )
    astra = summary["models"]["astra"]
    sol = summary["models"]["sol"]
    high = next(r for r in summary["jev_curve"] if r["threshold"] == 0.9)
    current = summary["current_policy"]
    next_policy = selected["selected_policy"]
    direct = summary["input_kinds"]["human_likely"]
    pasted = summary["input_kinds"]["ai_instruction_likely"]
    return f"""# 사후 정답 기반 제약 판단 모델 비교: 실험 결과

## 요약

실제 대화에서 {sample["selected"]}개를 뽑아 sol로 사후 정답을 만들었다. 최초 두 판정의 자기 일치율은 {measure(gold["self_agreement"])}였고, 판단 불가 {gold["labels"].get("uncertain", 0)}개를 정확도 분석에서 제외했다. Jev 현재 자동 기준의 정밀도는 {measure(summary["current_policy"]["auto_precision"])}, 자동·묻기를 합친 재현율은 {measure(summary["current_policy"]["combined_recall"])}였다. 사전 선택 규칙의 결과는 {choice}다. 제품 기준값은 변경하지 않았다.

## 방법

| 항목 | 값 |
|---|---|
| 설계 | [실험 설계](design.md), 봉인 커밋 `{summary["design_commit"]}` |
| 환경 | [env.json](env.json), 공식 CLI와 Jev HTTPS |
| 표본 | {sample["selected_projects"]}개 프로젝트·{sample["selected_sessions"]}개 세션·{sample["selected"]}개 입력 |
| 입력 형태 | 직접 입력 추정 {sample["by_kind"].get("human_likely", 0)}, 지시문 붙여넣기 추정 {sample["by_kind"].get("ai_instruction_likely", 0)} |
| 길이 층 | {mapping(sample["by_band"])} |
| 분석 단위 | 같은 입력, Jev 첫 반복. 나머지 반복은 안정성만 측정 |

정답은 뒤 사용자 입력과 assistant 진행을 본 sol의 독립 두 판정으로 만들었고 불일치 {gold["third_votes"]}개만 세 번째로 물었다. 정답 파일을 봉인한 뒤 모든 모델에 같은 앞 맥락과 입력만 주었다. 코드 블록은 사전 규칙대로 추출했다. 판단 지침과 모델 응답을 사후에 수정하지 않았다.

## 설계와 다른 점

| 변경 | 시점 | 이유 | 결론에 미친 영향 |
|---|---|---|---|
| Codex 알림 이벤트 파서 수정 | 정답 수집 중 | 도구 실행이 아닌 item.error를 도구 호출로 잘못 계산 | 저장된 {summary["deviations"]["parser_resume_saved"]}개 응답 재해석, 손실 {summary["deviations"]["parser_resume_missing"]}개·추가 호출 {summary["deviations"]["extra_calls_for_parser"]}회 |
| 중복 UUID를 프로젝트 길이에서도 제거 | 정답 수집 중 코드 감사 | 기존 코드는 후보에서만 중복 제거 | 선택 표본의 길이 층 변경 {summary["source_audit"]["selected_band_changes"]}개, 표본 재추출 없음 |
| 영구 거절·응답 모델·원자 저장 검사 보완 | 수집 중 정적 검토 | 실패 후 호출 누적·중단 후 불완전 봉인 방지 | 판정 지침·상한 유지 |
| 분석 코드·형식 실패의 usage 보존 추가 | 수집 중 | 사전 통계 규칙 구현·비용 누락 방지 | 원응답 보존, 형식 실패와 판단 오류 분리 |
| 실행 중 shell 진입점 수정 뒤 종료 오류 | 정답 봉인 직후 | shell의 변경된 파일 뒷부분 재열람 | 봉인·원응답 정상 확인 뒤 query 단계 재개, 모델 재호출 없음 |
| sol·terra 단가 근거 보완 | 수집 중 | 일반 요금표 대신 해당 모델 공식 페이지에서 확인 | 다른 세대 단가 대입 없이 호출당 비용 계산 |
| boolean 단독 정확도 탐색 추가 | 질의 수집 중 | Luna의 bool과 확률 의미 불일치 관측 | 확신값을 보정하지 않고 boolean 값만 별도 채점, 원래 확인 분석 유지 |
| 설계 표 문체 교정 | 수집 중 | 명사형 표기 검사 | 가설·수치·판정 기준 변경 없음 |

## 결과

### 흐름

{table(["단계", "수"], flow)}

최종 정답 분포는 `{gold["labels"]}`였고 라벨 형식 실패는 {gold["invalid_judgments"]}개였다. 전체 최초 쌍을 분모로 한 일치율은 {measure(gold["all_pair_agreement"])}다. 일치에는 판단 불가끼리의 일치도 포함하며, 인간 정답에 대한 정확도와 다르다.

### 확인 분석

{hypotheses}

### 모델 비교

{model_table(summary["models"])}

모든 입력을 제약 아님으로 답하는 기준 정확도는 {percent(baseline)}다. astra는 정확도 {astra["accuracy"]["k"]}/{astra["accuracy"]["n"]}·F1 {percent(astra["f1"]["value"])}로 가장 높았고, sol은 정밀도 {sol["precision"]["k"]}/{sol["precision"]["n"]}·재현율 {sol["recall"]["k"]}/{sol["recall"]["n"]}으로 놓치는 제약이 더 많았다. 이는 이 표본의 점추정 비교이며 별도 순위 검정의 결론은 아니다.

모델 비교의 이진 기준은 모두 P(제약) 기준이다. 표의 구간은 Wilson 명목 구간이고 F1은 입력 재표집 구간이다. 형식 실패는 정답 불가까지 포함한 전체 질의에서 셌고 정확도에서는 오답으로 남겼다. 지연과 호출당 비용 표는 첫 반복 기준이다. 지연은 CLI 시작을 포함한 벽시계 중앙값이다. 모델별 통로와 로컬 Rust 검사가 같은 컴퓨터에서 겹쳤으므로 격리된 API 지연으로 해석하지 않는다. 비용은 실제 usage와 [사전 단가](design.md#비용)로 계산한 API 상당액이며 구독 추가 청구액이 아니다. 일반 요금표에서 찾지 못했던 [sol 단가](https://developers.openai.com/api/docs/models/gpt-6-sol)와 [terra 단가](https://developers.openai.com/api/docs/models/gpt-5.6-terra)는 수집 중 공식 모델 페이지에서 확인해 보완했다. Codex는 Standard API 상당액을 사용하며 CLI 구독 요금과 구별한다.

프로젝트 군집을 다시 뽑은 구간은 다음과 같았다. 프로젝트가 적어 이 구간도 안정적인 모집단 보장은 아니다.

{clusters}

{failures}

### boolean 판단만 채점한 탐색

{table(["모델", "boolean 단독 정확도", "계약 실패 종류"], [[name, measure(row["boolean_only_exploratory"]), mapping(row["format_reasons"])] for name, row in summary["models"].items()])}

Luna의 일부 답은 false와 높은 P(제약)을 함께 반환했다. 원래 분석에서는 계약 실패이며, 위 표는 유효한 boolean 필드가 있는 응답만 사후에 다시 채점한 탐색이다. 확률을 뒤집거나 새 호출로 답을 교체하지 않았다. JSON 자체를 해석할 수 없는 답은 이 탐색의 분모에서도 제외했다.

### 입력 형태별 차이

{group_table}

입력 형태는 문장 구조를 기준으로 한 추정이며 실제 작성자를 인증하지 못했다. 구조화된 지시문은 적격 모집단에 있던 {sample["eligible_by_kind"].get("ai_instruction_likely", 0)}개를 모두 포함했다. 사람의 구조화된 글과 AI의 짧은 글이 섞일 수 있어 이 차이를 작성 주체의 인과 효과로 해석하지 않는다. 집단별 F1·형식 실패·지연·비용은 [집계 JSON](results/summary.json)에 함께 보존했다.

### Jev 곡선과 기준값

![Jev는 기준값을 높여도 관측 정밀도 90%에 도달하지 못하고 재현율이 감소](../../assets/constraint-model-compare-precision-recall.svg)

![Jev F1은 측정한 기준값 중 0.80에서 가장 높지만 구간이 넓게 중첩](../../assets/constraint-model-compare-f1.svg)

{curve}

{policy_table(summary)}

선택 결과는 {choice}다. {ask_note} 정밀도 명목·군집 구간 하한의 목표는 {robustness}. 묻기 부담과 결합 재현율 목표는 {ask_goal}. 기준값 선택은 같은 표본을 사용한 탐색이므로 독립 표본 확인 전 확정 기본값으로 권장하지 않는다. 결합 재현율은 사용자 확인이 항상 정답이라는 가정이며 실제 사용자 응답을 측정한 값이 아니다.

0.90에서는 정밀도 {high["precision"]["k"]}/{high["precision"]["n"]}으로 목표 90%와 최소 양성 10개를 모두 충족하지 못했다. 0.95의 정밀도는 예측 양성이 없어 측정 불가이며 100%가 아니다. 묻기 하한을 {current["ask"]:.2f}에서 {next_policy["ask"]:.2f}으로 낮추면 질문은 {current["ask_rate"]["k"]}개에서 {next_policy["ask_rate"]["k"]}개로 늘고, 잡는 실제 제약은 {current["combined_recall"]["k"]}개에서 {next_policy["combined_recall"]["k"]}개로 늘었다. 이번 결과로 자동 등록 기준을 채택하거나 묻기 하한을 운영에 반영하는 제안은 보류한다.

### 반복·대응 비교

{repeat_table}

Jev 확률 자체가 모두 같은 비율은 {measure(repeats["exact_probability"])}였고, 유효 세 반복 {repeats["complete"]}개에서 확률 표준편차 평균은 {number(repeats["probability_sd_mean"], 4)}이었다.

{differences}

차이는 같은 입력의 정오 차이를 재표집한 탐색 구간이다. 모델별 순위가 독립 확인 검정을 통과했다는 뜻은 아니다.

### 호출과 비용

{table(["종류", "실제 예약 수"], [[name, str(count)] for name, count in summary["calls"].items()])}

단계별 예약 수는 `{summary["call_phases"]}`였다. 실패·미완료도 호출에 포함했고 예약한 요청을 재전송하지 않았다. CLI 내부 네트워크 재시도는 프로세스 호출 수에 포함해 분해하지 못했다. 평가의 알려진 비용 합은 USD {number(summary["costs"]["known_query_usd"], 5)}, 비용 미상은 {summary["costs"]["unknown_query_calls"]}회였다. 정답 생성과 모델 확인은 모델 비교의 호출당 비용에 넣지 않았다.

Claude 응답 모델은 `{summary["actual_models"]}`로 집계했다. Codex는 공식 model/list와 명시적 CLI 모델 인수로 고정했으며 exec JSON에 별도 실제 모델 필드는 없었다. Claude 기록 폴더는 `~/.claude/projects/-Users-woonyong-workspace-oss-saturn-wt-experiment-382-model-compare--runtime-claude-work/`다.

## 논의

### 해석

이번 정답은 실제 뒤 진행을 사용해 현재 작업 지시와 지속 규칙을 구분했다. 그러나 sol의 사후 판정과 sol 자신의 사전 판단을 비교하므로 sol에 유리한 정의·표현 편향이 있을 수 있다. 자기 일치율은 이 편향을 제거하지 못한다. 사람 판정 없이 제품 수준의 정답이라고 확정하지 않는다.

직접 입력 추정군과 지시문 붙여넣기 추정군의 분석 분모는 각각 {direct["jev"]["counts"]["n"]}개와 {pasted["jev"]["counts"]["n"]}개였다. Jev 정확도는 {percent(direct["jev"]["accuracy"]["value"])}에서 {percent(pasted["jev"]["accuracy"]["value"])}로, Sonnet은 {percent(direct["sonnet"]["accuracy"]["value"])}에서 {percent(pasted["sonnet"]["accuracy"]["value"])}로 낮아졌고 astra는 {percent(direct["astra"]["accuracy"]["value"])}와 {percent(pasted["astra"]["accuracy"]["value"])}였다. 붙여넣기 추정군의 실제 제약은 {pasted["jev"]["counts"]["positives"]}개뿐이어서 모델별 재현율 차이도 매우 불안정하다.

Jev에는 기존 route 질문을 그대로 주었고, LLM에는 일회성 작업 지시를 제외하는 상세 정의문도 주었다. 따라서 Jev와 LLM 사이의 차이는 모델 능력과 질문의 구체성 효과를 분리하지 못한다. 특히 한 작업을 여러 입력에 걸쳐 진행하는 규칙과 여러 작업에 지속되는 규칙의 구분이 질문과 사후 정답에서 일치하는지 먼저 확인할 필요가 있다.

### 타당성 위협

| 종류 | 위협 | 이 실험에서 |
|---|---|---|
| 내적 | sol 정답의 편향·자기 유사성 | 독립 두 회차와 불일치 다수결, 판단 불가 제외 |
| 구성 | 뒤 맥락 창 밖의 지속·해제 | 같은 세션의 제한된 뒤 입력과 assistant 진행만 판정 |
| 구성 | 작성자 대리 지표 | 실제 작성자 판정 대신 형태 추정으로 제한 |
| 구성 | engine is_constraint 구현 부재 | 기존 constraint-deep 질문을 사용, 전체 engine 재현 아님 |
| 구성 | Jev 고정 질문과 LLM 상세 정의문의 차이 | 제품 질문 비교이며 순수 모델 능력 비교로 단정 불가 |
| 외적 | 한 사용자·균형 층 표본 | 제품 전체 발생률·정밀도로 일반화 제외 |
| 통계 | 작은 양성 분모·프로젝트 상관·선택 편향 | 분모·군집 구간·잠정 기준값으로 보고 |
| 시간 | CLI 시작·동시 실행·캐시 영향 | 실제 지연·버전·응답 모델·usage 보존 |

### 한계

현재 작업 하나에 대한 긴 지시문도 이후 여러 입력에서 유지될 수 있다. 이를 지속 제약으로 보는지에 대한 경계는 사후 맥락을 주어도 완전히 객관화되지 않는다. 사용자 확인 결과와 새 프로젝트를 사용한 독립 표본으로 후보를 다시 확인해야 한다. 자동 등록·묻기 외에 문장 분할, 해제, 대체, 후보 선택, 실제 engine 적용은 측정하지 않았다.

## 재현

```sh
./docs/experiments/constraint-model-compare/run.sh analyze
./docs/experiments/constraint-model-compare/run.sh verify
./docs/experiments/constraint-model-compare/run.sh figures
```

verify의 정확 키 잔존 검사는 키체인 값을 프로세스 환경으로 받은 상태에서 실행한다. 원문·응답은 비공개 실험 저장소에만 있으며 공개 저장소에는 [데이터 설명](data/README.md), 해시, 스크립트와 집계만 남겼다. analyze는 원응답에서 정규화와 분석을 다시 수행한다.

그림은 집계 JSON에서 만든 [차트 입력](results/chart.json)을 사용한다. figures는 기존 mutoscope 작업본을 사용하며 `MUTOSCOPE_CLI`로 CLI 경로를 지정할 수 있다. 도구 커밋과 Node 버전은 env.json에 기록했다. 정밀도 분모가 0인 점은 그래프에서 제외하고 차트 입력의 omitted에 남긴다. 전체 기준값은 위 표에 유지한다.

{table(["자료", "SHA-256"], [[name, "`" + checksum + "`"] for name, checksum in summary["hashes"].items()])}

## 결론

{table(["가설", "판정", "반영한 문서"], [[name, row["decision"], "없음"] for name, row in summary["hypotheses"].items()])}

{choice}를 보고서의 탐색 결과로 제시한다. 사용자 지시에 따라 docs/design, 머지, 이슈 댓글은 변경하지 않았다.
"""


if __name__ == "__main__":
    (PUBLIC / "report.md").write_text(render(read(PUBLIC / "results/summary.json")))
