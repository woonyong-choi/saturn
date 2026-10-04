# 하위 접속

| 항목 | 값 |
|---|---|
| 상태 | 구현 |
| 관련 결정 | [하위 접속은 새 engine 대신 떠 있는 engine에 출입증으로 붙는다](../decisions/2026-10-04-child-sessions-via-engine-pass.md), [engine만 router를 부르고 자식 프로세스 환경에서 router 키를 지운다](../decisions/2026-09-29-engine-as-router-proxy.md), [쓰기 에이전트는 기본으로 한 번에 하나만 실행한다](../decisions/2026-09-29-single-writer-default.md) |

## 요약

Saturn 안의 에이전트가 `saturn`을 실행해 다른 일을 맡기는 것과, Saturn 밖의 Claude나 Codex가 `saturn`을 실행해 일을 맡기는 것을 모두 허용한다. 어느 쪽도 새 `engine`을 띄우지 않고 사용자당 하나인 `engine`에 접속해 부탁만 한다. `engine`이 router 키를 쥔 채 대신 일하는 프록시라서 키는 누구에게도 넘어가지 않는다. Saturn 안에서 온 접속(하위 접속)은 `engine`이 준 출입증으로 식별한다. 출입증은 그 채팅의 하위 작업만 만들 수 있고, 부모 권한을 넘지 못하고, 깊이와 동시 수 상한이 있고, 부모가 멈추면 함께 끝난다. 출입증이 없는 접속(바깥 접속)은 사용자가 연 채팅과 같은 권한 규칙을 따른다.

## 동기

`saturn`은 지금까지 에이전트 작업 안에서 실행하면 거절했다. 에이전트가 Saturn을 쓰고 싶어도 쓸 수 없고, 거절을 풀어 두면 에이전트가 `engine`을 또 띄우거나 키 저장소를 건드리거나 부모보다 큰 권한으로 일을 시킬 수 있다. `engine`은 사용자당 하나여야 기록 저장소의 쓰는 쪽이 하나로 유지되므로 접속마다 `engine`을 띄우는 방식은 쓸 수 없다. 접속이 늘면 provider 프로세스도 같이 늘어 사용자 컴퓨터가 느려지므로 상한이 필요하다.

## 예시

### 에이전트가 하위 작업을 맡길 때

1. 사용자가 채팅 A에서 `리팩터링하고 테스트까지 돌려` 를 보내 Claude가 일을 시작한다.
2. Claude가 셸에서 `echo "테스트만 돌려" | saturn`을 실행한다. 프로세스 환경에는 `engine`이 넣은 `SATURN_PASS`가 있다.
3. `saturn`은 `engine`을 띄우지 않고 소켓에 붙어 출입증으로 하위 접속을 요청한다.
4. `engine`이 채팅 A의 작업 폴더를 물려받은 새 채팅 B를 만들고, 채팅 A의 작업 트리에 하위 에이전트로 올린다. A의 TUI에는 하위 에이전트가 하나 늘어난 것으로 보인다.
5. B의 결과 줄이 `saturn`의 표준 출력으로 나오고 Claude가 셸 명령의 결과로 읽는다.
6. `saturn`이 끝나면 B의 접속이 끊겨 하위 에이전트가 트리에서 내려간다.

### 하위 접속이 부모보다 큰 권한을 요청할 때

1. 채팅 A의 권한 모드는 `edit`이다.
2. Claude가 `saturn --mode full`을 실행한다.
3. `engine`이 `requested mode is above the parent mode edit`로 거절한다. 채팅과 동시 자리는 만들어지지 않는다.

### 동시 상한을 넘을 때

1. 채팅 A의 하위 접속이 이미 5개 돌고 있다.
2. 에이전트가 여섯 번째 `saturn`을 실행한다.
3. `engine`이 대기열에 세우고 `saturn`은 자리가 날 때까지 기다린다. 오류가 아니다.
4. 하위 접속 하나가 끝나면 대기열 맨 앞 요청이 이어서 붙는다.

### 부모를 멈출 때

1. 사용자가 채팅 A에서 멈춤을 보낸다.
2. `engine`이 A의 하위 접속을 먼저 모두 끝낸다. 하위 작업을 멈추고, 출입증을 회수하고, 그 채팅의 provider 연결을 닫는다. 기다리던 요청은 거절한다.
3. 그 뒤 A 자신을 멈춘다. A에 하위 에이전트가 남아 있지 않으므로 트리 유휴 확인이 바로 끝난다.

