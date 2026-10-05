# router

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [판단 규격은 Saturn이 정하고 router는 중립 이름과 출처로 기록한다](../decisions/2026-09-29-vendor-neutral-router-spec.md), [engine만 router를 부르고 자식 프로세스 환경에서 router 키를 지운다](../decisions/2026-09-29-engine-as-router-proxy.md), [판단 기록은 로컬에 쌓고 동의한 레코드만 서버로 올린다](../decisions/2026-09-29-local-first-judgment-collection.md), [router가 실패하면 재시도한 뒤 판단 없이 현재 모델로 진행한다](../decisions/2026-10-02-router-failure-keeps-going.md), [반대 지시는 끼워 넣고, 끼워 넣을 수 없으면 사용자에게 멈출지 묻는다](../decisions/2026-10-03-conflict-steers-then-asks.md) |

## 요약

router는 입력마다 뜻을 확률로 판단하는 작은 모델이다. 이어 가는 말인지, 새 작업인지, 어느 모델에 맡길지를 선택지별 확률로 답한다. `core`가 질문 세트, 기준값, 대체 규칙을 정하고, `engine`만 router를 부른다. router에는 외부 API인 기준 router와 로컬의 Saturn 모델이 있고, 판단 방식 설정으로 행동을 정할 router를 고른다.

## 동기

사용자는 에이전트가 일하는 중에도 입력을 보낸다. 그 입력이 하던 작업을 다듬는 말인지, 무관한 새 일인지는 문장의 뜻을 읽어야 가를 수 있다. 코드 규칙으로는 뜻을 가를 수 없고, 사용자에게 맡기면 입력마다 대상과 모델을 직접 골라야 한다. router는 뜻 판단만 확률로 맡고, 행동은 코드가 기준값과 비교해 정한다. 확률과 기준값을 나누면 확신이 낮은 판단으로 행동하지 않고 대체 규칙으로 갈 수 있다. 판단 기록은 로컬 Saturn 모델의 학습 재료가 된다.

## 예시

### 실행 중 입력이 이어 가는 말인지 판단할 때

1. 사용자가 에이전트 A가 코드를 고치는 중에 "에러 메시지도 한국어로 바꿔 줘"를 입력한다.
2. engine이 입력을 접수하고 `route`, `relation`, `send-opt` 질문을 요청 한 건에 묶어 router에 보낸다.
3. router가 `keep_current`에 0.91을 답하고, `relation_to_running`에서는 `refines`를 가장 높게 답한다.
4. `keep_current`가 0.8 이상이므로 `core`가 입력을 하던 에이전트 A로 보내기로 정한다.
5. engine이 진행 중인 턴에 입력을 끼워 넣고, 보낸 원문과 받은 원문을 판단 기록에 저장한다.

### 확신이 낮은 판단을 받을 때

1. 사용자가 실행 중에 "이거 말고 저쪽 먼저"를 입력한다.
2. router가 `relation_to_running`의 네 선택지에 비슷한 확률을 답해 확신도가 0.6 미만이 된다.
3. `core`는 관계 판단으로 행동하지 않고 대체 규칙대로 입력을 대기로 보낸다.
4. 사용자는 입력이 대기열에 들어간 것을 보고 직접 보내거나 취소한다.

### router 답이 형식에 맞지 않을 때

1. router가 `target_model` 질문에 허용 후보에 없는 모델을 답한다.
2. engine이 그 판단을 `invalid`로 기록한다.
3. `core`가 `target_model`의 대체 규칙대로 사용자가 고정한 모델, 기본 모델, 현재 모델 순으로 쓴다.
4. 판단 기록에는 받은 원문과 `invalid` 표시가 함께 남는다.

## 상세 설계

### 코드와 router의 경계

