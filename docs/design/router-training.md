# router 학습

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [판단 기록은 로컬에 쌓고 동의한 레코드만 서버로 올린다](../decisions/2026-09-29-local-first-judgment-collection.md), [판단 규격은 Saturn이 정하고 router는 중립 이름과 출처로 기록한다](../decisions/2026-09-29-vendor-neutral-router-spec.md) |

## 요약

router 학습은 판단 기록과 사용자 반응으로 router를 사용자에게 맞추는 기능의 설계다. 판단마다 기준값을 조금씩 옮기는 빠른 조정과 `/train` 때 중심값을 다시 계산하는 느린 조정의 계산이 있다. `/train`은 판단 기록을 채점하고 Saturn 모델을 학습한다. 새 모델은 승격 게이트를 통과할 때만 현재 router 버전이 된다.

지원 상태는 다음과 같다. 결과 신호와 피드백 답은 판단 기록에 남지만 기준값과 모델을 바꾸는 경로는 실행에 연결하지 않았다. 빠른 조정과 느린 조정은 `core`의 계산 규칙과 시험으로만 있고 engine은 호출하지 않는다([#337](https://github.com/woonyong-choi/saturn/issues/337) 폐기). 실행 중 정책은 [정책 고정](router.md#정책-고정)대로 입력 접수 때의 설정 번호와 engine 시작 때의 router로 고정된다. `train`, `router use`, `router versions`는 미지원 오류를 돌려주고 판단 방식 `collect`는 시작 때 설정 오류가 된다. 새 정책은 검증을 마친 뒤 사용자가 설정이나 router 버전으로 명시해 적용하며 이미 접수한 입력에 소급하지 않는다.

## 동기

router가 같은 기준값으로 모든 사용자를 대하면 사람마다 다른 입력 습관을 따라가지 못한다. 기준값이 낮으면 틀린 행동이 늘고, 높으면 사용자가 직접 할 일이 늘어난다. 외부 API인 기준 router만 쓰면 판단마다 비용이 들고 사용자 기록으로 나아지지 않는다. 이 기능은 사용자 반응으로 기준값을 맞추고, 쌓인 판단 기록으로 로컬 Saturn 모델을 만든다. 새 모델이 현재 모델보다 나빠지면 승격하지 않아 사용자가 성능 저하를 겪지 않는다.

## 예시

### 틀린 판단 뒤에도 기준값이 그대로일 때

1. router가 새 입력을 하던 작업에 이어 가는 입력으로 판단하고 engine이 끼워 넣는다.
2. 사용자가 곧바로 그 처리를 뒤집는다.
3. 다음 입력 3개가 지나거나 10분이 지나면 `routers`가 틀림 신호를 확정해 판단 기록에 쓴다.
4. 기준값은 그대로다. 신호는 나중에 채점과 학습 평가의 자료로만 쓰이고, 기준값을 바꾸려면 사용자가 설정을 고친다.

### 채점할 판단이 모자랄 때

1. 사용자가 `/train`을 실행한다.
2. 지난 실행 뒤 채점 안 된 판단이 83건뿐이다.
3. TUI가 `채점할 판단 83 / 200건 · 200건이 쌓이면 실행할 수 있습니다`를 보인다.
4. 채점과 학습은 시작하지 않는다.

### 새 Saturn 모델을 승격할 때

1. 사용자가 채점 안 된 판단이 200건 이상 쌓인 뒤 `/train`을 실행한다.
2. TUI가 채점 후보 수, 채점 모델, 예상 토큰, 기준값 조정 대상, 모델 추가 학습 여부를 학습 확인 창에 보인다.
3. 사용자가 확인하면 상태판의 학습 줄이 단계, 채점 건수, 경과 시간, 토큰을 보인다.
4. engine이 품질 게이트를 통과한 라벨로 Saturn 모델을 학습한다.
5. 같은 평가용 데이터에서 승격 게이트를 통과하면 새 모델이 현재 router 버전이 된다.

## 상세 설계

### router 버전

router 버전은 모델, 보정값, 질문별 목표 틀림 비율을 묶은 것이다. 모델이 바뀌어도 같은 목표를 지키기 위해서다.

- 기준값은 router 버전마다 목표 틀림 비율에서 다시 계산한다. 버전에 기준값 숫자 대신 목표를 남기기 위해서다.
- 판단 기록마다 router 버전, 그때의 기준값, 물은 확률 q를 남기고, 결과 신호와 물은 답이 생기면 같은 기록에 채운다. 기준값 조정 계산을 나중에 다시 하기 위해서다.
- 기준값은 설정 층에 두어 사용자 층과 폴더 층에서 조정한다. 릴리스 없이 기준값을 바꾸기 위해서다. 설정 층은 [설정](settings.md)에 있다.
- 버전 표기는 `v3`처럼 `v`와 1 이상 정수다. 명령줄의 `saturn router use`와 `saturn router train --from`은 `3`처럼 `v`를 뺀 표기도 `v3`으로 읽고, 다른 모양은 오류로 거절한다. 설정 키가 아니다([설정](settings.md#설정-키가-없는-고정-상수)).
- router 질문과 호출 규칙은 [router](router.md)에 있다.

`/router use`은 router 버전 화면을 연다. 이 화면은 버전별 router, 보정값, ECE를 보인다. 질문별로 목표 틀림 비율, 기준값, 최근 200건의 틀림 수와 판단 수도 보인다.

| 키 | 동작 |
|---|---|
| `Enter` | 버전 상세 표시 |
| `r` | 1차 영점으로 복귀, `/train --reset-thresholds`와 동일 |
| `t` | 고른 버전에서 다시 학습, `/train --from`과 동일 |
| `u` | 확인 한 줄 뒤 고른 버전 사용, `saturn router use`와 동일(명령줄은 `--yes`로 확인을 건너뜀) |
| `Esc` | 화면 종료 |

### 결과 신호

1. `routers`가 판단 뒤 관찰 시간이 지나면 결과 신호를 확정한다.
2. 관찰 시간은 다음 입력 3개 또는 10분이다.
3. router 판단으로 행동한 뒤 사용자가 뒤집거나 취소하면 틀림 신호다.
4. 행동하지 않았는데 사용자가 같은 행동을 직접 하면 놓침 신호다.
5. 사용자 반응이 없으면 정답으로 보지 않고 미확정(`Unconfirmed`)으로 확정한다.

- `routers`는 관찰 중에 처음 본 반응을 들고 있다가 관찰 시간이 끝난 뒤 한 번에 판단 기록에 쓴다. 관찰이 끝나기 전의 반응은 바뀔 수 있기 때문이다.
- 관찰이 끝나기 전에는 판단 기록의 결과 신호가 비어 있고, 비어 있는 판단은 느린 조정의 기록에 들지 않는다.
- 틀림 신호와 놓침 신호를 둘 다 쓴다. 행동한 판단만 보면 기준값이 끝없이 오르기 때문이다.

### 사용자에게 묻기

`routers`는 모든 판단에서 확률 q로 판단이 맞았는지 사용자에게 묻는다. 묻는 판단을 고르게 섞기 위해서다.

- q는 기준값 근처의 판단에서 높고 확실한 판단에서 낮되 0이 되지 않는다. 기준값 근처에서 신호를 더 모으기 위해서다.
- 느린 조정이 중심값을 계산할 때 행동하지 않은 판단은 물은 답만 1/q로 가중한다. 확률로 고른 표본의 치우침을 바로잡기 위해서다.
- 빠른 조정은 물은 피드백 답에서 온 신호만 1/q로 가중한다. 행동 신호는 묻지 않아도 항상 관찰되어 보정할 치우침이 없기 때문이다.
- 전체 묻는 빈도는 판단 20번에 1번을 넘지 않는다. 사용자에게 묻는 횟수가 늘어나는 일을 막기 위해서다.
- q는 `0.01 + 0.09 × exp(−|p − 기준값| / 0.1)`이다. 최근 물은 비율이 상한에 닿으면 최저값 0.01을 쓴다. 1/q 가중의 최대값을 100으로 묶기 위해서다.

TUI는 입력 에코 다음 줄에 피드백 질문을 보인다. 사용자는 `1`로 맞음, `2`로 틀림, `0`으로 닫기를 고른다. 8초 안에 답이 없으면 질문을 지운다. 틀림 답의 입력이 아직 보내지지 않았으면 TUI가 바로잡기를 제안한다. TUI가 붙어 있지 않은 동안에는 피드백 질문을 생략한다.

### 빠른 조정

연결하지 않은 설계다. 아래 계산은 `core`에 구현과 시험이 있지만 engine 실행 경로는 호출하지 않으며, 켜려면 모의 판단이 아니라 실제 판단 기록에서 검증하고 명시적으로 적용하는 새 결정이 필요하다.

1. `routers`가 판단마다 확정된 신호 하나로 기준값을 옮긴다.
2. 옮기는 범위는 질문별 중심값 ±0.05 안이다.
3. 신호 하나의 이동은 `0.002 × 가중 × 방향`이다. 방향은 틀림이면 `+(1 − α)`, 놓침이면 `−α`이고, α는 목표 틀림 비율이다.
4. 가중은 행동 신호(틀림은 사용자가 뒤집거나 취소, 놓침은 사용자가 직접 같은 행동)에서 1이다.
5. 확률 q로 골라 물은 판단에서 온 신호는 행동 신호와 겹쳐도 가중이 `1/q`다.

- 폭은 고정이고 줄이거나 멈추거나 다시 키우지 않는다. 폭을 줄이고 멈추면 급변 때 다시 키우는 재시작이 필요한데, 그 탐지가 사실상 위로만 발동해 기준값을 위로 밀었기 때문이다.
- 틀림은 기준값을 `0.002 × 가중 × (1 − α)`만큼 올리고, 놓침은 `0.002 × 가중 × α`만큼 내린다. 두 신호 중 틀림의 비율이 α일 때 기준값이 멈추게 하기 위해서다.
- 놓침을 기준값을 내리는 신호로 두는 것은 행동 신호라 항상 관찰되기 때문이다. 행동한 판단의 맞음 응답은 쓰지 않는다.
- 행동 신호에 1/q를 붙이지 않는 것은 신호 하나의 이동이 최대 100배가 되어 기준값이 크게 흔들리기 때문이다. 모든 신호에 1/q를 붙인 모의 판단에서 100건 안 폭이 0.053이었다.
- 모의 판단에서 목표 5%일 때 틀림 비율은 4.77%, 100건 안 폭 상한은 0.0085였고, 틀림 비율이 3%에서 10%로 바뀐 뒤 2,500건 안에 다시 맞춘 비율은 100%였다([#140](https://github.com/woonyong-choi/saturn/issues/140)).
- 빠른 조정이 가진 상태는 중심값에서 벗어난 차이 하나이고 engine 메모리에만 둔다. 다시 시작하면 차이는 0이다.

### 느린 조정

1. `/train`이 실행되면 `routers`가 모든 판단 기록으로 질문별 중심값을 다시 계산한다.
2. 판단 기록에서 확률 p, 그때의 기준값, 행동 여부(p가 그때의 기준값 이상이면 행동), 물었는지와 q, 결과 신호, 물은 답을 읽는다.
3. 행동한 판단은 틀림 신호(사용자가 뒤집거나 취소)를 그대로 틀림으로 쓰고 가중은 1이다. 반응이 없으면 틀리지 않은 것으로 센다.
4. 행동하지 않은 판단은 물은 답만 쓴다. 판단이 틀렸다는 답이면 틀림 가중이 1/q이고, 맞았다는 답이면 0이다. 묻지 않았거나 답이 없는 판단은 틀림 가중 0으로 분모에만 들어간다.
5. 기준값 격자는 질문의 최저값에서 최고값까지 0.005 간격이다. 격자의 각 값 t마다 p가 t 이상인 판단 전체의 틀림 가중 합을 그 판단 수로 나눈 값을 t의 틀림 비율로 본다.
6. 틀림 비율이 목표 틀림 비율 이하인 가장 낮은 t가 새 중심값 후보다. 그런 값이 없으면 최고값이다.
7. 새 중심값은 이전 중심값 ±0.05 안으로 제한하고 질문의 최저값과 최고값 안에 둔다.
8. 새 중심값이 잡히면 빠른 조정으로 생긴 차이를 0으로 되돌린다.

- 목표 틀림 비율의 기본값은 5%다.
- 기준값이 있는 질문 중 답이 `noul`인 질문마다 판단 기록 한 건이 관찰 하나다. 판단 하나의 결과 신호와 물은 답은 그 판단의 모든 관찰에 같이 적용한다. 신호와 답을 질문별로 따로 받지 않는 현재 기록 구조에 맞춘 것이다.
- 가장 낮은 값을 고르는 것은 목표 위험 안에서 행동 비율을 최대로 두기 위해서다.
- 놓침 신호는 쓰지 않는다. 놓침은 행동하지 않은 판단 중 맞은 것만 관찰되고 틀림은 행동한 판단 중에서만 관찰되어, 둘을 한 비율로 섞으면 모집단이 달라 중심값이 최저값이나 최고값으로 갈리기 때문이다.
- 행동하지 않은 판단의 틀림은 물은 답을 1/q로 키워 추정한다. 판단 기록에 q가 있어 행동하지 않은 판단의 결과를 몰라도 곡선을 만들 수 있다.
- 모의 판단(시드 120, 복제 100, 판단 1만 건, `/train` 1,000건마다)에서 목표 5% 조건의 9,000건 시점 중심값은 틀림 비율이 목표를 지키는 값 ±0.02 안이 66%, 최저값이나 최고값이 3%였다. 목표 3%, 5%, 10% 조건의 장기 틀림 비율은 4.96%, 4.98%, 5.91%였다([#166](https://github.com/woonyong-choi/saturn/issues/166)).
- 기준값 자동 조정은 그 질문의 쓰인 결과(행동한 판단과 물은 답)가 300건 이상일 때만 한다. 적은 결과로 한 조정은 잡음 수준이기 때문이다.
- `/train` 한 번에 중심값이 움직이는 폭은 빠른 조정의 범위와 같은 0.05로 제한한다. 한 번에 크게 움직이면 중심값이 최저값이나 최고값 쪽으로 쏠리기 때문이다. 일찍 적용하되 폭을 제한하면 행동 범위가 단계적으로 낮아져 낮은 확률의 결과가 쌓인다.
- 모의 판단(시드 120, 7개 조건, `/train` 1,000건마다, 빠른 조정 고정 폭, 복제 1,000)에서 이 방식은 목표 3%, 5%, 10% 조건의 최저값이나 최고값에 닿은 비율이 3.8%, 0.0%, 0.4%이고 장기 틀림 비율이 5.04%, 5.10%, 5.84%였다. 목표 5% 조건의 9,000건 시점 중심값은 목표를 지키는 값 ±0.02 안이 62%이고, 첫 조정은 1,000건째였다. 쓰인 결과를 3,000건으로 늦추고 폭을 제한하지 않으면 닿은 비율이 17.6%, 적중이 52%, 목표 10% 조건의 장기 틀림 비율이 7.78%였다. 폭 제한 없이 일찍 적용해도 닿은 비율은 16~20%로 같다([#190](https://github.com/woonyong-choi/saturn/issues/190)).

### 기준값 안전장치

- 질문마다 기준값의 최저값과 최고값을 두고, 되돌릴 수 없는 행동의 기준값은 0.8 미만으로 두지 않는다. 되돌릴 수 없는 행동의 오판을 막기 위해서다.
- 기준값을 바꾼 뒤 순차 검정으로 확실히 나빠졌다고 나오면 이전 값으로 되돌린다. 나빠진 기준값이 굳는 일을 막기 위해서다.
- 순차 검정은 행동한 판단의 틀림 비율이 α인지 2α인지 가르는 SPRT이고, 1종 오류율 0.05, 2종 오류율 0.2로 경계를 정한다. 되돌리는 대상은 느린 조정 직전 값이다.
- 행동 비율이 하한 아래로 내려가면 경고한다. 기준값이 올라 router가 거의 행동하지 않게 되는 일을 알리기 위해서다.

### 채점

채점은 판단 기록에 학습용 정답 라벨을 붙이는 처리다. 사람이나 코드가 아니라 설정으로 정한 채점 모델이 채점한다. 채점 모델이 바뀌어도 같은 흐름을 쓰기 위해서다.

1. `routers`가 결과 신호가 있거나 확신도가 낮거나 router끼리 답이 갈린 판단을 먼저 고른다.
2. 채점 모델마다 선택지 순서를 두 번 바꿔 풀고, 두 답이 같을 때만 채택한다.
3. 사후 판정은 결정 이후 대화를 보고, 결정 시점에 알 수 있던 정보를 기준으로 정답을 정한다.
4. `routers`가 채점 모델 답, 사후 판정, 결과 신호를 label model로 합친다.
5. 품질 게이트를 통과한 라벨만 학습용과 평가용으로 나눈다.

| 라벨 | 통과 조건 |
|---|---|
| 학습용 | 합의, 선택지 순서 일관, 반대 결과 없음 |
| 평가용 | 만장일치, 결과 신호와 일치 |

- 학습용 품질 게이트는 틀린 라벨로 학습하는 일을 막기 위해서다.
- 평가용 품질 게이트는 틀린 라벨로 승격을 판정하는 일을 막기 위해서다.
- AI 채점 라벨의 치우침은 사용자가 직접 답한 소량 라벨로 추정해 보정하고, 뒤집기와 취소는 약한 라벨로 가중한다. AI 채점 라벨은 AI 기준이기 때문이다.
- 합의는 채점 답의 과반이다. 반대 결과는 사후 판정이 합의와 다른 경우이고, 결과 신호와 일치는 사후 판정이 합의와 같은 경우로 본다. 결과 신호를 무엇과 비교할지는 미해결 질문이다([#105](https://github.com/woonyong-choi/saturn/issues/105)).
- 사용자가 직접 답한 라벨은 답이 하나로 모이면 평가용이다. 뒤집기와 취소 신호가 붙은 라벨의 가중치는 0.5다.
- 채점 모델 사이 일치도는 [#12](https://github.com/woonyong-choi/saturn/issues/12)에서, 결과 신호의 정확도는 [#14](https://github.com/woonyong-choi/saturn/issues/14)에서 잰다.

### `/train` 실행

1. 사용자가 `/train`이나 `saturn router train`을 실행한다. 명령줄은 `--yes`로 마지막 확인을 건너뛴다.
2. engine이 지난 실행 뒤 채점 안 된 판단 수를 센다.
3. 200건 미만이면 engine은 실행하지 않고 부족한 건수를 보인다.
4. 200건 이상이면 TUI가 학습 확인 창을 보이고 사용자 선택을 기다린다.
5. engine이 채점 모델로 후보를 채점하고 품질 게이트를 통과한 라벨을 저장한다.
6. `routers`가 결과 신호를 확정한 모든 판단 기록으로 `Observation` 목록을 만들어 질문마다 `recenter`에 넘기고 질문별 중심값을 다시 계산한다. 쓰인 결과가 300건 미만인 질문은 그대로 둔다.
7. engine이 로컬 학습기로 Saturn 모델을 학습하고 승격 게이트로 비교한다.

- `/train`은 지난 실행 뒤 채점 안 된 판단이 200건 이상일 때만 실행한다([#16](https://github.com/woonyong-choi/saturn/issues/16)). 적은 라벨로 한 조정은 잡음 수준이기 때문이다.
- 7단계의 모델 학습은 누적 학습용 라벨이 1,000건 이상이고 평가용 라벨이 200건 이상일 때만 한다. 모자라면 채점과 기준값 조정까지만 하고 학습은 건너뛴다. 기준값 하나를 맞추는 것보다 모델 가중치를 학습하는 데 라벨이 더 많이 필요하기 때문이다([#16](https://github.com/woonyong-choi/saturn/issues/16)).
- 구현 상태: 결과 신호 기록은 실행 경로에 있다. 빠른 조정, 느린 조정의 계산(`recenter`, `recenter_thresholds`)은 구현돼 있지만 실행 경로에 연결하지 않았고 6단계의 결과도 설정에 자동으로 쓰지 않는다. 채점 건수 미리보기, 채점 모델 호출, 학습기 실행, 승격 게이트, 기준값 되돌리기는 구현 전이다([#91](https://github.com/woonyong-choi/saturn/issues/91)).
- Saturn 모델 학습은 Python과 MLX로 한다. Apple Silicon에서 로컬로 학습하기 위해서다.
- 판단 기록은 로컬에 쌓고, 사용자가 동의한 레코드만 서버로 올린다([결정 기록](../decisions/2026-09-29-local-first-judgment-collection.md)).

### 승격 게이트

새 Saturn 모델은 현재 router 버전과 같은 평가용 데이터에서 비교한다. 아래 조건을 모두 채울 때만 새 모델을 현재 router 버전으로 승격한다. 새 모델이 현재 모델보다 나빠지는 일을 막기 위해서다.

| 항목 | 조건 |
|---|---|
| 정확도 | 새 모델과 현재 모델의 정확도 차이의 95% 신뢰구간 하한이 −1pp보다 큼 |
| 보정 | Brier와 ECE 악화 없음 |
| 처리 범위 | 처리 비율 감소 없음 |
| 순서 일관성 | 선택지 순서 일관성 저하 없음 |

Saturn 모델 후보별 정확도, Brier, 지연은 [#11](https://github.com/woonyong-choi/saturn/issues/11)에서 잰다.

### 베이스 모델 교체

1. Saturn 모델의 베이스 모델이 바뀌면 engine이 쌓인 판단 기록으로 다시 학습한다.
2. 새로 학습한 모델을 승격 게이트로 현재 모델과 비교한다.
3. 게이트를 통과해도 사용자가 승인할 때만 교체한다.

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 결과 신호는 관찰 시간(다음 입력 3개 또는 10분)이 지난 뒤에만 확정한다. | `saturn-terminal/engine/src/lifecycle/outcomes.rs`의 `settle_after_three_inputs_records_confirmed_signal`, `settle_before_observation_ends_leaves_signal_empty`, `settle_after_ten_minutes_without_reaction_records_unconfirmed`, `saturn-terminal/engine/src/outcomes.rs`의 `settled_before_window_and_inputs_is_empty`, `settled_after_three_inputs_returns_reaction` |
| 빠른 조정은 기준값을 중심값 ±0.05 밖으로 옮기지 않는다. | `saturn-terminal/core/src/routers/calibration/tests.rs`의 `observe_never_leaves_fast_range` |
| 빠른 조정은 신호 하나로 기준값을 `0.002 × 가중 × 방향`만큼만 옮기고, 이동 폭은 신호가 쌓여도 줄지 않는다. | `saturn-terminal/core/src/routers/calibration/tests.rs`의 `observe_step_stays_fixed_after_many_signals` |
| 빠른 조정은 행동 신호에 1/q를 붙이지 않고 물은 피드백 답의 신호에만 붙인다. | `saturn-terminal/core/src/routers/calibration/tests.rs`의 `observe_behavior_signal_moves_fixed_step_without_ask_weight`, `observe_asked_answer_is_weighted_by_inverse_q` |
| 빠른 조정은 같은 입력 열에서 모의 판단의 고정 폭 규칙과 같은 이동을 한다. | `saturn-terminal/core/src/routers/calibration/tests.rs`의 `observe_follows_simulation_b_on_same_signals` |
| 되돌릴 수 없는 행동의 기준값은 0.8 미만이 되지 않는다. | `saturn-terminal/core/src/routers/calibration/tests.rs`의 `observe_irreversible_floor_holds` |
| 느린 조정은 쓰인 결과가 300건 미만이면 중심값을 바꾸지 않고 300건 이상이면 바꾸며, 행동하지 않았고 묻지 않은 판단은 쓰인 결과로 세지 않는다. | `saturn-terminal/core/src/routers/calibration/tests.rs`의 `recenter_below_min_results_does_nothing`, `recenter_at_min_results_acts_and_below_does_not`, `recenter_unasked_skipped_judgments_are_not_results` |
| 느린 조정은 `/train` 한 번에 중심값을 이전 중심값 ±0.05 안으로만 움직인다. | `saturn-terminal/core/src/routers/calibration/tests.rs`의 `recenter_moves_at_most_fast_range_per_call`, `recenter_all_wrong_uses_upper_bound` |
| 느린 조정은 행동한 판단의 틀림 신호를 가중 1로 쓰고, 반응 없는 행동은 틀리지 않은 것으로 센다. | `saturn-terminal/core/src/routers/calibration/tests.rs`의 `recenter_acted_without_reaction_counts_as_not_wrong`, `recenter_all_wrong_uses_upper_bound` |
| 느린 조정은 행동하지 않은 판단은 물은 답만 1/q로 쓰고 놓침 신호는 쓰지 않는다. | `saturn-terminal/core/src/routers/calibration/tests.rs`의 `recenter_skipped_judgment_uses_only_asked_answer_with_inverse_q`, `recenter_skipped_judgment_with_missed_signal_is_not_used` |
| 느린 조정은 목표 틀림 비율 이하인 가장 낮은 격자 값을 새 중심값으로 고르고, 없으면 최고값을 고른다. | `saturn-terminal/core/src/routers/calibration/tests.rs`의 `recenter_picks_lowest_threshold_meeting_target`, `recenter_all_wrong_uses_upper_bound`, `threshold_grid_default_bounds_spans_bounds_by_half_percent` |
| 느린 조정은 행동이 목표를 지키는 값 위에서 멈춘 기록에서도 새 중심값을 그 값 근처로 둔다. | `saturn-terminal/core/src/routers/calibration/tests.rs`의 `recenter_where_actions_stop_above_oracle_lands_near_oracle` |
| 느린 조정은 같은 판단 기록에서 모의 판단의 위험 곡선 계산과 같은 중심값을 낸다. | `saturn-terminal/core/src/routers/calibration/tests.rs`의 `recenter_matches_simulation_s1q_on_same_records`, `recenter_matches_simulation_t1_on_same_records` |
| 느린 조정은 되돌릴 수 없는 행동의 최저값 아래로 중심값을 내리지 않는다. | `saturn-terminal/core/src/routers/calibration/tests.rs`의 `recenter_keeps_center_within_irreversible_floor` |
| 전체 묻는 빈도는 판단 20번에 1번을 넘지 않는다. | 많은 판단을 흘려 물은 비율이 상한 안인지 확인한다. |
| 피드백, 취소, 실패 100건을 넣어도 활성 정책 지문, 기준값, 모델 버전이 바뀌지 않고 정책 교체 중 접수한 입력의 번호가 섞이지 않는다. | `saturn-terminal/engine/src/lifecycle/policy.rs`의 `feedback_cancel_and_failures_leave_the_active_policy_unchanged`, `inputs_keep_the_policy_they_were_accepted_under_across_swap_rollback_and_restart` |
| 판단 기록마다 router 버전, 기준값, q를 남기고 결과 신호와 물은 답은 생긴 뒤 같은 기록에 채운다. | `saturn-terminal/engine/src/store/outcomes.rs`의 `observations_carry_signal_answer_and_q_of_the_judgment`, `saturn-terminal/engine/src/lifecycle/outcomes.rs`의 `answer_feedback_records_answer_in_judgment`, `answer_feedback_request_is_answered_through_socket` |
| `/train`은 판단 기록으로 `Observation` 목록을 만들어 `recenter`에 넘긴다. | `saturn-terminal/engine/src/training/mod.rs`의 `recenter_thresholds_with_enough_recorded_results_moves_center`, `recenter_thresholds_below_min_results_keeps_center`, `recenter_thresholds_ignores_judgments_still_being_observed` |
| `/train`은 채점 안 된 판단이 200건 미만이면 실행하지 않는다. | 구현 전([#91](https://github.com/woonyong-choi/saturn/issues/91)). 199건에서 실행을 거절하고 200건에서 시작하는지 확인한다. |
| 모델 학습은 학습용 라벨 1,000건 이상, 평가용 라벨 200건 이상일 때만 한다. | 구현 전([#91](https://github.com/woonyong-choi/saturn/issues/91)). 학습용 999건에서 학습을 건너뛰고 채점과 기준값 조정만 하는지 확인한다. |
| 품질 게이트를 통과하지 못한 라벨은 학습용과 평가용에 들어가지 않는다. | 구현 전([#91](https://github.com/woonyong-choi/saturn/issues/91)). 순서를 바꾼 두 답이 다른 판단이 라벨에서 빠지는지 확인한다. |
| 새 모델은 같은 평가 세트에서 현재 모델보다 나쁘지 않을 때만 승격한다. | [#11](https://github.com/woonyong-choi/saturn/issues/11) 실험으로 후보별 정확도와 Brier를 확인한다. |
| 채점 흐름은 채점 모델 사이 일치도를 확인한 뒤 쓴다. | [#12](https://github.com/woonyong-choi/saturn/issues/12) 실험으로 채점 모델 사이 일치도를 확인한다. |
| 결과 신호는 사후 판정과 일치한다. | [#14](https://github.com/woonyong-choi/saturn/issues/14) 실험으로 신호별 일치 비율을 확인한다. |
| 200건이면 승격 게이트를 통과할 만큼 라벨이 모인다. | [#16](https://github.com/woonyong-choi/saturn/issues/16) 실험으로 사용자당 필요 라벨 수를 확인한다. |

## 단점

- 채점 모델이 판단마다 여러 번 풀어 `/train`마다 토큰 비용이 든다.
- 평가용 라벨 조건이 엄격해 평가 세트가 천천히 쌓인다.
- 사용자 반응이 없는 판단은 미확정이라 학습 신호가 되지 않는다.
- 느린 조정은 누적 기록으로 계산해 사용 방식이 급변하면 늦게 따라간다. 급변은 빠른 조정이 맡는다.
- 물은 답만 신호인 조건에서는 쓰인 결과가 300건에 못 미쳐 9,000건까지 조정하지 못하는 복제가 15%다.
- 한 번에 0.05까지만 움직여 중심값이 목표 값에 닿기까지 `/train`이 여러 번 필요하다. 급변은 빠른 조정이 맡는다.

## 미해결 질문

- Saturn 모델 학습을 공개 체크포인트에서 이어 학습할지, 항목별 yes/no 확률 방식으로 직접 학습할지 ([#43](https://github.com/woonyong-choi/saturn/issues/43))
- 기준 router와 채점 모델의 출력을 비교와 평가에만 쓸지, 허용된 범위에서 학습에도 쓸지 ([#45](https://github.com/woonyong-choi/saturn/issues/45))
- 학습 레코드에서 subagent 출력을 출처로 구분할지, 그 턴을 빼거나 구분 없이 둘지 ([#64](https://github.com/woonyong-choi/saturn/issues/64))
- 멈춤 명령이 진행 중인 학습도 멈출지, 학습 전용 중지를 둘지 ([#55](https://github.com/woonyong-choi/saturn/issues/55))
- 라벨 품질 게이트의 결과 신호 일치를 사후 판정으로 볼지, 원래 판단의 답과 결과 신호로 볼지 ([#105](https://github.com/woonyong-choi/saturn/issues/105))
