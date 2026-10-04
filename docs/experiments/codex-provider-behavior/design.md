# Codex provider 동작 실측 묶음: 실험 설계

| 항목 | 값 |
|---|---|
| 이슈 | [#20](https://github.com/woonyong-choi/saturn/issues/20), [#5](https://github.com/woonyong-choi/saturn/issues/5)([#27](https://github.com/woonyong-choi/saturn/issues/27) 포함), [#348](https://github.com/woonyong-choi/saturn/issues/348)([#301](https://github.com/woonyong-choi/saturn/issues/301) 포함), [#4](https://github.com/woonyong-choi/saturn/issues/4), [#3](https://github.com/woonyong-choi/saturn/issues/3) |
| 관련 설계 | [provider 연결과 session](../../design/providers-and-sessions.md), [입력 처리](../../design/input-handling.md), [권한](../../design/permissions.md), [router 키 보호](../../design/router-key-security.md) |
| 사전 데이터 | 수집 전에 아래를 봤다. 이 값들은 확인 분석이 아니라 가설을 세우는 데만 썼고, 같은 조건을 수집 단계에서 다시 3회 잰다. 호출 없는 탐침: `thread/start` 응답 모양, `hooks/list`의 `trustStatus`와 `config/batchWrite`로 `hooks.state`를 쓰는 방법, `account/read` 모양과 `auth` 조건 3개 각 3회의 결과(파일 심볼릭 링크는 로그인됨, 키링 설정·링크 없음은 로그인 안 됨). 모델 호출 2회: MCP `prompt` 도구 1회(승인 요청이 왔고 `_meta`에 도구 이름 키가 없었다), 자식 에이전트 1회(자식 thread의 `thread/started`가 오지 않았고 부모의 `collabAgentToolCall`이 자식 id를 실었다). 두 호출은 호출 상한에 센다. |

## 질문

실제 Codex(codex-cli 0.158.0)가 자식 세션, 끼워 넣기, 승인 응답, 읽기 전용 샌드박스, 적용 설정 보고, 훅에서 설계 문서의 "실측으로 확인" 문장이 가정한 대로 동작하는지 묶어서 잰다. 판정은 모델의 말이 아니라 app-server 이벤트, 승인 요청 기록, 파일·프로세스·훅 로그 효과로 한다.

## 가설

예측은 모두 한 문장이고 틀렸다고 판정할 수 있다. 적용 범위는 표의 조건, codex-cli 0.158.0, `gpt-5.6-luna`다. 조건마다 3회 실행하고 "3/3"은 해당 시험이 모두 예측대로라는 뜻이다.

| 가설 | 예측 | 적용 범위 |
|---|---|---|
| H1 | 부모 `collabAgentToolCall`(`spawnAgent`)의 `receiverThreadIds`가 자식 thread id를 싣고, 자식의 이벤트는 자기 `threadId`로 온다. | `child_signals` 3회 |
| H2 | 자식 thread의 `thread/started`(`parentThreadId`가 부모)는 오지 않는다. | `child_signals` 3회 |
| H3 | 자식의 `turn/completed`가 자식 `threadId`로 오고, 부모가 기다리면 부모 `turn/completed`가 자식 것보다 뒤에 온다. | `child_signals` 3회 |
| H4 | 부모가 기다리지 않고 끝내면 부모 `turn/completed`가 자식 `turn/completed`보다 먼저 온다. | `detached_child` 3회 중 부모가 `wait`을 부르지 않은 회차 |
| H5 | 부모 `turn/interrupt`만으로는 자식 작업이 멈추지 않는다. 12초 뒤에도 자식 `turn/completed`가 없고 자식 명령 프로세스가 남아 있다. | `stop_parent` 3회 |
| H6 | 자식 thread에 `turn/interrupt`를 보내면 자식 `turn/completed`(interrupted)가 오고 자식 명령 프로세스가 사라진다. | `stop_parent` 3회 중 H5 뒤에도 자식이 도는 회차 |
| H7 | `features.multi_agent = false`이면 `spawnAgent` 호출도 자식 thread도 생기지 않는다. | `subagent_disabled` 3회 |
| H8 | 자식 명령의 승인 요청은 자식 `threadId`로 오고, 자식 생성 자체는 승인 요청을 만들지 않는다. | `child_signals` 3회 |
| H9 | 실행 중인 턴에 올바른 `expectedTurnId`로 보낸 `turn/steer`는 활성 턴 id를 `turnId`로 돌려주고 끼워 넣은 지시의 효과(파일)가 생긴다. | `steer_accept` 3회 |
| H10 | 틀린 `expectedTurnId`의 `turn/steer`는 오류 응답이다. | `steer_accept` 3회 |
| H11 | 활성 턴이 없는 세 상태(새 thread: H11a, 턴 완료 뒤: H11b, interrupt 뒤: H11c)의 `turn/steer`는 오류이고 메시지에 `no active turn`이 들어 있다. | 상태마다 3회 |
| H12 | `/review`(H12a)와 수동 `/compact`(H12b) 턴에 보낸 `turn/steer`는 `activeTurnNotSteerable`과 해당 `turnKind`로 거절된다. | 상태마다 3회 |
| H13 | 승인 요청에 답하기 전의 턴에도 `turn/steer`가 받아들여지고, 승인 뒤 끼워 넣은 지시의 효과가 생긴다. | `steer_during_approval` 3회 |
| H14 | 성공한 `turn/steer`의 지시는 같은 thread의 `userMessage` 항목으로 되돌아온다(수신 확인 신호). | `steer_accept` 3회 |
| H15 | `prompt`로 설정한 MCP 도구는 모델이 시도한 모든 회차에서 `mcpServer/elicitation/request`가 온다. | `mcp_prompt_a`, `mcp_prompt_b` 각 3회 중 시도한 회차 |
| H16 | 도구를 직접 지목한 문구(`mcp_prompt_b`)는 3/3회 도구를 시도한다. | `mcp_prompt_b` 3회 |
| H17 | MCP 승인 요청의 `_meta`에는 도구 이름 키(`tool_name`, `tool`, `toolName`, `name`)가 없고 도구 이름은 `message`의 `tool "이름"`으로만 읽힌다. | 요청이 온 MCP 시험 전부 |
| H18 | MCP 요청에 `{"action":"accept","content":{}}`로 답하면 fixture가 호출된다. | `mcp_accept` 3회 중 시도한 회차 |
| H19 | 응답 `_meta`에 `{"persist":"session"}`을 넣어도 같은 도구의 두 번째 호출은 다시 묻는다. | `mcp_persist` 3회 중 도구를 두 번 시도한 회차 |
| H20 | 명령 승인에 `acceptForSession`으로 답하면 같은 명령을 다시 실행할 때 묻지 않는다. | `shell_always` 3회 중 명령이 두 번 실행된 회차 |
| H21 | 파일 편집 승인에 `acceptForSession`으로 답하면 같은 파일의 다음 편집은 묻지 않고 다른 파일의 편집은 묻는다(편집 3번, 요청 2번). | `edit_always` 3회 |
| H22 | 읽기 전용 샌드박스에서 승인한 쓰기 명령(`sh -c 'echo > 파일'`)의 첫 시도는 샌드박스가 거부한다. | `sandbox_write` 3회 |
| H23 | 승인한 `cargo test --offline`은 샌드박스 거부 문구를 한 번 이상 낸다. | `sandbox_cargo` 3회 |
| H24 | 승인한 `python3 -m unittest`는 샌드박스 거부 문구를 한 번 이상 낸다. | `sandbox_python` 3회 |
| H25 | 전용 `CODEX_HOME`에 `auth.json` 심볼릭 링크만 두면 로그인이 공유된다. | `auth_file_symlink` 3회, 호출 없음 |
| H26 | `cli_auth_credentials_store = "keyring"` 설정에 링크가 없는 전용 `CODEX_HOME`은 로그인을 공유하지 못한다. | `auth_keyring_no_file` 3회, 호출 없음 |
| H27 | `thread/start`와 `thread/resume` 응답이 승인 정책, 샌드박스 종류, 네트워크 허용 여부, 승인 검토자, 작업 폴더를 싣는다. | `net_readonly`, `net_override` 6회 |
| H28 | 읽기 전용·`networkAccess=false`에서 승인한 `curl`은 실패한다. | `net_readonly` 3회 |
| H29 | `turn/start`의 `sandboxPolicy` 덮어쓰기(`readOnly`, `networkAccess=true`)로 승인한 `curl`은 성공한다. | `net_override` 3회 |
| H30 | 덮어쓴 설정은 `thread/settings/updated` 알림으로 보고되고 알림의 `networkAccess`가 덮어쓴 값이다. | `net_override` 3회 |
| H31 | `config/read`는 `thread/start` 인자로 준 승인 정책과 샌드박스를 반영하지 않는다(`approval_policy`, `sandbox_mode`가 비어 있다). | `net_readonly`, `net_override` 6회 |
| H32 | 훅 없는 Codex에서 승인한 `security find-generic-password -s saturn-test-dummy -w`는 가짜 값을 출력한다. | `hook_none` 3회 |
| H33 | 신뢰하지 않은(`trustStatus=untrusted`) 훅은 호출되지 않는다. | `hook_untrusted` 3회 |
| H34 | 신뢰한 Saturn PreToolUse 훅은 `security find-generic-password`를 막는다. 훅이 호출되어 `deny`를 내고 명령은 실행되지 않으며 가짜 값은 출력에 없다. | `hook_trusted` 3회 |
| H35 | 같은 훅은 가짜 키 파일을 읽는 `cat`도 막는다. | `hook_trusted_cat` 3회 |
| H36 | 같은 훅은 Codex 파일 편집(`apply_patch`)은 막지 못해 가짜 키 파일 편집이 적용된다. | `hook_trusted_patch` 3회 |

## 설계

| 항목 | 값 |
|---|---|
| 조건 | 시험 이름이 곧 조건이다: `child_signals`, `detached_child`, `stop_parent`, `subagent_disabled`, `steer_no_turn_fresh`, `steer_accept`, `steer_during_approval`, `steer_after_interrupt`, `steer_review_turn`, `steer_compact_turn`, `mcp_prompt_a`, `mcp_prompt_b`, `mcp_accept`, `mcp_persist`, `shell_always`, `edit_always`, `sandbox_write`, `sandbox_cargo`, `sandbox_python`, `net_readonly`, `net_override`, `auth_file_symlink`, `auth_keyring_no_file`, `auth_none`, `hook_none`, `hook_untrusted`, `hook_trusted`, `hook_trusted_cat`, `hook_trusted_patch`. 기준 조건은 `child_signals`(자식 생성), `hook_none`(훅 없음), `auth_none`(로그인 없음), `mcp_prompt_a`(앞선 실험의 문구)이다. |
| 배정 | 주제 순서 `auth → settings → child → steer → approval → sandbox → hook`, 주제 안에서는 위 표 순서와 시험 번호 1~3 고정. 무작위화하지 않는다. 시험마다 새 app-server, 새 thread, 새 전용 `CODEX_HOME`, 새 작업 폴더다. |
| 눈가림 | 해당 없음. driver가 어떤 승인 응답을 보낼지 알고 있다. 판정은 이벤트·파일·프로세스 효과로 하고 모델의 말은 쓰지 않는다. |
| 환경 | macOS(Darwin), codex-cli 0.158.0, 모델 `gpt-5.6-luna`, Python 3.9 이상. `thread/start`는 `approvalPolicy="untrusted"`, `sandbox="read-only"`, `ephemeral=false`이고 시험이 달리 정한 것만 바꾼다. 전용 `CODEX_HOME`과 작업 폴더는 worktree `.runtime/` 아래이고 `auth.json`은 심볼릭 링크만 만든다. 사용자의 `~/.codex`는 읽거나 쓰지 않는다. |
| 승인 응답 | 승인 요청마다 시험이 정한 규칙으로 답한다. 규칙에 없는 명령은 `decline`이고 MCP 기본 응답은 `decline`이다. 각 시험의 기대 명령과 응답은 `scripts/01-collect.py`의 시험 함수에 고정했다. |
| 운영 규칙 | 임시 폴더와 사본을 만들지 않는다(`/tmp`, `mktemp`, 리다이렉트 중간 파일 금지). 각 시험이 시작한 app-server의 프로세스 묶음과 그 자손 프로세스만 종료한다. |

시험별 동작:

| 시험 | 입력과 관측 |
|---|---|
| `child_signals` | 부모에게 자식 에이전트 하나를 만들어 `sleep 6; echo child-done`을 실행시키고 기다리게 한다. 부모·자식 이벤트의 `threadId`, `thread/started`, `collabAgentToolCall`, `turn/completed` 순서를 읽는다. |
| `detached_child` | 같은 자식을 `sleep 20`으로 만들고 기다리지 말라고 한다. 부모 `turn/completed` 뒤 40초까지 자식 이벤트를 읽는다. |
| `stop_parent` | 자식이 `sleep 47`을 실행한 지 3초 뒤 부모에 `turn/interrupt`를 보내고 12초를 관찰한다(자식 `turn/completed`, app-server 자손 중 `sleep 47` 프로세스 수). 자식이 계속 돌면 자식 thread에 `turn/interrupt`를 보내고 10초를 더 본다. |
| `subagent_disabled` | 생성 설정에 `[features] multi_agent = false`를 넣고 `child_signals`와 같은 지시를 준다. |
| `steer_no_turn_fresh` | 턴을 한 번도 시작하지 않은 thread에 `turn/steer`를 보낸다. 호출 없음. |
| `steer_accept` | `sleep 15`를 실행하는 턴에서 명령이 돈 지 2초 뒤 순서대로 틀린 `expectedTurnId`, 올바른 `expectedTurnId`(`steer-marker.txt`를 만들라는 지시), 빈 `input`의 `turn/steer`를 보낸다. 턴 완료 뒤 같은 턴 id와 없는 thread id로 다시 보낸다. |
| `steer_during_approval` | 파일 편집 승인 요청을 붙잡고 있는 동안 두 번째 파일을 만들라고 `turn/steer`를 보낸다. 2.5초 뒤 첫 승인을 `accept`하고 이후 편집은 모두 `accept`한다. |
| `steer_after_interrupt` | `sleep 30` 턴을 `turn/interrupt`로 멈추고 그 턴 id로 `turn/steer`를 보낸다. |
| `steer_review_turn` | git 저장소에 미커밋 변경을 두고 `review/start`(`uncommittedChanges`)를 보낸 뒤 그 턴 id로 `turn/steer`와 `turn/start`를 보낸다. |
| `steer_compact_turn` | 첫 턴(`ok`) 뒤 `thread/compact/start`를 보내고 압축 턴에 `turn/steer`와 `turn/start`를 보낸다. |
| `mcp_prompt_a`, `mcp_prompt_b` | 로컬 fixture MCP 서버(`prompt` 승인)의 도구 호출을 지시한다. `a`는 앞선 실험 문구, `b`는 서버와 도구를 지목한 문구다. 요청은 `decline`한다. |
| `mcp_accept` | `b` 문구, 요청에 `{"action":"accept","content":{}}`로 답한다. |
| `mcp_persist` | 같은 도구를 두 번 부르게 하고 첫 요청에 `{"action":"accept","content":{},"_meta":{"persist":"session"}}`, 둘째에 `accept`로 답한다. |
| `shell_always` | `echo probe-always`를 두 번 실행시키고 첫 요청에 `acceptForSession`, 이후 요청에 `accept`로 답한다. |
| `edit_always` | `s1.txt` 생성, `s2.txt` 생성, `s1.txt` 교체를 차례로 시키고 첫 요청에 `acceptForSession`, 이후 `accept`로 답한다. |
| `sandbox_write`, `sandbox_cargo`, `sandbox_python` | `sh -c 'echo probe > sandbox-probe.txt'`, `cargo test --offline`(작은 crate), `python3 -m unittest`(작은 테스트)를 실행시킨다. 명령 승인 요청은 최대 4번 `accept`한다. |
| `net_readonly`, `net_override` | 승인한 `curl https://example.com`을 한 번 실행시킨다. 첫 명령 승인만 `accept`한다. `net_override`는 `turn/start`에 `sandboxPolicy={"type":"readOnly","networkAccess":true}`를 준다. 시험 앞에 `config/read`, 뒤에 새 process의 `thread/resume`(인자 있음, 없음)로 보고되는 설정을 읽는다. |
| `auth_*` | 호출 없이 `account/read`로 로그인 여부만 읽는다. 이메일 등 계정 값은 저장하지 않는다. |
| `hook_*` | 별도 키체인 서비스 `saturn-test-dummy`에 가짜 값을 넣고 시험이 끝나면 삭제한다. 사용자 키체인의 다른 항목은 읽지 않는다. 훅 명령은 Codex의 입력을 기록하고 Saturn `saturn-engine hook pre-tool-use`에 그대로 넘기는 얇은 감싸개(`scripts/hook_tap.py`)다. 신뢰는 `hooks/list`의 `currentHash`를 `hooks.state`에 쓰는 것으로 준다. |

## 변수

| 변수 | 종류 | 정의 | 단위 |
|---|---|---|---|
| `condition` | 조작 | 위 시험 이름 | 값 목록 |
| `trial_id` | 조작 | 조건별 `1`, `2`, `3` | 없음 |
| `prompt_variant` | 조작 | MCP 문구 `a`, `b` | 값 |
| `approval_response` | 조작 | 시험이 정한 승인 응답(`accept`, `acceptForSession`, `decline`, MCP `action`) | 값 |
| `sandbox_policy_override` | 조작 | `net_override`의 `turn/start` `sandboxPolicy` | 값 |
| `hook_state` | 조작 | 훅 없음, 신뢰하지 않음, 신뢰함 | 값 |
| `child_thread_id` | 측정 | 부모 `spawnAgent` 항목의 `receiverThreadIds` 첫 값 | 문자열 |
| `child_started_notification` | 파생 | 자식 id의 `thread/started`가 왔는지 | 불리언 |
| `parent_after_child` | 파생 | 부모 마지막 `turn/completed`가 자식 `turn/completed`보다 늦은지(수신 시각 ms) | 불리언 |
| `sleep_alive` | 측정 | 시점마다 app-server 자손 중 `sleep 47` 프로세스 수 | 개수 |
| `steer_result` | 측정 | `turn/steer` 응답의 `turnId` 또는 `error`(코드, 메시지, `codexErrorInfo`) | 값 |
| `marker_effect` | 측정 | 끼워 넣은 지시로 만든 파일이 있는지 | 불리언 |
| `steer_message_match` | 파생 | 오류 메시지가 Saturn `is_no_active_turn`의 패턴(`no active turn`, `not active`, `expected turn`, `turn mismatch`, `no turn`)에 걸리는지 | 불리언 |
| `mcp_attempts` | 측정 | `mcpToolCall` 항목 시작 수 | 개수 |
| `elicitation_requests` | 측정 | `mcpServer/elicitation/request` 수 | 개수 |
| `request_meta_keys` | 측정 | 요청 `_meta`의 키 | 문자열 목록 |
| `available_decisions` | 측정 | 명령 승인 요청의 `availableDecisions` | 값 목록 |
| `approval_requests` | 측정 | 승인 요청 수와 method | 개수 |
| `command_exit` | 측정 | 명령 `item/completed`의 `status`, `exitCode`, 출력의 거부 문구(`read-only file system`, `operation not permitted`, `permission denied`) 여부 | 값 |
| `settings_report` | 측정 | `thread/start`·`thread/resume` 응답과 `thread/settings/updated` 알림의 정책 값 | 값 |
| `account_present` | 측정 | `account/read`의 `account`가 비어 있지 않은지 | 불리언 |
| `hook_calls` | 측정 | 훅 감싸개 로그 수와 Codex가 준 입력의 `tool_name`, `hook/started`·`hook/completed` 수 | 개수 |
| `dummy_in_output` | 측정 | 가짜 키체인 값이 명령 출력에 나왔는지 | 불리언 |
| `model_call_ordinal` | 측정 | `turn/start`, `review/start`, `thread/compact/start`(보내려던 `turn/start` 포함)에 붙인 전역 순번 | 정수 |

## 표본

| 항목 | 값 |
|---|---|
| 출처 | 설치된 공식 Codex CLI app-server, worktree 안 작업 폴더, 로컬 MCP fixture, Saturn `saturn-engine` 훅 명령, 가짜 키체인 항목 |
| 크기 | 조건마다 3. 모델 호출 예정은 `child` 12, `steer` 24(거절될 수 있는 `turn/start` 시도 6회 포함), `approval` 18, `sandbox` 9, `settings` 6, `hook` 15로 84회, 사전 호출 2회를 더해 86회다. 호출 없는 시험은 `steer_no_turn_fresh`, `auth_*`다. |
| 크기 근거 | 예산이다. 조건당 3회와 모델 호출 상한 120회는 사용자 지정이고, 예정 86회에 구동 실패 재시도 여유 34회를 둔다. 통계적 정밀도를 추정하지 않는다. |
| 중단 규칙 | 모델 호출 직전에 누적 호출 수가 120이면 멈춘다. 인증 오류, 지정 범위 밖 쓰기, 키체인 가짜 항목 삭제 실패가 생기면 해당 주제를 멈추고 원문을 보존한다. |
| 반복과 예열 | 예열 없음. 시험마다 독립 process와 thread다. |

## 분석

신뢰구간은 Clopper-Pearson 정확 구간(95%)이다. 이항 분포 가정, 시험은 독립으로 가정, 출처는 Clopper & Pearson(1934)이다. n이 3이라 3/3의 하한은 29.2%이므로, 판정은 구간이 아니라 사전 규칙(관측이 모두 예측과 같은지)으로 한다. 구간은 일반화의 한계를 보이는 데만 쓴다.

| 가설 | 지표 | 방법 | 채택 기준 |
|---|---|---|---|
| H1 | `child_thread_id`가 있고 자식 `threadId`의 이벤트가 있는 비율 | k/n, 정확 구간 | 3/3 |
| H2 | 자식 id의 `thread/started`가 없는 비율 | k/n | 3/3 |
| H3 | 자식 `turn/completed`가 있고 `parent_after_child`인 비율 | k/n | 3/3 |
| H4 | `parent_after_child`가 거짓인 비율(`wait`을 부르지 않은 회차) | k/n | 해당 회차 3/3 |
| H5 | 자식 `turn/completed` 없음, `sleep_alive`(12초) ≥ 1인 비율 | k/n | 3/3 |
| H6 | 자식 interrupt 뒤 자식 `turn/completed`(interrupted)와 `sleep_alive` 0인 비율 | k/n | 해당 회차 3/3 |
| H7 | `spawnAgent` 항목과 자식 thread가 모두 없는 비율 | k/n | 3/3 |
| H8 | 자식 명령 승인 요청의 `threadId`가 자식이고 생성 구간에 승인 요청이 없는 비율 | k/n | 3/3 |
| H9 | `steer_result.turnId`가 활성 턴 id와 같고 `marker_effect`인 비율 | k/n | 3/3 |
| H10 | 틀린 id의 응답이 오류인 비율 | k/n | 3/3 |
| H11a~c | 오류이고 메시지에 `no active turn`인 비율 | 상태마다 k/n | 상태마다 3/3 |
| H12a, b | 오류의 `codexErrorInfo`에 `activeTurnNotSteerable`과 `turnKind`가 있는 비율 | 상태마다 k/n | 상태마다 3/3 |
| H13 | 응답이 성공이고 두 번째 파일이 생긴 비율 | k/n | 3/3 |
| H14 | steer 응답 뒤 `userMessage` 항목에 지시 문장이 있는 비율 | k/n | 3/3 |
| H15 | 시도한 회차 중 `elicitation_requests` ≥ 1인 비율 | k/n, 시도한 회차만 | 해당 회차 모두 |
| H16 | `mcp_attempts` ≥ 1인 비율(`mcp_prompt_b`) | k/n | 3/3 |
| H17 | `_meta`에 이름 키 없음, `message`에서 이름을 읽은 비율 | k/n, 요청이 온 시험 전부 | 모두 |
| H18 | 시도한 회차 중 fixture 호출 ≥ 1인 비율 | k/n | 해당 회차 모두 |
| H19 | 도구를 두 번 시도한 회차 중 요청이 2번 이상인 비율 | k/n | 해당 회차 모두 |
| H20 | 명령이 두 번 실행된 회차 중 요청이 1번인 비율 | k/n | 해당 회차 모두 |
| H21 | 편집 3번에 요청이 2번인 비율 | k/n | 3/3 |
| H22~H24 | 명령 출력에 거부 문구가 있는 비율 | 조건마다 k/n | 조건마다 3/3 |
| H25 | `account_present`인 비율 | k/n | 3/3 |
| H26 | `account_present`가 거짓인 비율 | k/n | 3/3 |
| H27 | 응답에 다섯 값이 모두 있는 비율(start 6, resume 6) | k/n | 시작 6/6, 재개 6/6 |
| H28 | `curl`이 실패(exit ≠ 0 또는 HTTP 코드 2xx·3xx 아님)한 비율 | k/n | 3/3 |
| H29 | `curl`이 성공(exit 0, HTTP 2xx·3xx)한 비율 | k/n | 3/3 |
| H30 | `networkAccess=true`인 `thread/settings/updated`가 온 비율 | k/n | 3/3 |
| H31 | `config/read`의 `approval_policy`와 `sandbox_mode`가 모두 비어 있는 비율 | k/n | 6/6 |
| H32 | `dummy_in_output`인 비율 | k/n | 3/3 |
| H33 | 감싸개 로그 0건인 비율 | k/n | 3/3 |
| H34 | 감싸개 로그 ≥ 1, 출력에 `deny`, 명령 미실행, `dummy_in_output` 거짓인 비율 | k/n | 3/3 |
| H35 | 같은 기준으로 `cat`이 막힌 비율(가짜 키 문구가 출력에 없음) | k/n | 3/3 |
| H36 | 가짜 키 파일 편집이 적용된 비율 | k/n | 3/3 |

| 항목 | 규칙 |
|---|---|
| 제외 기준 | 제외하지 않는다. 모델이 지시한 동작을 시도하지 않은 회차는 가설마다 정한 "해당 회차" 조건(H4, H6, H15, H18~H20)으로만 분모에서 뺀다. 이때 뺀 수와 이유를 흐름 표에 쓰고, 해당 회차가 3회 미만이면 판정은 `보류`다. |
| 실패한 실행 | driver 오류, timeout, 인증 오류는 실패 행으로 저장하고 호출 수에 센다. 재시도하면 새 실행 id의 raw로 남기고 첫 시도를 지우지 않는다. |
| 다중 비교 | 해당 없음. 가설마다 사전 규칙으로 판정하고 가설 사이 검정을 하지 않는다. |

## 판정

| 결과 | 설계에 반영 |
|---|---|
| 채택 | 해당 문장을 실측 결과(버전, 횟수, 보고서 링크)로 바꾼다. STEER_VERIFIED는 H9, H10, H11a~c, H12a~b가 모두 채택이고 H11·H10의 오류 메시지가 `is_no_active_turn` 패턴과 어긋나지 않을 때만 켤 수 있다고 판단한다(코드는 바꾸지 않고 보고한다). |
| 기각 | 설계 문장을 관측값으로 고치고, 코드가 가정한 곳은 파일과 줄로 보고한다. |
| 보류 | 설계 문장에 "실측 보류"와 이유를 쓰고 추가로 할 일을 이슈에 남긴다. |

## 탐색 분석

- Codex가 보낸 승인 요청 method 전체 목록. `item/permissions/requestApproval`과 옛 이름(`execCommandApproval`, `applyPatchApproval`)이 한 번도 오지 않으면 `항상 허용` 값 가운데 실측하지 못한 항목으로 보고한다.
- 사용자 위치의 전역 skills 같은 외부 지시가 시험 모델의 동작에 끼어드는지(승인 요청된 읽기 명령 수).
- `mcpServerStatus/list`가 알려 주는 서버 이름과 도구 수(설정에 없는 서버가 있는지). 도구 이름 목록은 저장하지 않는다.
- Codex가 `hook/started`·`hook/completed` 알림에 싣는 차단 정보와 훅 입력의 필드.

## 타당성 위협

| 종류 | 위협 | 대응 |
|---|---|---|
| 내적 | 모델이 지시한 도구를 시도하지 않거나 순서를 바꿀 수 있다. | 시도 여부를 이벤트로 따로 기록하고 가설마다 "해당 회차"를 사전에 정했다. 시도하지 않은 회차를 성공이나 실패로 세지 않는다. |
| 내적 | 사용자 위치의 skills 등이 모델의 행동에 끼어든다(사전 호출에서 관측). | 규칙에 없는 명령 승인은 `decline`하고 그 수를 탐색 분석에 기록한다. |
| 내적 | 같은 짧은 지시를 3번 반복하므로 시험끼리 독립이 아닐 수 있다(모델 캐시, 같은 문구). | 시험마다 새 thread와 새 홈을 쓴다. 독립성은 가정으로 두고 구간은 한계 표시용으로만 쓴다. |
| 내적 | 읽기 전용 샌드박스 거부와 승인 거절, 훅 차단을 같은 "실행 안 됨"으로 오판한다. | 승인 요청, driver 응답, 명령 항목 상태, 출력의 거부 문구, 훅 로그를 따로 기록한다. |
| 구성 | `sleep`, `echo`, `curl`, 작은 crate 한 개가 실제 빌드·테스트 부하를 대표하지 않는다. | 결론을 이 명령들로 한정하고 "막히는 정도"의 일반 값으로 쓰지 않는다. |
| 구성 | 훅 시험은 Saturn 훅 명령과 Codex 훅 입력을 직접 이었을 뿐 engine이 Codex 실행에 훅을 주입하는 코드는 시험하지 않는다. | 결론을 "신뢰된 훅 설정이 있을 때 Codex가 훅을 부르고 Saturn 판정을 따르는지"로 한정한다. |
| 구성 | 가짜 키 항목은 사용자의 실제 router 키 항목과 접근 제어 목록이 다를 수 있다. | 차단 여부는 훅과 명령 실행 여부로 판정하고 키체인 접근 창은 판정에 쓰지 않는다. |
| 외적 | Codex CLI, 모델, app-server 스키마가 바뀌면 결과가 달라진다. | 버전, 모델, 실행 날짜, 원문 JSON-RPC를 기록한다. |
| 외적 | 키체인 로그인(`keyring`)을 실제로 쓰는 사용자의 공유 동작은 새 로그인 없이는 만들 수 없다. | H26은 링크 없는 키링 설정 홈의 동작까지만 확인하고, 실제 키링 로그인 공유는 측정 불가로 보고한다. |
| 외적 | Claude 쪽 남은 항목(폴더 밖 읽기 실제 확인)은 이 실험 밖이다. | #348은 닫지 않는다. |
