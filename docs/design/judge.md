# judge

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [판단 규격은 Saturn이 정하고 judge는 중립 이름과 출처로 기록한다](../decisions/2026-09-29-vendor-neutral-judge-spec.md), [engine만 judge를 부르고 자식 프로세스 환경에서 judge 키를 지운다](../decisions/2026-09-29-engine-as-judge-proxy.md), [판단 기록은 로컬에 쌓고 동의한 레코드만 서버로 올린다](../decisions/2026-09-29-local-first-judgment-collection.md) |

## 요약

judge는 입력마다 뜻을 확률로 판단하는 작은 모델이다. 이어 가는 말인지, 새 작업인지, 어느 모델에 맡길지를 선택지별 확률로 답한다. `core`가 질문 세트, 기준값, 대체 규칙을 정하고, `engine`만 judge를 부른다. judge에는 외부 API인 기준 judge와 로컬의 Saturn 모델이 있고, 판단 방식 설정으로 행동을 정할 judge를 고른다.

## 동기

사용자는 에이전트가 일하는 중에도 입력을 보낸다. 그 입력이 하던 작업을 다듬는 말인지, 무관한 새 일인지는 문장의 뜻을 읽어야 가를 수 있다. 코드 규칙으로는 뜻을 가를 수 없고, 사용자에게 맡기면 입력마다 대상과 모델을 직접 골라야 한다. judge는 뜻 판단만 확률로 맡고, 행동은 코드가 기준값과 비교해 정한다. 확률과 기준값을 나누면 확신이 낮은 판단으로 행동하지 않고 대체 규칙으로 갈 수 있다. 판단 기록은 로컬 Saturn 모델의 학습 재료가 된다.

## 예시

### 실행 중 입력이 이어 가는 말인지 판단할 때

1. 사용자가 에이전트 A가 코드를 고치는 중에 "에러 메시지도 한국어로 바꿔 줘"를 입력한다.
2. engine이 입력을 접수하고 `route`, `relation`, `send-opt` 질문을 요청 한 건에 묶어 judge에 보낸다.
3. judge가 `keep_current`에 0.91을 답하고, `relation_to_running`에서는 `refines`를 가장 높게 답한다.
4. `keep_current`가 0.8 이상이므로 `core`가 입력을 하던 에이전트 A로 보내기로 정한다.
5. engine이 진행 중인 턴에 입력을 끼워 넣고, 보낸 원문과 받은 원문을 판단 기록에 저장한다.

### 확신이 낮은 판단을 받을 때

1. 사용자가 실행 중에 "이거 말고 저쪽 먼저"를 입력한다.
2. judge가 `relation_to_running`의 네 선택지에 비슷한 확률을 답해 확신도가 0.6 미만이 된다.
3. `core`는 관계 판단으로 행동하지 않고 대체 규칙대로 입력을 대기로 보낸다.
4. 사용자는 입력이 대기열에 들어간 것을 보고 직접 보내거나 취소한다.

### judge 답이 형식에 맞지 않을 때

1. judge가 `target_model` 질문에 허용 후보에 없는 모델을 답한다.
2. engine이 그 판단을 `invalid`로 기록한다.
3. `core`가 `target_model`의 대체 규칙대로 사용자가 고정한 모델이나 현재 모델을 쓴다.
4. 판단 기록에는 받은 원문과 `invalid` 표시가 함께 남는다.

## 상세 설계

### 코드와 judge의 경계

