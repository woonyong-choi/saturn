# 입력 처리

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [쓰기 에이전트는 기본으로 한 번에 하나만 실행한다](../decisions/2026-09-29-single-writer-default.md), [provider마다 입력을 계속 받는 상시 연결을 만든다](../decisions/2026-09-29-persistent-provider-connections.md) |

## 요약

입력 처리는 사용자 입력을 기록 저장소에 먼저 접수하고, 채팅 상태에 맞춰 에이전트로 보내는 기능이다. 실행 중에 들어온 입력은 끼워 넣기, 새 작업, 대기 중 하나로 처리한다. 사용자가 멈춘 작업과 보내지 않은 입력은 보류로 두고, 사용자가 재개할 때만 이어 간다.

## 동기

Codex와 Claude Code를 함께 쓰는 개발자는 에이전트가 일하는 중에도 지시를 더하거나 다른 일을 맡긴다. 입력이 기록 없이 provider로 바로 가면 전달됐는지 알 수 없다. 전달 여부를 모르는 입력을 다시 보내면 같은 작업이 두 번 실행된다. 병렬 에이전트가 같은 작업 폴더에 쓰면 같은 파일을 동시에 고칠 수 있다. 이 기능은 모든 입력의 전달 상태를 기록으로 남기고, 멈춤과 재개와 쓰기 순서를 사용자 뜻대로 정한다.

## 예시

### 실행 중 지시를 더할 때

1. 사용자가 에이전트 A가 코드를 고치는 중에 "테스트도 같이 돌려줘"를 입력한다.
2. engine이 입력을 기록 저장소에 접수하고 설정 번호와 권한을 고정한다.
3. judge가 이 입력을 하던 작업을 다듬는 입력으로 판단한다.
4. engine이 진행 중인 턴에 입력을 끼워 넣는다.
5. 사용자는 입력 상태가 `전달 중`에서 `반영됨`으로 바뀌는 것을 본다.

### 무관한 일을 맡길 때

1. 사용자가 에이전트 A가 파일을 고치는 중에 다른 기능의 수정을 입력한다.
2. judge가 이 입력을 하던 일과 무관한 작업으로 판단한다.
3. `queue`가 같은 채팅 안에 보조 에이전트를 새 작업 B로 시작하려 한다.
4. 두 작업 모두 쓰기 권한이므로 B는 A의 트리 유휴까지 대기한다.
5. A와 그 아래 모든 subagent가 끝나면 쓰기 잠금이 풀리고 B가 실행된다.

### 멈춘 뒤 다시 이어 갈 때

1. 사용자가 작업 A가 실행 중이고 입력 C가 대기인 상태에서 멈춤을 요청한다.
2. `core`가 A와 C를 보류로 바꾸고, engine이 추적된 subagent부터 멈춤 신호를 보낸다.
3. engine은 트리 전체의 종료를 확인한 뒤에만 멈춤 완료를 보고한다.
4. 사용자가 대상 없이 `/continue`를 실행한다.
5. `queue`가 채팅의 보류 전부를 접수 순서대로 쓰기 규칙에 따라 하나씩 재개한다.

## 상세 설계

### 입력 접수

1. engine이 입력을 기록 저장소에 접수(ACK)한다.
2. `queue`가 접수 때 그 입력의 설정 번호와 권한을 고정한다.
3. 사용자가 모델을 고정한 입력이면 `queue`는 judge 호출을 생략한다.
4. 그 밖의 입력은 judge가 뜻을 판단한 뒤 처리 방식을 정한다.

