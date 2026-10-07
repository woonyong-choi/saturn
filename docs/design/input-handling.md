# 입력 처리

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [쓰기 에이전트는 기본으로 한 번에 하나만 실행한다](../decisions/2026-09-29-single-writer-default.md), [provider마다 입력을 계속 받는 상시 연결을 만든다](../decisions/2026-09-29-persistent-provider-connections.md), [반대 지시는 끼워 넣고, 끼워 넣을 수 없으면 사용자에게 멈출지 묻는다](../decisions/2026-10-03-conflict-steers-then-asks.md) |

## 요약

입력 처리는 사용자 입력을 기록 저장소에 먼저 접수하고, 채팅 상태에 맞춰 에이전트로 보내는 기능이다. 실행 중에 들어온 입력은 끼워 넣기, 새 작업, 대기 중 하나로 처리하고, 하던 작업과 반대되는 입력도 끼워 넣는다. 사용자가 멈춘 작업과 보내지 않은 입력은 보류로 두고, 사용자가 재개할 때만 이어 간다.

## 동기

Codex와 Claude Code를 함께 쓰는 개발자는 에이전트가 일하는 중에도 지시를 더하거나 다른 일을 맡긴다. 입력이 기록 없이 provider로 바로 가면 전달됐는지 알 수 없다. 전달 여부를 모르는 입력을 다시 보내면 같은 작업이 두 번 실행된다. 병렬 에이전트가 같은 작업 폴더에 쓰면 같은 파일을 동시에 고칠 수 있다. 이 기능은 모든 입력의 전달 상태를 기록으로 남기고, 멈춤과 재개와 쓰기 순서를 사용자 뜻대로 정한다.

## 예시

### 실행 중 지시를 더할 때

1. 사용자가 에이전트 A가 코드를 고치는 중에 "테스트도 같이 돌려줘"를 입력한다.
2. engine이 입력을 기록 저장소에 접수하고 설정 번호와 권한을 고정한다.
3. router가 이 입력을 하던 작업을 다듬는 입력으로 판단한다.
4. engine이 진행 중인 턴에 입력을 끼워 넣는다.
5. 사용자는 입력 상태가 `전달 중`에서 `반영됨`으로 바뀌는 것을 본다.

### 반대 지시를 할 때

1. 사용자가 에이전트 A가 jest로 테스트를 고치는 중에 "jest 말고 pytest로 해줘"를 입력한다.
2. router가 이 입력을 하던 작업과 반대되는 지시(`conflicts`)로 판단한다.
3. engine이 대기시키지 않고 진행 중인 턴에 입력을 끼워 넣는다. A는 다음 단계에서 입력을 읽고 방향을 바꾼다.
4. provider가 끼워 넣기를 받지 않으면 입력은 대기열 맨 앞에 두고 TUI가 `지금 멈추고 새 입력을 실행할까요?`를 묻는다.
5. 사용자가 `멈추고 실행`을 고르면 멈춤 규칙으로 A를 멈춘 뒤 그 입력을 실행하고, `대기`를 고르면 A가 끝난 뒤 다음 차례에 보낸다.

### 무관한 일을 맡길 때

1. 사용자가 에이전트 A가 파일을 고치는 중에 다른 기능의 수정을 입력한다.
2. router가 이 입력을 하던 일과 무관한 작업으로 판단한다.
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
3. 기본 수동 경로에서는 현재 작업에 대기시키고 사용자가 고른 모델을 쓴다. `router.mode`를 판단 방식으로 고른 경우에만 router가 입력의 관계와 처리 방식을 판단한다. 사용자가 모델을 고정한 입력은 `target_model`만 묻지 않는다.