### 바깥에서 부를 때

1. 사용자가 터미널에서 돌고 있는 다른 Claude Code에게 Saturn을 쓰라고 한다.
2. 그 Claude가 `echo "..." | saturn`을 실행한다. 환경에 출입증이 없다.
3. `saturn`은 사용자가 터미널에서 직접 연 것과 같은 새 채팅을 만든다. 권한은 Saturn 설정의 규칙을 그대로 따르고, 묻기로 판정된 호출은 TUI가 붙을 때까지 기다린다.

## 상세 설계

### 접속 두 종류

| 종류 | 식별 | 채팅 | 권한 |
|---|---|---|---|
| 하위 접속 | 환경 변수 `SATURN_PASS`의 출입증 | 출입증을 준 채팅의 하위 채팅 | 부모 모드를 넘지 못한다. 묻기는 거부한다. |
| 바깥 접속 | 없음 | 새 채팅 | 사용자가 연 채팅과 같다. |

- `saturn`은 `SATURN_PASS`가 있으면 하위 접속, 없고 `SATURN_AGENT`도 없으면 바깥 접속이다. `SATURN_AGENT`만 있고 출입증이 없으면(회수됐거나 만들지 못했을 때) 거절한다.
- 하위 접속은 항상 plain 방식이다. 표준 입력의 줄이 입력이 되고 결과 줄이 표준 출력으로 나온다. `--continue`, `--resume`, `--add-dir`, `-c`, 하위 명령은 쓸 수 없다. 작업 폴더, 더한 폴더, 환경, 실행 층은 `engine`이 부모에게서 물려주기 때문이다.
- 하위 접속은 `engine`이 떠 있지 않으면 새로 띄우지 않고 오류로 끝난다. `engine`도 에이전트 작업 안에서는 시작을 거절한다.
- `engine`의 소켓 경로는 환경 변수 `SATURN_ENGINE_SOCKET`으로 알려 준다. 없으면 기본 경로다.

### 출입증

- `engine`이 provider 연결을 시작할 때 채팅마다 출입증 하나를 만들어 provider 프로세스 환경에 `SATURN_PASS`로 넣는다. 토큰은 운영체제 난수 32바이트를 16진수로 쓴 `saturn-pass-` 글자이고 메모리에만 둔다.
- 토큰은 router 키가 아니다. 쓸 수 있는 일은 그 채팅의 하위 접속을 여는 것 하나다.
- 범위는 채팅이다. 토큰은 채팅 하나에 묶이고 그 채팅의 하위 채팅만 만든다. 어느 작업의 하위인지는 요청 때 부모 채팅에서 실행 중인 작업 가운데 가장 먼저 시작한 작업으로 정한다. 실행 중인 작업이 없으면 거절한다.
- 하위 채팅에도 자기 출입증이 생겨 provider 환경에 들어간다. 부모 토큰은 하위 환경에 들어가지 않는다. 깊이 상한 안에서 하위 채팅이 다시 하위 접속을 열 수 있다.
- 만료는 시계가 아니라 수명으로 정한다. 하위 채팅의 출입증은 그 접속이 끝나거나 회수되면 사라지고, 채팅의 출입증은 그 채팅의 provider 연결이 모두 닫히거나 `engine`이 끝나면 사라진다. 부모가 멈추면 하위 출입증은 회수되고 부모 자신의 출입증은 남는다.
- 토큰 글자는 로그, 오류, 디버그 출력에 나오지 않는다. `Masker`가 `saturn-pass-`로 시작하는 16진수 글자를 키와 같은 방식으로 가리고, 요청 `Debug`는 내용을 보이지 않으며, 거절 응답은 토큰을 담지 않는다.

### 상한과 대기열

| 상한 | 뜻 | 설정 키 | 기본값 |
|---|---|---|---|
| 깊이 | 사용자가 연 채팅이 0, 그 하위가 1 | `child.max_depth` | 2 |
| 채팅당 동시 수 | 한 채팅이 동시에 거느리는 하위 접속 | `child.max_concurrent` | 5 |
| 전체 동시 수 | `engine` 전체의 하위 접속 | `child.max_total` | 10 |