- 입력은 기록 저장소에 접수된 뒤에만 에이전트로 보낸다. 전달 여부를 모르는 입력이 생기는 일을 막기 위해서다.
- 입력은 접수 때 고정한 설정 번호로 끝까지 처리한다. 처리 중 설정이 바뀌어도 한 입력을 한 설정으로 처리하기 위해서다.
- judge에 묻는 질문과 기준값의 세부는 [judge](judge.md)에 있다.
- judge 호출이 실패하면 [judge 실패](judge.md#judge-실패)의 재시도를 거친 뒤 판단 없이 현재 에이전트와 현재 모델로 보낸다. judge 장애가 입력을 대기에 묶지 않게 하기 위해서다.

### 판단 차례와 적용

1. `queue`가 같은 채팅 입력에 ACK 순서대로 판단 차례를 준다.
2. `queue`가 판단 시점의 채팅 revision을 고정한다.
3. 채팅 revision은 작업 상태, 대기열 맨 앞, 마지막 판단으로 이루어진다.
4. judge는 앞 입력의 판단 결과를 state에 넣은 요청 한 건으로 필요한 질문을 한 번에 판단한다.
5. `queue`가 적용 직전에 채팅 revision을 비교(CAS)한다.
6. revision이 같으면 `queue`가 끼워 넣기, 새 작업, 대기 중 하나를 적용한다.

- 같은 채팅의 입력은 접수 순서대로 하나씩 판단한다. 두 입력이 같은 상태를 보고 함께 끼워 넣어지는 일을 막기 위해서다.
- 적용 직전 revision이 다르면 한 번 다시 판단하고, 또 다르면 대기로 보낸다. 바뀐 상태에 옛 판단을 적용하는 일을 막기 위해서다.
- 판단 기록은 적용 결과를 안 뒤에 쓴다. revision이 어긋나 버린 판단은 `superseded`로 쓰고, 적용한 판단만 결과 신호 관찰을 시작한다.
- 모델을 고정한 입력과 관계 판단 없이 대기하는 입력(`Tab`)은 judge를 부르지 않고 대기로 둔다(초안). 모델을 고정한 입력이 실행 중에 도착했을 때의 처리 방식은 정해지기 전이라 대기다([#168](https://github.com/woonyong-choi/saturn/issues/168)).
- 입력은 접수 때 권한을 쓰기로 고정한다. 권한 규칙이 구현되기 전에는 병렬 쓰기가 생기지 않는 쪽으로 둔다(초안, [#232](https://github.com/woonyong-choi/saturn/issues/232)).
- 판단 state는 채팅이 실행 중인지, 앞 입력의 처리 방식, 사용자 원문으로 만들고 비밀값과 절대 경로를 뺀다(초안).

### 실행 중 새 입력

judge는 실행 중 입력과 하던 작업의 관계를 `refines`, `continues`, `independent`, `conflicts` 중 하나로 고른다. 그 결과로 `queue`가 처리 방식 하나를 적용한다.

| 처리 방식 | 동작 |
|---|---|
| 끼워 넣기 | 진행 중인 턴에 입력을 더한다. Codex는 `turn/steer`, Claude는 스트림 입력 추가로 전달한다. |
| 새 작업 | judge가 무관한 작업으로 판단하면 같은 채팅 안에 보조 에이전트를 시작한다. |
| 대기 | 하던 작업 다음에 보낼 입력으로 대기열에 둔다. |

- 관계 판단의 확신도가 0.6 미만이면 대기로 보낸다.
- 보내는 방식 판단의 확신도가 0.6 미만이면 현재 에이전트에 대기 뒤 보낸다. 같은 이유로 확신 없는 판단으로 행동하지 않기 위해서다.
- 끼워 넣기 실측을 통과하기 전의 provider는 끼워 넣기를 대기로 바꿔 처리한다([#5](https://github.com/woonyong-choi/saturn/issues/5), [#27](https://github.com/woonyong-choi/saturn/issues/27)). 끼워 넣기 경로가 문서대로 동작하는지 실측으로 확인해야 하기 때문이다.
- 그 provider에 끼워 넣기를 대기로 바꿀 때 TUI에 `바로 반영: 준비 중`을 보인다. 사용자가 바로 반영되지 않는 이유를 알게 하기 위해서다.
- provider마다 켜 둔 채 입력을 받는 연결을 둔다. 한 번 실행 방식으로는 끼워 넣기가 불가능하기 때문이다.
- 보조 에이전트는 끝나면 결과를 전달한 뒤 바로 종료한다. 쉬는 메인 에이전트를 깨우지 않고 메인 에이전트의 다음 입력 때 결과를 전달한다.

### 입력 전송과 재전송

1. `queue`가 대기열 맨 앞 입력에 쓰기 규칙을 적용한다.
2. `sessions`가 보내는 순간 대상 session을 정한다.
3. engine이 새 턴이면 Codex `turn/start`, Claude 스트림 입력으로 보낸다.
4. `queue`가 입력 상태를 `전달 중`, `반영됨` 순서로 바꾼다.

- 대기열은 에이전트 session이 아니라 채팅에 둔다. session 교체 중 들어온 입력이 옛 session을 가리키는 일을 막기 위해서다.
- session 교체는 턴이 끝난 경계에서만 한다. 진행 중인 턴이 session 교체로 끊기는 일을 막기 위해서다.
- session 교체 중 들어온 입력은 새 session에 순서대로 보낸다. 입력 순서를 session 교체와 무관하게 지키기 위해서다.
- 보내기 전에 확정된 실패만 다시 보낸다. 같은 작업이 두 번 실행되는 일을 막기 위해서다. 같은 입력은 처음 시도를 포함해 3번(초안)까지 보내고, 그래도 실패하면 `거절됨`으로 두고 시작하려던 작업은 닫는다.
- 입력은 `전달 중`을 기록 저장소에 쓴 뒤에만 provider로 보내고, provider가 받으면 `반영됨`으로 바꾼다. 기록에 쓰지 못하면 보내지 않고 거절한다.
- 보낼 provider는 채팅의 메인 session이 있으면 그 provider이고, 없으면 설치된 Claude, 없으면 설치된 Codex이며 둘 다 없으면 오류를 보이고 보내지 않는다([#168](https://github.com/woonyong-choi/saturn/issues/168) 결정). 모델을 고정한 입력의 모델은 그대로 넘기고 모델에서 provider로 가는 대응은 정해지기 전이다.
- provider 연결은 채팅마다 둔다. 작업 폴더와 환경이 채팅마다 달라서다.
- 맥락 한도 초과로 provider가 거절한 입력은 정해지기 전이라 다른 `NotSent`와 같이 처리한다([#162](https://github.com/woonyong-choi/saturn/issues/162)).
- 보낸 뒤 결과가 불명인 입력은 자동으로 다시 보내지 않고 사용자 확인으로 넘긴다. 이미 반영된 입력을 두 번 실행하는 일을 막기 위해서다.
- 끼워 넣기와 대기 입력은 입력에 붙은 작업, 없으면 채팅의 메인 작업으로 보낸다. 메인 작업이 보류 중이면 새 작업을 메인으로 시작한다. 보류된 작업을 사용자 뜻 없이 이어 가지 않기 위해서다.
- 같은 채팅의 대기 입력은 앞 입력이 기다리면 함께 기다리고, 끼워 넣기만 실행 중인 턴에 바로 보낸다. 입력 순서를 쓰기 대기와 무관하게 지키기 위해서다.
- 앞 입력을 내준 뒤 전송 상태가 확정되기 전에도 뒤의 대기 입력은 기다린다. 앞 입력보다 먼저 실행되는 일을 막기 위해서다.

### 대기와 취소

- 대기는 보내기 전 채팅 대기열에 있는 입력의 상태다.
- 사용자는 대기 입력을 바로 보내거나 새 작업으로 보내거나 취소할 수 있다.
- 바로 보내기는 judge에 한 번 물어 끼워 넣기나 새 작업이면 그 처리 방식으로 바꾸고, 대기로 답하면 차례를 기다린다. 판단하지 못하면 `판단기 연결 없음 · 차례에 보냅니다`를 보이고 차례를 기다린다. 모델을 고정한 입력은 물을 것이 없어 차례를 기다린다.
- 새 작업으로 보내기는 judge 없이 새 작업으로 시작한다. 쓰기 규칙은 그대로 적용한다.
- 사용자가 판단을 뒤집은 것은 결과 신호로 남긴다. 취소는 `Wrong`, 대기로 판단한 입력을 바로 보내거나 새 작업으로 보내면 `Missed`다.
- 취소는 에이전트에 보내기 전 입력에만 적용한다. `전달 중`과 `반영됨` 입력은 취소할 수 없다.
- TUI를 닫은 뒤 대기 입력을 계속 보내는 규칙은 [engine 수명](engine-lifecycle.md)에 있다.

### 멈춤과 보류

1. 사용자가 멈춤을 요청한다.
2. `queue`가 실행 중 작업과 보내지 않은 대기 입력을 보류로 바꾼다.
3. `agents`가 추적된 subagent부터 멈춤 신호 대상 순서를 정한다.
4. engine이 Codex는 자식 session별 `turn/interrupt`, Claude는 interrupt 제어 요청으로 멈춤 신호를 보낸다.
5. 10초 뒤 남은 프로세스가 있으면 engine이 provider 프로세스 묶음에 중지 신호를 보낸다.
6. 그래도 남으면 engine이 강제 종료한다.
7. engine이 트리 전체의 종료를 확인한 뒤 완료를 보고한다.

- 멈춤은 Saturn session의 모든 에이전트와 subagent에 닿는다. 에이전트 하나만 멈추는 기능은 취소와 모델 교체 같은 내부 처리에서만 쓰기 때문이다.
- 멈춤은 트리 전체의 종료를 확인하기 전에는 완료라고 하지 않는다. subagent가 남은 채 멈췄다고 보이는 일을 막기 위해서다.
- 멈춘 작업은 자동으로 이어 가지 않고 보류하며, TUI가 없는 동안에도 보류를 그대로 둔다. 사용자가 멈춘 작업을 자동으로 이어 가지 않기 위해서다.
- 크래시 뒤 효과 범위가 증명되지 않은 실행도 보류가 된다. 그 판정은 [engine 수명](engine-lifecycle.md)에 있다.
- 보내지 않은 입력은 붙은 작업과 함께 보류한다. 붙은 작업이 없으면 새 작업 입력은 그 입력의 작업으로, 나머지는 메인 작업으로 묶는다. 대상이 있는 재개와 보류 종료를 작업 단위로 하기 위해서다.

### 보류 재개와 보류 종료

1. 대상이 있는 재개 요청이면 `queue`가 그 작업만 재개한다.
2. 대상이 없는 재개 요청이면 `queue`가 채팅의 보류 전부를 접수 순서대로 재개한다.
3. `queue`는 재개한 작업을 쓰기 규칙에 따라 한 번에 하나씩 실행한다.
4. `sessions`가 확인된 상태로 만든 새 입력을 보낸다.
5. 새 입력의 `resume_held`가 0.85 이상이면 judge 판단으로 보류 작업을 재개한다([#9](https://github.com/woonyong-choi/saturn/issues/9)).
6. `resume_held`가 0.85 미만이면 무시 횟수를 1 올린다.
7. 재개 뜻이 없는 새 입력이 3개 쌓이거나 Saturn session이 끝나면 보류를 종료한다.
8. 보류 종료 때 `sessions`가 에이전트 session을 끝내고 보내지 않은 입력을 취소 처리한다.

- 재개할 때 같은 패킷을 다시 보내지 않는다. 이미 반영된 입력을 두 번 실행하는 일을 막기 위해서다.
- 보류를 닫아도 기록은 지우지 않고 수정된 파일은 되돌리지 않는다. 사용자가 멈춘 작업의 결과를 사용자 뜻 없이 지우는 일을 막기 위해서다.
- 확인된 상태로 만든 새 입력은 멈춤 때 실행 중이던 작업에만 보낸다. 실행 전이던 작업은 보류 입력을 대기로 되돌리기만 한다. 끝난 턴을 다시 이어 붙이지 않기 위해서다.

### 쓰기 규칙

`queue`는 접수 때 고정한 권한으로 쓰기 여부를 판정한다. 읽기 전용 권한의 작업은 같은 폴더에서 병렬로 실행한다. 쓰기 권한의 작업은 쓰는 에이전트가 있으면 대기한다.

| 규칙 | 이유 |
|---|---|
| 쓰기는 한 번에 한 에이전트만 한다. | 병렬 에이전트가 같은 작업 폴더에 쓰는 충돌을 막기 위해서다. |
| 쓰기 규칙 판정은 접수 때 고정된 권한으로 한다. | 쓰기 여부를 추정 없이 결정론으로 판정하기 위해서다. |
| 파일 겹침 예측으로 병렬 쓰기를 허용하지 않는다. | 겹침 예측은 추정이라 충돌을 막지 못하기 때문이다. |
| 쓰기 잠금은 트리 유휴일 때 푼다. | subagent가 쓰는 중에 다음 쓰기가 시작되는 일을 막기 위해서다. |
| worktree 설정을 켜면 git 저장소일 때만 별도 worktree와 브랜치에서 병렬로 쓴다. | 쓰기 격리를 git이 맡기 위해서다. |
| worktree 결과는 변경 요약을 보인 뒤 사용자 확인으로만 합친다. | 자동 병합으로 변경을 잃는 일을 막기 위해서다. |
| worktree 결과를 합칠 때 git에 없는 파일은 누락을 경고한다. | git이 옮기지 않는 파일을 모른 채 잃는 일을 막기 위해서다. |

### 입력 전달 상태

| 상태 | 뜻 | 다음 상태 |
|---|---|---|
| `판단 중` | judge 답을 기다리는 입력 | `대기`, `전달 중`, `보류`, `취소됨` |
| `대기` | 보내기 전 대기열의 입력 | `전달 중`, `보류`, `취소됨` |
| `전달 중` | 에이전트에 보낸 입력 | `반영됨`, `거절됨` |
| `반영됨` | 에이전트가 받은 입력 | 없음 |
| `거절됨` | 에이전트가 받지 않은 입력 | 없음 |
| `보류` | 사용자가 멈춘 작업의 보내지 않은 입력 | `대기`, `취소됨` |
| `취소됨` | 보내기 전에 취소한 입력 | 없음 |

### 오류 처리

| 상황 | 동작 |
|---|---|
| judge 호출이 재시도 뒤에도 실패 | 모델 선택과 처리 방식 판단을 건너뛰고 현재 에이전트와 현재 모델로 보낸다. 입력을 대기로 보내지 않는다. |
| 적용 직전 revision 불일치 | 한 번 다시 판단하고, 또 어긋나면 대기로 보낸다. |
| 판단 중 revision 변경 | 판단을 `superseded`로 기록한다. |
| judge 호출 연속 3회 실패 | 새 입력 접수를 계속하고 상태판에 `판단 모델 연결 끊김`을 보인다. |
| Codex `turn/steer`가 활성 턴 없음으로 실패 | 확정 미전달로 기록하고 다시 판단하지 않고 같은 session에 `turn/start`로 보낸다. |
| Claude 끼워 넣기 중 턴 종료 | provider가 추가 메시지를 다음 턴에 처리하므로 따로 처리하지 않는다. |
| 보낸 뒤 결과 불명 | 자동으로 다시 보내지 않고 사용자 확인으로 넘긴다. 입력은 `전달 중`으로 두고 작업을 `결과 확인 필요`로 보이며, 실행 기록은 열어 둔다. |
| 보내기 전 확정 실패가 3번 이어짐 | 입력을 `거절됨`으로 두고 작업을 실패로 보인다. 끼워 넣기가 거절된 입력의 다음 처리는 [#60](https://github.com/woonyong-choi/saturn/issues/60)에서 정한다. |
| provider 연결이나 session 열기 실패, 설치된 provider 없음 | 보내지 않고 입력을 `거절됨`으로 두며 원인 한 줄을 작업 실패에 보인다. |
| 접수나 `전달 중` 기록 실패 | 어디에도 보내지 않는다. 접수 실패는 요청 오류로, `전달 중` 실패는 입력 거절로 알린다. |
| 멈춤 뒤 묶음 밖으로 빠져나간 프로세스 존재 | 완료라고 하지 않고 `멈춤 확인 안 됨 · N개 남음`을 보고한다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 입력은 기록 저장소에 접수된 뒤에만 에이전트로 보낸다. | `saturn-terminal/engine/src/lifecycle/intake.rs`의 `record_write_failure_sends_nothing_anywhere`, `accepted_input_reaches_first_provider_after_it_is_recorded` |
| 끼워 넣기가 활성 턴 없음으로 실패하면 다시 판단하지 않고 같은 session의 새 턴으로 보낸다. | `saturn-terminal/engine/src/lifecycle/deliver.rs`의 `steer_without_active_turn_sends_one_new_turn_without_rejudging`, `steer_new_turn_unknown_is_not_sent_again` |
| 보낸 뒤 결과가 불명인 입력은 자동으로 다시 보내지 않는다. | `saturn-terminal/engine/src/lifecycle/deliver.rs`의 `unknown_is_never_sent_again_and_the_task_needs_check` |
| 보내기 전에 확정된 실패만 다시 보낸다. | `saturn-terminal/engine/src/lifecycle/deliver.rs`의 `not_sent_is_sent_again_and_then_applied`, `not_sent_every_time_is_rejected_and_the_next_input_still_goes`, `open_failure_that_is_not_a_resend_case_rejects_without_sending` |
| 같은 채팅의 입력은 접수 순서대로 하나씩 판단한다. | `saturn-terminal/engine/src/lifecycle/intake.rs`의 `inputs_are_judged_one_at_a_time_in_accept_order` |
| judge 호출이 재시도 뒤에도 실패하면 입력을 대기로 보내지 않고 현재 에이전트와 현재 모델로 보낸다. | `saturn-terminal/core/src/judges/failure.rs`의 `route_after_failure_idle_sends_to_current_agent_and_model`, `route_after_failure_running_steers_instead_of_queueing` |
| 판단 뒤 채팅 상태가 바뀌었으면 한 번 다시 판단하고, 또 바뀌면 대기로 둔다. | `saturn-terminal/engine/src/lifecycle/decision.rs`의 `revision_conflict_supersedes_old_judgment_and_rejudges_once`, `second_conflict_puts_input_in_queue_without_another_judge_call` |
| 쓰기 권한 에이전트는 같은 작업 폴더에서 한 번에 하나만 실행한다. | `saturn-terminal/engine/src/lifecycle/decision.rs`의 `relation_answer_to_new_task_waits_for_the_write_turn_then_starts` |
| 취소는 에이전트에 보내기 전 입력에만 적용한다. | `saturn-terminal/engine/src/lifecycle/deliver.rs`의 `cancel_applies_only_before_the_input_is_sent` |
| 바로 보내기는 judge에 한 번 묻고, 판단하지 못하면 차례를 기다린다. | `saturn-terminal/engine/src/lifecycle/deliver.rs`의 `send_now_asks_the_judge_once_and_steers_into_the_running_turn`, `send_now_with_judge_down_leaves_the_input_waiting_in_order` |
| 멈춘 작업은 자동으로 이어 가지 않고 보류한다. | 멈춤 뒤 재개 요청 없이는 보류 작업이 실행되지 않는지 확인한다. |
| 멈춤 신호는 추적된 subagent까지 보낸다. | 멈춤 요청 뒤 추적된 subagent마다 멈춤 신호가 가는지 확인한다. |
| 멈춤 신호 10초 뒤 남은 프로세스 묶음에는 중지 신호를 보낸다. | 멈춤 신호를 무시하는 프로세스에 10초 뒤 중지 신호가 가는지 확인한다. |
| 끼워 넣기와 멈춤 신호는 provider별 경로로 전달된다. | [#5](https://github.com/woonyong-choi/saturn/issues/5)와 [#27](https://github.com/woonyong-choi/saturn/issues/27) 실험으로 경로와 불가 상태를 확인한다. |
| `resume_held` 기준값 0.85는 보류 작업을 잘못 재개하지 않는다. | [#9](https://github.com/woonyong-choi/saturn/issues/9) 실험으로 오탐 비율을 확인한다. |

## 대안

- 파일 겹침을 예측해 병렬로 쓰는 방식은 버렸다([쓰기 에이전트는 기본으로 한 번에 하나만 실행한다](../decisions/2026-09-29-single-writer-default.md)).

## 미해결 질문

- 실행 중 작업을 뒤집는 입력을 바로 멈출지, 사용자에게 확인할지, 대기로 둘지 ([#36](https://github.com/woonyong-choi/saturn/issues/36))
- provider가 끼워 넣기를 거절한 입력을 대기로 옮길지, 다시 판단할지, 사용자에게 물을지 ([#60](https://github.com/woonyong-choi/saturn/issues/60))
- 허가 거절 뒤 다르게 하라는 입력을 판단 없이 끼워 넣을지, 허가 창에서 받을지, 일반 입력으로 판단할지 ([#56](https://github.com/woonyong-choi/saturn/issues/56))
- 멈춤 명령이 진행 중인 학습도 멈출지, 학습 전용 중지를 둘지 ([#55](https://github.com/woonyong-choi/saturn/issues/55))
