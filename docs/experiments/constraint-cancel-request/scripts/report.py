"""집계 JSON에서 보고서 표를 만든다."""

from __future__ import annotations

from common import PUBLIC, read_json


def pct(value: float | None) -> str:
    return "측정 불가" if value is None else f"{value * 100:.1f}%"


def rate(m: dict) -> str:
    if not m["n"]:
        return "측정 불가 (n=0)"
    return f"{m['k']}/{m['n']}, {pct(m['value'])} [{pct(m['low'])}, {pct(m['high'])}]"


def table(headers: list[str], rows: list[list]) -> str:
    return "\n".join(
        [
            "| " + " | ".join(headers) + " |",
            "|" + "---|" * len(headers),
            *["| " + " | ".join(map(str, r)) + " |" for r in rows],
        ]
    )


def main() -> None:
    s = read_json(PUBLIC / "results/summary.json")
    h = s["hypotheses"]
    flow = s["flow"]
    sample = s["sample"]
    lines = [
        "# 등록 기록을 본 사용자의 제약 해제 요청 판단: 실험 결과",
        "",
        "## 요약",
        "",
        f"등록 기록이 있는 쌍별 질문의 요청 식별 정밀도는 {rate(h['H1_precision'])}, 재현율은 {rate(h['H1_recall'])}였다. 비해제 입력에서 자동 해제한 비율은 {rate(h['H3'])}였다. 사전 선택 규칙을 통과한 질문 형식·기준값은 "
        + ("없다." if not s["recommendation"] else str(s["recommendation"]) + "다.")
        + " 제품 설계와 기본값은 변경하지 않았다.",
        "",
        "## 방법",
        "",
        table(
            ["항목", "값"],
            [
                [
                    "설계",
                    f"[사전 등록](design.md), 봉인 커밋 `{s['run']['design_commit']}`",
                ],
                ["실행 id", s["run"]["run_id"]],
                ["환경", "[env.json](env.json)"],
                [
                    "평가 표본",
                    f"{sample['items']}개, 요청 양성 {sample['positive']}개·음성 {sample['negative']}개",
                ],
                ["규칙 원천·분석 군집", f"{sample['rules']}개·{sample['clusters']}개"],
                [
                    "새 호출",
                    f"Jev {s['calls'].get('jev', 0)}회, Codex {s['calls'].get('codex', 0)}회",
                ],
            ],
        ),
        "",
        "첫 반복만 성능 분모로 사용했다. 같은 입력의 기록 유무와 질문 형식을 대응시켰다. 부분·조건부 요청은 요청 식별 양성이지만 제약 전체의 영구 해제에는 음성으로 보았다. 생성 의도와 독립 라벨의 일치는 사람 정답 검증을 뜻하지 않는다.",
        "",
        "## 설계와 다른 점",
        "",
        "수집 중 분석·검증·보고 스크립트를 추가했다. 첫 Jev 호출 전에 비정상 답 객체를 invalid로 처리하는 파서를 보완하고 전송 동시 수를 8개로 늘렸다. 요청 순서는 봉인한 시드 순서를 유지하고 각 요청은 여전히 세 번 측정했다. 독립 검토에 따라 세 반복 일치율에 실패한 입력을 포함하고, 빈 분모는 측정 불가로 남겼다. 공개 생성문에는 실제 원문 전체와 일치하는 부분을 가리는 검사를 추가하고 규칙 ID를 공개용 가명으로 바꿨다. 측정 요청·정답 의미·기준값·가설 경계는 바꾸지 않았다. 변경 시점과 봉인 이후 코드 차이는 Git 이력으로 확인할 수 있다.",
        "",
        "## 결과",
        "",
        "### 흐름",
        "",
        table(
            ["단계", "수"],
            [
                ["합성 계획", flow["synthetic_planned"]],
                ["합성 채택", flow["synthetic_retained"]],
                ["실제 문장 라벨 시도", s["label_flow"]["real_screened"]],
                ["실제 문장 채택", flow["real_retained"]],
                ["실제 문장 합의 후 할당 초과 미선택", s["label_flow"]["real_unused"]],
                ["제외", flow["excluded"]],
                ["공개 생성문 추가 가림", sample["public_redactions"]],
            ],
        ),
        "",
        f"생성 의도와 독립 라벨의 최초 명확 합의는 {rate(s['label_flow']['synthetic_first_agreement'])}였다. 이후 중복 문장을 제외했다. Jev 답을 근거로 표본을 제외하지 않았다.",
        "",
        table(["제외 이유", "수"], list(s["exclusions"].items())),
        "",
        table(["응답 상태", "관측 수"], list(s["statuses"].items())),
        "",
        "### 확인 분석",
        "",
        "비율 구간은 명목 Wilson 95%다. 채택에는 사전 지정한 정확 단측 검정의 Holm 보정과 군집 bootstrap 조건을 함께 적용했다. 기각은 명목 구간이 목표 반대쪽에 완전히 있는 경우이며, 나머지는 보류다.",
        "",
        table(
            ["가설", "지표", "값과 구간", "군집 bootstrap 95%", "보정 p", "판정"],
            [
                [
                    key,
                    h[key]["metric"],
                    rate(h[key]),
                    f"[{pct(h[key]['bootstrap']['low'])}, {pct(h[key]['bootstrap']['high'])}]",
                    f"{h[key]['adjusted_p']:.3g}",
                    h[key]["verdict"],
                ]
                for key in (
                    "H1_precision",
                    "H1_recall",
                    "H2_recent",
                    "H2_content",
                    "H3",
                )
            ],
        ),
        "",
        table(
            [
                "가설",
                "대응 수",
                "앞 조건만 정답 / 뒤 조건만 정답",
                "정확도 차이·입력 bootstrap 95%",
                "군집 bootstrap 95%",
                "보정 p",
                "판정",
            ],
            [
                [
                    key,
                    h[key]["n"],
                    f"{h[key]['first_only']} / {h[key]['second_only']}",
                    f"{pct(h[key]['difference'])} [{pct(h[key]['low'])}, {pct(h[key]['high'])}]",
                    f"[{pct(h[key]['cluster_low'])}, {pct(h[key]['cluster_high'])}]",
                    f"{h[key]['adjusted_p']:.3g}",
                    h[key]["verdict"],
                ]
                for key in ("H4", "H5")
            ],
        ),
        "",
        "H4의 앞·뒤는 기록 없음·있음이고, H5는 쌍별·선택형이다. 차이 단위는 %p다. H2는 요청으로 식별한 실제 요청에 조건부이므로 모든 실제 요청 중 대상까지 도달한 비율을 함께 보고했다.",
        "",
        "### 질문 형식과 기준값",
        "",
        table(
            [
                "형식",
                "기준값",
                "요청 정밀도",
                "요청 재현율",
                "비해제 오해제율",
                "묻기",
                "결합 정확도",
            ],
            [
                [
                    form,
                    f"{threshold:.2f}",
                    *[
                        rate(s["curves"][f"{form}-1-{threshold:.2f}"][metric])
                        for metric in (
                            "precision",
                            "recall",
                            "false_release",
                            "ask_rate",
                            "accuracy",
                        )
                    ],
                ]
                for form in ("pair", "choice")
                for threshold in (0.5, 0.7, 0.8, 0.9, 0.95)
            ],
        ),
        "",
        "기록 없는 조건을 포함한 전체 곡선은 [thresholds.csv](results/tables/thresholds.csv), 분자·분모·구간은 [summary.json](results/summary.json)에 있다. 곡선에서 사후 선택한 값을 확인 가설 검정에 재사용하지 않았다.",
        "",
        "### 활성 제약 수와 지칭 방식",
        "",
        table(
            [
                "형식",
                "지칭",
                "활성 개수",
                "식별 후 대상 정확도",
                "모든 요청 중 대상 도달률",
            ],
            [
                [
                    c["form"],
                    c["reference"],
                    c["n"],
                    rate(c["metrics"]["target_accuracy"]),
                    rate(c["metrics"]["target_reach"]),
                ]
                for c in s["h2_conditions"]
                if c["record"]
            ],
        ),
        "",
        table(
            [
                "형식",
                "입력 유형",
                "표본",
                "결합 정확도",
                "요청 재현율",
                "비해제 오해제율",
            ],
            [
                [
                    c["form"],
                    c["value"],
                    c["n"],
                    *[
                        rate(c["metrics"][m])
                        for m in ("accuracy", "recall", "false_release")
                    ],
                ]
                for c in s["conditions"]
                if c["record"] and c["field"] == "category"
            ],
        ),
        "",
        "### 실제 문장과 생성 문장",
        "",
        table(
            ["형식", "출처", "표본", "요청 정밀도", "비해제 오해제율", "결합 정확도"],
            [
                [
                    c["form"],
                    c["value"],
                    c["n"],
                    *[
                        rate(c["metrics"][m])
                        for m in ("precision", "false_release", "accuracy")
                    ],
                ]
                for c in s["conditions"]
                if c["record"] and c["field"] == "source"
            ],
        ),
        "",
        "실제 문장은 비해제만 있으므로 해제 재현율을 측정할 수 없다. 실제 문장도 등록 기록이 있었던 실제 대화를 재현한 것이 아니라 합성 활성 상태에 붙인 스트레스 표본이다. 출처별 전체 정확도는 양성 비율이 달라 직접 우열로 읽지 않는다.",
        "",
        "### 부분·조건부 요청의 영구 해제 위험",
        "",
        table(
            ["형식", "요청 범위", "전체 제약을 자동 해제한 비율"],
            [
                [c["form"], c["value"], rate(c["metrics"]["limited_release"])]
                for c in s["conditions"]
                if c["record"]
                and c["field"] == "intent"
                and c["value"] in ("partial", "conditional")
            ],
        ),
        "",
        "요청 의도와 대상을 맞혀도 적용 범위를 따로 확인하지 않으면 일시 예외를 영구 해제로 바꿀 수 있다. 이번 두 질문 형식은 적용 범위를 출력하지 않는다.",
        "",
        "### 요청 크기와 세 반복",
        "",
        table(
            ["형식", "기록", "최소", "중앙", "p95", "최대 바이트"],
            [
                [c["form"], c["record"], c["min"], c["median"], c["p95"], c["max"]]
                for c in s["sizes"]
            ],
        ),
        "",
        f"선택형−쌍별 요청 바이트 차이 중앙값은 {h['H5']['byte_difference_median']}바이트, 대응 입력 bootstrap 구간은 [{h['H5']['byte_low']}, {h['H5']['byte_high']}]였다.",
        "",
        table(
            [
                "조건",
                "유효 세 반복 / 전체",
                "세 번 모두 유효·판정 일치",
                "유효한 경우의 조건부 일치",
            ],
            [
                [
                    r["condition"],
                    f"{r['valid']}/{r['total']}",
                    rate(r["agreement"]),
                    rate(r["conditional_agreement"]),
                ]
                for r in s["repeats"]
            ],
        ),
        "",
        "HTTP 상태별 수와 응답 모델은 집계 JSON에 보존했다. 반복은 추가 독립 표본으로 세지 않았다.",
        "",
        "## 논의",
        "",
        "### 해석",
        "",
        "등록 기록이 없는 자연 대화에서 암묵적 전이를 찾았던 앞 실험과 이번 명시 요청 세트는 서로 다른 문제다. 이번 성능을 과거 정밀도와 직접 나누어 개선 배수로 해석하지 않는다. 같은 상태의 기록 유무 대응 비교만 기록 문구의 추가 효과다.",
        "",
        "후속 검증의 질문 형식은 선택형을 권한다. 사전 비교 기준값 0.80에서 요청·대상 결합 정확도와 요청 크기가 모두 개선됐다. 다만 제품의 자동 해제 기준값은 보류한다. 비해제 오해제율 구간의 상한이 목표를 넘고, 부분·조건부 요청을 영구 해제로 바꾸는 위험이 남았다. 0.80은 이번 형식 비교의 기준이며 안전성이 확정된 제품 기본값이 아니다.",
        "H1의 요청 식별 기준값과 제품 자동 해제 게이트는 다르다. 쌍별 방식은 is_release 0.50 이상에서 관계 질문을 적용하므로 H1의 0.80 기준으로는 음성인 작업 중지 입력도 자동 해제될 수 있다. 작업 중지 오해제는 이 차이까지 포함한 결과다.",
        "사전 추천 규칙은 정밀도·재현율·오해제율의 구간을 모두 요구한다. 그 규칙을 통과하지 못하면 점추정이 좋아도 자동 해제의 권장값을 확정하지 않는다. 요청 인식·대상 제시와 실제 영구 해제는 구분해야 한다.",
        "",
        "### 타당성 위협",
        "",
        table(
            ["종류", "위협", "이 실험에서"],
            [
                [
                    "내적",
                    "두 모델 합의로 쉬운 문장만 남음",
                    "불일치·중복 제외를 공개했으며 사람 검토는 하지 않음",
                ],
                [
                    "구성",
                    "부분·일시 요청과 전체 해제 혼동",
                    "별도 영구 해제 위험 표로 보고",
                ],
                [
                    "구성",
                    "짧은 규칙과 정답 후보 제공",
                    "등록·후보 검색·직렬 호출·엔진 적용 검증 아님",
                ],
                ["외적", "실제 등록 후 해제 턴 부재", "실제 비해제 문장만 별도 보고"],
                [
                    "통계",
                    "같은 규칙 반복과 조건별 소표본",
                    "군집 구간·분모·측정 불가 병기",
                ],
                ["시간", "응답 변동", "동일 요청 세 반복과 모델·실패 기록"],
            ],
        ),
        "",
        "### 한계",
        "",
        "- 양성 비율은 의도적으로 높인 평가 설계다. 실제 제품의 해제 요청 발생률을 모르므로 정밀도를 실사용 정밀도로 일반화하지 않는다.",
        "- 활성 개수별 H2와 실제 문장 표본은 작은 차이를 입증할 검정력이 부족하다. 큰 구간과 보류를 성능 동등의 근거로 쓰지 않는다.",
        "- 지시어와 작업 중지 문장의 라벨 불일치 제외가 집중됐다. 이 두 유형의 채택 표본 성능은 실제 어려운 입력의 성능보다 높을 수 있다.",
        "- 공개 생성문에는 규칙 원문이 없으므로 공개 파일만으로 모든 Jev 요청을 재현할 수 없다. 원자료 소유자의 비공개 파일이 필요하다.",
        "",
        "## 재현",
        "",
        "```sh",
        "cd docs/experiments/constraint-cancel-request",
        "./run.sh verify",
        "./run.sh analyze",
        "```",
        "",
        "원문·응답의 해시와 공개·비공개 필드는 [데이터 안내](data/README.md)에 있다. 분석은 외부 모델을 호출하지 않고 원응답에서 집계를 재생성한다.",
        "",
        "## 결론",
        "",
        table(
            ["가설", "판정", "반영한 설계 문서"],
            [
                [
                    "H1",
                    f"정밀도 {h['H1_precision']['verdict']}, 재현율 {h['H1_recall']['verdict']}",
                    "없음",
                ],
                [
                    "H2",
                    f"직전 {h['H2_recent']['verdict']}, 내용 {h['H2_content']['verdict']}",
                    "없음",
                ],
                *[[key, h[key]["verdict"], "없음"] for key in ("H3", "H4", "H5")],
            ],
        ),
        "",
        "자동 해제 권장값은 사전 구간 기준을 통과한 경우에만 제시한다. 설계 변경·제품 반영·머지·이슈 댓글은 이 실험의 범위가 아니다.",
        "",
    ]
    (PUBLIC / "report.md").write_text("\n".join(lines))


if __name__ == "__main__":
    main()
