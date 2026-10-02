# 제약 식별과 대체 판정 정확도: 실험 설계

| 항목 | 값 |
|---|---|
| 이슈 | [#121](https://github.com/woonyong-choi/saturn/issues/121) |
| 관련 설계 | [맥락 고르기](../../design/context-selection.md), [judge](../../design/router.md) |
| 사전 데이터 | 없음. 평가 세트 문장과 라벨만 있고 judge 답은 아직 없다. |

## 질문

`is_constraint` 기준값 0.7과 `replaces_<n>` 기준값 0.8, 0.5를 그대로 둘지 정하려고 한국어 평가 세트에서 기준 judge의 판정 정확도를 잰다. 앞 말을 가리키는 간접 지시에서 두 원문을 나란히 놓은 쌍 비교가 대체를 잡는지도 따로 잰다.

## 가설

| 가설 | 예측 | 적용 범위 |
|---|---|---|
| H1 | `is_constraint` 0.7 기준의 정밀도는 80%를 넘는다. | 한국어 입력, `route@1.1`, 기준 judge `jev-1.13.0` |
| H2 | `is_constraint` 0.7 기준의 재현율은 80%를 넘는다. | 한국어 입력, `route@1.1`, 기준 judge `jev-1.13.0` |
| H3 | `replaces_1` 답이 0.8 이상인 쌍 중 대체 라벨 비율은 90%를 넘는다. | 한국어 제약 쌍, `constraint@1.0`, 기준 judge `jev-1.13.0` |
| H4 | `replaces_1` 답이 0.5 미만이거나 없는 쌍 중 양립 라벨 비율은 80%를 넘는다. | 한국어 제약 쌍, `constraint@1.0`, 기준 judge `jev-1.13.0` |
| H5 | 간접 지시 입력에서 `is_constraint` 0.7 기준의 정확도는 80%를 넘는다. | 앞 입력을 가리키는 한국어 입력, `route@1.1`, 기준 judge `jev-1.13.0` |
| H6 | 간접 지시 대체 쌍에서 `replaces_1` 답이 0.8 이상인 비율은 70%를 넘는다. | 나중 제약이 앞 제약을 가리키는 한국어 쌍, `constraint@1.0`, 기준 judge `jev-1.13.0` |

## 설계

| 항목 | 값 |
|---|---|
| 조건 | 단일 조건. 평가 세트 항목마다 기준 judge에 질문 하나를 한 번 묻는다. `is_constraint`는 입력 항목에, `replaces_1`은 쌍 항목에 묻는다. |
| 배정 | 입력 200건과 쌍 160건을 한 목록으로 합쳐 Python `random.Random(121).shuffle` 순서로 묻는다. |
| 눈가림 | 수집 스크립트는 라벨을 읽지 않는다. 수집하는 사람은 `./run.sh analyze` 전에 `data/raw/`의 답을 열지 않는다. |
| 환경 | 기준 judge `POST https://api.typesafe.ai/v1/systemone`, 요청 모델 `jev-1.13.0`, 응답의 `model`을 기록한다. Python 3.10 이상 표준 라이브러리만 쓴다. judge 샘플링은 고정할 수 없어 항목마다 한 번만 묻고 그 답을 쓴다. Jev API 키를 환경 변수 `SATURN_JUDGE_KEY`로 받는다. 키가 없으면 `01-collect`가 원인 한 줄을 남기고 끝난다. |

질문 문장은 [eval/questions.json](eval/questions.json)에 고정한다. judge 설계대로 질문은 영어로 쓰고 사용자 원문은 그대로 넣는다.

| 질문 | 질문 세트 | state | 문장 |
|---|---|---|---|
| `is_constraint` | `route@1.1` | `previous_user_input`, `latest_user_input` | `The user's latest input sets a rule that applies beyond this single request and limits how the work is done (language, tool, format, or prohibition) rather than what to do.` |
| `replaces_1` | `constraint@1.0` | `earlier_constraint`, `later_constraint`의 턴과 원문 | `Following the later constraint makes it impossible to keep the earlier constraint.` |

### 평가 세트

평가 세트는 공개 가능한 합성 한국어 문장이다. 실제 대화나 개인정보는 넣지 않는다.

| 파일 | 구간 | 수 | 라벨 |
|---|---|---|---|
| [eval/inputs.jsonl](eval/inputs.jsonl) | `constraint`: 제약만 있는 입력 | 50 | 제약 |
| [eval/inputs.jsonl](eval/inputs.jsonl) | `mixed`: 일과 제약이 섞인 입력 | 30 | 제약 |
| [eval/inputs.jsonl](eval/inputs.jsonl) | `request`: 일반 요청, 질문, 이번 한 번만 적용하는 방식 지정 | 70 | 제약 아님 |
| [eval/inputs.jsonl](eval/inputs.jsonl) | `indirect-constraint`: 앞 제약을 가리켜 바꾸는 간접 지시 | 25 | 제약 |
| [eval/inputs.jsonl](eval/inputs.jsonl) | `indirect-request`: 앞 요청을 가리켜 이번 일만 바꾸는 간접 지시 | 25 | 제약 아님 |
| [eval/pairs.jsonl](eval/pairs.jsonl) | `direct-replace`: 나중 제약이 같은 대상의 값을 직접 바꾼다 | 30 | 대체 |
| [eval/pairs.jsonl](eval/pairs.jsonl) | `indirect-replace`: 나중 제약이 앞 제약을 가리켜 바꾼다 | 30 | 대체 |
| [eval/pairs.jsonl](eval/pairs.jsonl) | `partial`: 나중 제약이 앞 제약의 일부 범위에 예외를 둔다 | 30 | 부분 |
| [eval/pairs.jsonl](eval/pairs.jsonl) | `compatible`: 두 제약을 함께 지킬 수 있다 | 60 | 양립 |
| [eval/pairs.jsonl](eval/pairs.jsonl) | `indirect-compatible`: 앞 제약을 가리키지만 함께 지킬 수 있다 | 10 | 양립 |

- 간접 지시가 아닌 입력의 `previous`는 중립 요청 10개 중 시드 121로 고른다. 간접 지시만 앞 입력을 갖는 단서를 없애기 위해서다.
- 쌍의 턴 번호는 시드 1210으로 앞 제약 1~20턴, 나중 제약은 그 뒤 1~30턴에 둔다.
- `indirect-request`에는 `아니, 영어로 바꿔`처럼 `indirect-constraint`와 겉모양이 같은 입력을 넣는다. 앞 말이 요청인지 제약인지로만 갈리게 하기 위해서다.

### 라벨 기준표

`is_constraint` 라벨은 맥락 고르기 설계의 세 조건을 모두 채울 때 제약이다.

| 조건 | 제약으로 보는 경우 | 제약으로 보지 않는 경우 |
|---|---|---|
| 지속 | `앞으로`, `항상`, `통일해`, `하지 마`가 있거나, 대상이 `커밋 메시지`, `주석`처럼 범주 전체다. | `이번`, `이 파일`, `이 줄만`, `방금 만든`처럼 대상이 하나로 정해진다. |
| 방식 | 언어, 도구, 형식, 이름 규칙, 길이, 금지, 확인 절차를 정한다. | 무엇을 할지, 어디를 볼지, 무엇을 알려 달라는지만 말한다. |
| 섞인 입력 | 일 요청 안에 지속하는 방식 지정이 하나라도 있으면 원문 전체를 제약으로 본다. | 방식 지정이 그 일에만 걸리면 제약이 아니다. |
| 간접 지시 | 앞 입력이 제약이고 지금 입력이 그 제약의 값이나 범위를 바꾸거나 푼다. | 앞 입력이 요청이나 질문이고 지금 입력이 그 결과만 바꾼다. |

`replaces_1` 라벨은 나중 제약을 따를 때 앞 제약이 어떻게 되는지로 정한다.

| 라벨 | 기준 | 예 |
|---|---|---|
| 대체 | 같은 대상에 다른 값을 정하거나 금지와 허용을 뒤집어, 나중 제약을 따르면 앞 제약을 어디서도 지킬 수 없다. | `에러 메시지는 영어로 통일해` 뒤 `아니, 한국어로 바꿔` |
| 부분 | 나중 제약이 앞 제약 대상의 일부에만 예외를 두어, 그 일부에서만 앞 제약을 지킬 수 없다. | `모든 출력은 영어로 해` 뒤 `에러 메시지는 한국어로 써` |
| 양립 | 대상이나 차원이 달라 두 제약을 함께 지킬 수 있다. 대상이 겹쳐도 값이 충돌하지 않으면 양립이다. | `커밋 메시지는 영어로 써` 뒤 `커밋 메시지는 50자 이내로 써` |

- 라벨은 문장을 쓸 때 이 표로 붙이고, 사전 등록 머지 뒤에는 바꾸지 않는다.
- 구간마다 맞는 라벨: 0.8 이상은 대체, 0.5 이상 0.8 미만은 부분, 0.5 미만이나 판단 없음은 양립이다.

## 변수

| 변수 | 종류 | 정의 | 단위 |
|---|---|---|---|
| `item_id` | 조작 | 평가 세트 항목 식별자. 입력은 `in-001`..`in-200`, 쌍은 `pr-001`..`pr-160` | 없음 |
| `answer` | 측정 | judge 답의 `noul` 확률. 없거나 0~1 밖이면 결측 | 확률 |
| `status` | 측정 | `ok`, `invalid`, `no_answer` | 없음 |
| `predicted` | 파생 | `answer ≥ 0.7`이면 제약. 결측이면 대체 규칙대로 제약 아님 | 0 또는 1 |
| `band` | 파생 | `answer ≥ 0.8`이면 `replace`, 0.5 이상이면 `possible`, 0.5 미만이나 결측이면 `none` | 없음 |
| `precision` | 파생 | `predicted`가 1이고 라벨이 제약인 수 / `predicted`가 1인 수 | 비율 |
| `recall` | 파생 | `predicted`가 1이고 라벨이 제약인 수 / 라벨이 제약인 수 | 비율 |
| `replace_band_precision` | 파생 | `band`가 `replace`이고 라벨이 대체인 수 / `band`가 `replace`인 수 | 비율 |
| `none_band_accuracy` | 파생 | `band`가 `none`이고 라벨이 양립인 수 / `band`가 `none`인 수 | 비율 |
| `indirect_accuracy` | 파생 | 간접 지시 입력 50건 중 `predicted`와 라벨이 같은 비율 | 비율 |
| `indirect_replace_recall` | 파생 | `indirect-replace` 30쌍 중 `band`가 `replace`인 비율 | 비율 |
| `latency_ms` | 측정 | 요청을 보낸 뒤 답을 받기까지 걸린 시간 | ms |

## 표본

| 항목 | 값 |
|---|---|
| 출처 | 이 폴더의 합성 평가 세트 [eval/inputs.jsonl](eval/inputs.jsonl), [eval/pairs.jsonl](eval/pairs.jsonl) |
| 크기 | 입력 200건(제약 105, 제약 아님 95), 쌍 160건(대체 60, 부분 30, 양립 70) |
| 크기 근거 | 정밀도. 비율 0.85에서 95% Wilson 구간 반폭이 n=100이면 약 7%p, n=50이면 약 10%p, n=30이면 약 13%p다. 간접 지시 구간 30건은 70% 기준을 13%p 폭으로 가를 수 있는 최소로 둔다. |
| 중단 규칙 | 360건을 모두 한 번씩 물으면 멈춘다. 키 거절(401, 403)이 나오면 그 자리에서 멈추고 그 실행 파일을 지운 뒤 처음부터 다시 수집한다. |
| 반복과 예열 | 항목마다 1회, 예열 없음 |

## 분석

| 가설 | 지표 | 방법 | 채택 기준 |
|---|---|---|---|
| H1 | `precision` | 비율과 95% Wilson 신뢰구간, H0: 비율 ≤ 80% 단측 점수 검정 | Holm 보정 p < 0.05이고 95% Wilson 신뢰구간 하한 > 80% |
| H2 | `recall` | H1과 같은 방법 | Holm 보정 p < 0.05이고 95% Wilson 신뢰구간 하한 > 80% |
| H3 | `replace_band_precision` | H1과 같은 방법, 기준 90% | Holm 보정 p < 0.05이고 95% Wilson 신뢰구간 하한 > 90% |
| H4 | `none_band_accuracy` | H1과 같은 방법, 기준 80% | Holm 보정 p < 0.05이고 95% Wilson 신뢰구간 하한 > 80% |
| H5 | `indirect_accuracy` | H1과 같은 방법, 기준 80% | Holm 보정 p < 0.05이고 95% Wilson 신뢰구간 하한 > 80% |
| H6 | `indirect_replace_recall` | H1과 같은 방법, 기준 70% | Holm 보정 p < 0.05이고 95% Wilson 신뢰구간 하한 > 70% |

- 95% Wilson 신뢰구간 상한이 기준보다 낮으면 기각, 채택도 기각도 아니면 보류다.

| 항목 | 규칙 |
|---|---|
| 제외 기준 | 해당 없음. 키 거절로 멈춘 실행은 파일을 남기지 않고, 그 밖의 항목은 제외하지 않는다. |
| 실패한 실행 | 무응답, 시간 초과, `invalid`는 judge 설계의 대체 규칙대로 센다. `is_constraint`는 제약 아님, `replaces_1`은 `none` 구간이다. 상태별 수를 따로 보고한다. |
| 다중 비교 | 확인 가설 6개의 단측 p값에 Holm-Bonferroni 보정 |

## 판정

| 결과 | 설계에 반영 |
|---|---|
| 채택 | 기준값 `is_constraint` 0.7, `replaces_<n>` 0.8과 0.5를 유지하고, 맥락 고르기와 judge 설계 요구사항 행의 검증 계획을 보고서 링크로 바꾼다. |
| 기각 | H1이면 탐색 분석의 기준값 표에서 정밀도 80%를 넘는 가장 낮은 기준으로 `is_constraint` 기본값을 올린다. H2이면 질문 문장을 고친 `route@1.2`를 다시 잰다. H3이면 0.8 이상도 대체 대신 충돌 가능으로만 기록한다. H4이면 0.5 기준을 낮춰 충돌 가능을 넓힌다. H5나 H6이면 간접 지시를 judge 약점으로 맥락 고르기 단점에 적고, 간접 지시 대체는 충돌 가능으로만 기록한다. |
| 보류 | 보류된 가설의 구간만 항목을 두 배로 늘린 평가 세트로 새 실험 이슈를 연다. |

## 탐색 분석

- `is_constraint` 기준값 0.5~0.9를 0.05 간격으로 바꿨을 때의 정밀도와 재현율
- `replaces_1` 0.5 이상 0.8 미만 구간의 부분 라벨 비율과 라벨 구성
- 구간별(`constraint`, `mixed`, `request`, `indirect-*`, `partial`, `compatible`) 정확도
- `indirect-compatible`을 대체로 잘못 판정한 비율
- `invalid`와 무응답 비율, `latency_ms` 분포

## 타당성 위협

| 종류 | 위협 | 대응 |
|---|---|---|
| 내적 | 문장과 라벨을 같은 사람이 써서 판정이 쉬운 문장으로 기운다. | 겉모양이 같은 간접 지시를 제약과 요청 양쪽에 두고, 라벨 기준표와 평가 세트를 수집 전에 머지해 고정한다. |
| 내적 | judge 답이 같은 입력에서도 실행마다 달라질 수 있다. | 항목마다 한 번만 묻고 응답 `model`을 기록한다. 반복 편차는 이 실험 범위 밖으로 보고서 한계에 적는다. |
| 구성 | 실제 엔진은 `is_constraint`를 `route` 질문 세트의 다른 질문과 함께 묻고 `replaces_<n>`을 최대 10쌍 함께 묻는데, 이 실험은 질문 하나씩 묻는다. | 보고서 한계에 적고, 엔진이 쌓은 판단 기록으로 judge 학습 채점에서 다시 확인한다. |
| 구성 | 질문 문장이 아직 코드에 없어 이 실험이 정한 문장을 잰다. | 문장을 `eval/questions.json`에 고정하고, 채택하면 같은 문장을 `judges`에 옮긴다. |
| 외적 | 합성 문장은 실제 사용자 입력보다 짧고 규칙적이다. | 적용 범위를 합성 한국어 입력으로 한정하고, 실제 판단 기록의 취소 신호로 judge 학습에서 다시 잰다. |
