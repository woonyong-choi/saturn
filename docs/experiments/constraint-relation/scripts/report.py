"""집계 JSON만으로 공개 보고서를 만든다."""

from __future__ import annotations

from support import PUBLIC, ratio


def percent(value: float | None) -> str:
    return "측정 불가" if value is None else f"{100 * value:.1f}%"


def proportion(value: dict) -> str:
    if not value["n"]:
        return "측정 불가 (n=0)"
    low, high = value["ci95"]
    return f"{value['k']}/{value['n']}, {percent(value['value'])} [{100 * low:.1f}, {100 * high:.1f}]"


def write_report(s: dict) -> None:
    original = s["cohorts"]["original"]
    replace_c1 = original["C1"]["primary"]["replace"]["precision"]
    replace_c2 = original["C2"]["primary"]["replace"]["precision"]
    new_http_failures = sum(
        v["requests"]["http_failures"]
        for conditions in s["cohorts"].values()
        for condition, v in conditions.items()
        if condition != "C0"
    )
    lines = [
        "# 후보를 줄인 제약 대체·해제 판단: 실험 결과",
        "",
        "## 요약",
        "",
        f"C1 대체 정밀도는 {proportion(replace_c1)}, C2는 {proportion(replace_c2)}였다. 두 조건 모두 자동 처리 기준을 통과하지 못해 사용자 선택을 권고한다. 새 Jev 호출의 HTTP 실패는 {new_http_failures}회였다. 요청 크기 문제를 줄여도 관계 판단의 정밀도는 제품 기준에 미달한다.",
        "",
        "## 방법",
        "",
        "| 항목 | 값 |",
        "|---|---|",
        f"| 설계 | [사전 등록](design.md), 봉인 커밋 `{s['run']['design_commit']}` |",
        f"| 실행 id | `{s['run']['run_id']}` |",
        "| 환경 | [env.json](env.json) |",
        f"| 원래 표본 | {s['constants']['original_turns']}턴 중 무작위 {s['constants']['sample']}턴, 조건별 같은 표본 |",
        f"| 추가 표본 | {s['extension']['planned_turns']}턴 라벨 시도, 유효 {s['extension']['turns']}턴. 무작위 {s['constants']['extension_sample']}턴 중 라벨 확보 {s['cohorts']['extension']['C1']['selected_turns']}턴 |",
        f"| 새 호출 | Jev {s['calls'].get('jev', 0)}회, Codex {s['calls'].get('codex', 0)}회 (라벨 {s['codex_call_roles'].get('label', 0)}, 검토 {s['codex_call_roles'].get('review', 0)}) |",
        "",
        "C0는 앞 실험 원응답을 읽어 같은 표본을 재계산했다. 새 호출은 없다. C1·C2는 기존 등록 후보 공급을 유지했고, C3는 질문 변경을 함께 적용했다. 해제 질문은 constraint@1.1의 보조 질문이다.",
        "",
        "## 설계와 다른 점",
        "",
        "첫 Jev 호출 전 후보 생성의 반복 작업을 줄이기 위해 대상 키를 메모이제이션하고, 후보를 제거할 때 전체 JSON을 반복 직렬화하는 대신 제거한 JSON 항목의 바이트를 뺐다. 최종 실제 직렬화 길이와 일치하는지 매 요청에서 검사했다. 같은 입력의 후보·질문·바이트 제한 결과는 봉인 코드와 대조했다. 후보 생성 중이던 자기 프로세스만 종료했고, 호출 장부에 예약된 요청은 재전송하지 않았다. 분석·검증·보고 스크립트는 수집 중 추가했다. 독립 검토에 따라 빈 프로젝트를 포함한 bootstrap, 종류별 후보 포함률, 유효 응답 조건부 정확도, 재개 시 요청 해시 대조를 보완했다. 원래 표본의 요청 해시는 저장된 모든 원응답과 재생성한 본문을 대조한 뒤 고정했고, 추가 표본은 전송 전에 고정했다. 라벨의 의미·표본·판정 경계는 변경하지 않았다.",
        "",
        "C0를 활성 제약 전체 요청으로 부르는 이슈 설명은 실제 스크립트와 달랐다. 실제 C0는 누적 등록 후보에서 단어·경로 겹침 순으로 최대 열 개를 골랐다. 이 차이는 수집 전 설계에 기록했다.",
        "",
        "## 결과",
        "",
        "### 흐름",
        "",
        "| 단계 | 수 |",
        "|---|---:|",
        f"| 새 Jev 호출 예약 | {s['calls'].get('jev', 0)} |",
        f"| Jev 미완료 호출 | {s['incomplete_calls']} |",
        f"| 추가 Codex 호출 예약 | {s['calls'].get('codex', 0)} |",
        f"| 추가 라벨 대화 / 턴 | {s['extension']['projects']} / {s['extension']['turns']} |",
        f"| 추가 라벨 시도 턴 | {s['extension']['planned_turns']} |",
        f"| 추가 라벨 검증 실패 제외 턴 | {s['extension']['excluded_turns']} |",
        f"| 추가 표본 라벨 실패 제외 턴 | {s['constants']['extension_sample'] - s['cohorts']['extension']['C1']['selected_turns']} |",
        f"| 추가 모호한 턴 | {s['extension']['ambiguous']} |",
        f"| 추가 라벨 무효 묶음 수 | {s['extension']['invalid_batches']} |",
        "",
        "| 표본 | 조건 | 현재 턴 정답 제외 | 평가 후보쌍 | 후보 밖 정답 | 앞 라벨 제외 정답 | invalid 쌍 |",
        "|---|---|---:|---:|---:|---:|---:|",
    ]
    for cohort, conditions in s["cohorts"].items():
        for condition, v in conditions.items():
            lines.append(
                f"| {cohort} | {condition} | {v['ambiguous_current_turns']} | {v['pairs']} | {v['missing_targets']} | {v['unevaluable_targets']} | {v['invalid_pairs']} |"
            )
    lines += [
        "",
        "### 확인 분석",
        "",
        "같은 원래 표본에서 기준값 0.80을 적용했다. 구간은 명목 Wilson 95%다. 재현율 분모에는 후보 밖 정답도 포함했다. C0는 재사용 비교, C3는 탐색이다.",
        "",
        "| 조건 | 대체 정밀도 | 대체 재현율 | 해제 정밀도 | 해제 재현율 | 표본 후보 포함률 |",
        "|---|---|---|---|---|---|",
    ]
    for condition, v in original.items():
        p = v["primary"]
        lines.append(
            f"| {condition} | {proportion(p['replace']['precision'])} | {proportion(p['replace']['recall'])} | {proportion(p['release']['precision'])} | {proportion(p['release']['recall'])} | {proportion(v['sample_coverage'])} |"
        )
    lines += [
        "",
        "| 조건 | 종류 | 프로젝트 bootstrap 정밀도 95% | 분모 없는 재추출 | 정확 이항 p | 판정 |",
        "|---|---|---|---:|---:|---|",
    ]
    for condition in ("C1", "C2"):
        for op in ("replace", "release"):
            m = original[condition]["primary"][op]
            boot = m["bootstrap"]
            ci = "[" + ", ".join(percent(x) for x in boot["precision_ci95"]) + "]"
            lines.append(
                f"| {condition} | {op} | {ci} | {boot['empty_precision_draws']} | {m['p_value']:.3g} | {original[condition]['hypotheses'][op]} |"
            )
    lines += [
        "",
        "기준값 곡선 전체는 [CSV](results/tables/thresholds.csv)와 [집계 JSON](results/summary.json)에 있다. 0.50~0.95의 각 점에 대체·해제의 분자·분모·정밀도·재현율·95% 구간을 보존했다. 곡선에서 고른 값을 확인 판정에 재사용하지 않았다.",
        "",
        "### 후보 포함률",
        "",
        "원래 전수의 정답 대상에 결정적 후보 규칙만 적용했다. 이 표를 만드는 과정에서는 Jev를 호출하지 않았다. 부분 예외를 포함한 검색 성능과 실제 대체·해제를 구분했다.",
        "",
        "| 조건 | 종류 | 크기 제한 전 | 크기 제한 후 |",
        "|---|---|---|---|",
    ]
    for condition, ops in s["coverage_full"].items():
        for op, v in ops.items():
            lines.append(
                f"| {condition} | {op} | {proportion(ratio(v['before'], v['n']))} | {proportion(ratio(v['after'], v['n']))} |"
            )
    lines += [
        "",
        "### 요청 크기와 실패",
        "",
        "단위는 실제 전송 JSON의 UTF-8 바이트다. C0 행은 같은 표본에서 재사용한 과거 호출이며 크기 제한을 소급 적용하지 않았다.",
        "",
        "| 표본 | 조건 | 호출 | 최소 / 중앙 / p95 / 최대 바이트 | HTTP 실패 | 잘린 요청 / 후보 | 호출 생략 사유 |",
        "|---|---|---:|---|---:|---|---|",
    ]
    for cohort, conditions in s["cohorts"].items():
        for condition, v in conditions.items():
            r = v["requests"]
            b = r["bytes"]
            sizes = " / ".join(str(b[k]) for k in ("min", "median", "p95", "max"))
            lines.append(
                f"| {cohort} | {condition} | {r['calls']} | {sizes} | {r['http_failures']} | {r['trimmed_trials']} / {r['trimmed_candidates']} | `{r['skipped']}` |"
            )
    lines += [
        "",
        "HTTP 상태별 수와 제한 전 바이트 분포는 집계 JSON에 보존했다. 요청 생략은 표본에서 제외하지 않고 행동 없음으로 처리했다.",
        "",
        "### 세 반복과 턴 거리",
        "",
        "| 조건 | 종류 세 번 일치 | 대체 이진 일치 | 해제 이진 일치 | 불완전 후보 묶음 수 |",
        "|---|---|---|---|---:|",
    ]
    for condition, v in original.items():
        lines.append(
            f"| {condition} | {proportion(v['repeats']['classification'])} | {proportion(v['repeats']['replace'])} | {proportion(v['repeats']['release'])} | {v['incomplete_triplets']} |"
        )
    lines += [
        "",
        "| 조건 | 턴 거리 | 종류 정확도 | 대체 재현율 | 해제 재현율 |",
        "|---|---|---|---|---|",
    ]
    for condition, v in original.items():
        for band, m in v["distance"].items():
            lines.append(
                f"| {condition} | {band} | {proportion(m['accuracy'])} | {proportion(m['replace'])} | {proportion(m['release'])} |"
            )
    lines += [
        "",
        "### 탐색 분석",
        "",
        "| 공통 후보 비교 | 쌍 | 앞 조건만 정답 b | 뒤 조건만 정답 c | 정확도 차이 | 프로젝트 bootstrap 95% |",
        "|---|---:|---:|---:|---|---|",
    ]
    for pair, v in s["paired"].items():
        lines.append(
            f"| {pair} | {v['n']} | {v['left_only_correct']} | {v['right_only_correct']} | {percent(v['difference'])} | [{', '.join(percent(x) for x in v['ci95'])}] |"
        )
    e = s["extension"]
    lines += [
        "",
        f"추가 독립 라벨의 최초 전이·대상·모호함 합의는 {proportion(e['initial_agreement'])}였다. 명확한 전이 대상 수는 `{e['transition_targets']}`였다. 목표 대비 남은 정답 부족분은 대체 {e['remaining_positive_targets']['replace']}개, 해제 {e['remaining_positive_targets']['release']}개다.",
        "",
        "| 추가 표본 조건 | 대체 정밀도 | 대체 재현율 | 해제 정밀도 | 해제 재현율 | 후보 포함률 |",
        "|---|---|---|---|---|---|",
    ]
    for condition, v in s["cohorts"]["extension"].items():
        p = v["primary"]
        lines.append(
            f"| {condition} | {proportion(p['replace']['precision'])} | {proportion(p['replace']['recall'])} | {proportion(p['release']['precision'])} | {proportion(p['release']['recall'])} | {proportion(v['sample_coverage'])} |"
        )
    baseline = s["baseline_full"]
    lines += [
        "",
        f"앞 실험 전체 C0의 대체 정밀도는 {proportion(baseline['by_operation']['replace']['precision'])}, 해제 정밀도는 {proportion(baseline['by_operation']['release']['precision'])}였다. 전체 정답 대상 {baseline['gold_transition_targets']}개 중 {baseline['uncovered_gold_targets']}개가 후보 밖이었다. 위 C0 표본 행과 분모가 다르다.",
        "",
        "## 논의",
        "",
        "### 해석",
        "",
        "후보 축소로 HTTP 실패와 요청 크기가 어떻게 달라졌는지는 요청 표에서 확인할 수 있다. 이 효과만으로 대체·해제를 자동 처리할 수 있다는 결론은 나오지 않는다. 정밀도 기준과 신뢰구간을 통과한 조건만 자동 가능 후보가 된다. 해제 정답이 없는 표본에서는 해제 재현율을 측정할 수 없다.",
        "",
        f"정확 이항 검정력 계산은 종류·조건별 독립 예측 양성 {s['power']['n']}개에서 성공 {s['power']['critical']}개 이상을 요구했고 검정력은 {percent(s['power']['power'])}였다. 전체 입력 수나 세 반복 수는 이 분모를 대신하지 않는다. 추가 짧은 기록의 후보 공급은 정답을 사용했으므로 원래 조건의 정밀도와 합치지 않았다.",
        "",
        "### 타당성 위협",
        "",
        "| 종류 | 위협 | 이 실험에서 |",
        "|---|---|---|",
        "| 내적 | 모델 라벨과 이전 사후 변경 | 원래 라벨을 유지했고 사람 검증을 추가하지 않음 |",
        "| 구성 | 극히 적은 대체·해제 양성 | 정답 종류별 수와 측정 불가 분모를 공개 |",
        "| 구성 | 명사 일치와 의미 범위의 차이 | 결정적 검색 포함률과 판단 성능을 별도로 계산 |",
        "| 구성 | 후보 축소와 요청 제한의 결합 | 제한 전후 포함률과 잘린 후보 수를 기록 |",
        "| 외적 | 단일 사용자와 같은 자료 | 독립 일반화 결론을 내리지 않고 추가 짧은 대화도 구분 |",
        "| 통계 | 프로젝트 내 상관 | 첫 반복만 사용하고 프로젝트 bootstrap과 빈 재추출 수를 병기 |",
        "",
        "### 한계",
        "",
        "- 등록 후보는 앞 실험의 확률로 고정되어 있다. 관계 적용 뒤 활성 집합을 추적하는 제품 동작은 검증하지 않았다.",
        "- C0와 C1·C2는 호출 시점이 다르다. 모델 버전은 고정했지만 서비스 시점 효과를 분리하지 못했다.",
        "- C3와 기준값 곡선은 탐색이며 독립 평가를 대신하지 않는다.",
        "",
        "## 재현",
        "",
        "```sh",
        "cd docs/experiments/constraint-relation",
        "./run.sh verify",
        "./run.sh analyze",
        "```",
        "",
        "외부 호출 없이 비공개 원응답과 불변 원자료에서 집계를 재생성한다. [데이터 안내](data/README.md)에 경로·필드·체크섬 규칙이 있다. 보고서의 수치는 results/summary.json에서만 읽는다.",
        "",
        "## 결론",
        "",
        "| 가설 | 판정 | 권고 | 반영한 설계 문서 |",
        "|---|---|---|---|",
    ]
    for name, condition in [("H1", "C1"), ("H2", "C2")]:
        v = original[condition]
        lines.append(
            f"| {name} | 대체 {v['hypotheses']['replace']}, 해제 {v['hypotheses']['release']} | {v['recommendation']} | 없음 |"
        )
    lines += [
        "",
        "사용자 결정에 따라 설계 문서와 제품 기본값을 바꾸지 않았다. 후속 결정은 이 보고서의 자동 가능 기준과 표본 부족을 함께 검토한다.",
    ]
    text = "\n".join(lines) + "\n"
    text = text.replace("사람 검증을 추가하지 않음", "사람 검증 미수행")
    (PUBLIC / "report.md").write_text(text)
