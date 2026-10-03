# provider 연결과 session

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [provider마다 입력을 계속 받는 상시 연결을 만든다](../decisions/2026-09-29-persistent-provider-connections.md), [provider 설정과 subagent 사용은 사용자 설정을 따르고 Saturn은 추적만 한다](../decisions/2026-09-29-minimal-provider-control.md), [권한 판단의 정본은 Saturn 설정의 `permission` 규칙 하나로 둔다](../decisions/2026-10-02-saturn-permission-authority.md), [provider 연결을 채팅마다 따로 둔다](../decisions/2026-10-02-per-chat-provider-connections.md) |

## 요약

engine은 Codex와 Claude Code에 켜 둔 연결을 두고, 한 채팅 안에서 여러 provider session을 이어 쓴다. 채팅마다 메인 에이전트가 하나 있고, 무관한 작업은 보조 에이전트가 맡는다. engine은 provider 이벤트를 Saturn 용어로 바꿔 기록하고, subagent 트리와 사용량을 추적한다.

## 동기

Codex와 Claude Code를 함께 쓰는 개발자는 에이전트를 바꿀 때마다 맥락이 끊긴다. provider session은 provider마다 따로 있어서, 한쪽에서 한 일을 다른 쪽이 모른다. 한 번 실행 방식인 `codex exec`와 `claude -p`는 실행 중 입력을 받지 못하고, Codex의 맥락 크기도 보여 주지 않는다. provider는 스스로 subagent를 띄우므로, subagent까지 끝났는지 모르면 멈춤, 쓰기 잠금, compaction 경계를 판정할 수 없다. 이 기능이 없으면 사용자는 provider를 바꿀 때마다 맥락을 다시 설명하고, 멈춘 줄 알았던 subagent가 계속 파일을 바꾼다.

## 예시

### Claude로 작업하다 Codex로 바꿔 이어 가기

1. 사용자는 Claude 메인 에이전트와 한 채팅에서 기능을 만든다.
2. 사용자는 다음 입력의 모델을 Codex로 고정해 보낸다.
3. `sessions`는 Claude 턴이 끝난 경계에서 Codex 메인 에이전트를 새로 시작한다.
4. engine은 Saturn 기록 원문에서 고른 패킷을 새 Codex session에 넘긴다.
5. 인수가 끝나면 engine은 이전 Claude 메인 session을 닫고 provider session ID를 보관한다.
6. 대기열과 기록은 채팅에 있으므로, 사용자는 같은 채팅에서 Codex로 작업을 이어 간다.

### Codex로 갔다가 Claude로 돌아오기

1. 사용자는 Claude로 40턴 작업한 뒤 Codex로 바꿔 짧게 작업한다.
2. 사용자가 다음 입력의 모델을 다시 Claude로 고정해 보낸다.
3. 보관한 Claude session의 마지막 턴 뒤 경과 시간이 캐시 유지 시간 안이다.
4. `sessions`는 새 session 대신 보관한 Claude session을 재개하고, 그 session이 받은 기록 번호 뒤의 변경분만 붙여 보낸다.
5. Claude는 원래 맥락과 캐시를 그대로 쓰며 Codex가 한 일을 이어받는다.

### 보조 에이전트 결과를 메인 에이전트가 받기

1. 메인 에이전트가 작업하는 동안 사용자는 하던 일과 무관한 질문을 보낸다.
2. router가 무관한 작업으로 판단하면 `queue`는 채팅에 속한 보조 에이전트를 시작한다.
3. 보조 에이전트는 일을 끝내면 결과를 전달한 뒤 바로 종료한다.
4. engine은 쉬고 있는 메인 에이전트를 이 결과로 깨우지 않는다.
5. 사용자가 메인 에이전트에 다음 입력을 보내면, engine은 보조 에이전트의 결과 요약과 수정 파일 경로를 붙여 보낸다.

### 멈춤 요청이 subagent까지 닿기

1. Claude 메인 에이전트는 subagent 둘을 띄워 작업한다.
2. 사용자가 멈춤을 요청하면 engine은 추적된 subagent부터 멈춤 신호를 보낸다.
3. 10초 뒤에도 남은 프로세스가 있으면 engine은 provider 프로세스 묶음에 중지 신호를 보낸다.
4. 그래도 남은 프로세스는 강제 종료한다.
5. engine은 트리 전체가 끝난 것을 확인한 뒤에만 멈춤 완료를 보고한다.

## 상세 설계

### provider 연결