- 기계적으로 판단할 수 있는 것은 코드가, 뜻을 이해해야 하는 것은 judge가 정한다. judge를 뜻 판단에만 쓰기 위해서다.
- 제어 명령이 아닌 입력마다 judge를 한 번 부르고 필요한 질문을 요청 한 건에 묶는다. 입력당 호출 수를 한 번으로 줄이기 위해서다.
- 사용자가 모델을 고정한 입력이면 judge 호출을 생략한다.
- 후보를 고르는 질문(`compact`, `file-rank`)은 후보가 상위 N개보다 많으면 `core`가 순위로 좁힌 뒤 묻는다. judge 요청 크기를 후보 수와 무관하게 두기 위해서다. 순위 규칙은 [맥락 고르기](context-selection.md)에 있다.
- 뜻 판단이 새로 필요하면 입력마다 보내는 요청에 질문을 더하는 방식을 먼저 쓴다. judge는 state를 한 번 읽고 모든 질문에 답하므로 호출 수가 늘지 않기 때문이다. 개수 세기, 날짜 비교, 앞 말을 가리키는 간접 지시처럼 judge가 약한 판단은 코드로 하거나 두 원문을 나란히 놓는 질문으로 바꾼다.
- judge는 앞 입력의 판단 결과를 state에 넣은 요청으로 판단한다. 판단 차례와 적용 직전 revision 비교는 [입력 처리](input-handling.md)에 있다.
- `keep_current`를 `is_actionable`보다 먼저 읽는다. 이어 가는 입력이 파일 탐색으로 빠지는 일을 막기 위해서다.
- `keep_current`가 이어 가는 입력으로 답하면 하던 에이전트로 보낸다. 새 작업이면 `is_actionable`, 모델 선택 순서로 판정한다.
- judge가 무관한 작업으로 판단하면 같은 채팅 안에 보조 에이전트를 시작한다.
- 모든 judge는 `core`의 `JudgeClient` trait 구현으로만 붙는다. `engine`의 `judges`는 외부 API용 `RemoteJudge`와 로컬 모델용 `LocalJudge`를 둔다.
- judge는 `engine`만 부른다. judge 키가 자식 프로세스나 다른 호스트로 새는 일을 막기 위해서다.
- judge 호출 한도와 지출 상한은 두지 않고 호출 수와 예상 비용만 보인다. 계정 상한 정보가 없어 상한을 정할 근거가 없기 때문이다.

### 질문 세트와 버전

질문 세트는 judge에 한 번에 묻는 질문 묶음과 그 버전이다. 판단 기록은 질문을 `ID@버전`으로 가리킨다.