- 기계적으로 판단할 수 있는 것은 코드가, 뜻을 이해해야 하는 것은 router가 정한다. router를 뜻 판단에만 쓰기 위해서다.
- 제어 명령이 아닌 입력마다 router를 한 번 부르고 필요한 질문을 요청 한 건에 묶는다. 입력당 호출 수를 한 번으로 줄이기 위해서다.
- 사용자가 모델을 고정한 입력도 router를 부른다. 고정은 `target_model` 질문만 뺀다. `/model` 뒤 모든 입력이 대기해 병렬 작업이 막히지 않게 하기 위해서다.
- 후보를 고르는 질문(`compact`, `file-rank`)은 후보 전체를 묻는다. 순위로 미리 자르면 router가 남길 항목을 놓치기 때문이다. 요청이 크기 한도를 넘으면 질문 단위로 나눠 보낸다. 순위는 router가 답하지 못한 항목의 순서와 같은 확률일 때의 순서에만 쓰며, 규칙은 [맥락 고르기](context-selection.md)에 있다.
- 뜻 판단이 새로 필요하면 입력마다 보내는 요청에 질문을 더하는 방식을 먼저 쓴다. router는 state를 한 번 읽고 모든 질문에 답하므로 호출 수가 늘지 않기 때문이다. 개수 세기, 날짜 비교, 앞 말을 가리키는 간접 지시처럼 router가 약한 판단은 코드로 하거나 두 원문을 나란히 놓는 질문으로 바꾼다.
- router는 앞 입력의 판단 결과와 같은 채팅의 작업 맥락을 state에 넣은 요청으로 판단한다. 맥락의 구성과 크기 처리는 [판단 요청 맥락](#판단-요청-맥락)에, 판단 차례와 적용 직전 revision 비교는 [입력 처리](input-handling.md)에 있다.
- `keep_current`를 `is_actionable`보다 먼저 읽는다. 이어 가는 입력이 파일 탐색으로 빠지는 일을 막기 위해서다.
- `keep_current`가 이어 가는 입력으로 답하면 하던 에이전트로 보낸다. 새 작업이면 `is_actionable`, 모델 선택 순서로 판정한다.
- router가 무관한 작업으로 판단하면 같은 채팅 안에 보조 에이전트를 시작한다.
- 모든 router는 `core`의 `RouterClient` trait 구현으로만 붙는다. `engine`의 `routers`는 외부 API용 `RemoteRouter`와 로컬 모델용 `LocalRouter`를 둔다.
- router는 `engine`만 부른다. router 키가 자식 프로세스나 다른 호스트로 새는 일을 막기 위해서다.
- router 호출 한도와 지출 상한은 두지 않고 호출 수와 예상 비용만 보인다. 계정 상한 정보가 없어 상한을 정할 근거가 없기 때문이다.

### 질문 세트와 버전

질문 세트는 router에 한 번에 묻는 질문 묶음과 그 버전이다. 판단 기록은 질문을 `ID@버전`으로 가리킨다.

- 질문 문장과 선택지는 `routers` 한 곳에서 버전으로 관리하고 릴리스로만 바꾼다. 판단 기록의 `ID@버전`으로 같은 질문을 다시 찾기 위해서다.
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

행동 조건은 router 답으로 행동하는 기준이다. 대체 규칙은 판단이 없거나, 확신도가 기준보다 낮거나, 답이 `invalid`일 때 따르는 동작이다. `keep_current` 0.8은 [한국어 이어 가기 실험](../experiments/continuation-judgment-korean/report.md)에서 측정했고 기본값을 유지한다. `resume_held` 0.85는 [#9](https://github.com/woonyong-choi/saturn/issues/9) 실험으로 확인한다.

| 질문 세트 | 질문 | 형식 | 행동 조건 | 대체 규칙 |
|---|---|---|---|---|
| `route` | `keep_current` | `noul` | 0.8 이상이면 현재 에이전트 유지 | 판단이 없거나 0.3~0.8이면 현재 에이전트 유지 |
| `route` | `is_actionable` | `noul` | 0.7 이상이면 명확 | 판단이 없으면 명확으로 간주하고 탐색 생략 |
| `route` | `target_model` | `choice` | 허용 후보 중에서만 선택 | 확신도 0.6 미만이면 사용자 고정 모델, 기본 모델, 현재 모델 순. 매뉴얼 모드에서는 묻지 않는다 |
| `route` | `difficulty` | `score` | 3단계로 답 | 확신도 0.6 미만이면 미사용 |
| `route` | `skills` | `choice` | 선택지에 `none` 포함 | 확신도 0.6 미만이면 힌트 생략 |
| `route` | `resume_held` | `noul` | 0.85 이상에서만 보류 작업 재개 | 0.85 미만이면 무시 횟수 1 증가. 판단이 없으면 무시 횟수를 올리지 않는다 |
| `route` | `is_constraint` | `noul` | `constraint.auto_apply`가 켜져 있을 때만 `is_constraint` 0.8 이상이면 제약으로 자동 등록, `constraint_ask` 0.7 이상 0.8 미만이면 사용자에게 묻기 | 판단이 없으면 미등록 |
| `constraint` | `constraint_change` | `choice` | 유효 제약 최대 10개와 함께 질문. 선택지는 `none`과 제약마다 `release`, `once`, `scoped`. `1 − P(none)`이 `constraint_release` 0.8 이상이면 해제나 예외 적용, 종류의 확률이 0.8 미만이면 종류를 사용자에게 묻기 | 판단이 없으면 해제도 예외도 하지 않는다 |
| `constraint` | `line_<k>_is_constraint` | `noul` | 자동 적용이 켜져 있고 200자를 넘는 입력의 문장마다 질문. 0.7 이상인 문장을 제약으로 등록 | 판단이 없으면 입력 전체 원문을 한 건으로 등록 |
| `relation` | `relation_to_running` | `choice` | `refines`, `continues`, `independent`, `conflicts` 중 선택 | 확신도 0.6 미만이면 대기 |
| `send-opt` | `steer_or_spawn` | `choice` | `target_model`과 함께 질문 | 확신도 0.6 미만이면 현재 에이전트에 대기 뒤 전송 |
| `file-rank` | `file_<n>_relevant` | `noul` | 후보 파일 전체와 `answer_present`를 함께 질문. 0.7 이상은 존재, 0.35 미만은 없음 | 판단이 없으면 후보 순위 그대로 |
| `context-select` | `pick` | `choice` | 게이트 `noul` 3개 평균이 0.3 미만이면 없음. 2차로 `fits_<n>` 질문 | 판단이 없으면 힌트 생략 |
| `compact` | `call_<id>_keep` | `noul` | 후보 호출 전체에 `result_<id>_keep`과 함께 질문. 기준값 없음. 항목의 확률은 두 답 중 큰 값이고, 확률이 높은 순, 같은 확률이면 후보 순위 순으로 예산까지 채움 | 답이 없는 항목은 후보 순위 순으로 답이 있는 항목 뒤에 두고, 판단이 전부 없으면 후보 순위 순서로 예산까지 채움 |
| `doc-filter` | `injection` | `noul` | 조각마다 `relevant`, `evidence`, `contradiction`과 함께 질문. 0.7 이상이면 제외 | 판단이 없으면 문서 조각 생략 |
| `loop` | `is_progressing` | `noul` | 0.2 미만이면 루프 | 판단이 없으면 멈춤과 사용자 알림 |
| `feedback` | `wrong_doc` | `noul` | `misunderstood_intent`, `code_error`와 함께 질문. 0.7 이상인 원인만 사용 | 판단이 없으면 원문 그대로 전달 |

- `keep_current`가 0.3 미만이면 새 작업으로 본다. 0.3 이상 0.8 미만은 판단 없음과 같다.
- [한국어 이어 가기 실험](../experiments/continuation-judgment-korean/report.md)에서 직전 작업 정보를 추가하면 0.80 이진 분류 정확도가 587턴에서 +51.4%p [41.2, 57.4] 올랐다. 현재 state의 재현율은 10.1%, 정보를 추가한 state는 69.7%였다. 사전 선택 규칙의 탐색 후보는 두 조건 모두 0.50이지만 새 작업 오접합률이 70.8%·52.8%여서 운영 채택은 보류한다. 현재 idle 동작은 0.3 이상을 대체 규칙으로도 유지하므로 상단 기준값 변경만으로 처리 결과가 달라지지 않는다. 질문 곡선과 현재 처리 규칙 재생은 구분하며 기본값은 유지한다.
- 실행 중이면 처리 방식을 `relation_to_running`과 `steer_or_spawn`으로 정한다. `refines`, `continues`면 `steer_or_spawn`의 `steer`, `queue`, `spawn`을 끼워 넣기, 대기, 새 작업으로 옮기고, `independent`면 새 작업이다. `conflicts`는 `steer_or_spawn` 답과 관계없이 끼워 넣기로 둔다. 모델이 다음 단계에서 입력을 읽고 방향을 바꾸게 하고, 끼워 넣을 수 없을 때의 확인은 [충돌 입력](input-handling.md#충돌-입력)이 정한다. 확신도가 0.6 미만이면 충돌로 보지 않고 대기로 둔다. 실행 중이 아니면 `keep_current`로 현재 에이전트 대기와 새 작업을 가른다.
- `target_model`의 선택지는 허용 후보와 `other`이고, `other`를 고르면 대체 규칙을 따른다. 후보가 없으면 묻지 않는다. 허용 후보는 provider가 알려 주는 모델 목록으로, `/model`이 보이는 목록과 같다([모델 고르기](providers-and-sessions.md#모델-고르기)). 채팅에 고정한 모델이 있으면 `target_model`만 묻지 않고 그 모델을 쓰며, 관계 판단 질문은 똑같이 묻는다. 모델 선택 방식(`model.mode`)의 기본은 매뉴얼이고, 오토는 실험 옵션이라 사용자가 켜야 `target_model`을 묻는다. 매뉴얼이면 `target_model`만 묻지 않고 기본 모델을 쓴다. `other`나 확신도 0.6 미만, 후보 밖 값, 적용 직전 모델 목록에 없는 선택, router 장애는 사용자 선호(품질을 확정한 후보), 기본 모델 순으로 받는다. 기본 모델이 없으면 현재 모델이다([기본 모델과 선택 방식](providers-and-sessions.md#기본-모델과-선택-방식)). engine은 provider 연결을 만들 때 받아 둔 모델 목록(`/model`이 보이는 목록)을 Claude, Codex 순으로 후보에 넣고, 선택지는 `<provider>/<model>` 글이다. 목록을 아직 못 받은 provider(연결 전이거나 목록 요청이 실패한 경우)는 후보에서 빠지고, 받은 목록이 없으면 묻지 않아 현재 모델(첫 입력은 기본 provider의 기본값)을 쓴다. 후보는 관계 판단과 같은 요청에서 묻지만, 고른 모델은 새 작업으로 판단된 입력에만 적용한다. 이어 가는 입력(대기)과 끼워 넣기는 현재 모델을 유지한다. 적용한 모델은 입력 기록(`inputs.pinned_model`)에 남겨 다시 시작해도 같은 모델로 보낸다. 그 새 작업이 메인이고 모델이 열린 메인 session의 모델과 다르면 새 메인 session 규칙을 따르고, 보조 에이전트면 그 모델로 session을 연다([모델 고르기](providers-and-sessions.md#모델-고르기))(초안).
- `difficulty`와 `skills`는 답을 쓰는 곳이 생기기 전까지 묻지 않고 대체 규칙(미사용, 힌트 생략)으로 둔다. 쓰지 않는 질문으로 판단 비용을 늘리지 않기 위해서다.
- 질문 세트는 `route@1.0`, `relation@1.0`, `send-opt@1.0`에서 시작한다. `route@1.1`은 `is_constraint`를 더한다. `constraint@1.0`은 `constraint_change`와 `line_<k>_is_constraint`로 시작한다. 기존 질문의 뜻은 바뀌지 않기 때문이다. `constraint_change`는 `constraint.auto_apply`가 켜져 있고 등록 대상이 아닌 입력에서만 별도 요청으로 묻는다.
- 행동 조건을 채운 선택지가 없으면(확신도 미만, `invalid`, 판단 없음, 허용 후보 없음) 질문마다 위 표의 대체 규칙으로 가고, 입력 처리 판단(`route`, `relation`, `send-opt`)은 사용자에게 따로 묻지 않는다. router 장애와 낮은 확신이 입력을 멈추지 않게 하기 위해서다. 제약 판단(`is_constraint`, `constraint_change`)은 낮은 확신일 때 대체 규칙으로 가지 않고 입력 처리를 멈추지 않는 확인 창으로 사용자에게 묻는다. 단 권한 모드가 `full`이면 묻지 않고 기록 줄로 대신한다([제약](constraints.md#사용자에게-묻기))([#103](https://github.com/woonyong-choi/saturn/issues/103), [#39](https://github.com/woonyong-choi/saturn/issues/39)).
- 기준값은 설정 층에 둔다. 릴리스 없이 사용자 층과 폴더 층에서 기준값을 조정하기 위해서다.
- 기준값은 판단 기록, 피드백, 취소, 실패로 자동 조정하지 않는다. 바뀌는 길은 설정 변경과 명시적으로 적용하는 새 정책뿐이다([정책 고정](#정책-고정)). 기록으로 기준값을 계산하는 규칙은 [router 학습](router-training.md)에 있고 실행 경로에는 연결하지 않았다.

### 판단 방식

판단 방식은 행동을 정하는 router를 고르는 설정이다.

| 판단 방식 | 행동을 정하는 router |
|---|---|
| `jev` | 기준 router |
| `saturn` | Saturn 모델 |

- 판단 기록은 판단 방식과 관계없이 전부 남긴다. 어느 방식에서도 학습과 비교용 기록을 쌓기 위해서다.
- `saturn` 방식에서 확신도가 기준보다 낮으면 행동하지 않고 질문별 대체 규칙으로 간다. 확신 없는 판단으로 행동하는 일을 막기 위해서다.
- `saturn` 방식에서는 `noul` 답도 확신도 `|2p − 1|`가 `choice` 확신도 기준(0.6)보다 낮으면 판단 없음으로 본다. 같은 이유다.
- 판단 방식과 router 주소는 폴더 층에서 바꿀 수 없다. 사용자 전용 항목이기 때문이다.
- 기록에는 중립 이름 `router_id`를 쓴다. 기준 router는 `jev`, Saturn 모델은 `saturn-local`이다(초안). 실제 모델과 버전은 설정의 `router_id` 매핑과 `router_manifest`에만 둔다. router를 바꿔도 스키마와 코드를 유지하기 위해서다.
- 판단과 라벨마다 출처와 사용 제한을 기록한다. 외부 출력이 섞인 학습 데이터를 골라 뺄 수 있게 하기 위해서다.
- router 모델은 버전을 고정하고, 별칭을 쓰면 응답의 `model`을 기록한다. 판단을 재현하기 위해서다.
- Saturn 모델의 학습과 승격은 [router 학습](router-training.md)에 있다.

### 정책 고정

판단 정책은 질문 세트 버전, 질문별 기준값, router 종류와 모델이다. 입력 하나는 접수 때 정해진 정책으로만 판단한다.

- 기준값은 입력의 설정 번호가 가리키는 스냅샷에서 읽는다. 설정 파일이 바뀌어도 이미 접수한 입력은 접수 때 번호의 기준값을 쓰고, 이후 접수하는 입력부터 새 번호를 쓴다. 한 입력의 판단에 옛 값과 새 값이 섞이지 않게 하기 위해서다.
- router 종류와 모델, 모델 평가 근거 목록 버전은 engine이 시작할 때 정해 끝까지 쓴다(목록은 배포에 묶여 있고 새 버전이나 되돌리기는 재시작으로 들어온다). 판단 기록의 `router_version`과 설정 번호로 그 판단의 정책을 다시 찾을 수 있다. 설정 파일의 `router.mode`, `router.model`을 중간에 바꿔도 다음 engine 시작 전에는 적용하지 않는다.
- 새 정책은 설정을 고치거나 engine을 다시 시작해 들어온다. 설정은 검사에 실패하면 이전 번호를 유지하고, 같은 내용으로 되돌리면 같은 번호를 다시 쓰므로 옛 정책으로 돌아가는 일(rollback)은 옛 설정으로 되돌리는 일이다. 재시작해도 접수한 입력의 설정 번호는 기록 저장소에 남아 있어 그대로다.
- 사용자 피드백, 취소, 뒤집기, router 실패는 판단 기록과 결과 신호로만 남는다. 기준값, 모델, 질문, 사례집, 스킬을 실행 중 바꾸지 않고 판단 모델도 실행 중 학습하거나 승격하지 않는다. 자동으로 정책을 바꾸는 계획([#337](https://github.com/woonyong-choi/saturn/issues/337))은 폐기했다.
- 정책 지문은 기준값 표, router 종류·모델, [모델 평가 근거 목록](model-evidence.md)의 버전의 SHA-256이고 judgment 디버그 로그에 남는다. 두 정책이 같은지 확인하는 용도다.
- 이 고정은 Saturn 쪽 정책 고정이다. 외부 판단 서비스가 같은 모델 이름 뒤의 가중치를 바꾸는지는 Saturn이 알 수 없다. 서비스가 버전 고정을 지원하지 않거나 버전을 공개하지 않으면 동일 모델 재현을 보장한다고 표시하지 않는다. 응답의 `model`이 요청과 다르면 기록하고, 분포 변화는 [#544](https://github.com/woonyong-choi/saturn/issues/544)의 재검증 사유일 뿐 정책을 자동으로 고칠 이유가 아니다.

### router 시작 확인

1. `routers`가 판단 방식이 쓰는 router를 고른다.
2. 외부 router면 `GET /v1/models`와 실제 판단 1건으로 확인한다.
3. 로컬 Saturn 모델이면 모델 로드나 API 서버 응답으로 확인한다. 로컬 서버는 루프백 `http` 주소(설정 `router.local.endpoint`)만 받고 기준 router와 같은 본문을 쓴다(초안). 모델 파일 실행 방식은 [#43](https://github.com/woonyong-choi/saturn/issues/43)에서 정하고, 그 전에는 확인이 실패한다.
4. 확인에 실패하면 `SATURN_KEY` 환경 변수, 비밀번호 관리자 명령(`router.key.command`) 순서로 키를 받아 다시 확인하고, 그래도 실패하면 화면이 있을 때 TUI가 숨김 입력으로 받아 보낸 키로 다시 확인한다. 화면이 없으면 묻지 않고 키 입력 방법을 안내하고 끝낸다.
5. 확인에 성공하면 `secrets`가 키를 저장하고 실행을 계속한다.

- router를 확인하기 전에는 router 키 제출과 채팅 붙기 밖의 요청을 받지 않는다. 확인하지 않은 router로 입력을 처리하지 않기 위해서다. 요청 규칙은 [engine 수명](engine-lifecycle.md)에 있다.
- 등록된 키가 확인되면 화면이 있어도 키를 묻지 않는다. 키가 없거나 확인에 실패했을 때만 묻는다.
- 판단 방식에 맞는 router를 만들 수 없으면 Saturn을 실행하지 않는다.
- 설치 검증 테스트 전용으로 시작 확인을 건너뛰는 설정(`router.skip_check`)을 두고, 도움말에는 보이지 않는다. router 없이 설치만 검증하기 위해서다.
- 키 입력, 저장, 보호는 [router 키 보호](router-key-security.md)에 있다.

### router 호출

1. `routers`가 요청을 `model`, `state`, `questions`로 구성한다.
2. `routers`가 `state`에서 비밀값, 절대 경로, 다른 대화 원문을 뺀다.
3. 요청이 64K를 넘거나 `state`와 가장 긴 질문의 합이 32K를 넘으면 나눠 보낸다.
4. `choice` 선택지가 255개를 넘으면 계층 선택으로 나눈다.
5. 답에 후보 밖 선택, NaN, 확률 누락이 있으면 그 판단을 `invalid`로 처리한다. 요청하지 않은 질문의 답, 형식이 다른 답, 0~1 밖의 확률, 합이 1에서 0.01 넘게 벗어난 분포(소수 둘째 자리 반올림으로 생기는 0.99와 1.01은 받는다)도 `invalid`다.
6. 판단 사이에 채팅 revision이 바뀌었으면 그 판단을 `superseded`로 처리한다.

- 입력 처리 판단의 `state`는 채팅이 실행 중인지, 앞 입력의 처리 방식, 같은 채팅의 작업 맥락, 사용자 원문으로 만든다([판단 요청 맥락](#판단-요청-맥락)). 관계 판단 없이 대기하는 입력은 router를 부르지 않는다. 바로 보내기는 router를 부르지 않는다([입력 처리](input-handling.md#대기와-취소)).
- 판단 기록은 `queue`가 판단을 적용한 뒤에 쓴다. 적용 직전 revision이 달라 버린 판단은 `superseded`로 쓴다.
- router 전송의 HTTPS, 허용 호스트, 인증 헤더 규칙은 [router 키 보호](router-key-security.md)에 있다.
- 기준 router는 `POST https://api.typesafe.ai/v1/systemone`에 `{"model","state","questions"}`를 보낸다. `choice` 기준은 선택지별 `null`, `score` 기준은 질문에 단계 설명이 없어 `level 1`..`level N`이다(초안). 답은 `noul`이나 `probabilities`를 읽고, 0~1 밖이거나 없으면 `invalid`다.
- 나눈 요청은 동시에 최대 8개(초안)까지 병렬로 보내고, 먼저 끝난 요청을 기다리지 않고 남은 요청을 이어 보낸다. 질문 40개와 약 7KB state 요청의 실측은 1개 0.22초, 4개 동시 0.26초, 8개 동시 0.27초, 4개 차례로 0.97초였다([#179](https://github.com/woonyong-choi/saturn/issues/179) 실측). 8개보다 많은 동시 요청의 속도 제한과 64KB에 가까운 요청의 지연은 재지 않았다.
- 조각을 병렬로 보내는 일과 조각별 실패 처리, 429 때 동시 수 감소는 구현 전이다. 지금 코드는 조각을 차례로 보내고 한 조각이 실패하면 거기서 멈춰 그 실패를 결과로 쓴다(`saturn-terminal/engine/src/routers/remote.rs`의 `exchange`).
- 조각마다 응답과 실패를 따로 처리한다. 실패한 조각의 항목은 순위 대체 규칙을 적용하고, 다른 조각의 답은 그대로 쓴다.
- 속도 제한(429, 529)을 받으면 다른 실패와 같이 5초 뒤 다시 보내고, 남은 요청은 동시 수를 줄여 보낸다. 줄이는 폭은 정하지 않았다(초안).
- 크기 한도는 토큰을 셀 수 없어 본문 바이트로 잰다(초안, 바이트 수는 토큰 수 이상이다). 나눌 때는 질문 단위로 나누고 조각마다 `state`를 그대로 싣는다. `core`의 `split_request`가 나눈 요청 목록을 만들고 `engine`이 보낸다. `state`와 질문 하나만으로 한도를 넘으면 나누지 못해 오류다.
- 255개를 넘는 선택지는 254개씩 나누고 조각마다 `none of these`를 더해 한 요청에 묻고, 조각의 `none of these`가 아닌 확률로 조각 무게를 정해 원래 분포로 합친다(초안, [#68](https://github.com/woonyong-choi/saturn/issues/68) 전).
- 응답 대기는 시도마다 5초(초안, 첫 시도 포함)이고, 재시도 간격과 횟수, 전체 마감은 [router 실패](#router-실패)에 있다. 리다이렉트는 허용 주소로 한 번만 인증 헤더를 뺀 채 따른다.
- 연속 실패는 응답이 없는 실패(무응답, 보낸 뒤 시간 초과, 속도 제한 포기, 키 거절)만 센다. `invalid`와 `superseded`는 router가 답한 것이라 세지 않는다.
- state의 비밀값은 가리고, 절대 경로는 끝 이름만 남겨 `[abs]/이름`으로 바꾼다(초안). 다른 대화 원문은 state를 만드는 `core`가 넣지 않는다.

### 판단 요청 맥락

이어 가기, 관계, 보류 재개 판단은 새 입력만으로는 비교 대상이 없다. 같은 후속 문장이 어느 목표에 붙는지는 비교할 작업의 목표를 봐야 가를 수 있어서, 요청을 만들 때 같은 채팅의 맥락을 `state`에 넣는다([#459](https://github.com/woonyong-choi/saturn/issues/459)).

| 칸 | 내용 |
|---|---|
| `chat`, `previous input handled as` | 채팅이 실행 중인지와 앞 입력의 처리 방식 |
| `current tasks` | 보류가 아닌 메인 작업마다 작업 번호, 진행 상태(`starting`, `running`, `idle`), 최초 목표, 최신 수정 |
| `previous user input` | 이 입력보다 먼저 접수한 같은 채팅의 가장 가까운 사용자 입력 |
| `held tasks` | 보류 작업마다 작업 번호와 최초 목표 |
| `user input` | 판단할 새 입력의 원문 |

- 최초 목표는 메인 작업을 시작한 입력이다. 최신 수정은 그 작업에 끼워 넣었거나 이어서 보낸 가장 나중의 사용자 입력이다. 재개 확인 입력처럼 사용자가 보내지 않은 입력은 목표나 수정이 되지 않는다.
- 맥락은 요청을 만드는 순간의 채팅 상태에서 만들고 다른 채팅의 입력과 작업은 넣지 않는다. 요청을 만든 revision은 기존대로 기록하고, 적용 직전 revision이 다르면 새 상태로 맥락을 다시 만들어 한 번 다시 판단한다.
- 사용자가 쓴 글(목표, 수정, 직전 입력, 보류 목표)은 `(user text)` 표시를 붙여 JSON 문자열로 감싼다. 글 안의 줄바꿈과 따옴표가 `held tasks:` 같은 다른 칸을 흉내 내지 못하게 하고, 사용자가 쓴 글이 router 지시가 아니라 자료임을 드러내기 위해서다. 새 입력은 맨 끝에 원문 그대로 둔다.
- 비밀값과 절대 경로는 자르기 전에 가린다. 자른 뒤에 가리면 잘린 비밀값 일부가 남을 수 있기 때문이다.
- 맥락 전체는 12KB(초안)를 넘지 않는다. 최초 목표와 최신 수정은 칸마다 3KB(초안), 직전 입력은 1KB, 보류 작업의 목표는 512바이트까지 담고, 넘으면 끝에 `...[truncated N bytes]`를 적는다. 사용자 입력과 질문은 이 한도에 들지 않고 줄이지 않는다. `state`와 가장 긴 질문의 합 32KB 한도에서 입력과 질문 몫을 남기기 위한 값이다.
- 한도가 모자라면 직전 입력의 끝을 자르고, 그래도 모자라면 오래된 보류 작업부터 빼고 `held tasks omitted: N`을 적는다. 오래된 진행 내역부터 줄이고 줄였음을 드러내 잘린 항목을 완전한 기록처럼 보이지 않게 하기 위해서다.
- 활성 작업의 최초 목표나 최신 수정이 잘렸거나 알 수 없거나 필수 칸만으로 한도를 넘으면 `context: incomplete`로 본다. 불완전한 맥락으로는 router를 부르지 않고 입력을 대기에 둬 사용자가 확인하게 한다. 일부만 보고 이어 붙이거나 새 작업으로 가르는 일을 막기 위해서다. 보류 작업의 목표는 알 수 없거나 잘려도 불완전로 보지 않는다. 보류가 여러 개이고 구분되지 않으면 기존 확인 흐름으로 대상을 고른다.
- 명시적인 새 작업, 취소, 대상 지정이 이전 주제와의 유사성보다 우선한다는 규칙은 질문 문장이 아니라 처리 흐름이 지킨다. 대상이 정해진 입력은 router를 부르지 않는다. 질문 문장은 이번 변경에서 바꾸지 않았다.
- 접수한 입력과 작업 목표는 engine 메모리에만 있다. engine을 다시 켜면 되살린 입력부터 기억하므로 재시작 직후 직전 입력은 앞서 끝난 입력을 모르고, 보류에서 복구한 작업은 보류 입력을 목표로 쓴다.

### 판단 기록

1. `secrets`가 보낸 원문과 받은 원문에서 비밀값을 가린다.
2. 기록을 끈 채팅(`/record off`)이면 `store`가 판단 기록 저장을 생략한다.
3. 그 밖의 채팅이면 `store`가 모든 판단 호출의 원문, 질문별 답, 비용, 시간을 기록한다.

- 판단 기록은 그때의 기준값과 물은 확률 q를 함께 남기고, 결과 신호와 물은 답은 생기면 채운다. 칸의 값과 NULL의 뜻은 [기록 저장과 보존](records.md#판단-기록)에 있다. 느린 조정이 모든 판단 기록으로 중심값을 다시 계산하기 위해서다([router 학습](router-training.md)).
- 판단 기록은 일반 정리에서 제외하고 판단 기록 전용 정리로만 지운다. 판단 기록을 Saturn 모델 학습에 쓰기 때문이다.
- 채점하지 않은 판단 기록도 JSONL로 내보낼 수 있다.
- 판단 기록은 로컬에 쌓고, `consent.share_with_server = true`인 레코드만 서버로 올린다. 동의한 레코드만 보내고 오프라인에서도 판단하기 위해서다.

### 모델 판단 그림자

모델 선택을 실제로 쓰기 전에 router가 어떻게 답하는지 재 보는 실험 옵션이다. 설정 `router.shadow.model_selection`(기본 꺼짐, 사용자 층 전용)을 켜면 입력 처리 요청에 후보별 질문을 묶는다. 켜도 실제 모델 선택은 바뀌지 않는다.

- 질문 세트는 `model-shadow@1.0`이고 후보마다 `sufficient:<provider>/<model>` 질문 하나다. 각 질문은 그 모델 하나만 보고 이 입력을 충분히 처리하는지 확률로 묻고, 다른 질문의 답을 전제하지 않는다. 모델과 추론 깊이를 한 질문에 섞지 않아 서로 모순되는 조합이 답으로 나오지 않는다. 추론 깊이 조합은 provider가 알려 주는 실제 지원 조합이 생기면 별도 질문으로 다룬다.
- 후보는 `target_model` 후보와 같은 모델 목록이다. 모델을 고정한 입력과 후보가 없는 입력은 묻지 않는다. 요청이 크기 한도로 나뉠 만큼 크면 묻지 않는다. 한 조각의 실패가 실제 판단을 막지 않게 하기 위해서다.
- 같은 요청에 묶으므로 입력 원문은 한 번만 간다. 요청과 답은 실제 판단이 읽기 전에 그림자 질문을 떼어 내고, 실제 판단은 그림자를 끈 요청과 같은 질문과 답만 읽는다. 그림자 답이 빠지거나 범위 밖이어도 실제 판단은 무효가 되지 않고 그림자 상태만 `invalid`다. 호출이 실패하면 실제 판단과 같은 대체 규칙을 따르고 그림자 상태는 `no-answer`다.
- 판단 기록과 함께 `model_shadows` 행을 남긴다([기록 저장과 보존](records.md#판단-기록)). 입력, 설정 번호, 채팅 revision, 정책 지문, 질문 세트 버전, 후보 지문, 후보별 확률과 [모델 평가 근거 목록](model-evidence.md) 기준 품질 확정 여부, 실제로 적용한 모델을 연결한다. 성과(실행, 사용량, 결과 신호)는 같은 입력 번호와 판단 번호로 잇는다. 사용량은 판단 기록의 토큰과 그림자 질문이 더한 바이트로 비교한다.
- 판단이 어긋나(`Superseded`) 적용하지 않았으면 상태는 `superseded`이고 적용한 모델은 비운다. 늦게 도착한 답이나 revision 변경은 실제 선택을 바꾸지 않는다.
- 그림자 선택이 실제 선택과 일치하는지만으로 효과를 판정하지 않는다. 효과는 모델 선택 순효과 실험([#542](https://github.com/woonyong-choi/saturn/issues/542))이 실제 성과로 판정한다.

### router 실패

router 호출이 실패하면 `engine`이 다시 보내고, 그래도 실패하면 판단 없이 현재 모델로 진행한다.

1. 호출이 실패하면 5초 뒤 한 번 다시 보낸다.
2. 다시 실패하면 5초 뒤 한 번 더 보낸다.
3. 첫 실패부터 10초가 지나도 실패하면 진행 중인 시도도 끊고 로그에 `판단 모델 실패로 모델 선택을 건너뜁니다`를 남기고 현재 모델로 진행한다.

- 다시 보내는 실패는 보내기 전에 확정된 실패, 응답 없음, 보낸 뒤 시간 초과, 속도 제한(429, 529)이다. 키 거절과 `invalid`는 router가 답했거나 다시 보내도 같으므로 다시 보내지 않는다.
- 보낸 뒤 시간 초과는 이미 처리됐을 수 있다. router 판단은 부작용이 없는 조회라 다시 보내고, 처리됐을 수 있는 호출의 비용은 `cost-unknown`으로 기록한다. provider 입력의 재전송 규칙([입력 처리](input-handling.md))과 다른 이유다.
- 간격은 응답의 `retry-after`와 무관하게 5초다. 속도 제한 규칙을 다른 실패와 하나로 맞추기 위해서다. 시도마다 응답을 5초까지 기다리되 첫 실패 시각부터 10초가 전체 마감이다. 마감까지 남은 시간이 5초보다 짧으면 그만큼만 기다리고, 마감에 걸린 시도는 끊는다. 응답 대기를 끊은 호출은 `cost-unknown`으로 센다. 응답이 아예 없으면 첫 시도 5초, 재시도 5초 대기 뒤 마지막 시도가 마감까지 5초를 기다려 첫 시도부터 최대 15초 뒤에 포기한다.
- 입력 처리 판단(`route`, `relation`, `send-opt`)이 실패하면 모델 선택과 끼워 넣기·대기·새 작업 판단을 건너뛰고 현재 에이전트와 현재 모델로 보낸다. 실행 중이면 현재 에이전트의 턴에 끼워 넣고, 아니면 바로 보낸다. 입력을 대기로 보내지 않는다. router 장애가 입력을 멈추지 않게 하기 위해서다. 채팅에 현재 모델이 없는 첫 입력은 현재 모델 대신 기본 모델(`model.default`)로 보내고, 기본 모델이 없으면 provider 기본값이다([기본 모델과 선택 방식](providers-and-sessions.md#기본-모델과-선택-방식)).
- `compact` 판단은 실험 옵션 `context.select.packet = jev`일 때만 부른다([설정](settings.md)). 이 옵션이 꺼져 있으면 아래 규칙이 닿지 않는다.
- 패킷의 `compact` 판단이 실패했을 때 router가 시작한 전환(router가 고른 모델이 현재와 달라 시작한 전환)은 건너뛰고 현재 모델로 진행한다. 순위 순서로 채운 패킷은 패킷 없음과 정답률이 같아(20.6%와 20.0%) 그 전환의 이득이 없기 때문이다([재측정 결과](../experiments/handoff-packet-quality-v2/report.md)).
- 사용자가 모델을 고정했거나 맥락 크기 규칙이 시작한 전환은 `compact` 판단이 실패해도 전환한다. 경쟁 구역은 순위 순서로 채우고 로그에 `판단 모델 실패로 기록 선택을 건너뜁니다`를 남긴다.
- 응답이 없는 실패가 연속 3회면 상태판에 `판단 모델 연결 끊김`을 보이고, 새 입력 접수는 계속하며 현재 모델로 처리한다. 성공 한 번이면 지운다. router 연결이 끊겨도 사용자가 작업을 이어 갈 수 있게 하기 위해서다.
- 로컬 Saturn 모델 호출은 다시 보내지 않고 같은 대체 규칙으로 간다.
- 로그에는 고정 문구만 쓰고 router 키와 요청 원문은 쓰지 않는다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| 판단 방식에 맞는 router를 만들 수 없음 | 원인 한 줄을 보이고 실행하지 않고 끝낸다. |
| 시작 확인 실패 | TUI에 키를 요청하고, 확인하기 전에는 일반 요청을 거절한다. |
| 호출 실패(보내기 전 실패, 응답 없음, 보낸 뒤 시간 초과, 속도 제한) | 5초 간격으로 두 번 다시 보낸다. 시도마다 응답을 5초까지 기다린다. |
| 첫 실패부터 10초 뒤에도 실패 | 로그를 남기고 현재 에이전트와 현재 모델로 진행하며 건너뛴 판단을 사유와 함께 기록한다. 입력을 대기로 보내지 않는다. |
| 답의 후보 밖 선택, NaN, 확률 누락 | 판단을 `invalid`로 기록하고 질문별 대체 규칙을 적용한다. |
| 판단 중 revision 변경 | 판단을 `superseded`로 기록하고, 새 상태로 맥락을 다시 만들어 한 번 다시 판단한다. |
| 판단 맥락이 불완전 | router를 부르지 않고 입력을 대기에 둔다. |
| 연속 3회 호출 실패 | 입력 접수를 계속하고 상태판에 `판단 모델 연결 끊김`을 보인다. |
| 요청 전송 뒤 시간 초과 | 다시 보내고 그 호출의 비용을 `cost-unknown`으로 기록한다. |
| 속도 제한 | 5초 뒤 다시 보내고 남은 요청의 동시 수를 줄인다. |
| 나눈 요청 일부 실패 | 실패한 조각의 항목만 순위 대체 규칙을 적용하고 다른 조각의 답은 쓴다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| router를 확인하기 전에는 일반 요청을 받지 않는다. | router 확인이 실패하는 환경에서 일반 요청이 거절되고, 키를 다시 확인한 뒤에는 처리되는지 확인한다. |
| `invalid` 답이면 질문별 대체 규칙을 적용한다. | 질문마다 `invalid` 답을 주고 표의 대체 규칙이 적용되는지 확인한다. |
| 호출이 실패하면 5초 뒤 한 번, 다시 5초 뒤 한 번 더 보내고 첫 실패부터 10초 뒤에도 실패하면 진행 중인 시도도 끊고 포기한다. 시도마다 응답 대기는 5초이고 응답이 없으면 15초 안에 포기한다. | `saturn-terminal/core/src/routers/failure.rs`의 `retry_delay_waits_five_seconds_twice_then_gives_up`, `retry_delay_gives_up_when_waiting_would_pass_the_deadline`, `saturn-terminal/engine/src/routers/remote/tests.rs`의 `failures_retry_twice_five_seconds_apart_then_give_up`, `first_retry_waits_five_seconds_before_sending`, `router_retry_gives_up_at_the_deadline_for_every_response_pattern` |
| 보낸 뒤 시간 초과와 속도 제한도 같은 간격으로 다시 보내고, 키 거절과 `invalid`는 다시 보내지 않는다. | `saturn-terminal/engine/src/routers/remote/tests.rs`의 `timeout_after_send_is_retried_and_counted_as_unknown_cost`, `rate_limit_retries_on_the_same_interval_ignoring_retry_after`, `auth_failure_and_invalid_status_are_not_retried` |
| 입력 처리 판단이 실패하면 현재 에이전트와 현재 모델로 보내고 입력을 대기로 보내지 않는다. | `saturn-terminal/core/src/routers/failure.rs`의 `route_after_failure_keeps_the_current_agent_and_model` |
| `compact` 판단이 실패하면 router가 시작한 전환은 건너뛰고 강제한 전환은 순위 순서로 채운다. | `saturn-terminal/core/src/routers/failure.rs`의 `compact_failure_skips_router_transition_and_fills_forced_one`, `saturn-terminal/core/src/sessions/ranking.rs`의 `order_after_router_no_verdicts_keeps_rrf_order` |
| 판단을 건너뛸 때 정한 문구를 로그에 남기고 비밀값은 남기지 않는다. | `saturn-terminal/engine/src/routers/mod.rs`의 `route_after_failure_logs_skip_message_and_keeps_current_model`, `compact_after_failure_logs_by_who_started_the_transition` |
| 연속 3회 실패해도 입력 접수를 멈추지 않고 연결 끊김만 표시한다. | `saturn-terminal/engine/src/routers/mod.rs`의 `three_failures_show_disconnected_and_success_resets`, `call_counts_only_unanswered_failures_and_keeps_accepting`, `saturn-terminal/tui/src/app/tests.rs`의 `submit_while_router_disconnected_still_sends_input` |
| 판단 호출의 보낸 원문, 받은 원문, 질문별 답을 모두 기록한다. | 기록을 켠 채팅에서 호출마다 원문과 답이 저장되고 `/record off` 채팅에서는 생략되는지 확인한다. |
| `choice` 확신도는 `(N·pmax − 1)/(N − 1)`로 계산한다. | 균등 분포에서 0, 한 선택지 확률 1에서 1이 나오는지 확인한다. |
| 요청이 크기 한도를 넘으면 질문 단위로 나눠 같은 state로 보낸다. | `saturn-terminal/core/src/routers/split.rs`의 `split_request_over_limit_splits_by_question_with_same_state` |
| 나눈 요청은 동시 최대 8개까지 병렬로 보내고 조각마다 실패를 따로 처리한다. | 조각 수가 8을 넘는 요청에서 동시 전송이 8개를 넘지 않는지, 한 조각만 실패시켜 그 항목만 순위 대체인지, 429에서 동시 수가 줄어드는지 확인한다. |
| 255개 초과 선택지는 나뉘어 전송된다. | 255개 초과 선택지가 나뉘어 전송되는지 확인한다. |
| 판단 요청에 같은 채팅의 직전 입력, 최초 목표, 최신 수정, 진행 상태, 보류 작업의 번호와 목표를 싣고, 같은 후속 문장도 목표마다 다른 요청이 된다. | `saturn-terminal/engine/src/lifecycle/judge_context.rs`의 `same_follow_up_carries_the_goal_of_each_conversation`, `latest_amendment_is_the_last_input_steered_into_the_task`, `held_tasks_are_listed_with_their_ids_and_goals` |
| 맥락이 한도를 넘으면 오래된 보류부터 빼고 생략을 표시하며, 최초 목표나 최신 수정이 잘리면 판단하지 않고 대기에 둔다. | `saturn-terminal/engine/src/judge_context.rs`의 `long_held_list_drops_the_oldest_and_says_so`, `goal_over_its_limit_is_marked_cut_and_incomplete`, `unknown_goal_of_an_active_task_is_incomplete`, `long_previous_input_is_cut_but_the_context_stays_complete`, `saturn-terminal/engine/src/lifecycle/judge_context.rs`의 `context_over_the_limit_is_not_judged_and_waits_for_the_user` |
| 맥락의 비밀값과 절대 경로는 가리고, 인젝션 문구는 따옴표 안에 머문다. | `saturn-terminal/engine/src/judge_context.rs`의 `secrets_are_masked_before_cutting_so_no_partial_secret_is_left`, `injected_lines_stay_inside_the_quoted_text`, `saturn-terminal/engine/src/lifecycle/judge_context.rs`의 `context_text_hides_secrets_and_absolute_paths`, `injected_instructions_stay_inside_quoted_text` |
| 판단 중 상태가 바뀌어 다시 판단하면 새 상태의 맥락으로 요청을 만든다. | `saturn-terminal/engine/src/lifecycle/judge_context.rs`의 `rerouted_request_after_a_revision_conflict_is_rebuilt_from_the_new_state` |
| `keep_current` 기준값 0.8은 한국어 입력에서도 이어 가기를 가른다. | [한국어 이어 가기 실험](../experiments/continuation-judgment-korean/report.md): 0.80에서 현재 state의 재현율 10.1%, 작업 정보를 더한 state 69.7%. 작업 정보를 state에 싣는 구현은 [판단 요청 맥락](#판단-요청-맥락)에 있고 실제 router 정확도는 이 구현으로 다시 재지 않았다. |
| 피드백, 취소, 반응 신호, router 실패를 100건 넣어도 활성 정책 지문, 기준값, router 모델, 설정 번호가 바뀌지 않는다. | `saturn-terminal/engine/src/lifecycle/policy.rs`의 `feedback_cancel_and_failures_leave_the_active_policy_unchanged` |
| 정책 교체 중 접수한 입력은 접수 때 설정 번호의 기준값으로만 판단하고, 옛 설정으로 되돌리면 옛 번호를 다시 쓰며, 재시작해도 입력의 번호가 같다. | `saturn-terminal/engine/src/lifecycle/policy.rs`의 `inputs_keep_the_policy_they_were_accepted_under_across_swap_rollback_and_restart` |
| 구현 전인 `train`, `router use`, 판단 방식 `collect`는 정책을 바꾸지 않고 미지원 오류나 설정 오류를 돌려준다. | `saturn-terminal/engine/src/lifecycle/requests.rs`의 `requests_each_get_one_response_in_order`, `saturn-terminal/engine/src/routers/mod.rs`의 `select_follows_method_and_endpoint_rules` |
| 영어 질문은 한국어와 인젝션 구간에서 판단 성능을 떨어뜨리지 않는다. | [#15](https://github.com/woonyong-choi/saturn/issues/15) 실험으로 구간별 성능 회귀를 확인한다. |
| 후보를 순위로 자르지 않고 전체를 묻는다. | `saturn-terminal/core/src/routers/tests.rs`의 `compact_questions_150_candidates_ask_all` |
| 모델 판단 그림자를 켜고 꺼도 실제 모델과 실제 질문이 같고 원문은 한 번만 가며, 켜면 후보·정책·확률·적용 모델을 내보낸다. | `saturn-terminal/engine/src/lifecycle/model_shadow.rs`의 `shadow_on_and_off_apply_the_same_model_and_the_same_real_questions` |
| 그림자 답이 빠지거나 틀리거나 router가 실패해도 실제 선택은 그대로다. | `missing_wrong_or_failed_shadow_answers_leave_the_real_choice_alone` |
| 어긋난 판단의 그림자는 미적용으로 남는다. | `a_superseded_judgment_leaves_its_shadow_unapplied` |
| 그림자 질문이 실제 요청·답과 갈라지고 크기를 넘으면 묻지 않는다. | `saturn-terminal/core/src/routers/shadow.rs`의 `shadow_answers_split_off_and_never_disturb_the_real_judgment` |
| 고정하지 않은 입력에 모델 목록을 `target_model` 후보로 묻고 고른 모델로 보낸다. 고정 모델이거나 매뉴얼 모드이거나 목록이 없으면 묻지 않고, 후보 밖이면 기본 모델이나 현재 모델이다. | [모델 고르기](providers-and-sessions.md#모델-고르기)의 `target_model` 테스트 |
| `is_constraint` 기준값과 `constraint_change`가 한국어 입력에서 기준 정확도를 넘는다. | [등록 기준값의 사람 확인](../experiments/constraint-human-check/report.md)에서 0.80 정밀도 91.3% [87.6, 94.0], 0.70 정밀도 84.8%·재현율 77.2%. [Jev와 Haiku 비교](../experiments/constraint-exception-judge/report.md)에서 해제·예외 종합 정확도 91.2%(사후 계산), 종류 정확도 87.8%로 기준 90%에는 못 미친다. 간접 지시 입력은 정확도 74.5% [68.0, 80.0]이고 앞 입력의 제약 등록 여부를 state에 넣어도 오르지 않았다([간접 지시 정확도](../experiments/indirect-constraint-accuracy/report.md)). |

## 단점

- 입력마다 외부 router를 부르므로 호출 비용과 지연이 입력 수에 비례한다.
- router가 실패하면 모델 선택과 처리 방식 판단 없이 현재 모델로 진행해 입력에 맞는 모델을 고르지 못하고, 재시도 때문에 입력 처리가 최대 15초 더 늦어진다.
- router 규격과 `router_id` 매핑을 Saturn이 직접 정의하고 유지한다.
- 큰 변경마다 옛 선택지에서 새 선택지로 가는 대응표를 코드에 함께 관리한다.
- state에 앞 입력의 제약 등록 여부를 넣어도 router는 그 값을 거의 쓰지 않아 간접 지시 입력 정확도가 오르지 않았다([간접 지시 정확도](../experiments/indirect-constraint-accuracy/report.md)). 질문 문장에 규칙을 더하면 올랐지만 일반 제약 입력의 재현율이 떨어졌다.

- 판단 요청 맥락은 이어 가기 재현율을 올리려고 넣었지만 새 작업 오접합을 함께 늘릴 수 있다. 두 값은 따로 재야 하고, 이 구현에서는 실제 router로 다시 재지 않았다. 기존 실험의 오접합 우려는 그대로 남는다.

## 대안

- router 벤더의 형식과 이름을 그대로 쓰는 방식은 router를 바꿀 때 스키마와 코드를 고쳐야 해 버렸다([판단 규격은 Saturn이 정하고 router는 중립 이름과 출처로 기록한다](../decisions/2026-09-29-vendor-neutral-router-spec.md)).
- Saturn 모델을 API 서버로 두고 서버가 판단 기록을 모으는 방식은 호출마다 상태가 서버로 가서 버렸다([판단 기록은 로컬에 쌓고 동의한 레코드만 서버로 올린다](../decisions/2026-09-29-local-first-judgment-collection.md)).

## 미해결 질문

- 독립 프로젝트 표본과 사람 확인 정답으로 `is_constraint` 0.8, `constraint_ask` 0.7을 다시 확인할 수 있는지 ([등록 기준값의 사람 확인](../experiments/constraint-human-check/report.md))
- 질문을 상위 범주에서 하위 판단으로 내려가는 계층 트리로 나눌지, 단계마다 호출할지, 지금처럼 한 번에 고를지 ([#68](https://github.com/woonyong-choi/saturn/issues/68))
- 판단 방식 `collect`를 기준 router가 결정하고 Saturn 모델은 기록만 하는 방식으로 할지, 반대로 할지 ([#40](https://github.com/woonyong-choi/saturn/issues/40))
- router에 넘기는 state에 subagent 목록을 넣을지, 개수만 넣을지, 넣지 않을지 ([#63](https://github.com/woonyong-choi/saturn/issues/63))
