# 보류된 제약 대체 구간과 간접 지시 정확도: 실험 설계

| 항목 | 값 |
|---|---|
| 이슈 | [#152](https://github.com/woonyong-choi/saturn/issues/152) |
| 관련 설계 | [맥락 고르기](../../design/context-selection.md), [judge](../../design/router.md) |
| 사전 데이터 | [제약 식별과 대체 판정 정확도 실험 결과](../constraint-judge-accuracy/report.md)의 요약 수치만 봤다. 그 실험의 항목별 judge 답은 열어 보지 않았다. 새 항목 문장은 그 답을 보지 않고 썼다. |

## 질문

[#121](https://github.com/woonyong-choi/saturn/issues/121)에서 보류된 `replaces_1` 구간과 간접 지시 입력의 정확도를 항목 수를 늘린 평가 세트로 다시 재서 판정한다. 앞 입력이 이미 제약으로 등록됐는지를 `route` state에 넣으면 간접 지시 입력의 `is_constraint` 정확도가 오르는지를 같은 입력 쌍으로 현행과 비교한다.

## 가설

| 가설 | 예측 | 적용 범위 |
|---|---|---|
| H1 | 현행 `route@1.1`의 간접 지시 입력 `is_constraint` 0.7 기준 정확도는 80%를 넘는다. | 앞 입력을 가리키는 한국어 입력 200건, 기준 judge `jev-1.13.0` |
| H2 | `replaces_1` 답이 0.8 이상인 쌍 중 대체 라벨 비율은 90%를 넘는다. | 한국어 제약 쌍 320건, `constraint@1.0`, 기준 judge `jev-1.13.0` |
| H3 | `replaces_1` 답이 0.5 미만이거나 없는 쌍 중 양립 라벨 비율은 80%를 넘는다. | 한국어 제약 쌍 320건, `constraint@1.0`, 기준 judge `jev-1.13.0` |
| H4 | 간접 지시 대체 쌍에서 `replaces_1` 답이 0.8 이상인 비율은 70%를 넘는다. | 나중 제약이 앞 제약을 가리키는 한국어 쌍 60건, `constraint@1.0`, 기준 judge `jev-1.13.0` |
| H5 | state에 앞 입력의 제약 등록 여부를 넣은 `flag` 조건의 간접 지시 입력 정확도는 현행보다 높다. | 앞 입력을 가리키는 한국어 입력 200건, 기준 judge `jev-1.13.0` |
| H6 | state에 등록된 제약 원문(없으면 `none`)을 넣은 `flag_text` 조건의 간접 지시 입력 정확도는 현행보다 높다. | 앞 입력을 가리키는 한국어 입력 200건, 기준 judge `jev-1.13.0` |
| H7 | `flag` 조건의 간접 지시가 아닌 입력 정확도는 90%를 넘는다. | 간접 지시가 아닌 한국어 입력 150건, 기준 judge `jev-1.13.0` |
| H8 | `flag_text` 조건의 간접 지시가 아닌 입력 정확도는 90%를 넘는다. | 간접 지시가 아닌 한국어 입력 150건, 기준 judge `jev-1.13.0` |

H1~H4는 #121 H5, H3, H4, H6을 새 항목을 더한 평가 세트로 다시 재는 가설이다. H5~H6은 대응 후보가 현행보다 낫다는 가설이고, H7~H8은 후보가 간접 지시가 아닌 입력을 해치지 않는다는 가설이다. #121의 간접 지시 입력 정확도는 80.0%(40/50)였다.

## 설계

| 항목 | 값 |
|---|---|
| 조건 | 입력 항목마다 5개 조건에 `is_constraint`를 한 번씩 묻는다. `baseline`: 현행 state(`previous_user_input`, `latest_user_input`). `flag`: 현행 state에 `previous_input_registered_as_constraint`(`yes` 또는 `no`)를 더한다. `flag_text`: 현행 state에 `registered_constraint`(앞 입력이 등록됐으면 그 원문, 아니면 `none`)를 더한다. `oracle_flag`: `flag`와 같은 모양에 값을 평가 세트 라벨로 넣는다. `flag_hint`: `flag` state에 질문 문장 `route@1.2-draft`를 쓴다. 쌍 항목에는 `replaces_1`을 한 번 묻는다. |
| 배정 | 등록 판정 질문을 먼저 묻고, 입력 1750건과 쌍 320건을 한 목록으로 합쳐 Python `random.Random(152).shuffle` 순서로 묻는다. |
| 눈가림 | 수집 스크립트는 라벨을 읽지 않는다. 수집하는 사람은 `./run.sh analyze` 전에 `data/raw/`의 답을 열지 않는다. |
| 환경 | 기준 judge `POST https://api.typesafe.ai/v1/systemone`, 요청 모델 `jev-1.13.0`, 응답의 `model`을 기록한다. Python 3.10 이상 표준 라이브러리만 쓴다. judge 샘플링은 고정할 수 없어 항목마다 한 번만 묻고 그 답을 쓴다. Jev API 키를 환경 변수 `SATURN_JUDGE_KEY`로 받는다. 키가 없으면 `01-collect`가 원인 한 줄을 남기고 끝난다. |

`flag`와 `flag_text`의 등록 여부는 engine이 실제로 아는 값이다. 그래서 라벨이 아니라 judge의 앞 입력 판정으로 만든다.

1. 앞 입력(`previous`)의 서로 다른 문장마다 `is_constraint`를 한 번 묻는다. 그 문장의 앞 입력은 간접 지시가 아닌 입력의 앞 입력 10개 중 시드 152로 고른 요청이다.
2. 답이 0.7 이상이면 등록된 것으로, 답이 없으면 등록되지 않은 것으로 본다.
3. 이 값을 `flag`, `flag_text`의 state에 넣는다.

`oracle_flag`는 등록 판정 오류를 뺀 상한을 보려는 탐색 조건이다. `flag_hint`는 state만 바꿔 효과가 없을 때 질문 문장까지 바꿔 보는 탐색 조건이고, 둘 다 확인 가설에 쓰지 않는다.

질문 문장은 [eval/questions.json](eval/questions.json)에 고정한다. judge 설계대로 질문은 영어로 쓰고 사용자 원문은 그대로 넣는다.

| 질문 | 질문 세트 | state | 문장 |
|---|---|---|---|
| `is_constraint` | `route@1.1` | `previous_user_input`, `latest_user_input`, 조건에 따라 등록 여부 | `The user's latest input sets a rule that applies beyond this single request and limits how the work is done (language, tool, format, or prohibition) rather than what to do.` |
| `is_constraint_hint` | `route@1.2-draft` | `flag`와 같다 | `is_constraint` 문장에 앞 입력이 등록된 제약이면 바꾸거나 풀거나 취소하는 입력은 제약이고, 등록되지 않았으면 그 요청의 결과만 고치는 입력은 제약이 아니라는 두 문장을 더한다. |
| `replaces_1` | `constraint@1.0` | `earlier_constraint`, `later_constraint`의 턴과 원문 | `Following the later constraint makes it impossible to keep the earlier constraint.` |

### 평가 세트

평가 세트는 공개 가능한 합성 한국어 문장이다. 실제 대화나 개인정보는 넣지 않는다. [#121](https://github.com/woonyong-choi/saturn/issues/121) 평가 세트 360건을 그대로 가져오고(`origin` 필드 `121`), 간접 지시 입력과 쌍을 새로 더한다(`origin` 필드 `new`). 항목 번호는 #121의 `in-001`..`in-200`, `pr-001`..`pr-160`을 유지하고 새 항목은 이어서 매긴다.

| 파일 | 구간 | 수 | 라벨 |
|---|---|---|---|
| [eval/inputs.jsonl](eval/inputs.jsonl) | `constraint`: 제약만 있는 입력 | 50(#121) | 제약 |
| [eval/inputs.jsonl](eval/inputs.jsonl) | `mixed`: 일과 제약이 섞인 입력 | 30(#121) | 제약 |
| [eval/inputs.jsonl](eval/inputs.jsonl) | `request`: 일반 요청, 질문, 이번 한 번만 적용하는 방식 지정 | 70(#121) | 제약 아님 |
| [eval/inputs.jsonl](eval/inputs.jsonl) | `indirect-constraint`: 앞 제약을 가리켜 바꾸는 간접 지시 | 100(#121 25, 새 75) | 제약 |
| [eval/inputs.jsonl](eval/inputs.jsonl) | `indirect-request`: 앞 요청을 가리켜 이번 일만 바꾸는 간접 지시 | 100(#121 25, 새 75) | 제약 아님 |
| [eval/pairs.jsonl](eval/pairs.jsonl) | `direct-replace`: 나중 제약이 같은 대상의 값을 직접 바꾼다 | 60(#121 30, 새 30) | 대체 |
| [eval/pairs.jsonl](eval/pairs.jsonl) | `indirect-replace`: 나중 제약이 앞 제약을 가리켜 바꾼다 | 60(#121 30, 새 30) | 대체 |
| [eval/pairs.jsonl](eval/pairs.jsonl) | `partial`: 나중 제약이 앞 제약의 일부 범위에 예외를 둔다 | 60(#121 30, 새 30) | 부분 |
| [eval/pairs.jsonl](eval/pairs.jsonl) | `compatible`: 두 제약을 함께 지킬 수 있다 | 120(#121 60, 새 60) | 양립 |
| [eval/pairs.jsonl](eval/pairs.jsonl) | `indirect-compatible`: 앞 제약을 가리키지만 함께 지킬 수 있다 | 20(#121 10, 새 10) | 양립 |

- 새 `indirect-replace` 쌍 30개는 새 `indirect-constraint` 입력 앞 30개와 같은 두 문장이다.
- 간접 지시가 아닌 입력의 `previous`는 #121과 같은 중립 요청 10개다. 간접 지시만 앞 입력을 갖는 단서를 없애기 위해서다.
- 새 쌍의 턴 번호는 시드 1520으로 앞 제약 1~20턴, 나중 제약은 그 뒤 1~30턴에 둔다.
- `indirect-request`에는 `아니, 영어로 바꿔`처럼 `indirect-constraint`와 겉모양이 같은 입력을 넣는다. 앞 말이 요청인지 제약인지로만 갈리게 하기 위해서다.
- 등록 판정 질문의 정답은 `indirect-constraint` 입력의 앞 입력만 제약이고 나머지 앞 입력은 요청이다.

### 라벨 기준표

라벨 기준은 [#121 실험 설계](../constraint-judge-accuracy/design.md)의 라벨 기준표를 그대로 쓴다. 요약은 다음과 같다.

| 대상 | 라벨 | 기준 |
|---|---|---|
| 입력 | 제약 | 지속(`앞으로`, `항상`, `하지 마`, 범주 전체), 방식(언어, 도구, 형식, 이름 규칙, 길이, 금지, 확인 절차), 섞인 입력이면 방식 지정이 하나라도 지속하면 제약 |
| 입력 | 제약(간접 지시) | 앞 입력이 제약이고 지금 입력이 그 제약의 값이나 범위를 바꾸거나 푼다. |
| 입력 | 제약 아님(간접 지시) | 앞 입력이 요청이나 질문이고 지금 입력이 그 결과만 바꾼다. |
| 쌍 | 대체 | 같은 대상에 다른 값을 정하거나 금지와 허용을 뒤집어, 나중 제약을 따르면 앞 제약을 어디서도 지킬 수 없다. |
| 쌍 | 부분 | 나중 제약이 앞 제약 대상의 일부에만 예외를 두어 그 일부에서만 앞 제약을 지킬 수 없다. |
| 쌍 | 양립 | 대상이나 차원이 달라 두 제약을 함께 지킬 수 있다. 대상이 겹쳐도 값이 충돌하지 않으면 양립이다. |

- 라벨은 문장을 쓸 때 이 표로 붙이고, 사전 등록 머지 뒤에는 바꾸지 않는다.
- 구간마다 맞는 라벨: 0.8 이상은 대체, 0.5 이상 0.8 미만은 부분, 0.5 미만이나 판단 없음은 양립이다.

## 변수

| 변수 | 종류 | 정의 | 단위 |
|---|---|---|---|
| `item_id` | 조작 | 평가 세트 항목 식별자. 입력은 `in-001`..`in-350`, 쌍은 `pr-001`..`pr-320` | 없음 |
| `condition` | 조작 | `baseline`, `flag`, `flag_text`, `oracle_flag`, `flag_hint`, 등록 판정 `registration`, 쌍 질문 `replaces` | 없음 |
| `answer` | 측정 | judge 답의 `noul` 확률. 없거나 0~1 밖이면 결측 | 확률 |
| `status` | 측정 | `ok`, `invalid`, `no_answer` | 없음 |
| `predicted` | 파생 | `answer ≥ 0.7`이면 제약. 결측이면 대체 규칙대로 제약 아님 | 0 또는 1 |
| `correct` | 파생 | `predicted`와 입력 라벨이 같으면 1 | 0 또는 1 |
| `band` | 파생 | `answer ≥ 0.8`이면 `replace`, 0.5 이상이면 `possible`, 0.5 미만이나 결측이면 `none` | 없음 |
| `indirect_accuracy` | 파생 | 조건별로 간접 지시 입력 200건 중 `correct`인 비율 | 비율 |
| `direct_accuracy` | 파생 | 조건별로 간접 지시가 아닌 입력 150건 중 `correct`인 비율 | 비율 |
| `accuracy_diff` | 파생 | 같은 입력에서 후보 조건의 `correct` 비율 − `baseline`의 `correct` 비율 | 비율 차이 |
| `replace_band_precision` | 파생 | `band`가 `replace`이고 라벨이 대체인 수 / `band`가 `replace`인 수 | 비율 |
| `none_band_accuracy` | 파생 | `band`가 `none`이고 라벨이 양립인 수 / `band`가 `none`인 수 | 비율 |
| `indirect_replace_recall` | 파생 | `indirect-replace` 60쌍 중 `band`가 `replace`인 비율 | 비율 |
| `latency_ms` | 측정 | 요청을 보낸 뒤 답을 받기까지 걸린 시간 | ms |

## 표본

| 항목 | 값 |
|---|---|
| 출처 | 이 폴더의 합성 평가 세트 [eval/inputs.jsonl](eval/inputs.jsonl), [eval/pairs.jsonl](eval/pairs.jsonl) |
| 크기 | 입력 350건(제약 180, 제약 아님 170)을 조건 5개에, 쌍 320건(대체 120, 부분 60, 양립 140)을 한 번씩. 등록 판정 질문은 앞 입력 서로 다른 문장 209개 |
| 크기 근거 | 검정력. H5, H6은 짝지은 두 정확도의 차이 8%p, 불일치 쌍 비율 16%, 단측 α = 0.05, 검정력 80%에서 필요한 입력이 약 153건이라 간접 지시 입력을 200건으로 둔다. H2~H4는 정밀도. 대체 구간 쌍이 약 100건이면 95% Wilson 구간 반폭이 약 6%p, 간접 지시 대체 쌍 60건이면 약 9%p다. |
| 중단 규칙 | 등록 판정 209건과 입력 1750건, 쌍 320건 모두 한 번씩 물으면 멈춘다. 키 거절(401, 403)이 나오거나 `invalid`, 무응답이 10건 연속이면 그 자리에서 멈추고 그 실행 파일을 지운 뒤 원인을 보고한다. |
| 반복과 예열 | 항목과 조건마다 1회, 예열 없음 |

judge 요청은 모두 2279건이다. 요청 하나는 질문 하나를 담는다.

## 분석

| 가설 | 지표 | 방법 | 채택 기준 |
|---|---|---|---|
| H1 | `indirect_accuracy`(`baseline`) | 비율과 95% Wilson 신뢰구간, H0: 비율 ≤ 80% 단측 점수 검정 | Holm 보정 p < 0.05이고 95% Wilson 신뢰구간 하한 > 80% |
| H2 | `replace_band_precision` | H1과 같은 방법, 기준 90% | Holm 보정 p < 0.05이고 95% Wilson 신뢰구간 하한 > 90% |
| H3 | `none_band_accuracy` | H1과 같은 방법, 기준 80% | Holm 보정 p < 0.05이고 95% Wilson 신뢰구간 하한 > 80% |
| H4 | `indirect_replace_recall` | H1과 같은 방법, 기준 70% | Holm 보정 p < 0.05이고 95% Wilson 신뢰구간 하한 > 70% |
| H5 | `accuracy_diff`(`flag` − `baseline`, 간접 지시 입력) | 2×2 표와 불일치 수 b(후보만 맞음), c(현행만 맞음). b+c < 25면 정확 이항 단측 검정, 아니면 McNemar 단측 검정. 차이의 95% 신뢰구간은 Newcombe 짝지은 비율 방법 | Holm 보정 p < 0.05이고 차이의 95% 신뢰구간 하한 > 0 |
| H6 | `accuracy_diff`(`flag_text` − `baseline`, 간접 지시 입력) | H5와 같은 방법 | Holm 보정 p < 0.05이고 차이의 95% 신뢰구간 하한 > 0 |
| H7 | `direct_accuracy`(`flag`) | H1과 같은 방법, 기준 90% | Holm 보정 p < 0.05이고 95% Wilson 신뢰구간 하한 > 90% |
| H8 | `direct_accuracy`(`flag_text`) | H1과 같은 방법, 기준 90% | Holm 보정 p < 0.05이고 95% Wilson 신뢰구간 하한 > 90% |

- H1~H4, H7, H8은 95% Wilson 신뢰구간 상한이 기준보다 낮으면 기각, H5, H6은 차이의 신뢰구간 상한이 0 이하이면 기각이다. 채택도 기각도 아니면 보류다.
- 후보 채택 규칙: 후보의 개선 가설(`flag`는 H5, `flag_text`는 H6)과 해치지 않음 가설(H7, H8)이 모두 채택이고, 그 후보의 간접 지시 입력 정확도 95% Wilson 신뢰구간 하한이 80%를 넘으면 후보를 채택한다. 둘 다 채택 조건을 채우면 간접 지시 입력 정확도가 높은 쪽, 같으면 `flag`를 고른다.

| 항목 | 규칙 |
|---|---|
| 제외 기준 | 해당 없음. 키 거절이나 연속 실패로 멈춘 실행은 파일을 남기지 않고, 그 밖의 항목은 제외하지 않는다. |
| 실패한 실행 | 무응답, 시간 초과, `invalid`는 judge 설계의 대체 규칙대로 센다. `is_constraint`는 제약 아님, `replaces_1`은 `none` 구간, 등록 판정은 미등록이다. 상태별 수를 따로 보고한다. |
| 다중 비교 | 확인 가설 8개의 단측 p값에 Holm-Bonferroni 보정 |

## 판정

| 결과 | 설계에 반영 |
|---|---|
| 채택 | 후보 채택 규칙을 채운 후보를 맞춰 [맥락 고르기](../../design/context-selection.md) 제약 식별 절과 [judge](../../design/router.md)의 `is_constraint` 행에 state 구성(앞 입력의 등록 여부)과 이유를 적고 관련 요구사항 행에 보고서를 링크한다. H1~H4는 채택된 가설의 기준값을 확인된 값으로 적는다. |
| 기각 | H1이면 간접 지시 약점을 맥락 고르기 단점에 수치와 함께 적는다. H2이면 0.8 이상도 대체 대신 충돌 가능으로만 기록한다. H3이면 0.5 기준을 낮추거나 부분 충돌을 따로 묻는 질문을 새 실험 이슈로 연다. H4이면 간접 지시 대체는 충돌 가능으로만 기록한다. H5~H8이 기각이면 그 후보를 설계에 넣지 않고 이유를 judge 설계에 적는다. |
| 보류 | 보류된 가설의 구간만 항목을 더 늘려 새 실험 이슈를 연다. |

## 탐색 분석

- 조건 5개의 입력 구간별 정확도와 정밀도, 재현율
- `oracle_flag`와 `flag_hint`가 `baseline`보다 나은 정도(같은 입력 쌍, 단측 p는 보정하지 않고 보고)
- 등록 판정 질문의 정확도와 틀린 방향(잘못 등록, 놓침)
- `baseline`의 간접 지시 입력 정확도를 #121 항목과 새 항목으로 나눈 값
- `replaces_1` 구간별 쌍 수와 0.5 이상 0.8 미만 구간의 라벨 구성
- `invalid`와 무응답 비율, `latency_ms` 분포

## 타당성 위협

| 종류 | 위협 | 대응 |
|---|---|---|
| 내적 | 문장과 라벨을 같은 사람이 써서 판정이 쉬운 문장으로 기운다. | 겉모양이 같은 간접 지시를 제약과 요청 양쪽에 같은 수로 두고, 라벨 기준표와 평가 세트를 수집 전에 머지해 고정한다. |
| 내적 | judge 답이 같은 입력에서도 실행마다 달라질 수 있다. | 항목과 조건마다 한 번만 묻고 응답 `model`을 기록한다. 반복 편차는 이 실험 범위 밖으로 보고서 한계에 적는다. |
| 내적 | 조건 사이 차이가 state 때문이 아니라 요청 순서나 시각 때문일 수 있다. | 모든 조건과 쌍 질문을 한 목록으로 섞어 시드 순서로 묻고, 같은 입력의 조건끼리 짝지어 비교한다. |
| 구성 | 등록 여부를 judge의 앞 입력 판정으로 만들어, 앞 입력을 잘못 판정하면 오류가 이어진다. | 실제 engine과 같은 조건이라 그대로 두고, 라벨 값을 넣은 `oracle_flag`와 등록 판정 정확도를 탐색 분석에 같이 보고한다. |
| 구성 | 실제 엔진은 `is_constraint`를 `route` 질문 세트의 다른 질문과 함께 묻고 `replaces_<n>`을 최대 10쌍 함께 묻는데, 이 실험은 질문 하나씩 묻는다. | 보고서 한계에 적고, 엔진이 쌓은 판단 기록으로 judge 학습 채점에서 다시 확인한다. |
| 구성 | 새 state 필드 이름과 값은 이 실험이 정한 문자열이다. | 필드 이름과 값을 `eval/questions.json`과 `scripts/01-collect.py`에 고정하고, 채택하면 같은 이름을 `judges`에 옮긴다. |
| 외적 | 합성 문장은 실제 사용자 입력보다 짧고 규칙적이며, 새 간접 지시 문장은 같은 사람이 쓴 같은 틀이 많다. | 적용 범위를 합성 한국어 입력으로 한정하고, 실제 판단 기록의 취소 신호로 judge 학습에서 다시 잰다. |