- 질문 문장과 선택지는 `judges` 한 곳에서 버전으로 관리하고 릴리스로만 바꾼다. 판단 기록의 `ID@버전`으로 같은 질문을 다시 찾기 위해서다.
- 뜻이 같은 작은 변경은 `route@3.1`처럼 소수 버전을 올린다. 작은 변경에서는 옛 답과 라벨을 그대로 쓰기 위해서다.
- 뜻이나 선택지가 바뀌는 큰 변경은 `route@4`처럼 주 버전을 올린다.
- 큰 변경은 옛 선택지에서 새 선택지로 가는 대응표를 코드에 함께 둔다. `/train` 때 옛 판단을 새 질문으로 다시 채점하기 위해서다.
- 답이 하나인 `choice` 질문에는 `기타` 선택지를 넣는다. 선택지를 늘려도 옛 답이 덜 깨지게 하기 위해서다.
- 여러 개가 맞을 수 있는 속성은 `noul`로 하나씩 묻는다. 같은 이유다.
- 질문과 기준은 영어로 쓰고 사용자 원문은 그대로 넣는다. 영어 밖 언어에서 판단 정확도가 떨어지는 일을 질문 쪽에서 줄이기 위해서다([#15](https://github.com/woonyong-choi/saturn/issues/15)).
- 판단 기록에는 질문 버전과 설정 번호를 남긴다. 버전이 바뀐 뒤에도 옛 기록을 다시 해석하기 위해서다.

### 답 형식

| 형식 | 돌려주는 값 | 제약 |
|---|---|---|
| `choice` | 선택지별 확률과 확신도 | 선택지 255개 이하 |
| `noul` | P(yes) | 없음 |
| `score` | 단계별 분포 | 2~10단계 |

`choice`의 확신도는 `(N·pmax − 1)/(N − 1)`이다. N은 선택지 수, pmax는 가장 높은 확률이다. 확신도는 모든 선택지가 같은 확률이면 0, 한 선택지가 확률 1이면 1이다.

### 질문별 기준값과 대체 규칙

행동 조건은 judge 답으로 행동하는 기준이다. 대체 규칙은 판단이 없거나, 확신도가 기준보다 낮거나, 답이 `invalid`일 때 따르는 동작이다. `keep_current` 0.8은 [#6](https://github.com/woonyong-choi/saturn/issues/6), `resume_held` 0.85는 [#9](https://github.com/woonyong-choi/saturn/issues/9) 실험으로 확인한다.

| 질문 세트 | 질문 | 형식 | 행동 조건 | 대체 규칙 |
|---|---|---|---|---|
| `route` | `keep_current` | `noul` | 0.8 이상이면 현재 에이전트 유지 | 판단이 없거나 0.3~0.8이면 현재 에이전트 유지 |
| `route` | `is_actionable` | `noul` | 0.7 이상이면 명확 | 판단이 없으면 명확으로 간주하고 탐색 생략 |
| `route` | `target_model` | `choice` | 허용 후보 중에서만 선택 | 확신도 0.6 미만이면 사용자 고정 모델이나 현재 모델 |
| `route` | `difficulty` | `score` | 3단계로 답 | 확신도 0.6 미만이면 미사용 |
| `route` | `skills` | `choice` | 선택지에 `none` 포함 | 확신도 0.6 미만이면 힌트 생략 |
| `route` | `resume_held` | `noul` | 0.85 이상에서만 보류 작업 재개 | 0.85 미만이면 무시 횟수 1 증가 |
| `route` | `is_constraint` | `noul` | 0.7 이상이면 입력 원문을 제약으로 등록 | 판단이 없으면 미등록 |
| `term` | `same_<n>` | `noul` | 확인 대기 용어 짝 최대 20개를 트리 유휴 때 함께 질문. 0.8 이상이면 확인, 0.3 미만이면 거절 | 판단이 없으면 대기 유지 |
| `constraint` | `replaces_<n>` | `noul` | 기존 제약 최대 10개와 함께 질문. 0.8 이상이면 대체, 0.5 이상 0.8 미만이면 충돌 가능 | 판단이 없으면 대체와 충돌 가능 기록 생략 |
| `relation` | `relation_to_running` | `choice` | `refines`, `continues`, `independent`, `conflicts` 중 선택 | 확신도 0.6 미만이면 대기 |
| `send-opt` | `steer_or_spawn` | `choice` | `target_model`과 함께 질문 | 확신도 0.6 미만이면 현재 에이전트에 대기 뒤 전송 |
| `file-rank` | `file_<n>_relevant` | `noul` | 순위 상위 파일 20개와 `answer_present`를 함께 질문. 0.7 이상은 존재, 0.35 미만은 없음 | 판단이 없으면 후보 순위 그대로 |
| `context-select` | `pick` | `choice` | 게이트 `noul` 3개 평균이 0.3 미만이면 없음. 2차로 `fits_<n>` 질문 | 판단이 없으면 힌트 생략 |
| `compact` | `call_<id>_keep` | `noul` | 순위 상위 N개 호출마다 `result_<id>_keep`과 함께 질문. 0.5 이상이면 유지 | 판단이 없으면 후보 순위 상위 N개를 유지 |
| `doc-filter` | `injection` | `noul` | 조각마다 `relevant`, `evidence`, `contradiction`과 함께 질문. 0.7 이상이면 제외 | 판단이 없으면 문서 조각 생략 |
| `loop` | `is_progressing` | `noul` | 0.2 미만이면 루프 | 판단이 없으면 멈춤과 사용자 알림 |
| `feedback` | `wrong_doc` | `noul` | `misunderstood_intent`, `code_error`와 함께 질문. 0.7 이상인 원인만 사용 | 판단이 없으면 원문 그대로 전달 |

- `keep_current`가 0.3 미만이면 새 작업으로 본다. 0.3 이상 0.8 미만은 판단 없음과 같다.
- 실행 중이면 처리 방식을 `relation_to_running`과 `steer_or_spawn`으로 정한다. `refines`, `continues`면 `steer_or_spawn`의 `steer`, `queue`, `spawn`을 끼워 넣기, 대기, 새 작업으로 옮기고, `independent`면 새 작업이다. `conflicts`는 미해결 질문이 정해지기 전까지 대기로 둔다. 실행 중이 아니면 `keep_current`로 현재 에이전트 대기와 새 작업을 가른다.
- `target_model`의 선택지는 허용 후보와 `other`이고, `other`를 고르면 대체 규칙을 따른다. 후보가 없으면 묻지 않는다.
- `difficulty`와 `skills`는 답을 쓰는 곳이 생기기 전까지 묻지 않고 대체 규칙(미사용, 힌트 생략)으로 둔다. 쓰지 않는 질문으로 판단 비용을 늘리지 않기 위해서다.
- 질문 세트는 `route@1.0`, `relation@1.0`, `send-opt@1.0`에서 시작한다. `is_constraint`를 더한 `route`는 `route@1.1`이고, `constraint`는 `constraint@1.0`, `term`은 `term@1.0`에서 시작한다. 기존 질문의 뜻은 바뀌지 않기 때문이다.
- 기준값은 설정 층에 둔다. 릴리스 없이 사용자 층과 폴더 층에서 기준값을 조정하기 위해서다.
- 기준값을 판단 기록으로 자동 조정하는 규칙은 [judge 학습](judge-training.md)에 있다.

### 판단 방식

판단 방식은 행동을 정하는 judge를 고르는 설정이다.

| 판단 방식 | 행동을 정하는 judge |
|---|---|
| `jev` | 기준 judge |
| `saturn` | Saturn 모델 |

- 판단 기록은 판단 방식과 관계없이 전부 남긴다. 어느 방식에서도 학습과 비교용 기록을 쌓기 위해서다.
- `saturn` 방식에서 확신도가 기준보다 낮으면 행동하지 않고 질문별 대체 규칙으로 간다. 확신 없는 판단으로 행동하는 일을 막기 위해서다.
- `saturn` 방식에서는 `noul` 답도 확신도 `|2p − 1|`가 `choice` 확신도 기준(0.6)보다 낮으면 판단 없음으로 본다. 같은 이유다.
- 판단 방식과 judge 주소는 폴더 층에서 바꿀 수 없다. 사용자 전용 항목이기 때문이다.
- 기록에는 중립 이름 `judge_id`를 쓴다. 기준 judge는 `jev`, Saturn 모델은 `saturn-local`이다(초안). 실제 모델과 버전은 설정의 `judge_id` 매핑과 `judge_manifest`에만 둔다. judge를 바꿔도 스키마와 코드를 유지하기 위해서다.
- 판단과 라벨마다 출처와 사용 제한을 기록한다. 외부 출력이 섞인 학습 데이터를 골라 뺄 수 있게 하기 위해서다.
- judge 모델은 버전을 고정하고, 별칭을 쓰면 응답의 `model`을 기록한다. 판단을 재현하기 위해서다.
- Saturn 모델의 학습과 승격은 [judge 학습](judge-training.md)에 있다.

### judge 시작 확인

1. `judges`가 판단 방식이 쓰는 judge를 고른다.
2. 외부 judge면 `GET /v1/models`와 실제 판단 1건으로 확인한다.
3. 로컬 Saturn 모델이면 모델 로드나 API 서버 응답으로 확인한다. 로컬 서버는 루프백 `http` 주소(설정 `judge.local.endpoint`)만 받고 기준 judge와 같은 본문을 쓴다(초안). 모델 파일 실행 방식은 [#43](https://github.com/woonyong-choi/saturn/issues/43)에서 정하고, 그 전에는 확인이 실패한다.
4. 확인에 실패하면 그 자리에서 숨김 입력으로 키를 요청하고, 받은 키로 다시 확인한다.
5. 확인에 성공하면 `secrets`가 키를 저장하고 실행을 계속한다.

- judge 확인에 실패하면 Saturn을 실행하지 않는다.
- 설치 검증 테스트 전용으로 시작 확인을 건너뛰는 설정(`judge.skip_check`, 키 이름 초안)을 두고, 도움말에는 보이지 않는다. judge 없이 설치만 검증하기 위해서다.
- 키 입력, 저장, 보호는 [judge 키 보호](judge-key-security.md)에 있다.

### judge 호출

1. `judges`가 요청을 `model`, `state`, `questions`로 구성한다.
2. `judges`가 `state`에서 비밀값, 절대 경로, 다른 대화 원문을 뺀다.
3. 요청이 64K를 넘거나 `state`와 가장 긴 질문의 합이 32K를 넘으면 나눠 보낸다.
4. `choice` 선택지가 255개를 넘으면 계층 선택으로 나눈다.
5. 답에 후보 밖 선택, NaN, 확률 누락이 있으면 그 판단을 `invalid`로 처리한다. 요청하지 않은 질문의 답, 형식이 다른 답, 0~1 밖의 확률, 합이 1에서 0.01 넘게 벗어난 분포도 `invalid`다.
6. 판단 사이에 채팅 revision이 바뀌었으면 그 판단을 `superseded`로 처리한다.

- judge 전송의 HTTPS, 허용 호스트, 인증 헤더 규칙은 [judge 키 보호](judge-key-security.md)에 있다.
- 기준 judge는 `POST https://api.typesafe.ai/v1/systemone`에 `{"model","state","questions"}`를 보낸다. `choice` 기준은 선택지별 `null`, `score` 기준은 질문에 단계 설명이 없어 `level 1`..`level N`이다(초안). 답은 `noul`이나 `probabilities`를 읽고, 0~1 밖이거나 없으면 `invalid`다.
- 크기 한도는 토큰을 셀 수 없어 본문 바이트로 잰다(초안, 바이트 수는 토큰 수 이상이다). 나눌 때는 질문 단위로 나누고 조각마다 `state`를 그대로 싣는다.
- 255개를 넘는 선택지는 254개씩 나누고 조각마다 `none of these`를 더해 한 요청에 묻고, 조각의 `none of these`가 아닌 확률로 조각 무게를 정해 원래 분포로 합친다(초안, [#68](https://github.com/woonyong-choi/saturn/issues/68) 전).
- 재시도 기본값은 보내기 전 실패 3회, 응답 대기 30초, 속도 제한(429, 529) 대기 2초(`retry-after`가 있으면 그 값)이고, 속도 제한은 3회까지 다시 보낸다(초안). 리다이렉트는 허용 주소로 한 번만 인증 헤더를 뺀 채 따른다.
- 연속 실패는 응답이 없는 실패(무응답, 보낸 뒤 시간 초과, 속도 제한 포기, 키 거절)만 센다. `invalid`와 `superseded`는 judge가 답한 것이라 세지 않는다.
- state의 비밀값은 가리고, 절대 경로는 끝 이름만 남겨 `[abs]/이름`으로 바꾼다(초안). 다른 대화 원문은 state를 만드는 `core`가 넣지 않는다.

### 판단 기록

1. `secrets`가 보낸 원문과 받은 원문에서 비밀값을 가린다.
2. 기록을 끈 채팅(`/record off`)이면 `store`가 판단 기록 저장을 생략한다.
3. 그 밖의 채팅이면 `store`가 모든 판단 호출의 원문, 질문별 답, 비용, 시간을 기록한다.

- 판단 기록은 일반 정리에서 제외하고 판단 기록 전용 정리로만 지운다. 판단 기록을 Saturn 모델 학습에 쓰기 때문이다.
- 채점하지 않은 판단 기록도 JSONL로 내보낼 수 있다.
- 판단 기록은 로컬에 쌓고, `consent.share_with_server = true`인 레코드만 서버로 올린다. 동의한 레코드만 보내고 오프라인에서도 판단하기 위해서다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| 시작 확인 실패 | 원인 한 줄을 보이고 실행하지 않고 끝낸다. |
| judge 무응답 | 그 입력을 대기로 보낸다. |
| 실행 중 일시 실패 | 질문별 대체 규칙을 적용하고 건너뛴 판단을 사유와 함께 기록한다. |
| 답의 후보 밖 선택, NaN, 확률 누락 | 판단을 `invalid`로 기록하고 질문별 대체 규칙을 적용한다. |
| 판단 중 revision 변경 | 판단을 `superseded`로 기록한다. |
| 연속 3회 호출 실패 | 새 입력 접수를 멈추고 연결 복구를 안내한다. |
| 요청 전송 전 실패 | 설정된 횟수 안에서 다시 보낸다. |
| 요청 전송 뒤 타임아웃 | `cost-unknown`으로 기록하고 다시 보내지 않는다. |
| 속도 제한 | 기다렸다가 다시 보낸다. |

- 보내기 전에 확정된 실패만 다시 보낸다. 같은 요청이 두 번 처리되는 일을 막기 위해서다.

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| judge 확인에 실패하면 Saturn을 실행하지 않는다. | judge 확인이 실패하는 환경에서 실행이 원인 한 줄과 함께 끝나는지 확인한다. |
| 실행 중 judge 호출이 실패하면 질문별 대체 규칙을 적용한다. | 질문마다 호출 실패와 `invalid` 답을 주고 표의 대체 규칙이 적용되는지 확인한다. |
| 판단 호출의 보낸 원문, 받은 원문, 질문별 답을 모두 기록한다. | 기록을 켠 채팅에서 호출마다 원문과 답이 저장되고 `/record off` 채팅에서는 생략되는지 확인한다. |
| `choice` 확신도는 `(N·pmax − 1)/(N − 1)`로 계산한다. | 균등 분포에서 0, 한 선택지 확률 1에서 1이 나오는지 확인한다. |
| 요청이 크기 한도를 넘으면 나눠 보낸다. | 64K 초과 요청과 255개 초과 선택지가 나뉘어 전송되는지 확인한다. |
| `keep_current` 기준값 0.8은 한국어 입력에서도 이어 가기를 가른다. | [#6](https://github.com/woonyong-choi/saturn/issues/6) 실험으로 한국어 평가 세트의 오분류율을 확인한다. |
| 영어 질문은 한국어와 인젝션 구간에서 판단 성능을 떨어뜨리지 않는다. | [#15](https://github.com/woonyong-choi/saturn/issues/15) 실험으로 구간별 성능 회귀를 확인한다. |
| 후보가 상위 N개보다 많으면 순위로 좁힌 뒤 묻는다. | 후보 150개의 `compact` 요청에 N개 몫의 질문만 있는지 확인한다. |
| `is_constraint`와 `replaces_<n>`이 한국어 입력에서 기준 정확도를 넘는다. | [#121](https://github.com/woonyong-choi/saturn/issues/121) |

## 단점

- 입력마다 외부 judge를 부르므로 호출 비용과 지연이 입력 수에 비례한다.
- judge 규격과 `judge_id` 매핑을 Saturn이 직접 정의하고 유지한다.
- 큰 변경마다 옛 선택지에서 새 선택지로 가는 대응표를 코드에 함께 관리한다.

## 대안

- judge 벤더의 형식과 이름을 그대로 쓰는 방식은 judge를 바꿀 때 스키마와 코드를 고쳐야 해 버렸다([판단 규격은 Saturn이 정하고 judge는 중립 이름과 출처로 기록한다](../decisions/2026-09-29-vendor-neutral-judge-spec.md)).
- Saturn 모델을 API 서버로 두고 서버가 판단 기록을 모으는 방식은 호출마다 상태가 서버로 가서 버렸다([판단 기록은 로컬에 쌓고 동의한 레코드만 서버로 올린다](../decisions/2026-09-29-local-first-judgment-collection.md)).

## 미해결 질문

- 질문을 상위 범주에서 하위 판단으로 내려가는 계층 트리로 나눌지, 단계마다 호출할지, 지금처럼 한 번에 고를지 ([#68](https://github.com/woonyong-choi/saturn/issues/68))
- 판단 방식 `collect`를 기준 judge가 결정하고 Saturn 모델은 기록만 하는 방식으로 할지, 반대로 할지 ([#40](https://github.com/woonyong-choi/saturn/issues/40))
- 통과한 후보가 없거나 provider가 비정상 종료했을 때 사용자에게 확인할지, 현재 에이전트를 유지할지 ([#39](https://github.com/woonyong-choi/saturn/issues/39))
- judge에 넘기는 state에 subagent 목록을 넣을지, 개수만 넣을지, 넣지 않을지 ([#63](https://github.com/woonyong-choi/saturn/issues/63))
- 관계 판단이 `conflicts`일 때 바로 멈출지, 사용자에게 확인할지, 대기로 둘지 ([#36](https://github.com/woonyong-choi/saturn/issues/36))
