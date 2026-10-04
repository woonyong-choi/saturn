# 이어 가기 판단의 새 작업 오접합 줄이기: 실험 설계

| 항목 | 값 |
|---|---|
| 이슈 | [#6](https://github.com/woonyong-choi/saturn/issues/6) |
| 관련 설계 | [router](../../design/router.md), [입력 처리](../../design/input-handling.md) |
| 사전 데이터 | 앞 실험의 설계·스크립트·집계, B 첫 반복 0.80 오접합 10건의 입력·라벨·응답 열람. 같은 규칙의 현재 모집단 추가 적격 수 1건 확인 |

## 질문

작업 정보를 준 B에서 새 작업 오접합을 줄이면서 이어 가기 재현율을 유지하는 방법을 측정한다. 오접합 5% 이하와 재현율 60% 이상을 함께 만족하는 조건·기준값을 후속 검증 후보로 고른다.

## 가설

| 가설 | 예측 | 적용 범위 |
|---|---|---|
| H1 | B1은 0.80에서 오접합 5% 이하이고 재현율 60% 이상이다. | 기존 명확 라벨, 첫 반복 유효 응답 |
| H2 | B2는 0.80에서 오접합 5% 이하이고 재현율 60% 이상이다. | 기존 명확 라벨, 첫 반복 유효 응답 |

같은 표본의 오접합을 보고 문장을 만들므로 사전 등록은 수집 뒤 규칙 변경을 막는 역할이다. 독립 검증이나 사람 정답의 확인을 뜻하지 않는다.

## 설계

| 항목 | 값 |
|---|---|
| 조건 | B0는 앞 실험 B의 첫 두 반복을 읽기만 함. B1은 질문 변경, B2는 두 질문 결합, B3는 작업 정보 길이와 순서 변경 |
| 배정 | 시드 607. 반복, 기존 표본 순서, 추가 표본 순서로 순회하며 각 턴의 B1~B3 순서를 섞음. 동시에 최대 6회 |
| 눈가림 | 새 Jev 수집은 라벨을 읽지 않음. 추가 라벨은 앞 실험의 독립 지시·모델·재판정 규칙 유지 |
| 환경 | Python 표준 라이브러리, Jev `jev-1.13.0`, 공식 `codex exec`의 `gpt-6-astra`·`gpt-5.6-luna`. Claude 호출 없음. LLM 난수 고정 불가 |

### 조건 고정

B0~B2의 state는 앞 실험 B와 같다. 직전 입력 전체, 목표 첫 2,000자, 진행 마지막 6,000자를 제공한다. B1~B3는 `is_actionable`과 running의 `relation_to_running`, `steer_or_spawn`을 그대로 유지한다. 이어 가기 분석은 이 질문들의 현재 engine 동작 재생과 다르다.

B1은 `keep_current` 질문만 아래 문장으로 바꾼다. 같은 파일의 다른 결함, 이전 실측을 참고한 독립 문서 작성, 정리 뒤 별도 실험, 짧은 후속 답 뒤 우세한 독립 요청이라는 앞 오접합 양상을 반영했다. 실제 원문과 사례 식별자는 공개하지 않는다.

```text
Does this input continue, correct, complete, or verify the same concrete goal or deliverable as the previous task? Answer no for a separately completable goal or deliverable, even in the same repository, file, topic, or broader project, and even when it uses the previous result as reference or calls itself a follow-up. A new defect, a separate documentation deliverable after implementation, or a separate experiment after cleanup is a new task unless it was already an unfinished part of the previous goal. An answer to the previous question, progress request, correction, changed constraint, or verification of that same deliverable continues it. For mixed requests, judge the dominant goal; do not treat a short introductory follow-up as evidence that the independent main request continues.
```

B2는 기존 `keep_current` 문장을 유지하고 같은 HTTP 요청에 다음 `noul` 질문 `is_new_task`를 추가한다. 두 확률은 같은 모델·맥락의 답이므로 통계적으로 독립이라고 가정하지 않는다.

```text
Does the main request introduce a separately completable new goal or deliverable rather than completing, correcting, or verifying the previous task? Sharing a repository, file, topic, or using previous results as reference does not make goals identical. A separately scoped defect, documentation deliverable, or experiment is new unless already an unfinished part of the previous goal. A progress question, answer, correction, changed constraint, or verification of the same deliverable is not new. For mixed requests, judge the dominant goal.
```

B3는 탐색 조건이다. 질문은 B0 그대로 두고 `previous task context`의 필드 순서를 `goal_excerpt`, `progress_excerpt`, `input`으로 바꾼다. 목표는 첫 500자, 진행은 마지막 1,500자, 직전 입력 전체는 유지한다. 길이와 순서를 함께 바꾸므로 둘의 개별 효과는 추정하지 않는다.

### 판정과 묻기

`p`는 이어 가기 확률, `q`는 새 작업 확률, `t`는 0.50~0.95의 0.05 간격 기준값이다.

| 조건 | 이어 가기 | 새 작업 | 가상 묻기 |
|---|---|---|---|
| B0·B1·B3 | `p >= t` | `p < 0.30` | 그 밖과 응답 실패 |
| B2 | `p >= t` 그리고 `q < 1-t` | `p < 0.30` 그리고 `q >= 0.70` | 그 밖과 응답 실패 |

경계 `q=1-t`는 묻기로 둔다. 0.50의 `p=q=0.50`이 이어 가기가 되는 것을 막는다. 부동소수점 비교는 `1-t`를 소수 둘째 자리로 반올림한다. 가상 묻기 비율은 전체 선택 턴을 분모로 한다. 현재 router의 실제 묻기 비율이나 운영 동작을 바꾼다고 해석하지 않는다.

### 요청과 저장

요청은 compact UTF-8 JSON으로 직렬화하고 전송 전 100,000바이트 이하를 확인한다. 넘으면 축약·호출 없이 `oversize`로 저장한다. HTTPS·기본 TLS 검증을 유지하고 리다이렉트는 거절한다. 질문별 확률의 범위와 분포 합, 답의 질문 집합, 실제 모델명을 검사한다. 키는 키체인 서비스에서 프로세스 환경으로만 받는다. 저장할 문자열에서 키 일치 부분을 가린다. 라벨러의 환경에서는 키를 제거한다.

기존 `.local/experiments/continuation-ko/`는 읽기만 하고 파일 해시로 전후 불변성을 확인한다. 새 원문·응답·라벨·호출 장부는 지정 worktree의 `.local/experiments/continuation-misjoin/`에만 둔다. 기존 파일의 사본을 만들지 않는다. 임시 경로는 그 안 `runtime/`으로 제한한다. 공개 자료는 설계·스크립트·집계·해시다.

## 변수

| 변수 | 종류 | 정의 | 단위 |
|---|---|---|---|
| condition, threshold | 조작 | B0~B3, 0.50~0.95 | 범주, 확률 |
| p, q | 측정 | 질문별 응답 | 확률 |
| false_join_rate | 파생 | 이어 가기로 보낸 new / 유효 응답의 new | 비율 |
| recall | 파생 | 이어 가기로 보낸 continue / 유효 응답의 continue | 비율 |
| precision | 파생 | 이어 가기로 보낸 continue / 이어 가기로 보낸 명확 라벨 | 비율 |
| ask_rate | 파생 | 묻기 / 전체 선택 턴, uncertain 포함 | 비율 |
| operational_recall | 파생 | 이어 가기로 보낸 continue / 실패 포함 전체 continue | 비율 |
| request_bytes | 측정 | 전송 직전 본문 길이 | 바이트 |
| paired_delta | 파생 | 동일 유효 턴에서 조건−B0 지표 차이 | %p |

## 표본

| 항목 | 값 |
|---|---|
| 출처 | 앞 실험 608턴·동일 정답 재사용. 당시 적격 모집단 전수. 원본 Claude 프로젝트 로그는 자료로만 읽음 |
| 크기 | 주 비교 608턴, 명확 continue 515·new 72·uncertain 21. 추가 적격 최대 58턴을 별도 보조 분석 |
| 크기 근거 | 아래 exact binomial 검정력 계산. 기존 new 72는 부족. 새 표본은 현재 존재하는 적격 전수에서 추가하며 복제·합성으로 채우지 않음 |
| 중단 규칙 | Jev 4,000회, Codex 300회. 파일 잠금·fsync 장부에 호출 직전 예약. 실패 포함. 예약 후 응답 없는 요청은 재호출하지 않음. 인증·모델 거절이면 중단 |
| 반복과 예열 | B1~B3 각 2회, 첫 반복만 주 분석. 두 번째는 안정성. 예열 없음. 기존 608×3×2=3,648회, 추가 최대 58×3×2=348회, 합계 최대 3,996회 |

### 검정력과 추가 추출

오접합 귀무 경계 0.05, 개선된 실제 비율 0.02, 단측 α=0.0125, 검정력 80%를 고정한다. 두 조건의 오접합 상한·재현율 하한 네 경계에 Bonferroni를 적용한다. `X~Bin(n,p)`에서 `P(X<=k | p=0.05)<=0.0125`인 최대 k를 정하고 `P(X<=k | p=0.02)>=0.80`인 최소 n을 탐색하면 new 363개, k=9, 검정력 0.8051이다. 기존 new보다 291개 더 필요하다. 이는 독립 표본 가정의 최소 요구이며 세션 상관은 필요 수를 늘릴 수 있다. 출처는 [NIST 비율 구간의 이항 역산](https://www.itl.nist.gov/div898/handbook/prc/section2/prc241.htm)이다.

수집 시 같은 `~/.claude/projects/*/*.jsonl`을 앞 실험 추출 함수로 다시 읽는다. 기존 ID와 프로젝트·UUID를 제외하고 추가 적격을 시드 607로 섞어 최대 58턴까지 쓴다. 추가 적격이 부족하면 있는 자료만 수집하고 검정력 미달을 보고한다. 기존 라벨 분포의 new 72/608을 가정하면 291개 추가 확보에는 약 2,458턴이 필요하여 호출 예산으로도 부족하다. 현재 사전 조사에서 추가 적격은 1건이었다. 추가 표본은 B0 응답이 없으므로 B0 비교와 주 판정에 섞지 않는다.

추가 라벨의 추출·가림·지시·15개 묶음·독립 두 모델·불일치 재판정·JSON 수리 한 번은 [앞 실험](../continuation-judgment-korean/design.md)의 규칙을 그대로 재사용한다. `codex exec` 외의 라벨 API나 Claude 모델은 쓰지 않는다. 최초 라벨과 불일치를 보존한다.

## 분석

| 가설 | 지표 | 방법 | 채택 기준 |
|---|---|---|---|
| H1 | B1 0.80 오접합·재현율 | 단측 98.75% Clopper–Pearson 경계, 프로젝트 층별 세션 bootstrap 2,000회 명목 95% 구간 | 오접합 exact 상한·cluster 상한 모두 <=5%, 재현율 exact 하한·cluster 하한 모두 >=60%이면 채택 |
| H2 | B2 0.80 오접합·재현율 | H1과 같음 | H1과 같음 |

점추정이 경계를 벗어나면 해당 표본의 목표 미달로 기록하되, 가설 기각은 exact·cluster 구간 모두 오접합 하한>5% 또는 재현율 상한<60%일 때만 한다. 그 밖은 보류다. B3와 다른 기준값은 탐색이며 보정 구간으로 확인 채택하지 않는다.

| 항목 | 규칙 |
|---|---|
| 제외 기준 | 기존 라벨 변경 없음. uncertain은 정확도 분모에서 제외. 새 추출의 첫 턴·비한국어·중복·변경 파일 제외 수 기록 |
| 실패한 실행 | 유효 응답 조건부 곡선과 실패를 묻기로 둔 운영 재현율 병기. 실패를 정상 새 작업 성공으로 세지 않음 |
| 대응 비교 | B0와 각 조건 양쪽 유효한 동일 턴. 기준값별 불일치 b/c, 지표 차이, 프로젝트 내 세션 대응 bootstrap 95% 구간. 검정 p값은 사용하지 않음 |
| 다중 비교 | 확인 H1·H2의 네 경계에 Bonferroni. 40개 곡선의 점추정 후보 선택은 탐색 |
| 후보 선택 | 점추정 오접합<=5%·재현율>=60%인 모든 조건·기준값 중 재현율 최대, 오접합 최소, 묻기 최소, 조건 번호 최소, 기준값 최소 순. 없으면 권장 없음 |
| 반복 | 같은 표본 두 응답의 이진 일치율 보고. 반복을 표본 수에 합산하지 않음 |
| 요청 크기 | 조건별 min·p50·p90·p95·p99·max, 100KB 초과 미전송과 HTTP 실패 수 |
| 유효 숫자 | 비율 0.1%, 차이 0.1%p, JSON은 계산 정밀도 유지 |

## 판정

| 결과 | 설계에 반영 |
|---|---|
| 목표·구간 만족 | 후보와 근거를 보고서에만 기록, 독립 검증 필요 |
| 점추정만 만족 | 후속 검증 후보, 운영 채택 보류 |
| 미달·자료 부족 | 부족한 표본과 실패 이유 보고, 기본값 유지 |

사용자 지정에 따라 `docs/design/`은 수정하지 않고 머지·이슈 댓글도 하지 않는다.

## 탐색 분석

- B3와 전체 기준값 곡선, 기존 최초 라벨 일치 사례만의 민감도, 추가 표본을 별도로 보고한다.
- B2의 기존 질문 단독과 결합 결과를 같은 응답에서 비교하여 추가 질문이 바꾼 확률과 결합 규칙의 효과를 구분한다.

## 타당성 위협

| 종류 | 위협 | 대응 |
|---|---|---|
| 내적 | 기존 오접합을 본 뒤 같은 자료로 평가 | 재사용 개발 표본임을 표시, 새 결과를 본 뒤 질문 변경 금지 |
| 내적 | 모델 합의는 사람 의도 정답이 아님 | 기존 라벨 고정, 최초 일치 민감도와 추가 라벨 불일치 보고 |
| 구성 | 발췌·복원 state와 실제 router 동작 차이 | 가상 묻기·이어 가기 곡선으로 한정, 제품 동작 변경 없음 |
| 구성 | B3 길이와 순서, B2 질문과 결합 효과가 섞임 | B3 개별 인과 주장 금지, B2 단독 확률도 비교 |
| 통계 | new 희소·세션 상관·기준값 선택 편향 | exact 경계·cluster 구간·검정력 부족·탐색 구분 |
| 외적 | 한 사용자와 특정 프로젝트 편중, 역사적 B0 | 다른 사용자로 일반화 금지, 모델명·호출 시각·반복 안정성 공개 |