- 입력은 기록 저장소에 접수된 뒤에만 에이전트로 보낸다. 전달 여부를 모르는 입력이 생기는 일을 막기 위해서다.
- 입력은 접수 때 고정한 설정 번호로 끝까지 처리한다. 처리 중 설정이 바뀌어도 한 입력을 한 설정으로 처리하기 위해서다.
- `router.mode = "manual"`에서는 키 확인, router 호출, 판단 기록 없이 현재 작업에 대기시킨다. 대상 작업을 명시한 입력은 해당 작업에 끼워 넣는다. 새 작업과 암묵적 제약은 자동으로 판단하지 않는다. 사용자가 provider를 바꾸면 다음 입력에 앞 대화의 원본 패킷을 붙인다.
- router에 묻는 질문과 기준값의 세부는 [router](router.md)에 있다.
- 대상 작업이 분명한 입력(허가를 거절하고 이어 쓴 말)은 `SubmitToTask`로 받아 관계 판단 없이 그 작업에 끼워 넣는다. router를 부르지 않으므로 다른 작업으로 갈 위험이 없다. 그 작업이 이미 끝났거나 없으면 보통 입력처럼 판단한다. 이 대상은 기록 저장소에 남지 않아 engine이 접수와 전송 사이에 죽으면 복원한 입력은 대상 없이 `skip_relation` 입력처럼 대기한다.
- router 호출이 실패하면 [router 실패](router.md#router-실패)의 재시도를 거친 뒤 판단 없이 현재 에이전트와 현재 모델로 보낸다. router 장애가 입력을 대기에 묶지 않게 하기 위해서다.

### 판단 차례와 적용

1. `queue`가 같은 채팅 입력에 ACK 순서대로 판단 차례를 준다.
2. `queue`가 판단 시점의 채팅 revision을 고정한다.
3. 채팅 revision은 작업 상태, 대기열 맨 앞, 마지막 판단으로 이루어진다.
4. router는 앞 입력의 판단 결과와 같은 채팅의 작업 맥락을 state에 넣은 요청 한 건으로 필요한 질문을 한 번에 판단한다.
5. `queue`가 적용 직전에 채팅 revision을 비교(CAS)한다.

- router 호출은 요청 처리와 별도 작업으로 돌고 결과는 engine 루프로 돌아와 적용한다. 호출이 도는 동안 멈춤, 취소 같은 다른 요청은 기다리지 않는다. 호출이 도는 사이 멈춤이나 취소로 입력이 더는 `판단 중`이 아니면 늦게 온 결과는 `superseded`로 기록하고 버린다.
6. revision이 같으면 `queue`가 끼워 넣기, 새 작업, 대기 중 하나를 적용한다.

- 같은 채팅의 입력은 접수 순서대로 하나씩 판단한다. 두 입력이 같은 상태를 보고 함께 끼워 넣어지는 일을 막기 위해서다.
- 적용 직전 revision이 다르면 한 번 다시 판단하고, 또 다르면 대기로 보낸다. 바뀐 상태에 옛 판단을 적용하는 일을 막기 위해서다.
- 같은 요청의 제약 질문(`is_constraint`, `constraint_change`)은 채팅 revision이 아니라 제약 revision과 입력 상태로 비교한다. 작업 상태가 바뀌었다는 이유로 사용자가 한 말의 제약 여부를 버리지 않기 위해서다([제약](constraints.md#적용-직전-비교)).
- 판단 기록은 적용 결과를 안 뒤에 쓴다. revision이 어긋나 버린 판단은 `superseded`로 쓰고, 적용한 판단만 결과 신호 관찰을 시작한다.
- 관계 판단 없이 대기하는 입력(`Tab`)은 router를 부르지 않고 대기로 둔다. 모델을 고정한 입력도 다른 입력과 똑같이 router 관계 판단(끼워 넣기, 대기, 새 작업)을 받는다. 고정은 `target_model` 선택만 대신하고, 보낼 모델은 고정값이다. 고정이 관계 판단을 건너뛰면 `/model` 뒤 모든 입력이 대기해 병렬 작업이 막히기 때문이다(사용자 결정).
- 입력은 접수 때 권한을 고정한다. 권한 모드가 읽기 전용(`read-only`)이면 읽기 전용, 그 밖의 모드는 쓰기다. 읽기 전용 모드여도 쓰기를 열 수 있는 규칙(`allow`, `ask`)이 하나라도 있으면 쓰기로 둔다. 읽기 전용 입력은 같은 폴더의 다른 읽기 작업과 병렬로 실행한다(사용자 결정, [#232](https://github.com/woonyong-choi/saturn/issues/232)). `allow`나 `ask` 규칙이 있을 때 쓰기로 두는 것은 쓰기를 막는 모드 기본 규칙을 규칙이 풀 수 있어서 둔 보수적 선택이다(초안). `ask`는 사용자가 승인한 쓰기가 쓰기 잠금 없이 나가는 일을 막으려고 같게 본다. `deny` 규칙만 있으면 읽기 전용이다.
- 판단 state는 채팅이 실행 중인지, 앞 입력의 처리 방식, 같은 채팅의 직전 입력, 메인 작업의 최초 목표와 최신 수정, 보류 작업의 번호와 목표, 사용자 원문으로 만들고 비밀값과 절대 경로를 뺀다. 칸과 크기 한도는 [판단 요청 맥락](router.md#판단-요청-맥락)이 정한다. 맥락은 요청을 만드는 순간의 채팅 상태에서 만들고, 적용 직전 revision이 다르면 새 상태로 다시 만든 요청으로 한 번 다시 판단한다. 최초 목표나 최신 수정을 온전히 담지 못하면 router를 부르지 않고 입력을 대기에 둔다.

### 실행 중 새 입력

router는 실행 중 입력과 하던 작업의 관계를 `refines`, `continues`, `independent`, `conflicts` 중 하나로 고른다. 그 결과로 `queue`가 처리 방식 하나를 적용한다.

| 처리 방식 | 동작 |
|---|---|
| 끼워 넣기 | 진행 중인 턴에 입력을 더한다. Codex는 `turn/steer`, Claude는 스트림 입력 추가로 전달한다. `refines`와 `continues`에서 router가 고르거나 `conflicts`일 때 쓴다. |
| 새 작업 | router가 무관한 작업으로 판단하면 같은 채팅 안에 보조 에이전트를 시작한다. |
| 대기 | 하던 작업 다음에 보낼 입력으로 대기열에 둔다. |

- 관계 판단의 확신도가 0.6 미만이면 대기로 보낸다.
- 쓰기 권한으로 접수한 입력은 읽기 전용으로 접수한 실행에 끼워 넣지 않고 대기한다([쓰기 규칙](#쓰기-규칙)). `refines`, `continues`, `conflicts`, 바로 보내기 모두 같다.
- 관계가 `conflicts`이면 `steer_or_spawn` 답과 관계없이 끼워 넣는다. 충돌 입력을 멈추지 않고 모델이 읽게 하는 것이 사용자 결정이다(아래 [충돌 입력](#충돌-입력)).
- 보내는 방식 판단의 확신도가 0.6 미만이면 현재 에이전트에 대기 뒤 보낸다. 같은 이유로 확신 없는 판단으로 행동하지 않기 위해서다.
- 끼워 넣기를 켠 provider만 끼워 넣고, 켜지 않은 provider는 끼워 넣기를 대기로 바꿔 처리한다. 지금은 Codex와 Claude 모두 켜지 않아(어댑터의 `STEER_VERIFIED`가 거짓) 끼워 넣기 대신 대기로 바뀐다. Codex `turn/steer`는 실측에서 받아들여졌으나([실측](../experiments/codex-provider-behavior/report.md), 거절 상태 목록은 [provider 연결과 session](providers-and-sessions.md)) 끼워 넣기 실패 경로 측정([#5](https://github.com/woonyong-choi/saturn/issues/5))이 끝나기 전에는 켜지 않는다.
- 그 provider에 끼워 넣기를 대기로 바꿀 때 TUI에 `바로 반영 준비 중`을 보인다. 충돌 입력은 이 표시 대신 멈출지 묻는다. 사용자가 바로 반영되지 않는 이유를 알게 하기 위해서다.
- provider가 끼워 넣은 입력을 받았는데 기록 저장소에 실행 연결과 `Applied`를 쓰지 못하면, 입력은 `Delivering`으로 두고 메모리 상태와 `InputChanged` 알림은 저장에 성공한 뒤에만 바꾼다. 그 사실은 `Alert::InputNotRecorded`로 알리고(TUI는 `입력은 전달됨 · 기록 저장이 늦어짐`), engine이 기록만 주기적으로 다시 쓴다. 이미 받은 입력이므로 provider에는 다시 보내지 않는다.
- provider마다 켜 둔 채 입력을 받는 연결을 둔다. 한 번 실행 방식으로는 끼워 넣기가 불가능하기 때문이다.
- 보조 에이전트는 끝나면 결과를 전달한 뒤 바로 종료한다. 쉬는 메인 에이전트를 깨우지 않고 메인 에이전트의 다음 입력 때 결과를 전달한다.

### 충돌 입력

1. router가 관계를 `conflicts`로 고르면 `queue`가 그 입력을 끼워 넣기로 적용하고 충돌 입력으로 표시한다.
2. 끼워 넣기가 받아들여지면 일반 끼워 넣기와 같다.
3. provider가 끼워 넣기를 받지 않으면(끼워 넣기를 켜지 않은 provider이거나 `NotSent`로 거절) `queue`가 입력을 대기열 맨 앞에 두고 멈출지 묻는 상태(`ConfirmStop`)로 바꾼다.
4. TUI가 `지금 멈추고 새 입력을 실행할까요?` 창을 띄우고 `대기`와 `멈추고 실행` 중 하나를 `AnswerStopConfirm`으로 보낸다.
5. `멈추고 실행`이면 engine이 멈춤 규칙으로 채팅을 멈추고, 완료를 확인한 뒤 그 입력이 붙은 작업을 재개해 입력을 실행한다. `대기`이면 입력은 맨 앞 대기 그대로 현재 작업이 끝난 뒤 다음 차례에 새 턴으로 간다.

- 모델이 입력을 읽고 방향을 바꾸는 것이 기본이고, 작업을 완전히 멈추는 것은 사용자의 멈춤(`Ctrl+C`)이다. 충돌 판단이 틀려도 멀쩡한 작업이 멈추는 일을 막기 위해서다(사용자 결정, [#36](https://github.com/woonyong-choi/saturn/issues/36)).
- engine은 사용자의 답 없이 멈추지 않는다. 묻는 동안 작업은 계속되고 입력은 맨 앞 대기와 같은 순서를 지킨다.
- 묻는 동안 현재 작업이 먼저 끝나면 입력은 다음 차례로 가서 질문은 사라진다. 다른 TUI가 먼저 답해도 질문은 사라지고 늦은 답은 거절한다.
- 질문은 입력마다 한 번만 한다. `대기`를 고른 입력을 바로 보내기로 다시 끼워 넣다가 거절되면 묻지 않고 맨 앞 대기로 둔다. `멈추고 실행`은 기존 멈춤 규칙이라 같은 채팅의 다른 대기 입력과 실행 중 작업도 보류하고, 입력이 붙은 작업의 보류 입력이 접수 순서대로 먼저 간다.
- 충돌이 아닌 입력은 이 질문을 거치지 않고 [입력 전송과 재전송](#입력-전송과-재전송)의 규칙(거절되면 맨 앞 대기)을 그대로 따른다. 확신도가 0.6 미만인 관계는 충돌로 보지 않고 대기로 둔다.
- 끼워 넣을 활성 턴이 없다는 실패(`NoActiveTurn`)는 질문 없이 같은 session의 새 턴으로 보내는 기존 규칙을 따른다. 멈출 턴이 없기 때문이다.

### 입력 전송과 재전송

1. `queue`가 대기열 맨 앞 입력에 쓰기 규칙을 적용한다.
2. `sessions`가 보내는 순간 대상 session을 정한다.
3. engine이 새 턴이면 Codex `turn/start`, Claude 스트림 입력으로 보낸다.
4. `queue`가 입력 상태를 `전달 중`, `반영됨` 순서로 바꾼다.

- 대기열은 에이전트 session이 아니라 채팅에 둔다. session 교체 중 들어온 입력이 옛 session을 가리키는 일을 막기 위해서다.
- session 교체는 턴이 끝난 경계에서만 한다. 진행 중인 턴이 session 교체로 끊기는 일을 막기 위해서다.
- session 교체 중 들어온 입력은 새 session에 순서대로 보낸다. 입력 순서를 session 교체와 무관하게 지키기 위해서다.
- 보내기 전에 확정된 실패만 다시 보낸다. 같은 작업이 두 번 실행되는 일을 막기 위해서다. 같은 입력은 처음 시도를 포함해 3번(초안)까지 보내고, 그래도 실패하면 `거절됨`으로 두고 시작하려던 작업은 닫는다. 끼워 넣기는 이 규칙을 따르지 않는다. provider가 끼워 넣기를 거절(`NotSent`)하면 다시 끼워 넣지 않고 입력을 대기열 맨 앞으로 옮겨 다음 차례에 새 턴으로 보낸다(사용자 결정, [#60](https://github.com/woonyong-choi/saturn/issues/60)). 사용자가 지금 반영되길 원한 입력이라 순서를 뒤로 미루지 않고, 거절은 보내지 않음이 확정된 실패라 다시 보내도 되기 때문이다. 입력은 `대기`로 돌아가고 `거절됨`이 되지 않는다. 충돌 입력만 이 자리에서 사용자에게 멈출지 묻는다([충돌 입력](#충돌-입력)).
- 입력은 `전달 중`을 기록 저장소에 쓴 뒤에만 provider로 보내고, provider가 받으면 `반영됨`으로 바꾼다. 기록에 쓰지 못하면 보내지 않고 거절한다.
- `전달 중`에서 보내지 않음이 확정되면 입력은 `대기`나 `보류`로 돌아간다. 끼워 넣기를 거절당하면 `대기`로 맨 앞에 두고, 멈춘 채팅의 전달이나 맥락 한도 초과로 패킷을 보내지 못하면 작업과 함께 `보류`로 둔다. 그 밖의 확정 실패는 3번까지 다시 보낸 뒤 `거절됨`이 된다.
- 보낼 provider는 입력에 고정한 모델이나 router가 고른 모델의 provider, 없으면 채팅의 메인 session이 있으면 그 provider이고, 없으면 설치된 Claude, 없으면 설치된 Codex이며 둘 다 없으면 오류를 보이고 보내지 않는다([#168](https://github.com/woonyong-choi/saturn/issues/168) 결정). 고정한 모델은 session을 여는 모델로 넘기고 모델이 바뀌면 새 메인 session을 연다([모델 고르기](providers-and-sessions.md#모델-고르기)).
- provider 연결은 채팅마다 둔다. 작업 폴더와 환경이 채팅마다 달라서다.
- 새 session에 넘기는 패킷을 provider가 맥락 한도 초과로 거절하면 다른 `NotSent`와 달리 같은 패킷을 다시 보내지 않는다. 경쟁 구역을 줄여 한 번만 다시 보내고, 그래도 거절되거나 고정 구역만으로 넘치면 보내지 않고 멈춘다([#162](https://github.com/woonyong-choi/saturn/issues/162), [패킷 구성](context-management.md#패킷-구성)). 이 재전송은 같은 입력 3번 규칙의 횟수에 들지 않는다.
- 보낸 뒤 결과가 불명인 입력은 자동으로 다시 보내지 않고 사용자 확인으로 넘긴다. 이미 반영된 입력을 두 번 실행하는 일을 막기 위해서다.
- 끼워 넣기와 대기 입력은 입력에 붙은 작업, 없으면 채팅의 메인 작업으로 보낸다. 메인 작업이 보류 중이면 새 작업을 메인으로 시작한다. 보류된 작업을 사용자 뜻 없이 이어 가지 않기 위해서다.
- 같은 채팅의 대기 입력은 앞 입력이 기다리면 함께 기다리고, 끼워 넣기만 실행 중인 턴에 바로 보낸다. 입력 순서를 쓰기 대기와 무관하게 지키기 위해서다.
- 앞 입력을 내준 뒤 전송 상태가 확정되기 전에도 뒤의 대기 입력은 기다린다. 앞 입력보다 먼저 실행되는 일을 막기 위해서다.

### 대기와 취소

- 대기는 보내기 전 채팅 대기열에 있는 입력의 상태다.
- 사용자는 대기 입력을 바로 보내거나 새 작업으로 보내거나 취소할 수 있다.
- 바로 보내기는 router를 부르지 않는다(사용자 결정). 실행 중인 작업에 끼워 넣기를 시도하고, 끼워 넣을 수 없으면(실행 중인 작업이 없거나 provider가 끼워 넣기를 아직 지원하지 않으면) 같은 채팅 대기열 맨 앞에 두어 다음 차례를 기다린다. 사용자가 지금 반영되길 원했으므로 앞선 대기 입력보다 먼저 가게 하기 위해서다. 판단이 끼워 넣기가 아니었던 입력을 바로 보내면 판단을 놓친 신호로 기록하고, 모델을 고정한 입력도 같다.
- 새 작업으로 보내기는 router 없이 새 작업으로 시작한다. 쓰기 규칙은 그대로 적용한다.
- 사용자가 판단을 뒤집은 것은 결과 신호로 남긴다. 취소는 `Wrong`, 대기로 판단한 입력을 바로 보내거나 새 작업으로 보내면 `Missed`다.
- 취소한 입력이 등록한 제약은 함께 해제한다([제약](constraints.md#식별-순서)).
- 취소는 에이전트에 보내기 전 입력에만 적용한다. `전달 중`과 `반영됨` 입력은 취소할 수 없다.
- TUI를 닫은 뒤 대기 입력을 계속 보내는 규칙은 [engine 수명](engine-lifecycle.md)에 있다.

### 멈춤과 보류

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/stop-flow.ko.dark.svg">
  <img src="../assets/stop-flow.ko.light.svg" alt="멈춤 요청은 작업을 보류로 바꾸고 subagent부터 멈춤 신호를 보내며, 트리 전체가 끝난 것을 확인한 뒤에만 완료를 보고한다" width="100%">
</picture>

1. 사용자가 멈춤을 요청한다.
2. `queue`가 실행 중 작업과 보내지 않은 대기 입력을 보류로 바꾼다.
3. `agents`가 추적된 subagent부터 멈춤 신호 대상 순서를 정한다.
4. engine이 Codex는 자식 session별 `turn/interrupt`, Claude는 interrupt 제어 요청으로 멈춤 신호를 보낸다.
5. 10초 뒤 남은 프로세스가 있으면 engine이 provider 프로세스 묶음에 중지 신호를 보낸다.
6. 그래도 남으면 engine이 강제 종료한다.
7. engine이 트리 전체의 종료를 확인한 뒤 완료를 보고한다.

- 설정 `tui.on_exit`의 `stop`과 종료 확인 창의 `멈추기`도 이 규칙을 그대로 쓴다. 작업이 있는 모든 채팅에 적용한다([engine 수명](engine-lifecycle.md#tui-종료-뒤-동작)).
- 멈춤은 Saturn session의 모든 에이전트와 subagent에 닿는다. 에이전트 하나만 멈추는 기능은 취소와 모델 교체 같은 내부 처리에서만 쓰기 때문이다.
- 멈춤은 트리 전체의 종료를 확인하기 전에는 완료라고 하지 않는다. subagent가 남은 채 멈췄다고 보이는 일을 막기 위해서다.
- 멈출 때 부모가 이미 답을 끝내고 subagent만 남은(백그라운드) 에이전트는 멈춘 뒤의 완료 신호를 기다리지 않는다. 그런 신호가 오지 않으므로 subagent 종료와 프로세스 묶음 중지만 확인한다. 그 사이 subagent가 끝나 트리가 유휴가 되면 멈춤은 그때 끝난다([#505](https://github.com/woonyong-choi/saturn/issues/505)).
- Codex 자식이 `turn/interrupt` 뒤에도 명령을 계속 돌리면 위 5단계의 10초 유예 뒤에 프로세스 묶음 중지로 끝난다. 실제 관측에서 `/stop` 10초 뒤에 남았다가 이어 사라진 것은 이 경로이고 설계대로다. 중지는 Codex의 공유 app-server 묶음에 자손 범위로 보낸다.
- 멈춘 작업은 자동으로 이어 가지 않고 보류하며, TUI가 없는 동안에도 보류를 그대로 둔다. 사용자가 멈춘 작업을 자동으로 이어 가지 않기 위해서다.
- 크래시 뒤 효과 범위가 증명되지 않은 실행도 보류가 된다. 그 판정은 [engine 수명](engine-lifecycle.md)에 있다.
- 보내지 않은 입력은 붙은 작업과 함께 보류한다. 붙은 작업이 없으면 새 작업 입력은 그 입력의 작업으로, 나머지는 메인 작업으로 묶는다. 대상이 있는 재개와 보류 종료를 작업 단위로 하기 위해서다.
- 멈춤 요청은 보류와 멈춤 신호를 보낸 뒤 바로 응답한다. 보류가 된 입력은 그때 바로 보이고, 작업의 보류 표시와 `멈춤` 줄은 완료를 확인한 뒤에 보인다. 멈추는 중에도 다른 요청과 이벤트를 처리하기 위해서다.
- 이미 멈추는 중인 채팅에 멈춤을 다시 요청하면 신호를 다시 보내지 않는다.
- provider 응답을 기다리는 전달(연결 시작, session 열기, 턴 시작)이 있어도 멈춤은 기다리지 않고 처리한다. 열기 전이면 그 입력을 보류하고 provider에 보내지 않으며, 턴 전송 중이면 입력을 받은 것으로 두고 작업을 멈춘 채로 둔다([provider 요청 작업](providers-and-sessions.md#provider-요청-작업)).
- 멈춘 에이전트의 session은 `보류`로 두고 provider에 열린 채 남긴다. 멈춘 작업을 다시 열지 않고 이어 가기 위해서다.
- 완료를 확인하는 조건은 [provider 연결과 session](providers-and-sessions.md#트리-전체-중지)에 있다.

### 재시작 뒤 입력 복원

`engine`을 다시 켜면(정상 종료와 크래시 모두) 시작 복구의 마지막에 기록 저장소에서 끝 상태가 아닌 입력을 접수 순서로 읽어 대기열에 되살린다. 접수만 하고 보내지 않은 입력을 잃지 않기 위해서다. 앞선 복구가 이미 되살린 입력은 건너뛴다.

| 기록의 상태 | 되살린 뒤 |
|---|---|
| `판단 중` | `판단 중`으로 되살려 접수 순서대로 다시 판단한다. 채팅에 보류 작업이 있으면 `보류`로 바꿔 둔다. |
| `대기` | `대기`로 되살려 쓰기 규칙에 따라 차례로 보낸다. 판단 때 정한 처리 방식은 저장하지 않으므로 `대기` 입력으로 본다. 채팅에 보류 작업이 있으면 `보류`로 바꿔 둔다. |
| `보류` | `보류`로 되살려 채팅의 보류 작업에 붙인다. 보류 작업이 없으면 그 입력을 첫 입력으로 하는 보류 작업을 연다. 처음 붙는 TUI에 `/continue`를 제안하고, `/continue`로만 보낸다. |
| `전달 중` | 보냈는지 모르므로 다시 보내지 않고 `전달 중`으로 둔다. 실행 기록이 남은 입력은 크래시 복구가 보류와 확인 입력으로 잇는다. |

- 채팅에 보류 작업이 있는데 보내지 않은 입력이 새 작업으로 앞서 나가면 멈춘 작업의 순서가 깨진다. 그래서 멈춤과 같게 그 입력들도 `보류`로 두고 사용자의 재개를 기다린다.
- 멈춤은 보류가 된 입력의 상태를 기록 저장소에 바로 쓴다. 다시 켠 뒤에도 사용자가 멈춘 입력이 대기로 돌아가 자동으로 나가지 않게 하기 위해서다.
- 복원한 채팅에 붙은 TUI가 없으면 `engine` 프로세스 환경으로 채팅 환경을 만든다. 붙는 TUI가 있으면 그 TUI의 환경으로 바뀐다.
- 복원한 입력 하나가 실패하면 경고를 남기고 나머지를 잇는다.

### 보류 재개와 보류 종료

1. 대상이 있는 재개 요청이면 `queue`가 그 작업만 재개한다.
2. 대상이 없는 재개 요청이면 `queue`가 채팅의 보류 전부를 접수 순서대로 재개한다.
3. `queue`는 재개한 작업을 쓰기 규칙에 따라 한 번에 하나씩 실행한다.
4. `sessions`가 확인된 상태로 만든 새 입력을 보낸다.
5. 새 입력의 `resume_held`가 0.85 이상이면 router 판단으로 보류 작업을 재개한다([#9](https://github.com/woonyong-choi/saturn/issues/9)).
6. `resume_held`가 0.85 미만이면 무시 횟수를 1 올린다. 판단이 없으면(router 장애, 응답 없음, 이 질문의 답 없음) 무시 횟수를 올리지 않는다. 보류 종료가 보내지 않은 입력을 취소하므로 장애 중에 사용자 모르게 닫히지 않게 하기 위해서다.
7. 재개 뜻이 없는 새 입력이 3개 쌓이거나 Saturn session이 끝나면 보류를 종료한다.
8. 보류 종료 때 `sessions`가 에이전트 session을 끝내고 보내지 않은 입력을 취소 처리한다.

- `resume_held`는 채팅에 보류 작업이 있을 때만 묻는다. 0.85 이상이면 채팅의 보류를 모두 재개하고 무시 횟수를 0으로 되돌리며, 0.85 미만이면 무시 횟수를 올려 3번째에 보류 작업을 모두 닫는다. 판단 없이 접수하는 입력(`skip_relation`)은 세지 않는다. 판단은 적용한 뒤에 반영하고, 반영하는 중에 확인 입력 접수가 다시 판단을 부르지 않게 하기 위해서다.
- 재개할 때 같은 패킷을 다시 보내지 않는다. 이미 반영된 입력을 두 번 실행하는 일을 막기 위해서다.
- 보류를 닫아도 기록은 지우지 않고 수정된 파일은 되돌리지 않는다. 사용자가 멈춘 작업의 결과를 사용자 뜻 없이 지우는 일을 막기 위해서다.
- 확인된 상태로 만든 새 입력은 멈춤 때 실행 중이던 작업에만 보낸다. 실행 전이던 작업은 보류 입력을 대기로 되돌리기만 한다. 끝난 턴을 다시 이어 붙이지 않기 위해서다.
- 새 입력은 멈춘 턴을 연 입력의 원문 앞에 그 턴의 결과를 오류 결과(`Previous turn result (error): Interrupted before a result was recorded · It may have partially run`)로 두는 글이다. 중단돼 결과를 모른다는 사실은 별도 경고나 상태 확인 지시 문장 없이 Claude Code, Codex가 도구 결과를 보이는 방식과 같게 결과 자리의 오류 결과 하나로만 넣는다([#282](https://github.com/woonyong-choi/saturn/issues/282)). 접수할 때 관계 판단 없이 그 작업에 대기로 붙이고, 쓰기 규칙을 그대로 적용한다.
- 같은 작업에 보류 입력이 있으면 접수 순서가 앞선 그 입력이 먼저 가고 새 입력은 그 턴이 끝난 뒤에 간다.
- 입력 하나를 재개하는 요청은 그 입력이 붙은 작업 전체를 재개한다. 같은 작업의 보류 입력은 접수 순서대로 모두 가고, 확인 입력은 그 뒤에 간다. 재개와 보류 종료를 작업 단위로 맞추고, 한 작업의 입력을 일부만 보내 순서가 어긋나거나 일부만 남지 않게 하기 위해서다.
- 보낸 뒤 결과를 모르는 작업은 `/continue`에 작업을 가리킬 때만 잇는다. 대상이 없는 재개가 결과를 모르는 작업을 건드리지 않는 것은 이미 반영됐을 수 있는 일을 사용자 뜻 없이 이어 가지 않기 위해서다. 잇는 방법은 멈춘 작업과 같은 확인 입력이고, 원래 입력은 `전달 중`으로 남기고 다시 보내지 않는다. 그 실행 기록은 닫는다. 원래 입력을 다시 보내면 이미 반영됐을 수 있는 일을 두 번 하기 때문이다.
- 보류 종료는 보내지 않은 입력을 취소하고, 그 작업의 에이전트 session을 provider에서 닫고 끝낸다. 기록과 수정된 파일은 그대로 둔다.

### 쓰기 규칙

`queue`는 접수 때 고정한 권한으로 쓰기 여부를 판정한다. 읽기 전용 권한의 작업은 같은 폴더에서 병렬로 실행한다. 쓰기 권한의 작업은 쓰기 범위가 겹치는 쓰는 에이전트가 있으면 대기한다. 쓰기 범위는 채팅의 작업 폴더와 더한 폴더이고, engine이 링크를 푼 경로로 만들어 접수 때 고정한다. 보내기 전에 폴더를 더하면 아직 보내지 않은 입력(보류 중 포함)의 범위를 다시 정한다([채팅 폴더](engine-lifecycle.md#채팅-폴더와-이어-열기)). 두 범위에서 경로 하나씩 골랐을 때 같거나 한쪽이 다른 쪽의 조상이면 겹친다. 이름의 앞부분만 같은 폴더는 겹치지 않는다. 멈춘 작업은 재개할 때까지 쓰기 잠금을 쥐고, 같은 채팅의 쓰기 입력이 기다리는 동안 뒤 입력도 함께 기다린다. 다만 앞 입력이 기다리는 잠금을 쥔 에이전트를 이어 갈 입력(보류 재개의 확인 입력)은 앞 입력보다 먼저 보낸다. 새 작업으로 판단된 보류 입력이 멈춘 작업의 잠금을 기다리는 사이 그 작업을 이어 갈 입력이 뒤에 막히면 서로 기다려 영원히 시작하지 못하기 때문이다.

| 규칙 | 이유 |
|---|---|
| 쓰기는 쓰기 범위가 겹치는 에이전트끼리 한 번에 하나만 한다. | 병렬 에이전트가 같은 작업 폴더나 상하위 폴더, 같은 더한 폴더에 쓰는 충돌을 막기 위해서다. |
| 쓰기 범위는 작업 폴더와 더한 폴더를 링크를 푼 경로로 비교한다. | 허가 판정이 경로를 푸는 방식과 맞춰, 링크로 같은 폴더를 가리켜도 잠금을 피하지 못하게 하기 위해서다. 폴더가 없으면 푸는 대신 적은 그대로 비교한다. |
| 쓰기 규칙 판정은 접수 때 고정된 권한으로 한다. | 쓰기 여부를 추정 없이 결정론으로 판정하기 위해서다. |
| 읽기 전용으로 접수한 실행은 `/permissions`로 모드를 올려도 쓰기 허가를 받지 못한다. 쓰려면 새 입력으로 보내야 한다. | 쓰기 잠금 없이 도는 실행이 다른 쓰기 작업과 같은 폴더에 함께 쓰는 일을 막기 위해서다([권한](permissions.md#권한-규칙)). 거부는 알림으로 보인다. |
| 쓰기 권한 입력은 읽기 전용으로 접수한 실행에 끼워 넣지 않고 `쓰기 차례` 대기로 둔다. 그 실행이 끝나면 쓰기 잠금을 얻어 새 턴으로 보낸다. | 쓰기 잠금 없이 도는 실행에 쓰기 입력을 끼워 넣으면 그 입력이 잠금 없이 쓰기 때문이다. 끼워 넣기가 활성 턴 없음으로 실패해 같은 session의 새 턴으로 갈 때도 같다. 대기 이유는 입력 줄의 `쓰기 차례`로 보인다. |
| 파일 겹침 예측으로 병렬 쓰기를 허용하지 않는다. | 겹침 예측은 추정이라 충돌을 막지 못하기 때문이다. |
| 쓰기 잠금은 트리 유휴일 때 푼다. | subagent가 쓰는 중에 다음 쓰기가 시작되는 일을 막기 위해서다. |
| worktree 설정을 켜면 git 저장소일 때만 별도 worktree와 브랜치에서 병렬로 쓴다. | 쓰기 격리를 git이 맡기 위해서다. |
| worktree 결과는 변경 요약을 보인 뒤 사용자 확인으로만 합친다. | 자동 병합으로 변경을 잃는 일을 막기 위해서다. |
| worktree 결과를 합칠 때 git에 없는 파일은 누락을 경고한다. | git이 옮기지 않는 파일을 모른 채 잃는 일을 막기 위해서다. |

### 입력 전달 상태

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/input-states.ko.dark.svg">
  <img src="../assets/input-states.ko.light.svg" alt="입력은 판단 중으로 시작해 대기와 전달 중을 거쳐 반영됨이 되고, 멈추면 보류로 가며, 끼워 넣기를 거절당하면 대기로 돌아오고, 거절됨이나 취소됨으로 끝날 수도 있다" width="100%">
</picture>

| 상태 | 뜻 | 다음 상태 |
|---|---|---|
| `판단 중` | router 답을 기다리는 입력 | `대기`, `전달 중`, `보류`, `취소됨` |
| `대기` | 보내기 전 대기열의 입력 | `전달 중`, `보류`, `취소됨` |
| `전달 중` | 에이전트에 보낸 입력 | `반영됨`, `거절됨`, `대기`(끼워 넣기 거절), `보류`(보내지 않음이 확정) |
| `반영됨` | 에이전트가 받은 입력 | 없음 |
| `거절됨` | 에이전트가 받지 않은 입력 | 없음 |
| `보류` | 사용자가 멈춘 작업의 보내지 않은 입력 | `대기`, `취소됨` |
| `취소됨` | 보내기 전에 취소한 입력 | 없음 |

### 오류 처리

| 상황 | 동작 |
|---|---|
| router 호출이 재시도 뒤에도 실패 | 모델 선택과 처리 방식 판단을 건너뛰고 현재 에이전트와 현재 모델로 보낸다. 입력을 대기로 보내지 않는다. |
| 적용 직전 revision 불일치 | 한 번 다시 판단하고, 또 어긋나면 대기로 보낸다. |
| 판단 중 revision 변경 | 판단을 `superseded`로 기록한다. |
| router 호출 연속 3회 실패 | 새 입력 접수를 계속하고 상태판에 `판단 모델 연결 끊김`을 보인다. |
| Codex `turn/steer`가 활성 턴 없음으로 실패 | 확정 미전달로 기록하고 다시 판단하지 않고 같은 session에 `turn/start`로 보낸다. |
| Claude 끼워 넣기 중 턴 종료 | provider가 추가 메시지를 다음 턴에 처리하므로 따로 처리하지 않는다. |
| 보낸 뒤 결과 불명 | 자동으로 다시 보내지 않고 사용자 확인으로 넘긴다. 입력은 `전달 중`으로 두고 작업을 `결과 확인 필요`로 보이며, 실행 기록은 열어 둔다. 사용자는 `/continue <작업>`으로 확인 입력을 보내 잇는다. |
| 패킷의 고정 구역이 `P_send`를 넘어 새 session으로 옮기지 못하는 경우 | 보내지 않고 입력을 작업과 함께 보류하며 제약 목록을 보인다. `/continue`로 다시 시도한다. |
| 패킷이 맥락 한도 초과로 거절되고 줄인 패킷도 거절되거나 줄일 수 없음 | 보내지 않고 입력을 작업과 함께 보류하며 `맥락 한도 초과로 멈춤 · /continue로 다시 시도하세요`를 보인다. `/continue`로 다시 시도한다. |
| 보내기 전 확정 실패가 3번 이어짐 | 입력을 `거절됨`으로 두고 작업을 실패로 보인다. 입력 에코에 빨간색 `거절됨`이 붙고 마지막 실패의 이유가 빨간색으로 보인다. |
| 도구 실행의 결과를 모른 채 작업이 실패하거나 멈추거나 결과 확인 필요가 됨 | 그 도구 셀에 빨간색 `중단됨`을 보인다. 이어 갈 때 provider에 넘기는 기록에는 그 도구 결과를 오류 결과(`Interrupted before a result was recorded · It may have partially run`)로 넣는다. |
| provider가 끼워 넣기를 거절(`NotSent`) | 다시 끼워 넣지 않고 입력을 `대기`로 되돌려 대기열 맨 앞에 둔다. 현재 작업이 끝나면 다음 차례에 새 턴으로 보낸다. |
| 충돌 입력을 provider가 끼워 넣기 거절이나 끼워 넣기를 켜지 않아 받지 않음 | 입력을 대기열 맨 앞에 두고 `지금 멈추고 새 입력을 실행할까요?`를 묻는다. 답 전까지 멈추지 않는다. |
| 멈춤 확인에 답했는데 이미 답했거나 다음 차례로 간 입력 | 요청을 거절하고(`INVALID_PARAMS`) 상태는 바꾸지 않는다. |
| provider 연결이나 session 열기 실패, 설치된 provider 없음 | 보내지 않고 입력을 `거절됨`으로 두며 원인 한 줄을 작업 실패에 보인다. |
| 다시 켠 뒤 기록 저장소에서 입력 하나를 되살리지 못함 | 경고를 남기고 나머지 입력을 되살린다. 그 입력은 기록에 남아 다음 시작 때 다시 시도한다. |
| 접수나 `전달 중` 기록 실패 | 어디에도 보내지 않는다. 접수 실패는 요청 오류로, `전달 중` 실패는 입력 거절로 알린다. |
| 멈춤 뒤 묶음 밖으로 빠져나간 프로세스 존재 | 완료라고 하지 않고 `멈춤 확인 안 됨 · N개 남음`을 보고한다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 입력은 기록 저장소에 접수된 뒤에만 에이전트로 보낸다. | `saturn-terminal/engine/src/lifecycle/intake.rs`의 `record_write_failure_sends_nothing_anywhere`, `accepted_input_reaches_first_provider_after_it_is_recorded` |
| 끼워 넣기가 활성 턴 없음으로 실패하면 다시 판단하지 않고 같은 session의 새 턴으로 보낸다. | `saturn-terminal/engine/src/lifecycle/deliver.rs`의 `steer_without_active_turn_sends_one_new_turn_without_rerouting`, `steer_new_turn_unknown_is_not_sent_again` |
| 보낸 뒤 결과가 불명인 입력은 자동으로 다시 보내지 않는다. 끼워 넣기의 결과가 불명이어도 다시 끼워 넣거나 새 턴으로 보내거나 대기로 되돌리지 않는다. | `saturn-terminal/engine/src/lifecycle/deliver.rs`의 `unknown_is_never_sent_again_and_the_task_needs_check`, `saturn-terminal/engine/src/lifecycle/steer_rejected.rs`의 `steer_with_an_unknown_result_is_not_sent_again_or_requeued` |
| 결과를 모르는 도구 실행은 `중단됨`으로 보이고 이어 가는 기록에 오류 결과로 들어간다. | `saturn-terminal/tui/src/view/transcript.rs`의 `interrupted_tool_shows_a_red_interrupted_line`, `saturn-terminal/tui/src/app/tests.rs`의 `interrupted_tool_is_marked_when_the_task_is_held_without_a_result`, `saturn-terminal/engine/src/handoff.rs`의 `interrupted_tool_call_is_an_error_result_not_a_warning`, `waiting_held_and_interrupted_inputs_are_open_items` |
| 거절된 입력은 빨간색 `거절됨`으로 보인다. | `saturn-terminal/tui/src/view/transcript.rs`의 `rejected_input_shows_a_red_rejected_badge`, `failed_cause_is_red` |
| 보류 입력 하나를 재개하면 그 작업의 보류 입력을 접수 순서대로 모두 재개한다. | `saturn-terminal/engine/src/lifecycle/stop.rs`의 `continue_input_resumes_the_task_of_that_input`, `continue_sends_held_input_and_then_a_state_check_for_the_interrupted_task` |
| 결과를 모르는 작업은 `/continue <작업>`으로만 확인 입력을 보내 잇고 원래 입력은 다시 보내지 않는다. | `saturn-terminal/engine/src/lifecycle/stop.rs`의 `task_with_unknown_result_is_continued_only_when_named` |
| 보내기 전에 확정된 실패만 다시 보낸다. | `saturn-terminal/engine/src/lifecycle/deliver.rs`의 `not_sent_is_sent_again_and_then_applied`, `not_sent_every_time_is_rejected_and_the_next_input_still_goes`, `open_failure_that_is_not_a_resend_case_rejects_without_sending` |
| 판단 요청에 직전 입력, 최초 목표, 최신 수정, 보류 작업의 번호와 목표를 싣고, 맥락이 불완전하면 판단 없이 대기한다. | `saturn-terminal/engine/src/lifecycle/judge_context.rs`의 `same_follow_up_carries_the_goal_of_each_conversation`, `held_tasks_are_listed_with_their_ids_and_goals`, `context_over_the_limit_is_not_judged_and_waits_for_the_user` |
| 같은 채팅의 입력은 접수 순서대로 하나씩 판단한다. | `saturn-terminal/engine/src/lifecycle/intake.rs`의 `inputs_are_routed_one_at_a_time_in_accept_order` |
| router 호출이 재시도 뒤에도 실패하면 입력을 대기로 보내지 않고 현재 에이전트와 현재 모델로 보내며, 현재 모델이 없는 첫 입력은 기본 모델로 보낸다. | `saturn-terminal/core/src/routers/failure.rs`의 `route_after_failure_keeps_the_current_agent_and_model`, `saturn-terminal/engine/src/lifecycle/model_mode.rs`의 `default_model_is_used_when_every_router_judgment_fails`, `manual_mode_uses_the_default_model_when_every_router_judgment_fails`, `failed_judgment_keeps_the_current_model_of_an_open_main_instead_of_the_default` |
| 판단 뒤 채팅 상태가 바뀌었으면 한 번 다시 판단하고, 또 바뀌면 대기로 둔다. | `saturn-terminal/engine/src/lifecycle/decision.rs`의 `revision_conflict_supersedes_old_judgment_and_reroutes_once`, `second_conflict_puts_input_in_queue_without_another_router_call` |
| router 호출이 도는 동안에도 다른 요청을 바로 처리하고, 판단 중 멈춤이나 취소가 있으면 늦게 온 판단은 적용하지 않는다. | `saturn-terminal/engine/src/lifecycle/deliver.rs`의 `requests_are_answered_while_a_judgment_is_in_flight`, `saturn-terminal/engine/src/lifecycle/decision.rs`의 `stop_while_judging_holds_the_input_and_drops_the_late_judgment` |
| 쓰기 권한 에이전트는 같은 작업 폴더에서 한 번에 하나만 실행한다. | `saturn-terminal/engine/src/lifecycle/decision.rs`의 `relation_answer_to_new_task_waits_for_the_write_turn_then_starts` |
| 상하위 작업 폴더나 같은 더한 폴더를 가진 채팅의 쓰기 작업도 한 번에 하나만 실행하고, 이름의 앞부분만 같은 폴더는 따로 실행한다. 링크를 푼 경로로 비교한다. | `saturn-terminal/engine/src/lifecycle/write_scope.rs`의 `write_in_a_subfolder_waits_for_a_write_in_the_parent_folder`, `write_in_the_parent_folder_waits_for_a_write_in_a_subfolder`, `writes_with_a_shared_added_folder_wait_for_each_other`, `write_in_a_folder_that_only_shares_a_name_prefix_runs_at_once`, `write_in_a_symlink_to_a_subfolder_waits_for_a_write_in_the_parent_folder`, `saturn-terminal/core/src/queue/tests.rs`의 `try_acquire_overlapping_scopes_returns_false`, `try_acquire_shared_added_folder_returns_false` |
| 취소는 에이전트에 보내기 전 입력에만 적용한다. | `saturn-terminal/engine/src/lifecycle/deliver.rs`의 `cancel_applies_only_before_the_input_is_sent` |
| 쓰기 권한 입력은 읽기 전용으로 접수한 실행에 끼워 넣지 않고 쓰기 차례로 기다린다. | `saturn-terminal/engine/src/lifecycle/read_only_steer.rs`의 `write_input_is_not_steered_into_a_read_only_run_and_waits_for_a_write_turn`, `waiting_write_input_takes_a_new_turn_after_the_read_only_run_ends`, `write_input_that_would_be_sent_as_a_new_turn_cannot_write_during_a_read_only_run` |
| 바로 보내기는 router를 부르지 않고 끼워 넣기를 시도하며, 안 되면 대기열 맨 앞에 둔다. | `saturn-terminal/engine/src/lifecycle/send_now.rs`의 `send_now_steers_into_the_running_turn_without_calling_the_router`, `send_now_that_cannot_steer_goes_first_in_the_queue`, `send_now_without_verified_steer_goes_back_to_waiting`, `send_now_on_a_sent_input_is_refused`, `saturn-terminal/core/src/queue/tests.rs`의 `send_now_steers_a_running_task_ahead_of_earlier_waiting_inputs`, `send_now_that_cannot_steer_waits_first_in_line`, `send_now_refuses_inputs_that_are_not_waiting` |
| 충돌로 판단한 입력은 대기시키지 않고 진행 중인 턴에 끼워 넣는다. | `saturn-terminal/core/src/routers/tests.rs`의 `decide_route_conflicts_steers_and_marks_the_conflict`, `saturn-terminal/engine/src/lifecycle/conflict_steer.rs`의 `conflict_input_is_steered_into_the_running_turn` |
| `SubmitToTask`는 router를 부르지 않고 이름 붙은 작업에 끼워 넣고, 없는 작업이면 보통 입력처럼 판단한다. | `saturn-terminal/engine/src/lifecycle/decision.rs`의 `submit_to_task_steers_the_named_task_without_the_router`, `submit_to_a_task_that_is_gone_gets_the_relation_judgment` |
| 충돌 입력을 provider가 받지 않으면 멈추지 않고 사용자에게 멈출지 묻고, 충돌이 아닌 입력은 묻지 않는다. | `saturn-terminal/engine/src/lifecycle/conflict_steer.rs`의 `refused_conflict_steer_asks_whether_to_stop_and_does_not_stop`, `conflict_steer_to_a_provider_without_steer_asks_whether_to_stop`, `saturn-terminal/core/src/queue/tests.rs`의 `refused_steer_asks_the_user_only_for_a_conflict_input`, `deferred_steer_asks_the_user_only_for_a_conflict_input`, `saturn-terminal/engine/src/lifecycle/steer_rejected.rs`의 `refused_steer_is_not_sent_again_and_is_not_rejected` |
| `대기`를 고르면 입력은 맨 앞 대기로 다음 차례에 가고, `멈추고 실행`을 고르면 기존 멈춤 규칙 뒤에 입력을 실행한다. | `saturn-terminal/engine/src/lifecycle/conflict_steer.rs`의 `answering_wait_keeps_the_input_in_front_for_the_next_turn`, `answering_stop_stops_the_chat_and_then_runs_the_input`, `saturn-terminal/core/src/queue/tests.rs`의 `keep_waiting_ends_the_question_and_the_input_takes_the_next_turn`, `stop_ends_the_question_and_a_second_answer_is_refused` |
| 묻는 중이 아닌 입력의 멈춤 확인 답은 거절한다. | `saturn-terminal/engine/src/lifecycle/conflict_steer.rs`의 `answering_without_a_question_is_refused` |
| provider가 거절한 끼워 넣기는 다시 끼워 넣지 않고 대기열 맨 앞으로 옮겨 다음 차례에 보낸다. 턴이 끝난 뒤에 거절이나 수락 응답이 늦게 와도 같은 입력을 한 번만 보낸다. | `saturn-terminal/engine/src/lifecycle/steer_rejected.rs`의 `refused_steer_is_not_sent_again_and_is_not_rejected`, `refused_steer_goes_to_the_front_and_takes_the_next_turn`, `refused_steer_through_send_now_also_goes_to_the_front`, `refused_steer_answered_after_the_turn_ended_still_takes_one_turn`, `accepted_steer_answered_after_the_turn_ended_is_not_sent_again`, `saturn-terminal/core/src/queue/tests.rs`의 `refused_steer_returns_to_the_front_as_a_queued_input`, `refused_steer_needs_a_delivering_input` |
| 다시 켠 `engine`은 보내지 않은 입력을 접수 순서대로 되살린다. `판단 중`은 다시 판단하고 `대기`는 대기로 보내며, 보류 작업이 있는 채팅의 입력은 보류로 두고 `전달 중`은 다시 보내지 않는다. | `saturn-terminal/engine/src/lifecycle/restore_inputs.rs`의 `waiting_inputs_are_sent_in_accept_order_after_a_clean_restart`, `judging_input_is_judged_again_and_sent_after_a_restart`, `restored_inputs_of_one_chat_are_judged_one_at_a_time_in_accept_order`, `unsent_inputs_are_held_with_the_crashed_task_and_resume_in_order`, `delivering_input_is_never_resent_after_restart` |
| 멈춤으로 보류한 입력은 다시 켠 뒤에도 보류로 남아 `/continue`로만 보낸다. | `saturn-terminal/engine/src/lifecycle/restore_inputs.rs`의 `held_input_stays_held_after_restart_until_continue` |
| 멈춘 작업은 자동으로 이어 가지 않고 보류한다. | `saturn-terminal/engine/src/lifecycle/stop.rs`의 `stopped_work_is_not_continued_without_a_request`, `stop_with_nothing_running_holds_the_waiting_input_at_once`, `stop_twice_signals_once` |
| 재개는 보류 입력을 접수 순서로 보내고 멈춘 작업에는 중단 결과를 붙인 새 입력을 보낸다. | `saturn-terminal/engine/src/lifecycle/stop.rs`의 `continue_sends_held_input_and_then_a_state_check_for_the_interrupted_task`, `continue_input_resumes_the_task_of_that_input` |
| 보류 종료는 보내지 않은 입력을 취소하고 session을 끝낸다. | `saturn-terminal/engine/src/lifecycle/stop.rs`의 `close_held_cancels_unsent_input_and_ends_the_session` |
| 결과를 모르는 작업은 가리킬 때만 확인 입력으로 잇는다. | `saturn-terminal/engine/src/lifecycle/stop.rs`의 `task_with_unknown_result_is_continued_only_when_named` |
| 멈춤 신호는 추적된 subagent까지 보낸다. | `saturn-terminal/engine/src/lifecycle/stop.rs`의 `stop_signals_the_deepest_subagent_first_and_finishes_only_when_the_tree_is_idle` |
| 멈춤 신호 10초 뒤 남은 프로세스 묶음에는 중지 신호를 보낸다. | `saturn-terminal/engine/src/processes/mod.rs`의 `stop_sends_term_after_grace` |
| 트리 유휴와 프로세스 중지를 모두 확인한 뒤에만 멈춤 완료를 보고한다. 부모가 먼저 답한 백그라운드 subagent도 끝나면 완료한다. | `saturn-terminal/engine/src/lifecycle/stop.rs`의 `stop_after_the_parent_answered_finishes_when_the_background_subagent_ends`, `stop_is_not_complete_until_the_process_group_is_confirmed_stopped`, `stop_without_a_finished_turn_signal_is_not_complete`, `processes_left_outside_the_group_are_reported_instead_of_done` |
| 끼워 넣기와 멈춤 신호는 provider별 경로로 전달된다. | Codex는 [실측](../experiments/codex-provider-behavior/report.md)으로 경로와 불가 상태를 확인했다. Claude는 [#5](https://github.com/woonyong-choi/saturn/issues/5) 실험으로 확인한다. |
| 판단이 없는 입력은 무시 횟수를 올리지 않아 보류를 닫지 않는다. | `saturn-terminal/engine/src/lifecycle/intake.rs`의 `inputs_without_a_resume_judgment_never_close_the_held_work` |
| 보류 작업이 있는 채팅의 새 입력만 `resume_held`를 묻고, 0.85 이상이면 보류를 재개하고 0.85 미만이 3번 쌓이면 보류를 닫는다. | `saturn-terminal/engine/src/lifecycle/intake.rs`의 `resume_held_is_asked_only_when_the_chat_has_held_work`, `resume_intent_at_threshold_resumes_every_held_task`, `resume_intent_below_threshold_keeps_the_work_held_and_counts_the_input`, `third_input_without_resume_intent_closes_the_held_work` |
| `resume_held` 기준값 0.85는 보류 작업을 잘못 재개하지 않는다. | [#9](https://github.com/woonyong-choi/saturn/issues/9) 실험으로 오탐 비율을 확인한다. |

## 대안

- 파일 겹침을 예측해 병렬로 쓰는 방식은 버렸다([쓰기 에이전트는 기본으로 한 번에 하나만 실행한다](../decisions/2026-09-29-single-writer-default.md)).

## 미해결 질문

- 판단 요청 맥락을 실제 router로 쓴 오접합과 이어 가기 누락의 정확도. 맥락 구성은 정했고 구현했다([판단 요청 맥락](router.md#판단-요청-맥락), [#459](https://github.com/woonyong-choi/saturn/issues/459)). 기존 측정은 실험용 상태 형식으로 쟀고 구현한 형식과 같지 않다([한국어 이어 가기 실험](../experiments/continuation-judgment-korean/report.md), [오접합 실험](../experiments/continuation-misjoin/report.md), [새 작업 표본 확대](../experiments/continuation-newtask/report.md), [#6](https://github.com/woonyong-choi/saturn/issues/6)) 합친 새 작업 표본의 오접합은 2.0%이고 재현율은 71.3%였다. 구현한 상태 형식으로 두 값을 따로 다시 재는 일과 구현 방식(Jev 또는 저렴한 LLM)은 [#382](https://github.com/woonyong-choi/saturn/issues/382)의 비교 실험 뒤에 정한다.

- 멈춘 작업의 트리 유휴 신호가 끝내 오지 않을 때 완료 보고를 기다리는 한도 ([#464](https://github.com/woonyong-choi/saturn/issues/464))