engine은 채팅마다 provider별로 켜 둔 채 입력을 받는 연결을 두고([결정 기록](../decisions/2026-10-02-per-chat-provider-connections.md)), 작업 폴더와 환경은 그 채팅에 고정한 값을 쓴다. Codex는 app-server로, Claude Code는 stream-json 입력으로 연결한다. 한 번 실행 방식으로는 끼워 넣기가 불가능하기 때문이다. 고정 모델도 이어 갈 메인 session도 없는 첫 입력은 설치된 Claude로, 없으면 Codex로 보내고 둘 다 없으면 보내지 않는다([입력 처리](input-handling.md#입력-전송과-재전송)).

`core`는 provider 연결 공통 규격인 `ProviderClient` trait을 정의하고, engine의 `providers` 모듈이 `CodexClient`와 `ClaudeClient`로 구현한다. 구현은 끼워 넣기, 멈춤 신호, compaction, 사용량 보고를 Saturn 용어로 넘긴다. provider 고유 이름은 `providers/codex`, `providers/claude` 안에서만 쓴다. TUI와 앱이 provider를 몰라도 화면을 그리게 하기 위해서다.

| 동작 | Codex | Claude Code |
|---|---|---|
| 새 턴 | `turn/start` | 스트림 입력 |
| 끼워 넣기 | `turn/steer` | 스트림 입력 추가 |
| 멈춤 신호 | `turn/interrupt` | interrupt 제어 요청 |
| compaction 요청 | `thread/compact/start` | `/compact` 전송 |
| 허가 답 | 승인 요청의 JSON-RPC 번호로 결정 응답 | `can_use_tool`의 `control_response` |
| session 닫기 | 채팅의 app-server를 창구로 정리 | 프로그램 종료 |
| session 재개 | 채팅의 app-server를 창구로 재개 | stream-json 방식 `--resume` |
| 프로세스 수명 | 연결 창구로 유지, session은 턴 끝 뒤 5분 유예에 정리 | 턴 진행 중과 턴 끝 뒤 5분 유예까지 |

Codex app-server 규약은 codex-cli 0.158.0의 `codex app-server generate-json-schema` 결과로 확인했다.

- 메시지는 한 줄에 JSON 하나이고 `jsonrpc` 필드가 없다. 초기화는 `initialize` 요청 뒤 `initialized` 알림이다.
- session 닫기는 `thread/unsubscribe`다. 기록을 지우는 `thread/archive`, `thread/delete`는 쓰지 않는다.
- 자식 작업은 `thread/started` 알림의 `thread.parentThreadId`로 부모 thread에 잇는다.
- 활성 턴 없음은 오류 코드가 따로 없어 `turn/steer` 오류 문구로 판정한다(초안).
- 맥락 크기는 `thread/tokenUsage/updated`의 `last.totalTokens`이고 메인 턴 끝에 보낸다. 누적 사용량의 새 입력은 `total.inputTokens - cachedInputTokens`다.
- 명령 대응표는 `compact` → `thread/compact/start`, `review` → `review/start`(대상 `uncommittedChanges`)이고, 명령 목록에서 `new`, `resume`, `fork`, `quit`, `exit`를 뺀다(초안). 스킬은 `turn/start` 입력에 `{"type":"skill","name","path"}` 항목으로 넣는다.
- 승인 요청(`item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `item/permissions/requestApproval`, 옛 이름 `execCommandApproval`·`applyPatchApproval`, `_meta.codex_approval_kind`가 있는 `mcpServer/elicitation/request`)은 요청의 JSON-RPC 번호를 숫자와 문자열 그대로 기억했다가 같은 번호로 응답한다. 번호는 `PermissionRequested`의 `request_id`로 올린다. 승인이 아닌 elicitation과 `item/tool/requestUserInput`은 `InputRequested`로 올리고 같은 방식으로 번호를 기억했다가 응답한다([입력 요청](input-requests.md)). 사용자 답을 provider 값으로 바꾸는 표는 [권한](permissions.md#허가-요청-창과-답)에 있다.
- 채팅에 더한 폴더([engine 수명과 복구](engine-lifecycle.md#채팅-폴더와-이어-열기))는 `thread/start`와 `thread/resume`의 `config`에 `sandbox_workspace_write.writable_roots`로 넘긴다(초안). app-server에는 `--add-dir`에 해당하는 인자가 없어서다. 읽기 전용 샌드박스에서 이 값이 효과가 있는지는 실측하지 않았다.
- 권한은 Saturn 규칙을 전용 `CODEX_HOME`과 `thread/start` 인자로 넘기고([권한](permissions.md)), 사용자 설정은 `~/.codex/config.toml`의 루트와 선택된 프로필에서 `model_auto_compact_token_limit` 키가 있는지만 본다.

Claude Code 실행 인자는 Claude Code 2.1.285의 `--help`로 확인했다.

- 새 session은 Saturn이 만든 UUID를 `--session-id`로 넘긴다. stream-json은 첫 입력 전에 `system/init`을 내지 않으므로 session id를 미리 알기 위해서다. 재개는 `--resume <id>`이고, 500ms(초안) 안에 프로그램이 끝나면 재개 실패로 본다.
- 채팅에 더한 폴더는 `--add-dir <폴더>...` 하나로 넘긴다(Claude Code 2.1.285 `--help`의 여러 값 인자). 열린 session에는 넣지 않는다.
- 권한은 `--permission-prompt-tool stdio`와 `--settings`의 `ask` 목록으로 Saturn 규칙에 넘기고([권한](permissions.md)), 안전망 `--autocompact` 값은 허용 범위 100000~1000000으로 맞춘다.
- 사용자 설정은 `~/.claude/settings.json`, `<작업 폴더>/.claude/settings.json`, `settings.local.json`의 `autoCompactEnabled`와 환경 변수 `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, `DISABLE_COMPACT`가 있는지만 본다(초안).
- 명령 목록에서 `clear`, `resume`, `exit`, `quit`를 뺀다(초안). 허가 요청은 `control_request`의 `can_use_tool`로 받고 `control_response`로 답한다. 요청의 `input`은 허용 응답의 `updatedInput`으로 되돌려 주려고 요청 번호와 함께 기억한다(`Bash`만 실측, [권한](permissions.md)).
- 맥락 크기는 마지막 메인 `assistant` 메시지 `usage`의 입력, 캐시 읽기, 캐시 쓰기 합이다(초안).
- interrupt 제어 응답은 10초, session 닫기 뒤 종료는 5초까지 기다리고, 넘으면 프로세스 묶음 중지로 넘어간다(초안).

끼워 넣기 실측을 통과하기 전의 provider에서는 끼워 넣기를 대기로 바꿔 처리한다. 끼워 넣기 경로가 문서대로 동작하는지 실측으로 확인해야 하기 때문이다([#5](https://github.com/woonyong-choi/saturn/issues/5), [#27](https://github.com/woonyong-choi/saturn/issues/27)). 이때 TUI는 `바로 반영 준비 중`을 보인다. 사용자가 바로 반영되지 않는 이유를 알게 하기 위해서다. 입력을 어디로 보낼지는 [입력 처리](input-handling.md)가 정한다.

### provider 실행과 기본값 인자

1. `settings`는 입력 접수 때 고정한 설정 번호의 값을 읽는다.
2. `secrets`는 자식 환경에서 제외 목록에 있는 변수를 지운다.
3. `providers`는 권한 규칙을 provider 실행 설정으로 번역하고, 사용자 provider 설정에 값이 없으면 안전망 인자를 더한다.
4. `providers`는 Claude 실행에 Saturn 소유 PreToolUse 훅을 실행별 설정으로 넘긴다.
5. `processes`는 provider 프로세스를 실행하고 감시를 시작한다.
6. `providers`는 provider 명령 목록을 모은다.

권한은 Saturn의 `permission` 규칙이 정본이므로 Saturn이 provider의 승인 정책과 샌드박스를 실행마다 정한다([권한](permissions.md)). 그 밖의 provider 설정(모델, MCP 서버, 네트워크 등)과 subagent 사용은 막거나 바꾸지 않고 추적만 한다. 사용자가 정한 provider 동작을 유지하기 위해서다. provider 설정을 바꾸는 provider 명령도 막지 않고, 실제 적용된 값을 읽어 기록한다. provider 설정 파일은 어느 경우에도 고치지 않는다.

권한 외 Saturn 기본값은 자동 압축 안전망 값이다. 사용자 설정이 없을 때도 동작을 정해 두기 위해서다. 기본값은 사용자 provider 설정에 값이 없을 때만 실행 인자로 넘긴다. 사용자가 정한 provider 설정을 덮어쓰지 않기 위해서다.

| provider | 안전망 인자 |
|---|---|
| Claude Code | `--autocompact` |
| Codex | `-c model_auto_compact_token_limit` |

안전망은 실행 인자로만 넘기고 사용자 설정 파일은 건드리지 않는다. 안전망 값의 계산은 [맥락 정리](context-management.md)에 있다.

권한 외에 추적만 하는 규칙의 예외는 router 키 보안뿐이다. router 키가 provider 자식 프로세스로 새는 일을 막기 위해서다. 2단계의 환경 변수 제거와 4단계의 훅은 [router 키 보호](router-key-security.md)에 있다.

### provider 명령과 스킬 전달

| 항목 | Codex | Claude Code |
|---|---|---|
| 명령 목록 수집 | 명령 대응표와 `skills/list` | `system/init`의 `slash_commands` |
| 명령과 스킬 전달 | 명령 이름과 app-server 메서드 대응표로 호출 | 프롬프트에 `/이름`을 그대로 넣어 전송 |

provider 명령 목록에서 TUI 전용 명령과 Saturn session 명령이 대신하는 명령은 뺀다. 뒤에서 연결할 때 쓸 수 없거나 Saturn session 기록과 어긋나기 때문이다. 채팅 이어 열기와 폴더 추가는 [engine 수명과 복구](engine-lifecycle.md#채팅-폴더와-이어-열기)에 있다. Claude 스트림에서 provider 명령의 결과와 허가 요청이 어떻게 오는지는 실측으로 확인한다([#26](https://github.com/woonyong-choi/saturn/issues/26)).

### 이벤트 수신과 변환

1. `providers`는 provider 이벤트를 Saturn 용어의 이벤트로 바꾼다.
2. `providers`는 도구 호출마다 Saturn 도구 종류, 파일 경로 목록, 읽은 줄 범위, 바뀐 줄 수를 `ToolCall`의 `detail`에 싣는다.
3. `providers`는 셸 명령이 코드로 끝났으면 종료 코드를 `ToolResult`의 `exit_code`에 싣는다.
4. `providers`는 Codex 신호를 provider session id(`thread_id`)별로 나눈다.
5. `providers`는 Codex 자식 작업을 부모 작업 아래에 등록한다.
6. `providers`는 Claude subagent를 Task/Agent 도구 호출 id와 `parent_tool_use_id`로 부모에 잇는다.
7. `providers`는 Codex `tokenUsage`에 session 누적 범위를, Claude 사용량에 턴 값 범위를 표시한다.
8. `providers`는 입력 없이 provider가 시작한 턴에 `origin = provider-wake`를 표시한다.
9. `store`는 이벤트, subagent, 사용량 보고 원값을 기록한다.
10. `engine`은 모든 연결의 이벤트를 한 루프에서 도착 순서로 받고, 이벤트를 `store`에 먼저 쓴 뒤에만 `agents`와 TUI에 반영한다.

- Saturn 도구 종류는 셸, 테스트 실행, 파일 읽기, 파일 수정, 그 밖이다. 명령의 낱말이 `unittest`, `pytest`, `cargo test` 같은 알려진 테스트 도구와 맞으면 테스트 실행이고, 그 밖의 명령은 셸이다.
- provider가 구조로 주지 않은 값은 비운다. 추측으로 채우면 메모가 틀리기 때문이다. 경로는 provider가 낸 글자 그대로 싣고, 작업 폴더 기준으로 바꾸는 일은 메모를 만드는 쪽이 한다.
- Claude는 `Edit`와 `MultiEdit` 입력의 바뀌기 전과 후 글에서 앞뒤 공통 줄을 빼고 줄 수를 센다. `Write`의 줄 수와 시작 줄만 있는 `Read`의 범위는 알 수 없어 비운다.
- Codex는 `commandActions`의 경로를 경로 목록에 싣고, 모든 동작이 읽기, 목록, 검색이면 파일 읽기로 본다. 종료 코드는 `commandExecution`의 `exitCode`에서 읽는다.
- Codex 파일 수정은 `fileChange`의 `changes`에서 경로를 모으고, 고친 파일은 diff의 `+`와 `-` 줄을, 새 파일은 글 전체를 더한 줄로, 지운 파일은 지운 줄로 센다. 결과 글은 파일마다 경로 줄과 diff를 이어 붙인 것이다.
- Codex 추론 항목은 `Thinking` 활동과 `Reasoning` 종류로 남긴다. 실행 줄의 `생각 중` 표시에 쓰고, 도구 결과 후보에서는 뺀다. 추론은 도구가 아니고 결과가 없기 때문이다.
- `/bin/zsh -lc '...'`처럼 셸이 감싼 명령은 안쪽 명령을 명령 글로 싣는다. 사용자가 보낸 명령이 provider마다 다르게 기록되지 않게 하기 위해서다.
- Claude는 실패한 `Bash` 결과 첫머리의 `Exit code N`에서 종료 코드를 읽고, 실패가 아닌 결과는 0으로 둔다. 중단처럼 코드가 없는 실패는 비운다.
- 이벤트를 기록하지 못하면 화면에도 상태에도 반영하지 않는다. 화면이 기록에 없는 일을 보이는 것을 막기 위해서다.
- 진행 중인 실행이 없을 때 메인 에이전트의 글이나 도구 호출이 오면 입력 없이 provider가 시작한 턴으로 보고 입력 없는 새 실행을 기록한다. 사용량, 도구 결과처럼 그 밖의 늦은 이벤트는 가장 나중 실행에 붙인다. 멈춰 보류한 session의 늦은 출력은 새 실행을 만들지 않고 가장 나중 실행에 붙인다. 멈춘 작업이 저절로 이어지는 일을 막기 위해서다.
- 사용량 보고는 `store`에 사용량 행으로만 쓰고 기록 번호를 받지 않는다. 사용량이 패킷 재료인 기록에 섞이지 않게 하기 위해서다.
- 흐름이 완료 신호 없이 끊기면(`StreamLost`, 연결 종료) 효과 범위를 `unobserved`로 기록하고, 작업을 `결과 확인 필요`로 보이며, 실행 기록은 열어 두고, 열려 있던 session을 닫힌 것으로 다룬다. 다음 입력 때 보관한 ID로 다시 연다. 사용자에게 묻는 창은 띄우지 않고 그 작업만 확인으로 넘기며, 입력은 자동으로 다시 보내지 않는다([#245](https://github.com/woonyong-choi/saturn/issues/245), [#287](https://github.com/woonyong-choi/saturn/issues/287), [#39](https://github.com/woonyong-choi/saturn/issues/39)).
- 허가 요청은 TUI에 올리고 답을 기다리는 동안 작업을 `허가 기다림`으로 보인다. 답이 오기 전에 턴이 끝나거나 흐름이 끊기면 그 창을 모든 TUI에서 지운다. 더는 답할 수 없는 요청을 남기지 않기 위해서다.

### 메인 에이전트와 보조 에이전트

1. `sessions`는 채팅마다 메인 에이전트 하나를 유지한다.
2. 모델이나 provider가 바뀌면 `sessions`는 대상 provider의 메인 session을 재개하거나 새로 시작하고, 이전 메인 session은 인수 뒤 닫고 provider session ID를 보관한다.
3. router가 무관한 작업으로 판단하면 `queue`는 채팅에 속한 보조 에이전트를 시작한다.
4. 보조 에이전트는 끝나면 결과를 전달한 뒤 바로 종료한다.

살아 있는 메인 session은 `종료`가 아닌 메인 session이다. 열린 메인 session은 채팅마다 하나이고, `닫힘·재개 가능` 메인 session은 provider마다 하나까지 보관한다. 같은 provider의 더 오래된 보관 session은 `종료`로 둔다. `보류` session은 이 한도에 세지 않는다. 같은 provider면 열린 session에 그대로 보내고, `닫힘·재개 가능`이나 `보류`면 보관한 provider session ID로 재개한다. provider가 다르면 그 provider로 돌아가기 규칙으로 재개와 새 session을 가른다. 보관한 ID가 없으면 새 session을 연다. 보조 에이전트는 작업마다 새 session을 연다. 끝나면 바로 종료하므로 재개할 session이 없기 때문이다.

보조 에이전트가 물려받는 설정 층은 [설정](settings.md)에 있다. router가 무엇을 묻는지는 [router](router.md)에 있다.

### 모델 고르기

사용자는 TUI `/model`로 다음 입력부터 쓸 모델을 고른다([TUI](tui.md#영역)). Claude Code와 Codex의 `/model`이 session 안에서 다시 바꿀 때까지 유지되는 것과 같게, 고른 모델은 그 채팅에서 다시 고를 때까지 모든 입력에 붙는다([#168](https://github.com/woonyong-choi/saturn/issues/168) 결정).

- 목록은 provider가 알려 준다. Codex는 app-server `model/list`에서 숨기지 않은 모델을 쪽마다 `nextCursor`를 따라 모으고, Claude는 Claude Code `/model`이 보이는 별칭(기본, `opus`, `sonnet`, `haiku`)이다. 기본은 `--model`을 넘기지 않는 것과 같다. 설치된 provider만 보이고, 목록은 Claude, Codex 순이다. 첫 입력의 기본 provider와 같은 순서다.
- TUI가 `ListModels`(provider를 주면 그 provider만)로 요청하면 engine이 `Models` 알림으로 답한다. provider 하나가 목록을 못 주면 그 provider를 빼고 로그만 남기고, 모두 못 주면 빈 목록을 보내고 오류로 답한다. TUI 창이 끝없이 기다리지 않게 하기 위해서다.
- 고른 모델은 TUI가 `SetModel`로 알리면 engine이 채팅별로 기록 저장소에 저장하고(`chats.pinned_model`) 붙은 모든 TUI에 `ModelPinned`로 알린다. 고정한 채팅에 붙을 때도 같은 알림을 보낸다. 그래서 TUI를 다시 열거나 채팅을 옮겨도 다시 바꿀 때까지 유지된다. 입력은 모델을 싣지 않고, engine이 접수 때 채팅의 고정값을 읽어 입력에 남긴다. 모델이 provider를 정하고, 같은 이름이 두 provider에 있어도 구분되도록 `<provider>/<model>` 글로 저장한다.
- `sessions` 기록은 session을 열 때 고른 모델(`model`)을 남긴다. 고르지 않았으면 provider 기본값이라 비어 있다. 입력의 모델이 열린 메인의 모델과 다르면 그 메인을 쓰지 않고 새 메인 session을 열어 패킷을 넘기고, 떠나는 메인은 보관한다([provider 전환](#provider-전환)). 다른 모델로 연 보관 session은 되돌아갈 때도 재개하지 않는다. 열린 session의 모델을 provider 안에서 바꾸는 방식은 쓰지 않는다. session 기록의 모델과 실제 모델이 어긋나지 않게 하기 위해서다. 같은 provider 안에서 모델만 바뀐 교체는 provider 전환 안내 줄을 남기지 않는다.
- 맥락 정리로 여는 새 session은 이전 session의 모델을 이어 쓴다.
- provider의 `/model` 명령은 provider 명령 목록에서 빼고 Saturn `/model`로만 처리한다. provider가 몰래 모델을 바꿔 기록과 어긋나는 일을 막기 위해서다. 모델을 고르지 않은 채 provider 설정이나 환경으로 정해진 모델은 막지 않고 추적만 한다(위 provider 실행 절).
- router의 `target_model` 후보는 이 목록과 같다([router](router.md)). engine은 연결을 만들 때 받아 둔 목록을 후보로 넣고, router가 새 작업으로 판단한 입력에는 고른 모델을 그 입력의 모델로 적용하고, 이어 가기와 끼워 넣기는 현재 모델을 유지한다. 목록을 받기 전이거나 후보 밖 값이면 현재 모델로 보내고, 모델이 바뀌면 위 규칙대로 새 메인 session을 연다.

### provider 전환

한 채팅 안에서 Codex와 Claude를 바꿔 가도 채팅은 하나로 이어진다. 채팅마다 열린 메인 session은 하나다. `sessions`는 입력을 보내는 순간 대상 session을 정한다.

session 교체는 같은 채팅·역할 안에서 턴이 끝난 경계에만 한다. 진행 중인 턴이 교체로 끊기는 일을 막기 위해서다. session ID는 재사용하지 않는다. 대기열은 에이전트 session이 아니라 채팅에 둔다. 교체 중 들어온 입력이 옛 session을 가리키는 일을 막기 위해서다. 교체 중 들어온 입력은 새 session에 들어온 순서대로 보낸다. 입력 순서를 교체와 무관하게 지키기 위해서다.

새 session에는 패킷을 넘기고, 그 뒤로는 받지 않은 기록 번호 뒤의 변경분만 넘긴다. 새 session의 전달 기록 번호는 이전 session이 받은 번호보다 작아지지 않는다. 패킷이 그 번호까지의 기록을 담으므로 같은 결과를 두 번 붙이지 않기 위해서다. 패킷을 어떻게 고르는지는 [맥락 정리](context-management.md)에 있다.

- 메인 에이전트 번호는 session을 바꿔도 같게 이어 간다. 대기열의 작업이 에이전트 번호로 session을 가리키므로 번호가 달라지면 이어 갈 입력이 보낼 곳을 잃기 때문이다.
- 각 session은 자기 provider가 낸 이벤트를 받은 것으로 친다. 이벤트를 기록할 때마다 그 session의 전달 기록 번호를 올리고 턴이 끝날 때 저장한다. 그래서 돌아온 session에는 자기가 낸 결과가 다시 붙지 않는다.
- 패킷은 provider를 바꾸는 입력이거나 이어 갈 메인이 없는 채팅의 첫 턴(provider에는 턴 하나)으로 보낸다. 그 턴의 완료 신호는 작업 끝이 아니다. 입력이 보낸 턴의 완료만 작업 끝으로 보기 위해서다. 그 턴의 답은 입력이 연 실행에 함께 기록한다.
- 떠나는 메인은 새 session이 열린 뒤에 보관한다. 새 session을 열지 못하면 떠나는 메인은 그대로 열려 있다. provider 둘 다 열리지 않은 채팅이 생기는 일을 막기 위해서다.
- 패킷의 고정 구역이 `P_hard`도 넘으면 새 session을 열지 않고 입력을 작업과 함께 보류하며 TUI에 `고정 제약이 길어 맥락 정리를 미룹니다`와 제약 목록을 보인다. 사용자가 `/continue`를 하면 다시 판정한다.
- 패킷을 provider가 맥락 한도 초과로 거절하면 경쟁 구역을 줄여 한 번만 다시 보낸다. 줄인 패킷도 거절되거나 고정 구역만으로 넘치면 입력을 보류하고 `맥락 한도 초과로 멈춤 · /continue로 다시 시도하세요`를 보인다([맥락 정리](context-management.md#패킷-구성)).
- 열린 메인이 있으면 그 메인이 이어 갈 session이다. 보관 session이 더 나중에 등록돼도 열린 메인을 먼저 본다. provider를 오간 뒤 등록 순서와 열린 순서가 다르기 때문이다.
- 입력에 고정한 모델이 있으면 그 모델의 provider로 보낸다([모델 고르기](#모델-고르기)). 모델을 고르지 않은 입력의 provider는 `engine`의 내부 호출(`switch_provider`)로만 바꾼다.

### 그 provider로 돌아가기

provider를 바꿀 때 대상 provider에 보관한 메인 session이 있으면 `sessions`는 다음 순서로 판정한다.

1. 보관 session의 마지막 턴이 끝난 뒤 경과 시간이 그 provider의 캐시 유지 시간 안이면 재개한다.
2. 캐시 유지 시간이 지났으면 패킷 크기 `P`가 그 session의 마지막 활성 맥락 `A`보다 작을 때(`P < A`) 새 session을 열고 패킷을 넘기고, 아니면 재개한다.

캐시 유지 시간은 상수이고 설정 키가 없다. Codex는 5분이다. Claude는 구독 로그인이면 1시간, 그 밖(API 키 등)이면 5분이다. 구독 여부는 `providers/claude`가 `system/init`의 `apiKeySource`로 판단한다. 값이 `none`이면 API 키 없이 로그인한 구독이고, 다른 값이거나 값이 없으면 5분으로 본다. `providers/claude`는 판단한 값을 `CacheWindow` 이벤트로 알리고, `engine`은 이를 대화 기록에 넣지 않고 provider별로 마지막 값을 기록 저장소의 `meta` 표에 저장한다. 돌아갈 때는 그 provider 연결이 닫혔거나 `engine`을 다시 시작해 `system/init`이 아직 오지 않았어도 저장한 값으로 판정한다. 값을 한 번도 받지 못했을 때만 5분이다. 판정은 다음 입력을 보낼 때 한다.

- 재개하면 그 session이 마지막으로 받은 기록 번호 뒤의 변경분만 붙인다. 앞부분이 그대로인 맥락 뒤에 덧붙여 캐시와 원문 맥락을 함께 지키기 위해서다.
- 변경분이 많으면 [맥락 고르기](context-selection.md) 순서로 고른다.
- 2단계는 [맥락 정리](context-management.md)의 유휴 복귀 조건과 같은 식이다. 캐시가 끝난 뒤에는 `A`를 다시 쓰는 비용이 패킷을 새로 쓰는 비용보다 크기 때문이다.
- `sessions`는 보관 session마다 마지막 턴의 `A`와 끝 시각을 `record_last_turn`으로 받아 두고, 전송 대상을 정할 때 돌아갈 provider의 설정과 패킷 크기 `P`, 현재 시각(`ReturnInputs`)으로 `decide_return`을 부른다. 값을 저장하고 되살리는 일은 `store`의 몫이다. `engine`은 턴이 끝날 때 `store`의 `sessions` 표에 마지막 활성 맥락과 끝 시각을 먼저 쓰고 그다음 `record_last_turn`을 부른다. 시작할 때는 끝나지 않은 메인 session을 `register`하고 저장한 값으로 `record_last_turn`을 불러 재시작 전과 같은 판정을 낸다. 끝나지 않은 메인을 모두 되살리는 것은 다른 provider로 돌아갈 때 열려 있던 메인을 알아야 판정이 어긋나지 않기 때문이다. provider를 바꿀 때 `engine`은 떠나는 메인을 `set_state(ClosedResumable)`로 보관하고 새 session을 `register`하며, 그때 바뀐 같은 채팅의 session 상태를 한 거래로 저장한다. `Resume` 판정이면 `set_state(Open)`으로 돌리고 `attach_from`부터 붙인다. 마지막 턴 값이 없는 보관 session은 재개한다. 잴 수 없는 맥락을 이유로 새 session을 열지 않기 위해서다.
- 판정은 경과 시간, `A`, `P`, 캐시 유지 시간만 쓰는 `sessions` 코드다. Codex 원격 압축 요약처럼 맥락 내용을 볼 수 없어도 같은 규칙으로 판정하기 위해서다.
- 재개한 session의 첫 턴 캐시 적중은 [#10](https://github.com/woonyong-choi/saturn/issues/10)에서 잰다.

### session 닫기와 재개

1. 트리 유휴 뒤 5분 유예가 지나면 `sessions`는 session을 닫는다. 유예 중 새 턴이 시작되면 유예 시계를 멈춘다.
2. `store`는 닫은 session의 provider session ID를 보관한다.
3. 새 입력이 오면 `sessions`는 보관한 ID로 session을 재개한다.

`store`는 Saturn 밖에서 연 provider session을 채팅에 연결할 수 있다. Saturn이 관리하는 session은 이 연결 목록에서 뺀다. 닫았다 재개한 session의 재개 시간과 첫 턴 캐시 적중은 실측으로 확인한다([#10](https://github.com/woonyong-choi/saturn/issues/10)).

### 기록 번호로 결과 전달

1. `sessions`는 session마다 마지막으로 전달받은 기록 번호를 기록한다.
2. 다음 입력 때 `sessions`는 그 기록 번호 뒤에 쌓인 다른 에이전트의 결과 요약과 수정 파일 경로를 붙인다.
3. router가 쌓인 항목 전체에서 관련 항목을 고르고, 순서는 [맥락 고르기](context-selection.md)를 따른다.

에이전트끼리 직접 통신하지 않는다. 보조 에이전트 결과로 쉬는 메인 에이전트를 깨우지 않고, 메인 에이전트의 다음 입력 때 전달한다. 두 규칙 모두 맥락 전달을 Saturn 기록 번호 하나로 맞추기 위해서다.

### subagent 트리 추적

1. `agents`는 이벤트에서 subagent의 시작, 진행, 끝을 기록한다.
2. `agents`는 subagent를 부모 에이전트 아래에 등록한다.
3. `agents`는 부모 턴 완료만 작업 끝 후보로 본다.
4. 답이 먼저 나왔는데 subagent가 남아 있으면 `agents`는 `answered-tree-running`으로 둔다.
5. 에이전트와 모든 subagent가 끝나면 `agents`는 트리 유휴로 판정한다.
6. 입력 없이 provider가 시작한 턴은 `origin = provider-wake`로 기록한다.

작업 끝은 메인 에이전트와 모든 subagent가 끝난 때로 판정한다. 멈춤, 쓰기 잠금, compaction 경계는 subagent까지 끝났는지 알아야 정할 수 있기 때문이다. Codex 작업 끝은 부모 작업의 `turn/completed`로만 판정한다. 자식 작업의 끝을 작업 끝으로 잘못 보지 않기 위해서다. Claude subagent 이벤트와 Codex 자식 session 신호는 실측으로 확인한다([#17](https://github.com/woonyong-choi/saturn/issues/17), [#20](https://github.com/woonyong-choi/saturn/issues/20)).

부모 턴이 끝난 뒤 메인 에이전트의 글이나 도구 호출이 오면 새 턴이 시작된 것으로 본다. 입력 없이 provider가 시작한 턴도 트리 유휴로 잘못 보지 않기 위해서다. 흐름이 완료 신호 없이 끝나면 트리를 끝난 것으로 두고 관찰 끊김으로 기록한다. 더 받을 이벤트가 없는 트리를 실행 중으로 남겨 두지 않기 위해서다. 멈춤 신호 순서는 깊은 subagent부터이고, 깊이가 같으면 subagent id 순서, 마지막이 메인이다.

### 사용량 보고와 턴 값

1. `agents`는 사용량 보고의 원값, 범위, 대상 에이전트, 모델을 한 행으로 기록한다.
2. 범위는 `main-turn`, `tree-total`, `thread-cumulative` 중 하나다.
3. `agents`는 턴 값을 저장하지 않고, 필요할 때 기록 순서로 계산한다.
4. 누적 범위면 같은 session의 이번 누적에서 직전 누적을 뺀다.
5. 턴 범위면 보고값을 그대로 턴 값으로 쓴다.
6. session이 바뀌면 누적 계산을 새로 시작한다.

누적 범위의 직전 누적은 같은 session에서 같은 에이전트와 subagent의 앞 누적 보고에서 칸마다 찾는다. 바로 앞 보고에 그 칸이 없었거나 두 보고 사이에 부모 턴이 둘 이상 끝났으면 차이가 여러 턴에 걸친다고 표시한다. 누적이 직전보다 작으면 그 칸의 턴 값은 NULL로 두고 여러 턴에 걸친다고 표시한다. 계산할 수 없는 턴 값을 지어내지 않기 위해서다. `tree-total`은 그 턴의 트리 합계이므로 턴 범위처럼 그대로 쓴다.

사용량 보고는 원값과 범위를 그대로 넘기고 0으로 채우지 않는다. Codex session 누적을 그대로 더하면 중복 계산되기 때문이다. provider가 보고하지 않은 값은 NULL로 둔다. 지어낸 값을 막기 위해서다. 캐시 토큰은 Claude `cache_read_input_tokens`와 `cache_creation_input_tokens`, Codex `cachedInputTokens`와 `cacheWriteInputTokens`를 각각 캐시 읽기와 캐시 쓰기 열에 기록한다. Claude 사용량 보고의 범위는 실측으로 확인한다([#19](https://github.com/woonyong-choi/saturn/issues/19)).

### 트리 전체 중지

1. `agents`는 추적된 subagent부터 멈춤 신호 대상 순서를 정한다.
2. `providers`는 Codex에는 자식 session별 `turn/interrupt`를, Claude에는 멈춤 제어 신호를 보낸다.
3. 10초 뒤 남은 프로세스가 있으면 `processes`는 provider 프로세스 묶음에 중지 신호를 보낸다.
4. 중지 신호 뒤 5초(초안)가 지나도 남으면 `processes`는 강제 종료한다.
5. `processes`는 트리 전체의 종료를 확인한 뒤 완료를 보고한다.

- provider는 새 프로세스 묶음의 리더로 실행한다. 자식 환경은 비운 뒤 제외 목록 변수를 지운 환경과 중첩 표지만 넣는다. 실행 명세의 환경 값은 디버그 출력에 담지 않는다.
- `processes`는 1초(초안)마다 프로세스 표를 읽어 리더의 자손을 기억한다. 묶음 밖으로 빠져나간 자손에는 신호를 보내지 않고, 살아 있으면 남은 수로 센다.
- 리더를 남기는 중지(Codex app-server처럼 session을 이어 쓸 때)는 리더를 뺀 묶음 구성원에만 신호를 보낸다. 멈춘 session은 보류로 두고 provider에 열린 채 남겨, 이을 때 다시 열지 않는다. 지금은 두 provider 모두 이 방식이다.
- 완료는 세 조건을 모두 확인한 뒤에만 보고한다. 모든 에이전트가 멈춘 뒤의 완료 신호(`TurnCompleted` 또는 흐름 끊김)를 보냈거나 멈출 때 이미 답을 마쳤고, 트리가 유휴이고, 모든 프로세스 묶음의 중지 결과가 돌아왔다. 멈춤 직전에 보낸 턴의 낡은 유휴 표시를 완료로 읽지 않기 위해서다. 중지 결과가 하나라도 `남은 프로세스`이거나 확인하지 못하면 완료 대신 `멈춤 확인 안 됨 · N개 남음`을 보고한다.
- 멈춤 요청은 응답한 뒤 별도 작업에서 묶음을 중지한다. 중지를 기다리는 동안에도 다른 요청과 provider 이벤트를 처리하기 위해서다.

멈춤은 Saturn session의 모든 에이전트와 subagent에 닿는다. 에이전트 하나만 멈추는 기능은 취소와 모델 교체 같은 내부 처리에서만 쓰기 때문이다. 트리 전체 종료를 확인하기 전에는 완료라고 하지 않는다. subagent가 남은 채 멈췄다고 보이는 일을 막기 위해서다. Claude 백그라운드 subagent의 중지는 실측으로 확인한다([#18](https://github.com/woonyong-choi/saturn/issues/18)). 멈춘 작업의 보류와 재개는 [입력 처리](input-handling.md)에 있다.

### session 상태

| 상태 | 뜻 | 다음 상태 |
|---|---|---|
| `열림` | provider 대화에 연결 중인 session | `닫힘·재개 가능`, `보류`, `종료` |
| `닫힘·재개 가능` | 유예 뒤나 provider 전환 뒤 닫고 provider session ID를 보관한 session | `열림`, `종료` |
| `보류` | 멈춤이나 증명되지 않은 크래시로 보류한 session | `열림`, `종료` |
| `종료` | 새 session으로 교체됐거나, 같은 provider의 새 보관 session에 밀렸거나, 보류 종료로 끝난 session | 없음 |

크래시 뒤 session을 어떻게 나누는지는 [engine 수명과 복구](engine-lifecycle.md)에 있다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| Codex `turn/steer`가 활성 턴 없음으로 실패 | 확정 미전달로 기록하고, 다시 판단하지 않고 같은 session에 `turn/start`로 보낸다. |
| Claude 끼워 넣기 중 턴 종료 | provider가 추가 메시지를 다음 턴에 처리하므로 따로 처리하지 않는다. |
| 멈춤 뒤 묶음 밖으로 빠져나간 프로세스 존재 | 완료라고 하지 않고 `멈춤 확인 안 됨 · N개 남음`을 보고한다. |
| 완료 신호 없는 흐름 끝, 읽는 중 종료, 끝이 없는 subagent | 관찰 끊김으로 보고 `effect_scope`를 `unobserved`로 기록한다. |
| provider 흐름의 관찰 중단 | `effect_scope`를 `unobserved`로 기록하고 자동으로 이어 가지 않는다. |
| 중간 사용량 보고 누락 | 차이가 여러 턴에 걸친다고 표시하고 0으로 채우지 않는다. |

관찰 끊김을 `unobserved`로 두는 것은 관찰하지 못한 외부 효과가 있을 수 있기 때문이다.

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 채팅마다 열린 메인 session은 하나이고, 보관 session은 provider마다 하나까지다. | `saturn-terminal/core/src/sessions/tests.rs`의 `provider_switches_keep_one_open_and_one_archive_per_provider`, `register_second_archive_ends_older_one_and_drops_its_last_turn` |
| 캐시 유지 시간 안인 보관 session으로 돌아가면 `A`와 무관하게 재개하고 변경분만 붙인다. 지났으면 `P < A`일 때만 새 session을 연다. | `saturn-terminal/core/src/sessions/context.rs`의 `decide_return_matches_rule_table`, `saturn-terminal/core/src/sessions/tests.rs`의 `target_for_send_warm_below_threshold_resumes_archive`, `target_for_send_warm_above_threshold_resumes_archive` |
| Claude 캐시 유지 시간은 `apiKeySource`가 `none`이면 1시간, 아니면 5분이고, 저장한 값이 없으면 5분이다. | `saturn-terminal/engine/src/providers/claude/tests.rs`의 `cache_window_without_api_key_is_one_hour`, `cache_window_with_api_key_is_five_minutes_and_missing_source_is_unknown`, `saturn-terminal/engine/src/lifecycle/sessions.rs`의 `cache_window_without_provider_report_is_five_minutes`, `cache_window_reported_by_provider_survives_closed_connection_and_restart` |
| 재시작 뒤에도 보관 session의 재개 판정이 같다. | `saturn-terminal/engine/src/lifecycle/sessions.rs`의 `cache_window_inside_resumes_archived_session`, `cache_window_inside_resumes_even_when_context_is_large`, `cache_window_expired_opens_new_session`, `cache_window_expired_resumes_when_context_is_smaller_than_packet`, `restart_without_last_turn_resumes_archived_session` |
| provider를 바꿀 때 떠나는 메인은 보관하고, 보관과 재개 상태를 저장한다. | `saturn-terminal/engine/src/lifecycle/sessions.rs`의 `archive_main_ends_older_archive_and_saves_both_states`, `resume_main_opens_archive_and_returns_delivered_number` |
| session 교체 뒤 새 session에는 받지 않은 기록 번호 뒤의 변경분만 넘긴다. | `saturn-terminal/engine/src/lifecycle/switch_round_trip.rs`의 `codex_to_claude_to_codex_hands_over_without_duplicates_or_gaps`, `delivered_numbers_never_go_down_across_switches` |
| provider 전환에서 패킷을 만들고 session별 전달 기록 번호를 지킨다. | `saturn-terminal/engine/src/lifecycle/switch_round_trip.rs`의 `codex_to_claude_to_codex_hands_over_without_duplicates_or_gaps`, `switch_tells_the_user_which_provider_took_over`, `saturn-terminal/engine/src/handoff.rs`의 `packet_carries_input_answer_and_tool_result_with_session_title` |
| 패킷으로 연 턴의 완료는 작업 끝이 아니다. | `saturn-terminal/engine/src/lifecycle/switch_round_trip.rs`의 `packet_turn_completion_does_not_end_the_task_on_the_new_provider`, `saturn-terminal/engine/src/lifecycle/turn_end.rs`의 `packet_turn_completion_is_not_the_end_of_the_task` |
| 패킷의 고정 구역이 `P_hard`를 넘으면 새 session을 열지 않고 입력을 보류한다. | `saturn-terminal/engine/src/lifecycle/switch_round_trip.rs`의 `packet_over_the_hard_limit_is_not_sent_and_the_input_is_held` |
| 열린 메인이 있으면 보관 session이 더 나중에 등록돼도 열린 메인이 이어 갈 session이다. | `saturn-terminal/core/src/sessions/tests.rs`의 `live_main_prefers_the_open_main_over_a_later_registered_archive` |
| 이벤트는 처리 전에 기록하고, 기록하지 못하면 화면과 상태에 반영하지 않는다. | `saturn-terminal/engine/src/lifecycle/events.rs`의 `event_is_recorded_before_it_reaches_the_screen_and_the_state`, `events_get_one_number_each_in_arrival_order`, `event_from_the_provider_pump_reaches_the_engine_loop` |
| 입력 없이 시작한 턴과 늦은 이벤트, 끊긴 흐름을 기록한다. | `saturn-terminal/engine/src/lifecycle/events.rs`의 `output_after_the_turn_ended_starts_a_run_without_input`, `lost_stream_records_unobserved_and_needs_a_check`, `usage_is_recorded_as_a_usage_row_and_not_as_a_ledger_event` |
| 턴 끝에서 마지막 턴 값을 기록하고 트리 유휴일 때만 작업을 끝낸다. | `saturn-terminal/engine/src/lifecycle/turn_end.rs`의 `turn_end_records_the_last_turn_value_and_finishes_the_task`, `answer_with_a_running_subagent_ends_the_turn_only_when_the_tree_is_idle`, `waiting_input_is_sent_after_the_turn_ends` |
| 권한은 Saturn 규칙이 정본이고, 권한 외 Saturn 기본값은 사용자 provider 설정에 값이 없을 때만 실행 인자로 넘긴다. | [권한](permissions.md)의 요구사항을 확인하고, 사용자 설정에 값이 있는 안전망 항목의 인자가 실행 명령에 없는지 확인한다. |
| 작업 끝은 메인 에이전트와 모든 subagent가 끝난 때로 판정한다. | `saturn-core`의 `agents` 시험 `on_event_answer_with_subagent_left_is_answered_tree_running`, `saturn-terminal/engine/src/lifecycle/turn_end.rs`의 `answer_with_a_running_subagent_ends_the_turn_only_when_the_tree_is_idle` |
| 누적 범위 사용량의 턴 값은 같은 session의 직전 누적을 뺀 값이다. | Codex 누적 보고 두 개에서 턴 값이 차이로 나오는지 확인한다. |
| 멈춤 신호는 추적된 subagent까지 보낸다. | `saturn-terminal/engine/src/lifecycle/stop.rs`의 `stop_signals_the_deepest_subagent_first_and_finishes_only_when_the_tree_is_idle` |
| 멈춤 신호 10초 뒤 남은 프로세스 묶음에는 중지 신호를 보낸다. | `saturn-terminal/engine/src/processes/mod.rs`의 `stop_sends_term_after_grace` |
| 트리 유휴와 프로세스 중지를 모두 확인한 뒤에만 멈춤 완료를 보고한다. | `saturn-terminal/engine/src/lifecycle/stop.rs`의 `stop_is_not_complete_until_the_process_group_is_confirmed_stopped`, `stop_without_a_finished_turn_signal_is_not_complete`, `processes_left_outside_the_group_are_reported_instead_of_done` |
| 끼워 넣기와 멈춤 신호가 문서대로 provider에 전달된다. | [#5](https://github.com/woonyong-choi/saturn/issues/5), [#27](https://github.com/woonyong-choi/saturn/issues/27) |
| 닫은 session을 보관한 ID로 재개한다. | [#10](https://github.com/woonyong-choi/saturn/issues/10) |
| 채팅의 고정 모델이 저장되고 TUI가 붙을 때 알려진다. 고정 입력도 router 관계 판단을 받고 고정 모델로 전송된다. | `saturn-terminal/engine/src/lifecycle/model.rs`의 `pinned_model_is_saved_and_told_to_every_tui_that_attaches`, `pinned_input_gets_the_relation_judgment_and_the_pinned_model`, `saturn-terminal/engine/src/lifecycle/decision.rs`의 `pinned_model_input_still_gets_the_relation_judgment_while_task_runs` |
| 고정한 모델이 session을 여는 모델과 provider를 정하고 session 기록에 남는다. | `saturn-terminal/engine/src/lifecycle/model.rs`의 `pinned_model_opens_the_session_with_that_model_and_records_it`, `pinned_model_decides_the_provider`, `saturn-terminal/engine/src/store/sessions.rs`의 `session_model_round_trips_through_live_mains` |
| 모델이 바뀌면 새 메인 session을 열고, 같은 모델이면 열린 session을 쓴다. | `saturn-terminal/engine/src/lifecycle/model.rs`의 `changing_the_model_opens_a_new_main_session_with_a_packet`, `same_model_keeps_using_the_open_session` |
| router가 후보를 받아 고른 모델로 보내고 새 작업으로 판단되면 그 모델의 session을 연다(메인이면 새 메인 session). 고정 모델이면 묻지 않고, 목록을 받기 전이거나 후보 밖 값이면 현재 모델을 쓴다. | `saturn-terminal/engine/src/lifecycle/target_model.rs`의 `target_model_candidates_are_the_model_list_in_provider_order`, `target_model_chosen_by_the_router_is_applied`, `target_model_picks_the_provider_of_the_chosen_model`, `target_model_on_a_second_new_task_opens_a_session_with_that_model`, `target_model_is_not_asked_when_the_model_is_pinned`, `target_model_is_not_asked_before_the_model_list_arrives`, `target_model_is_ignored_when_the_input_continues_current_work`, `target_model_other_keeps_the_default_model`, `target_model_outside_the_candidates_is_ignored` |
| 모델 목록은 설치된 provider 순서로 오고 provider로 거를 수 있다. Codex는 숨긴 모델을 빼고, Claude 기본은 `--model`을 넘기지 않는다. | `saturn-terminal/engine/src/lifecycle/model.rs`의 `model_list_comes_in_provider_order`, `model_list_can_be_limited_to_one_provider`, `saturn-terminal/engine/src/providers/codex/tests.rs`의 `model_list_entries_skip_hidden_models_and_fall_back_to_the_id`, `saturn-terminal/engine/src/providers/claude/tests.rs`의 `default_model_is_not_passed_to_claude` |
| subagent의 시작과 끝을 이벤트로 추적한다. | [#17](https://github.com/woonyong-choi/saturn/issues/17), [#20](https://github.com/woonyong-choi/saturn/issues/20) |
| Claude 백그라운드 subagent까지 멈춘다. | [#18](https://github.com/woonyong-choi/saturn/issues/18) |
| Claude 사용량 보고의 범위를 올바르게 표시한다. | [#19](https://github.com/woonyong-choi/saturn/issues/19) |
| Claude 스트림에서 provider 명령 결과와 허가 요청을 받는다. | [#26](https://github.com/woonyong-choi/saturn/issues/26) |
| 채팅에 더한 폴더가 session을 열 때 provider 실행 인자로 간다. | `saturn-terminal/engine/src/providers/claude/tests.rs`의 `add_dir_follows_the_model_as_one_variadic_flag_before_the_permission_args`, `saturn-terminal/engine/src/providers/codex/tests.rs`의 `add_dir_goes_to_the_thread_config_when_a_session_opens` |
| 허가 답이 없는 동안 턴이 멈춰 있고, 답한 뒤 이어진다. 답은 Codex에는 요청과 같은 JSON-RPC 번호로, Claude에는 `control_response`로 나간다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `command_approval_is_answered_with_the_same_numeric_request_id`, `file_change_approval_keeps_a_string_request_id`, `saturn-terminal/engine/src/providers/claude/tests.rs`의 `allow_once_answers_can_use_tool_with_the_request_input` |
| 허가 요청을 TUI에 올리고 사용자 답을 provider에 넘기며, 받지 못했으면 다시 답할 수 있다. | `saturn-terminal/engine/src/lifecycle/events.rs`의 `permission_request_reaches_the_tui_and_the_answer_reaches_the_provider`, `answer_for_a_request_nobody_asked_is_refused`, `answer_the_provider_did_not_take_keeps_the_request_for_another_try`, `turn_end_withdraws_requests_nobody_answered` |
| 이미 답했거나 모르는 허가 요청에 답하면 보내지 않는다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `answering_an_unknown_or_answered_request_is_not_sent`, `saturn-terminal/engine/src/providers/claude/tests.rs`의 `answering_an_unknown_or_answered_request_is_not_sent` |
| Claude Code 도구 호출의 도구 종류, 경로, 읽은 줄 범위, 바뀐 줄 수와 셸 종료 코드를 이벤트에 싣는다. | `saturn-terminal/engine/src/providers/claude/tests.rs`의 `detail_of_edit_counts_changed_lines_and_keeps_path`, `detail_of_multi_edit_sums_every_edit`, `detail_of_write_has_no_line_change`, `detail_of_read_range_needs_offset_and_limit`, `detail_of_test_command_is_test_run`, `shell_exit_code_reads_prefix_only_for_errors` |
| 명령이 테스트 실행인지 셸인지 가르고, 바뀐 줄 수에서 앞뒤 공통 줄을 뺀다. | `saturn-terminal/engine/src/providers/tool_detail.rs`의 `classify_command_test_runners_are_test_runs`, `classify_command_other_commands_are_shell`, `line_change_replaced_lines_count_both_sides`, `line_change_common_lines_are_not_counted`, `line_change_empty_old_counts_only_added` |
| Codex 명령의 경로와 종료 코드, 파일 수정의 경로와 바뀐 줄 수와 수정 내용을 같은 칸에 싣는다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `detail_of_read_command_keeps_action_paths`, `detail_of_test_command_is_test_run_without_paths`, `detail_of_file_change_counts_diff_lines`, `detail_of_file_change_added_file_counts_whole_text`, `tool_output_file_change_is_path_and_diff`, `exit_code_of_command_reads_code_and_ignores_other_items`, `turn_events_are_converted_in_order` |
| Codex 추론 항목은 `Reasoning` 종류로 남기고 도구 결과 후보에서 뺀다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `detail_of_reasoning_is_not_a_candidate`, `saturn-protocol/src/event.rs`의 `is_candidate_only_reasoning_is_excluded` |
| 셸이 감싼 명령은 안쪽 명령으로 기록한다. | `saturn-terminal/engine/src/providers/tool_detail.rs`의 `unwrap_shell_login_shell_wrapper_gives_inner_command`, `unwrap_shell_escaped_single_quote_stays_in_inner_command`, `unwrap_shell_plain_or_unbalanced_command_is_unchanged`, `saturn-terminal/engine/src/providers/codex/tests.rs`의 `activity_of_wrapped_command_is_inner_command` |
| Codex와 Claude Code의 도구 결과를 같은 충실도로 기록으로 바꾼다. | [변환 수정 뒤 기록 전환 품질 측정](../experiments/record-fidelity-stage2/report.md): 같은 받는 쪽에서 Codex 기록 패킷과 Claude Code 기록 패킷의 정답률 차이 0.0%p [0.0, 0.0]로 채택. 경로 120/120 대 120/120, 메모 필드 192/192 대 191/192 |

## 단점

- provider별 연결 규약을 구현하고, 규약이 바뀌면 계속 따라가야 한다.
- subagent 추적을 직접 구현해야 한다.
- 설정으로 증명하지 못한 실행은 크래시 뒤 자동으로 이어 가지 못한다.

## 대안

- 한 번 실행 방식(`codex exec`, `claude -p`)은 끼워 넣기와 Codex 맥락 크기 관찰이 불가능해 버렸다([결정 기록](../decisions/2026-09-29-persistent-provider-connections.md)).
- 실행 인자로 subagent와 네트워크를 고정하는 방식은 사용자 설정을 무시해 버렸다([결정 기록](../decisions/2026-09-29-minimal-provider-control.md)).

## 미해결 질문

- 채팅에 더한 폴더를 열린 session에 넣는 방법(Claude stream-json 제어 요청, Codex 턴 단위 쓰기 폴더)과, Codex 읽기 전용 샌드박스에서 `writable_roots`가 효과가 있는지. 지금은 다음 session부터 적용한다 ([#193](https://github.com/woonyong-choi/saturn/issues/301))
- 메인이 아닌 provider의 명령을 고르면 그 provider session을 새로 열지, 메인 전환을 물을지, 거절할지 ([#41](https://github.com/woonyong-choi/saturn/issues/41))
- router 상태에 subagent 목록을 넣을지, 개수만 넣을지, 넣지 않을지 ([#63](https://github.com/woonyong-choi/saturn/issues/63))
- 수정 파일 목록을 실행 경계의 파일 상태 차이로 계산할지, provider 이벤트로 계산할지 ([#65](https://github.com/woonyong-choi/saturn/issues/65))
- 패킷 고정 구역의 "현재 목표"와 "끝나지 않은 항목"을 무엇으로 뽑을지. 정해지기 전에는 목표는 마지막 입력이고 제약은 빈 목록이며 끝나지 않은 항목은 결과가 없는 도구 호출이다 ([#90](https://github.com/woonyong-choi/saturn/issues/90))
- 보낸 뒤 결과를 모르는 작업을 사용자가 푸는 방법. 지금은 `/continue <작업>`이 중단 결과를 붙인 새 입력을 보내고 원래 입력은 `전달 중`으로 둔다 ([#90](https://github.com/woonyong-choi/saturn/issues/90))