- 기본값은 [부하 시험](#부하-시험)으로 정했다. 키는 사용자 전용이라 폴더 설정이 올릴 수 없다([설정](settings.md#폴더-층에서-바꿀-수-없는-항목)).
- 깊이를 넘으면 거절한다. 동시 수를 넘으면 거절하지 않고 대기열 끝에 세운다. 대기 순서는 요청 순서이고, 자리가 나면 대기열 앞에서부터 자리가 있는 요청을 허용한다. 한 부모의 상한에 막힌 요청은 건너뛰어 다른 부모의 요청을 막지 않는다.
- 기다리는 동안 `ChildQueued { position }`을 한 번 보낸다. 응답은 자리가 날 때 한다.
- 기다리던 연결이 끊기거나 부모가 멈추면 대기열에서 빠진다. 부모가 멈춘 경우에는 거절 응답을 받는다.
- 하위 접속이 끝나면 자리를 돌려준다. 하위 채팅의 provider 연결은 그때 바로 닫는다. 상한이 provider 프로세스 수를 막으려는 것이라 평소의 5분 유예를 두지 않는다.

### 하위 채팅 만들기

1. 연결 작업이 요청을 읽어 출입증과 상한을 확인한다(아래 [요청 병렬 처리](#요청-병렬-처리)). 거절이면 여기서 응답한다.
2. 허용된 요청만 요청 처리 루프에 `Child` 이벤트로 온다.
3. `engine`이 부모에 실행 중인 작업이 있는지, 요청 모드가 부모의 현재 모드를 넘지 않는지 다시 확인한다.
4. 부모의 작업 폴더, 더한 폴더, 환경(키 변수와 출입증 변수를 뺀 것), 채팅 층(권한 규칙)과 부모 TUI의 `-c` 값을 물려받는 새 채팅을 만들고 채팅 층의 모드를 요청 모드로 쓴다. 요청 모드가 없으면 부모 모드다.
5. 출입증을 묶고, 접속을 그 채팅에 붙이고(`Attach`와 같은 알림), 부모 작업의 하위 에이전트로 트리에 올린다. 이벤트 `SubagentStarted`의 이름은 `saturn-child-{채팅 번호}`다.
6. 접속이 끊기거나 부모가 멈추거나 부모 연결이 닫히면 `SubagentEnded`를 올려 내리고, 하위 채팅의 작업을 멈추고, provider 연결을 닫는다.

하위 채팅은 보통 채팅이라 기록 저장소에 남고 `saturn --resume`으로 볼 수 있다. 부모와의 연결은 메모리에만 있어 `engine`을 다시 켜면 하위 채팅은 독립 채팅이 된다.

### 결과 돌려주기

하위 결과는 `saturn`의 표준 출력이다. 부모 에이전트는 그것을 셸 명령의 결과로 읽는다. 에이전트끼리 메시지를 주고받는 길은 만들지 않으며, 맥락 전달을 기록 번호 하나로 맞추는 원칙([아키텍처](../architecture.md#불변-조건))은 그대로다. 하위 작업은 부모 작업 아래 하위 에이전트로 보이고 하위 채팅의 기록은 따로 남는다.

### 권한 상속

- 하위 채팅의 모드는 부모 쪽으로 올라가며 가장 낮은 모드다. 판정할 때마다 계산하므로 부모가 `/permissions`로 모드를 낮추거나 설정 파일이 바뀌면 하위 채팅도 다음 판정부터 따라간다.
- 요청 모드가 부모 모드보다 크면 거절한다. 하위 채팅이 `SetPermissionMode`로 부모 모드보다 큰 모드를 고르는 것도 거절한다. 부모 모드 이하는 고를 수 있다.
- 권한 규칙(`permission.*`)은 같은 작업 폴더의 사용자·폴더 층과 부모에게서 복사한 채팅 층이 그대로 적용된다. 항상 허용 저장도 작업 폴더 단위라 공유한다.
- 하위 채팅에는 답할 사용자가 없어 묻기로 판정된 호출은 묻지 않고 거부한다. 부모 모드를 올리거나 허용 규칙을 두어야 한다. 에이전트 질문 기능도 하위 채팅에는 켜지 않는다.
- 쓰기 잠금은 부모가 쥔 채로 하위 채팅이 쓴다. 하위 채팅의 입력은 쓰기 범위를 비워 접수해 부모의 잠금을 기다리지 않는다. 기다리면 부모는 하위 결과를, 하위는 부모의 잠금을 서로 기다려 멈춘다. 하위 접속이 트리에 올라 있는 동안 부모 트리가 유휴가 아니므로 잠금은 풀리지 않는다.

### router 키

- 키는 `engine`만 갖는다. 하위 채팅의 환경은 부모 환경에서 키 변수를 뺀 것이고, provider를 띄울 때 다시 한 번 제외 목록을 적용한다.
- 권한 모드 `full`이어도 키 보호는 꺼지지 않는다. Saturn 소유 PreToolUse 훅과 명령 샌드박스의 키 저장소 읽기 금지는 모드와 무관하게 하위 채팅에도 같게 들어간다([router 키 보호](router-key-security.md)). 에이전트가 키를 직접 읽는 길은 그쪽의 샌드박스와 읽기 금지가 막는다.
- 하위 접속과 바깥 접속은 router를 부르지 않는다. 판단은 `engine`이 한다.

### 요청 병렬 처리

- 접속마다 읽기 작업과 쓰기 작업이 따로 돌아 한 접속의 느린 요청이 다른 접속을 막지 않는다.
- 출입증 확인, 깊이와 모드 검사, 동시 상한과 대기열은 요청 처리 루프를 거치지 않는다. 연결 작업이 공유 상태(`PassGate`)에서 처리한다. 공유 상태는 짧은 메모리 조회와 갱신에만 잠금을 쥐고 `await` 사이에는 쥐지 않는다.
- 상한에 막힌 요청은 연결 작업이 따로 띄운 대기 작업이 기다린다. 그 연결의 다음 줄도 계속 읽고, 다른 요청은 기다리지 않는다. 자리가 나면 대기 작업이 깨어 허용된 요청을 루프에 넘긴다.
- 기록 저장소 쓰기와 채팅 상태 변경은 루프 한 곳에서 순서대로 한다. 쓰는 쪽이 하나여야 한다는 규칙 때문이다. 루프는 provider 응답이나 router 응답을 기다리지 않고 맡긴 뒤 결과 메시지만 받는다.
- 프로세스 감시는 묶음마다 프로세스 표를 읽지 않고 감시 작업들이 표 하나를 나눠 쓴다. 표 읽기가 묶음 수에 비례해 늘면 하위 접속 100개에서 요청이 멈추는 것을 부하 시험에서 확인해 고쳤다.

### 부하 시험

가짜 provider로 하위 작업 N개가 동시에 붙어 입력을 보내고 한꺼번에 끊기는 동안, 다른 접속이 20ms마다 `Version` 요청을 보내 응답 시간을 쟀다. 하위 채팅마다 `/bin/sleep` 프로세스 하나를 `Supervisor`로 띄워 프로세스 감시 비용을 포함했다. 시험 중 상한은 풀었고(`child.max_concurrent`와 `child.max_total` 1000), 조건마다 3번 재서 표에는 범위를 적었다. 같은 컴퓨터에서 다른 빌드가 함께 돌고 있었다.

| 동시 하위 작업 | 응답 시간 중앙값 | 응답 시간 95분위 | 최대 | engine 메모리 증가(하위 하나당) |
|---|---|---|---|---|
| 10 | 0.45~0.53ms | 6~14ms | 0.11~0.24초 | 약 200KiB |
| 50 | 0.53~0.55ms | 9~448ms | 0.41~1.19초 | 약 65~70KiB |
| 100 | 0.49~0.54ms | 447~857ms | 0.90~1.36초 | 약 48KiB |

- 중앙값은 100개에서도 1ms 아래다. 요청이 서로 기다리지 않는다는 뜻이다. 95분위는 하위 접속이 한꺼번에 몰리는 구간에서 늘어난다. 접속 하나를 만드는 일(기록 저장소 쓰기, 설정 병합)이 루프에서 순서대로 처리되기 때문이다.
- engine 메모리는 `ps`로 잰 시험 프로세스의 상주 메모리다. 실제 provider 프로세스의 메모리는 가짜 provider로는 잴 수 없어 포함되지 않는다. 그 비용이 상한을 두는 이유이고, 하위 접속 하나가 provider 프로세스 하나다.
- 기본 상한은 응답 시간 95분위가 15ms 이하였던 10에 맞춰 전체 10, 채팅당 5로 정했다. 깊이 2는 이 시험이 아니라 하위가 하위를 거느리는 폭발을 막는 값이다(초안). 10보다 큰 값이 필요한 사용자는 사용자 설정에서 올린다. 상한을 넘는 요청은 실패하지 않고 대기하므로 몰려도 느려질 뿐 멈추지 않는다.
- 시험은 `#[ignore]`라 일반 `cargo test`에서 돌지 않는다. 실행: `cargo test -p saturn-engine --lib child_load -- --ignored --nocapture --test-threads=1`. 환경 변수 `CHILD_LOAD_NO_PROCESSES`를 주면 `sleep` 프로세스를 띄우지 않는다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| 출입증이 없거나 회수됨 | `pass is unknown or revoked`로 거절한다. 오류 번호는 `CHILD_REJECTED`(-32002)다. |
| 깊이 상한 초과 | `child depth limit {n} exceeded`로 거절한다. |
| 요청 모드가 부모 모드 초과 | `requested mode is above the parent mode {모드}`로 거절한다. |
| 부모에 실행 중인 작업이 없음 | `parent has no running task`로 거절하고 자리를 돌려준다. |
| 모르는 모드 이름 | `INVALID_PARAMS`로 거절한다. |
| 동시 상한 초과 | 대기열에 세운다. |
| 대기 중 부모가 멈춤 | `parent was stopped before a place opened`로 거절한다. |
| 허용받은 접속이 채팅을 만들기 전에 끊김 | 자리를 다음 요청에 준다. |
| 난수를 읽지 못해 출입증을 만들지 못함 | provider 연결을 시작하지 못하고 보내기 전 실패로 다룬다. 하위 접속은 자리를 돌려준다. |
| 에이전트 작업 안의 `engine` 시작 | 거절한다. |
| 에이전트 작업 안인데 출입증이 없음 | `saturn`이 거절한다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 하위 접속은 부모 채팅의 실행 중인 작업 하나의 하위 에이전트로 트리에 오르고, 작업 폴더와 더한 폴더를 물려받는다. | `saturn-terminal/engine/src/lifecycle/child_passes.rs`의 `child_attaches_as_a_subagent_of_the_running_parent_task`, `child_chat_inherits_the_parent_folder_and_added_folders` |
| 부모 모드를 넘는 요청과 모드 변경은 거절하고, 부모가 모드를 낮추면 하위도 따라간다. | 같은 파일의 `request_above_the_parent_mode_is_rejected_without_creating_anything`, `request_below_the_parent_mode_sets_the_child_mode`, `child_cannot_raise_its_mode_above_the_parent_mode`, `lowering_the_parent_mode_lowers_the_child_too`, `saturn-terminal/core/src/passes/tests.rs`의 `request_above_the_parent_mode_is_rejected`, `check_mode_limits_a_child_to_its_parent_mode_and_leaves_roots_free` |
| 깊이 상한을 넘는 요청은 거절한다. | 같은 파일의 `depth_above_the_limit_is_rejected`, core의 `depth_above_the_limit_is_rejected` |
| 동시 상한을 넘는 요청은 대기열에 서고 자리가 나면 이어서 붙으며, 끊긴 대기 요청은 자리를 받지 않는다. | 같은 파일의 `request_over_the_concurrent_limit_waits_and_attaches_when_a_place_frees`, `queued_request_that_disconnects_does_not_take_a_place`, `grant_for_a_client_that_left_goes_to_the_next_request`, core의 `finished_child_hands_its_place_to_the_first_waiter`, `total_limit_holds_requests_of_other_parents_too` |
| 부모를 멈추거나 부모 연결이 닫히면 하위 접속이 함께 끝나고 출입증이 회수되며, 하위 접속이 끊기면 하위 작업이 끝난다. | 같은 파일의 `stopping_the_parent_ends_the_child_with_it`, `parent_connection_loss_expires_the_pass_and_ends_the_children`, `child_disconnect_ends_the_child_and_frees_its_place` |
| 출입증 토큰은 로그, 오류, 디버그 출력에 나오지 않는다. | `saturn-terminal/engine/src/secrets/mask.rs`의 `pass_tokens_are_masked_by_shape`, `passes.rs`의 `token_debug_never_shows_the_text`, core의 `token_text_never_appears_in_debug_output`, 같은 파일의 `unknown_pass_is_rejected_by_the_connection_without_the_engine_loop` |
| router 키는 하위 채팅에 가지 않고, `full`이어도 키 보호 설정이 같으며, 부모 출입증은 하위 환경에 가지 않는다. | 같은 파일의 `router_key_never_reaches_a_child_even_in_full_mode`, `child_launch_env_carries_its_own_pass_not_the_parents` |
| 하위 채팅의 묻기 판정은 거부하고 부모 TUI에 올리지 않는다. | 같은 파일의 `child_permission_ask_is_denied_without_asking_the_parent_tui` |
| 하위 채팅의 쓰기 입력은 부모의 쓰기 잠금을 기다리지 않는다. | 같은 파일의 `child_write_input_does_not_wait_for_the_parent_write_lock` |
| 부모에 실행 중인 작업이 없으면 거절한다. | 같은 파일의 `child_is_rejected_when_the_parent_has_no_running_task` |
| 출입증 확인은 요청 처리 루프 없이 하고, 대기 요청과 다른 요청이 서로 기다리지 않는다. | 같은 파일의 `unknown_pass_is_rejected_by_the_connection_without_the_engine_loop`, `queued_children_do_not_hold_back_other_requests`, `queued_request_does_not_block_the_next_request_on_the_same_connection` |
| 출입증이 있으면 하위 접속, 없고 표지만 있으면 거절, 둘 다 없으면 바깥 접속이다. | `saturn-terminal/cli/src/launch.rs`의 `origin_with_marker_and_no_pass_is_error`, `origin_with_an_empty_pass_is_error`, `origin_without_marker_or_pass_is_outside`, `origin_with_a_pass_is_a_child_on_the_given_socket` |
| 동시 하위 작업 10, 50, 100개에서 응답 시간과 메모리를 잰다. | `saturn-terminal/engine/src/lifecycle/child_load.rs`의 `load_10_children`, `load_50_children`, `load_100_children`(`#[ignore]`) |

## 단점

- 하위 채팅은 묻지 않고 거부하므로 `edit` 모드 부모 아래의 하위 작업은 묻기 규칙에 걸린 명령을 하지 못한다. 부모 모드를 `full`로 올리거나 허용 규칙을 미리 두어야 한다.
- 출입증은 채팅 단위라 같은 채팅에서 작업이 여럿 돌 때 어느 작업이 부탁했는지 알 수 없다. 가장 먼저 시작한 작업 아래에 올라간다.
- 접속 하나를 만드는 일이 루프에서 순서대로 처리되어 한꺼번에 많이 몰리면 응답 시간 95분위가 늘어난다.
- 부모와의 연결이 메모리에만 있어 `engine`을 다시 켜면 하위 채팅은 독립 채팅이 된다.

## 대안

- 하위 접속마다 새 `engine`을 띄우고 결과 파일로 돌려받는 방식은 사용자당 `engine` 하나와 기록 저장소의 쓰기 하나를 깨서 버렸다([결정 기록](../decisions/2026-10-04-child-sessions-via-engine-pass.md)).
- 계속 거절하는 방식은 에이전트가 Saturn을 쓸 수 없어 버렸다.
- 출입증 없이 환경 표지만으로 하위를 알아보는 방식은 표지를 복사하면 누구나 하위처럼 행세하고 범위와 만료가 없어 버렸다.
- 요청마다 요청 처리 루프에서 출입증을 확인하는 방식은 대기하는 요청이 루프를 잡거나 별도 대기 장치를 루프 안에 둬야 해 버렸다.

## 미해결 질문

- 하위 채팅의 묻기를 부모 TUI로 올려 사용자가 답하게 할지, 지금처럼 거부할지. 부모 TUI가 하위 채팅의 작업 번호를 몰라 창 형식을 정해야 한다 ([#469](https://github.com/woonyong-choi/saturn/issues/469))
- 실제 provider 프로세스의 메모리를 재어 상한 기본값을 다시 정할지 ([#469](https://github.com/woonyong-choi/saturn/issues/469))
