# provider 연결과 session

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [provider마다 입력을 계속 받는 상시 연결을 만든다](../decisions/2026-09-29-persistent-provider-connections.md), [provider 설정과 subagent 사용은 사용자 설정을 따르고 Saturn은 추적만 한다](../decisions/2026-09-29-minimal-provider-control.md), [권한 판단의 정본은 Saturn 설정의 `permission` 규칙 하나로 둔다](../decisions/2026-10-02-saturn-permission-authority.md), [provider 연결을 채팅마다 따로 둔다](../decisions/2026-10-02-per-chat-provider-connections.md), [provider는 열린 id의 어댑터로 붙이고 확장은 Saturn 저장소에 설치해 session을 열 때 주입한다](../decisions/2026-10-04-open-providers-and-saturn-extensions.md), [Codex와 Claude는 직접 연결을 유지하고 새 provider는 ACP 어댑터로 시작한다](../decisions/2026-10-04-direct-adapters-and-acp-for-new-providers.md) |

## 요약

engine은 Codex와 Claude Code에 켜 둔 연결을 두고, 한 채팅 안에서 여러 provider session을 이어 쓴다. 채팅마다 메인 에이전트가 하나 있고, 무관한 작업은 보조 에이전트가 맡는다. engine은 provider 이벤트를 Saturn 용어로 바꿔 기록하고, subagent 트리와 사용량을 추적한다. provider는 Saturn 인터페이스, 어댑터, 기능 전달 세 계층으로 연결하고, 열린 id로 식별하며, 어댑터 레지스트리에 등록한다.

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

`core`는 provider 연결 공통 규격인 `ProviderClient` trait을 정의하고, engine의 `providers` 모듈이 `CodexClient`와 `ClaudeClient`로 구현한다. 구현은 끼워 넣기, 멈춤 신호, compaction, 사용량 보고를 Saturn 용어로 넘긴다. provider 고유 이름은 어댑터 폴더 `providers/codex`, `providers/claude` 안에서만 쓴다. TUI와 앱이 provider를 몰라도 화면을 그리게 하기 위해서다. 어댑터를 열린 id로 더하는 구조는 [provider 계층과 어댑터](#provider-계층과-어댑터)에 있다.

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
- session 닫기는 `thread/unsubscribe`다. 기록을 지우는 `thread/delete`는 쓰지 않고, `thread/archive`는 크래시로 끊긴 자식 thread를 정리할 때만 쓴다.
- 자식 thread는 부모 `collabAgentToolCall`(`tool`이 `spawnAgent`) 완료 항목의 `receiverThreadIds`로 알고, 자식의 이벤트는 자기 `threadId`로 온다. 자식 thread의 `thread/started` 알림(`parentThreadId`)은 자식을 만든 18회 모두 오지 않았고 `initialize`에 `experimentalApi`를 켜도 같았다([실측](../experiments/codex-provider-behavior/report.md)). 어댑터는 자식을 두 경로로 등록한다. 부모의 `spawnAgent` 완료 항목은 항목을 낸 thread가 이미 등록돼 있고 항목의 `senderThreadId`도 그 thread일 때만 `receiverThreadIds`의 자식을 그 thread의 자식으로 등록하며(손자는 자식이 낸 항목으로 등록한다), `thread/started`는 `parentThreadId`가 등록된 thread일 때만 등록한다. 처음 보는 `threadId`라는 이유만으로 현재 채팅의 자식으로 붙이지 않는다. 자식의 알림은 생성 완료 항목보다 먼저 올 수 있어(실측 순서: 자식의 `thread/status/changed`가 부모의 완료 항목 앞에 오고, 변환하는 `turn/started`와 승인 요청은 18회 모두 완료 항목 뒤에 왔다), 부모 관계를 모르는 thread의 변환 대상 알림(`thread/closed` 포함)과 승인·입력 요청은 thread 8개, thread마다 64개 안에서 받은 순서대로 쥐어 두었다가 등록 직후 한 번만 처리한다. 등록 전에 닫힌 자식은 등록 직후 시작과 끝을 함께 알리고 잊는다. 시작과 끝은 턴이 있었는지와 무관하게 짝으로 알린다. 자식마다 시작을 알린 뒤 끝을 아직 알리지 않았는지를 기억해, 턴 시작 없이 닫히거나 보관 한도로 `turn/started`가 밀려난 자식도 닫힐 때 끝을 알리고, 턴 완료로 이미 끝을 알린 자식은 닫혀도 다시 알리지 않는다. `thread/closed`는 실측에서 한 번도 관측하지 못했고, 등록 전에 온 알림과 요청의 순서는 가짜 알림으로만 확인했다. 한도를 넘기거나 끝내 관계가 확인되지 않아 밀려난 알림은 버리고, 밀려난 승인은 거절 결정으로, 입력 요청은 오류 응답으로 돌려 provider가 답을 기다리며 멈추지 않게 한다. 거절 응답을 실제 Codex가 어떻게 받는지는 확인하지 않았다. 멈춤은 자식 thread마다 `turn/interrupt`로 보내고(깊은 subagent부터), 부모의 `turn/interrupt`는 자식을 끝내지 않는다. 남은 명령 프로세스는 Saturn이 소유한 프로세스 묶음 중지로 정리한다([멈춤](input-handling.md)). 연결 번호와 provider session 번호는 다른 값이고 이 등록은 provider thread 번호만 쓴다.
- 활성 턴 없음은 오류 코드가 따로 없어 `turn/steer` 오류 문구로 판정한다(초안).
- 맥락 크기는 `thread/tokenUsage/updated`의 `last.totalTokens`이고 메인 턴 끝에 보낸다. 누적 사용량의 새 입력은 `total.inputTokens - cachedInputTokens`다.
- 명령 대응표는 `compact` → `thread/compact/start`, `review` → `review/start`(대상 `uncommittedChanges`)이고, 명령 목록에서 `new`, `resume`, `fork`, `quit`, `exit`를 뺀다(초안). 스킬은 `turn/start` 입력에 `{"type":"skill","name","path"}` 항목으로 넣는다.
- 승인 요청(`item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `item/permissions/requestApproval`, 옛 이름 `execCommandApproval`·`applyPatchApproval`, `_meta.codex_approval_kind`가 있는 `mcpServer/elicitation/request`)은 요청의 JSON-RPC 번호를 숫자와 문자열 그대로 기억했다가 같은 번호로 응답한다. 번호는 `PermissionRequested`의 `request_id`로 올린다. 승인이 아닌 elicitation과 `item/tool/requestUserInput`은 `InputRequested`로 올리고 같은 방식으로 번호를 기억했다가 응답한다([입력 요청](input-requests.md)). 사용자 답을 provider 값으로 바꾸는 표는 [권한](permissions.md#허가-요청-창과-답)에 있다.
- 채팅에 더한 폴더([engine 수명과 복구](engine-lifecycle.md#채팅-폴더와-이어-열기))는 `thread/start`와 `thread/resume`의 `config`에 `sandbox_workspace_write.writable_roots`로 넘긴다(초안). app-server에는 `--add-dir`에 해당하는 인자가 없어서다. 작업 폴더 쓰기 샌드박스에서 이 값이 쓰기 가능 폴더가 된다(읽기 전용 샌드박스에서도 편집 승인 응답 뒤에는 효과가 있었다). `writable_roots`에 넣은 폴더 안 편집은 승인 응답 뒤 3/3회 적용됐다([실측](../experiments/provider-permission-real/report.md)). 값은 session을 열 때만 넘기고 열린 session에는 넣지 않아 다음 session부터 적용한다.
- `thread/start`와 `thread/resume` 응답은 적용된 `approvalPolicy`, `approvalsReviewer`, `sandbox`(`type`과 `networkAccess`), `cwd`, `runtimeWorkspaceRoots`를 싣는다(6/6). `turn/start`의 `sandboxPolicy` 덮어쓰기는 적용됐지만(네트워크를 허용한 `curl` 3/3 성공) 덮어썼다는 알림은 없었다. `thread/settings/updated` 알림도 `turn/started`의 설정 값도 오지 않았고(0/3), 새 process의 `thread/resume`은 덮어쓰기 전 값(`networkAccess=false`)을 돌려줬다(3/3). `config/read`는 `thread/start` 인자를 반영하지 않는다(`approval_policy`, `sandbox_mode`가 비어 있음, 6/6). 그래서 적용 설정의 근거는 `thread/start`와 `thread/resume` 응답뿐이고 턴 단위 덮어쓰기는 보낸 Saturn만 안다([실측](../experiments/codex-provider-behavior/report.md)).
- `/review` 턴에서는 `turn/started` 알림의 턴 id가 `review/start` 응답과 `turn/completed`의 턴 id와 다르다(3/3). 서버가 활성으로 보는 id는 응답의 id였다. 코드는 `turn/started`의 id를 활성 턴으로 둔다(`codex/convert.rs:114`).
- 권한은 Saturn 규칙을 전용 `CODEX_HOME`과 `thread/start` 인자로 넘기고([권한](permissions.md)), 사용자 설정은 `~/.codex/config.toml`의 루트와 선택된 프로필에서 `model_auto_compact_token_limit` 키가 있는지만 본다.

Claude Code 실행 인자는 Claude Code 2.1.285의 `--help`로 확인했다.

- 새 session은 Saturn이 만든 UUID를 `--session-id`로 넘긴다. stream-json은 첫 입력 전에 `system/init`을 내지 않으므로 session id를 미리 알기 위해서다. 재개는 `--resume <id>`이고, 500ms(초안) 안에 프로그램이 끝나면 재개 실패로 본다.
- 채팅에 더한 폴더는 `--add-dir <폴더>...` 하나로 넘긴다(Claude Code 2.1.285 `--help`의 여러 값 인자). 열린 session에는 넣지 않는다.
- 권한은 `--permission-prompt-tool stdio`와 `--settings`의 `ask` 목록으로 Saturn 규칙에 넘기고([권한](permissions.md)), 안전망 `--autocompact` 값은 허용 범위 100000~1000000으로 맞춘다.
- 사용자 폴더는 `CLAUDE_CONFIG_DIR`(비어 있지 않은 절대 경로)이 있으면 그 폴더, 없으면 `~/.claude`다. 설정 읽기, 직접 설치 탐색, 제외 명령 검사가 같은 함수로 이 폴더를 정하고, provider 프로세스는 같은 환경을 받아 같은 폴더를 읽는다. 상대 경로는 폴더를 정할 수 없어 session을 열지 않고 `NotSent`로 알린다. 폴더가 없거나 읽을 수 없으면 그 층의 파일이 없는 것으로 본다. 심볼릭 링크는 따라 읽는다.
- Codex의 사용자 폴더(읽기용)는 `CODEX_HOME`(비어 있지 않은 절대 경로)이 있으면 그 폴더, 없으면 `HOME/.codex`다. 설정 읽기, 직접 설치 탐색, 전용 폴더 준비가 같은 함수로 이 폴더를 정한다. TUI가 `CODEX_HOME`을 `Attach`로 넘긴다. provider 프로세스가 받는 `CODEX_HOME`은 그 사용자 폴더가 아니라 engine이 만든 Saturn 전용 폴더(주입용)이고, 사용자 폴더는 읽기만 한다. 상대 경로는 폴더를 정할 수 없어 session을 열지 않고 `NotSent`로 알린다.
- 사용자 설정은 사용자 폴더의 `settings.json`, `<작업 폴더>/.claude/settings.json`, `settings.local.json`의 `autoCompactEnabled`와 환경 변수 `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, `DISABLE_COMPACT`가 있는지만 본다(초안).
- 명령 목록에서 `clear`, `resume`, `exit`, `quit`를 뺀다(초안). 허가 요청은 `control_request`의 `can_use_tool`로 받고 `control_response`로 답한다. 요청의 `input`은 허용 응답의 `updatedInput`으로 되돌려 주려고 요청 번호와 함께 기억한다(`Bash`만 실측, [권한](permissions.md)).
- 맥락 크기는 마지막 메인 `assistant` 메시지 `usage`의 입력, 캐시 읽기, 캐시 쓰기 합이다(초안).
- interrupt 제어 응답은 10초, session 닫기 뒤 종료는 5초까지 기다리고, 넘으면 프로세스 묶음 중지로 넘어간다(초안).

Codex `turn/steer`는 실측에서 받아들여졌다([실측](../experiments/codex-provider-behavior/report.md)). 실행 중인 턴에 활성 턴 id로 보낸 입력은 받아들여지고 `turnId`를 돌려주며 끼워 넣은 지시가 실행에 반영됐다(3/3). 승인 요청에 답하기 전의 턴도 받아들였다(3/3). 거절은 모두 JSON-RPC 오류 -32600이고 문구로 구분한다.

| 상태 | 오류 문구 |
|---|---|
| 활성 턴 없음(새 thread, 턴 완료 뒤, interrupt 뒤) | `no active turn to steer` |
| 틀린 `expectedTurnId` | ``expected active turn id `요청한 id` but found `활성 id` `` |
| `/review` 턴 | `cannot steer a review turn`, `data.codexErrorInfo.activeTurnNotSteerable.turnKind`는 `review` |
| 수동 `/compact` 턴 | `cannot steer a compact turn`, `turnKind`는 `compact` |
| 없는 thread | `thread not found: id` |
| 빈 `input` | `input must not be empty` |

같은 두 턴에 보낸 `turn/start`는 `failed to submit turn input: ActiveTurnNotSteerable { turn_kind: Review }`(코드 -32603)로 거절돼 구조화된 정보가 없다. 지금은 Codex와 Claude 모두 어댑터의 `STEER_VERIFIED`가 거짓이라 끼워 넣기를 대기로 바꿔 처리한다([#5](https://github.com/woonyong-choi/saturn/issues/5)). 이때 TUI는 `바로 반영 준비 중`을 보인다. 사용자가 바로 반영되지 않는 이유를 알게 하기 위해서다. 입력을 어디로 보낼지는 [입력 처리](input-handling.md)가 정한다.

### provider 계층과 어댑터

provider 연결은 세 계층으로 나눈다. 이 절의 계층 분리, 설명자, 레지스트리는 구현했고([#412](https://github.com/woonyong-choi/saturn/issues/412)), 명령 목록 전달과 provider CLI 버전 알림은 구현했고, 확장 주입은 스킬, 명령, MCP 서버의 형식만 구현했다(실제 provider 확인 전, [기능 목록과 확장](extensions.md#주입)). 인터페이스는 `core`의 `ProviderClient` trait과 `ProviderEvent`, `protocol`의 `Provider`이고, engine `providers/adapter.rs`가 어댑터 계약(`Adapter`, `AdapterConnection`)을 정한다.

| 계층 | 하는 일 | 위치 |
|---|---|---|
| Saturn 인터페이스 | provider와 무관하게 정한 계약. 판 번호를 붙여 따로 유지한다. | `core`의 `providers`와 `protocol` |
| 어댑터 | provider마다 계약을 구현한다. 실행 인자, provider 설정, 이벤트 변환, 권한 번역, 확장 주입은 여기에만 있다. | engine의 `providers/<id>` |
| 기능 전달 | 어댑터가 알린 명령, 스킬, 플러그인, 모드를 이름과 종류로 TUI 명령과 권한 규칙에 보인다. | engine의 `providers`와 TUI |

Saturn 인터페이스 계약은 아래 동작을 provider 이름 없이 정한다.

- session 열기, 닫기, 재개
- 턴 보내기, 끼워 넣기, 멈춤, 맥락 정리 요청
- 권한 요청과 입력 요청에 대한 답
- 이벤트 수신
- 모델 목록과 기능 목록
- 오류 구분. 보내기 전 실패는 `NotSent`, 보낸 뒤 결과를 모르는 실패는 `Unknown`이다.

- 계약에는 판 번호(`interface_version`)를 붙인다. 한 판 안에서는 항목의 뜻을 바꾸거나 빼지 않고, 바꿀 때는 판을 올린다. 1판은 지금 `ProviderClient`의 동작이다. 어댑터가 알린 판을 engine이 지원하지 않으면 그 어댑터를 등록하지 않고 로그만 남긴다. 판이 어긋난 어댑터가 session 도중에 동작이 달라지는 일을 막기 위해서다.
- 어댑터 밖 공통 코드는 `protocol`, `core`, `providers/<id>` 밖의 engine 모듈, `tui`, `cli`다. 공통 코드는 provider id를 불투명한 글자로 저장하고 전달하고 같은지 비교할 뿐, id 값으로 동작을 가르지 않는다. provider마다 달라지는 값(표시명, 기본값, 지원하는 기능)은 어댑터 설명자와 기능 목록으로 받는다. 비테스트 코드에서 `providers/<id>` 밖에 provider 이름 분기가 없게 하기 위해서다.
- 어댑터를 engine 프로세스 밖에 두는 방식은 [직접 연결과 ACP 어댑터](#직접-연결과-acp-어댑터)에 있다.

### provider id와 설명자

provider는 닫힌 enum이 아니라 열린 id 글자로 식별한다. id는 소문자 영문, 숫자, `-`만 쓰고 어댑터 폴더 이름과 같다(`codex`, `claude`). `/`를 쓰지 않는다. 모델 고정 글 `<provider>/<model>`을 첫 `/`에서 나누기 위해서다.

id 값은 구현했다. `protocol`의 `Provider`는 id 글자를 담은 값이고 형식(1~32자의 소문자 영문, 숫자, `-`)이 틀린 글자는 만들지 못한다. 값은 프로세스 안에서 글자를 한 번만 잡아 두고 복사해 쓰므로, 서로 다른 id는 한 프로세스에 64개까지만 받는다. 엉뚱한 입력이 메모리를 늘리지 못하게 하기 위해서다. 값을 읽을 때는 대소문자를 가리지 않는다.

어댑터는 등록할 때 설명자를 알린다.

| 값 | 뜻 |
|---|---|
| id | provider 식별 글자 |
| 표시명 | 화면과 사용량에 보이는 이름 |
| 실행 파일 | 설치 여부를 확인하고 실행할 파일 이름 |
| 기본 순서 | 고정 모델도 이어 갈 메인 session도 없는 첫 입력과 모델 목록이 provider를 고르는 순서. 지금은 Claude, Codex 순이다. |
| 기능 | 이 어댑터가 구현하는 동작 중 선택 항목(끼워 넣기, 맥락 정리 요청 등) |
| 지시 문서 이름 | provider가 스스로 읽는 문서의 이름. 패킷에 넣지 않을 문서를 정한다(`AGENTS.md`, `CLAUDE.md`). |
| 인터페이스 판 | 구현한 계약의 판 번호 |
| 설정 키와 기본값 | `provider.<id>.*` 아래의 키와 기본값([설정](settings.md#설정-키)) |

옛 기록은 그대로 읽힌다.

- 기록 저장소의 `sessions`와 `runs` 표 provider 열은 지금 `Codex`, `Claude`로 쓰여 있다. 열린 id로 바뀐 뒤에도 이 값은 대소문자를 가리지 않고 같은 id(`codex`, `claude`)로 읽는다. 옛 행은 고치지 않고 스키마 이관도 하지 않는다. 새로 쓰는 값은 id 글자다. 옛 버전으로 되돌리는 것은 지원하지 않는다([기록 저장과 보존](records.md#스키마-이관)).
- 모델 고정 글 `<provider>/<model>`의 provider 자리는 지금도 소문자 `codex`, `claude`이고 id와 같다. 기존 값이 그대로 읽힌다.
- 레지스트리에 없는 id가 기록에 있으면(어댑터를 뺀 뒤) 기록은 그대로 두고 그 provider의 session은 열지 않는다. 화면은 id 글자를 표시명 대신 보인다.

### 어댑터 등록

engine은 시작할 때 레지스트리에 어댑터를 등록한다. 레지스트리는 설명자 목록을 기본 순서로 돌려주고, id로 어댑터를 만들고, 실행 파일이 있는지 확인한다. 표시명, 설치 확인, 연결 만들기, 첫 입력 기본 provider를 레지스트리가 맡는다.

- 새 어댑터는 `providers/<id>` 폴더를 더하는 것으로 붙는다. 공통 코드의 수정은 없다. 이것을 가짜 provider 하나로 시험한다.
- provider 고유 설정 키는 `provider.<id>.*` 열린 이름공간에 둔다. 지금 `context.codex`, `context.claude` 키의 이관 규칙은 [설정](settings.md#설정-키)에 있다.
- 새 provider를 더할 때 구현해야 하는 것. 공통 동작은 `Adapter` trait의 기본 메서드와 공용 도우미가 하고, 어댑터는 provider마다 다른 부분만 채운다.

| 구분 | 필수 | 선택(기본 구현이 있다) |
|---|---|---|
| 설명자 | id, 표시명, 실행 파일, 기본 순서, 지시 문서 이름, 인터페이스 판, 맥락 기본값 | 기능 목록(끼워 넣기, 맥락 정리 요청), 확장 주입 형식(`extensions`, 기본은 주입하지 않음) |
| 연결 | `connect`와 `AdapterConnection`(session 열기, 턴, 이벤트 변환) | 끼워 넣기, 맥락 정리, 줄 세운 입력 보내기 |
| 실행 설정 | 없음 | `translate_permission`(기본은 규칙을 번역하지 않고 질문 기능만 따름), `rules_fingerprint`, `read_version`(기본은 `--version`), `injectability`(기본은 설명자의 주입 형식), `direct_installs`(기본은 없음, 사용자가 provider에 직접 설치한 항목을 읽기만 해서 올림) |
| 설정 키 | 없음 | `provider.<id>.*` 키와 기본값은 설명자가 알린다([설정](settings.md#설정-키)) |
| 확장 주입 | 설명자의 주입 형식과, provider 형식으로 바꾸는 코드(설정 파일 쓰기, 실행 인자) | 파일 배치, 정의 읽기, 이름 겹침 처리는 공용 도우미([기능 목록과 확장](extensions.md#새-provider에-확장-주입을-붙일-때)) |

- 등록은 컴파일할 때 정한 목록이다. engine `providers/builtin.rs`가 시작할 때 어댑터마다 한 줄씩 등록하고, 등록하지 못한 어댑터(같은 id나 지원하지 않는 판)는 로그만 남기고 건너뛴다(초안). 어댑터가 시작할 때 스스로 등록하는 방식은 프로세스 밖 어댑터를 붙일 때 다시 정한다.

### 직접 연결과 ACP 어댑터

Codex와 Claude는 지금처럼 직접 연결(Codex app-server, Claude stream-json)을 유지하고 ACP로 연결하지 않는다. 끼워 넣기, 하위 에이전트 관리, 맥락 정리 지시, 사용량 상세가 ACP 안정판에 없거나 RFD 단계이고, 보내기 전 실패와 보낸 뒤 불명의 구분은 명세에서 확인하지 못했으며(2026-10-04 확인), 확장으로 보태면 provider, 원본 번역기, ACP 명세 세 겹을 따라가야 해 유지보수가 더 들기 때문이다([결정 기록](../decisions/2026-10-04-direct-adapters-and-acp-for-new-providers.md)).

- 새 provider는 처음에 ACP 어댑터 하나로 붙여 기본 기능으로 쓴다. 주력으로 쓰게 되고 그 provider가 깊은 기능을 열어 주면 직접 어댑터로 바꾼다. 계층이 나뉘어 있으므로 이 교체는 어댑터만 바꾸는 일이다. ACP 어댑터는 지금 구현하지 않는다.
- ACP는 Zed와 JetBrains가 함께 관리하는 공개 표준이고([관리 문서](https://agentclientprotocol.com/community/governance), 2026-10-04 확인), 저장소 라이선스는 Apache-2.0이다([저장소](https://github.com/agentclientprotocol/agent-client-protocol), 2026-10-04 확인). 확장 메서드는 `_` 접두사와 `_meta` 필드로 한다([확장 문서](https://agentclientprotocol.com/protocol/extensibility), 2026-10-04 확인).
- 직접 연결의 업데이트는 engine이 시작할 때 provider CLI 버전을 읽어 마지막으로 확인한 버전과 다르면 알리는 것으로 대응한다. 버전이 바뀌면 실제 provider로 핵심 흐름(입력, 전환, 다시 열기)만 도는 빠른 확인 절차를 `scripts/e2e`에 둔다. 차이는 어댑터 안에서만 고친다. 버전 알림은 구현했고 빠른 확인 절차는 구현 전이다([#412](https://github.com/woonyong-choi/saturn/issues/412)).
- 버전은 어댑터의 `read_version`이 읽는다. 기본은 설정이 아닌 `PATH`의 실행 파일에 `--version`을 주고(자식 환경은 `PATH`와 `HOME`만, 5초 제한) 출력 첫 줄에서 숫자로 시작하는 첫 낱말을 쓴다. 읽은 값은 [기록 저장소](records.md)의 `provider_versions` 표와 비교한다. 같으면 아무것도 하지 않고, 처음 확인하는 provider는 알림 없이 기록만 하며, 다르면 기록을 덮어쓰고 처음 붙는 TUI에 `Alert::ProviderUpdated`를 한 번 보낸다. 읽지 못한 provider는 기록을 그대로 둔다. 읽은 버전은 붙을 때 `StartInfo`의 provider 버전에도 실린다(초안).

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

provider 명령 목록에서 TUI 전용 명령과 Saturn session 명령이 대신하는 명령은 뺀다. 뒤에서 연결할 때 쓸 수 없거나 Saturn session 기록과 어긋나기 때문이다. 어댑터가 올린 목록은 연결 작업이 요청과 이벤트를 처리한 뒤 다시 읽어 마지막으로 알린 것과 다르면 `Commands` 알림으로 그 채팅에 붙은 TUI에 보내고, 나중에 붙는 TUI에는 붙을 때 마지막 목록을 보낸다. 연결이 끝나면 기억한 목록을 버린다. 목록이 처음부터 비어 있으면 알리지 않는다. Saturn 명령이 아닌 `/이름`은 고른 항목이 어느 provider의 것이든 원문 그대로 메인 에이전트의 provider에 보내고, 다른 provider session을 새로 열거나 전환을 묻지 않는다. 목록의 항목 종류와 노출 규칙은 [기능 목록과 확장](extensions.md#기능-목록)에 있다. 채팅 이어 열기와 폴더 추가는 [engine 수명과 복구](engine-lifecycle.md#채팅-폴더와-이어-열기)에 있다. Claude 스트림에서 슬래시 명령은 사용자 입력 줄에 글 그대로 보내면 되고, 결과와 허가 요청은 명령 종류에 따라 다르게 온다. 프로젝트 명령은 모델 턴으로 확장되어 도구 호출이 `can_use_tool`로 오고 `result` 하나로 끝난다. `/context` 같은 로컬 명령은 허가 요청 없이 `assistant` 이벤트 하나(글이 `local_command_source`에 실린다)와 글이 있는 `result` 하나로 끝난다. `/compact`는 `status`, `compact_boundary`, 합성 `user` 이벤트를 거쳐 글이 빈 `result`로 끝난다. `init`의 목록에 없는 명령은 오류 없이 모델에 글로 전달되어 모델이 답하는 한 턴이 된다(Claude Code 2.1.288, 각 3/3, [실험](../experiments/claude-provider-behavior/report.md)).

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
- Codex는 한 턴에 해설과 최종 답처럼 응답 메시지를 여럿 보낼 수 있다. `providers/codex`가 글 조각의 `itemId`를 기억해 같은 thread에서 항목이 바뀌면 새 글 조각 앞에 빈 줄 하나(`\n\n`)를 붙인다. 메시지가 줄바꿈 없이 이어 붙어 한 문장처럼 보이는 것을 막기 위해서다. `itemId`가 없는 조각은 그대로 두고, 새 턴이 시작되면 기억을 비워 턴 맨 앞에는 붙이지 않는다. 공통 코드는 이 구분을 몰라도 된다.
- Codex 추론 항목은 `Thinking` 활동과 `Reasoning` 종류로 남긴다. 실행 줄의 `생각 중` 표시에 쓰고, 도구 결과 후보에서는 뺀다. 추론은 도구가 아니고 결과가 없기 때문이다.
- `/bin/zsh -lc '...'`처럼 셸이 감싼 명령은 안쪽 명령을 명령 글로 싣는다. 사용자가 보낸 명령이 provider마다 다르게 기록되지 않게 하기 위해서다.
- Claude는 실패한 `Bash` 결과 첫머리의 `Exit code N`에서 종료 코드를 읽고, 실패가 아닌 결과는 0으로 둔다. 중단처럼 코드가 없는 실패는 비운다.
- 이벤트를 기록하지 못하면 화면에도 상태에도 반영하지 않는다. 화면이 기록에 없는 일을 보이는 것을 막기 위해서다.
- 진행 중인 실행이 없을 때 메인 에이전트의 글이나 도구 호출이 오면 입력 없이 provider가 시작한 턴으로 보고 입력 없는 새 실행을 기록한다. 사용량, 도구 결과처럼 그 밖의 늦은 이벤트는 가장 나중 실행에 붙인다. 멈춰 보류한 session의 늦은 출력은 새 실행을 만들지 않고 가장 나중 실행에 붙인다. 멈춘 작업이 저절로 이어지는 일을 막기 위해서다.
- 사용량 보고는 `store`에 사용량 행으로만 쓰고 기록 번호를 받지 않는다. 사용량이 패킷 재료인 기록에 섞이지 않게 하기 위해서다.
- 흐름이 완료 신호 없이 끊기면(`StreamLost`, 연결 종료) 효과 범위를 `unobserved`로 기록하고, 작업을 `결과 확인 필요`로 보이며, 실행 기록은 열어 두고, 열려 있던 session을 닫힌 것으로 다룬다. 다음 입력 때 보관한 ID로 다시 연다. 사용자에게 묻는 창은 띄우지 않고 그 작업만 확인으로 넘기며, 입력은 자동으로 다시 보내지 않는다. 사용자는 `/continue <작업>`으로 확인 입력을 보내 잇고, 원래 입력은 `전달 중`으로 남겨 다시 보내지 않는다. 이미 반영됐을 수 있는 일을 두 번 하지 않기 위해서다([#245](https://github.com/woonyong-choi/saturn/issues/245), [#287](https://github.com/woonyong-choi/saturn/issues/287), [#39](https://github.com/woonyong-choi/saturn/issues/39)).
- Claude의 `result`는 `is_error`가 참이거나 `subtype`이 `success`가 아니면 오류 결과라 `TurnCompleted`로 올리지 않는다. 사용량과 맥락 크기를 올린 뒤 완료 신호 없이 끊긴 흐름과 같게 `StreamLost`로 올려 작업을 `결과 확인 필요`로 두고 자동으로 다시 보내지 않는다. 턴이 이미 돌았을 수 있어 보내지 않음이 확정이 아니기 때문이다. 맥락 초과(`terminal_reason`이 `prompt_too_long`)도 같고, 공통 오류 `ContextExceeded`는 보내지 않음이 확정일 때만 쓰므로 여기에 쓰지 않는다. 멈춤 요청 뒤에 온 결과는 오류 모양이어도 요청한 완료로 본다. 오류 결과 뒤 같은 session을 다시 열 때 아직 남은 이전 프로세스는 먼저 닫는다. 오류 결과의 모양은 Claude Code 2.1.288 설치본에서 읽었고 실제 호출로 재지 않았다([#328](https://github.com/woonyong-choi/saturn/issues/328)).
- 허가 요청은 TUI에 올리고 답을 기다리는 동안 작업을 `허가 기다림`으로 보인다. 답이 오기 전에 턴이 끝나거나 흐름이 끊기면 그 창을 모든 TUI에서 지운다. 더는 답할 수 없는 요청을 남기지 않기 위해서다.

### 원시 이벤트 관측 기록

provider가 어떤 순서로 무엇을 보내는지 실제 실행에서 보려는 디버그 전용 기록이다. 변환한 이벤트는 기록 저장소에 남지만, 변환하지 않고 버린 메시지나 변환 전의 순서는 남지 않는다. 예를 들어 Codex가 자식 등록 전에 `thread/closed`나 승인 요청을 보내는지([#438](https://github.com/woonyong-choi/saturn/issues/438))는 이 기록 없이 알 수 없다. provider 응답 원문을 run별로 모으는 일은 [원시 응답 수집](records.md#provider-원시-응답-수집)이 맡고, 이 기록은 값을 남기지 않아 그 일을 대신하지 않는다. 두 기록은 같은 줄을 읽는 자리에서 나오지만 따로 켜고 끈다. 원시 응답 수집은 항상 켜져 있다.

1. 사용자가 사용자 설정에 `debug.provider_events = true`를 둔다. 기본은 꺼짐이고 폴더 층에서는 바꿀 수 없다([설정](settings.md)).
2. provider 연결이 메시지 하나를 받을 때마다 어댑터가 그 메시지의 방법 이름과 자리(요청, 알림, 응답, 줄)를 정해 공통 기록기에 넘긴다.
3. 공통 기록기는 메시지에서 ID와 필드 이름만 뽑아 `<Saturn 홈>/logs/provider-events-<날짜>.log`에 한 줄을 더한다.

- 한 줄은 JSON이고 값은 `seq`(기록 순서, 파일 하나에서 연결 전체에 걸쳐 1부터 이어짐), `at`(받은 시각, UTC 밀리초), `provider`, `connection`(연결 번호, 같은 engine에서 연결마다 다름), `frame`(`request`, `notification`, `response`, `line`), `kind`(방법 이름), `ids`, `fields`다. 필드가 상한을 넘어 잘렸으면 `truncated`가 참이다.
- `ids`는 필드 경로를 키로, 그 값의 배열을 값으로 갖는다. 이름이 `id`이거나 `Id`, `Ids`, `_id`, `_ids`로 끝나는 필드의 글자와 숫자만 담고(thread, turn, 항목, 요청 ID, 자식 thread 목록), 128자를 넘는 값은 ID로 보지 않는다.
- `fields`는 메시지에 있는 필드 이름의 경로 목록이다(`params.item.receiverThreadIds`처럼 점으로 잇고 배열 칸은 `[]`). 깊이는 8, 개수는 300까지다. 이름으로 쓸 수 있는 것은 영문자나 밑줄로 시작하고 영문자, 숫자, 밑줄뿐이며 숫자가 6개 이상 이어지지 않는 이름이다. 경로나 ID를 키로 쓴 표처럼 값이 이름 자리에 들어온 경우는 `*`로 적는다.
- 방법 이름도 영문자, 숫자, `/`, `.`, `:`, `_`, `-`뿐인 80자 이하일 때만 그대로 적고, 아니면 `?`로 적는다.
- 메시지의 글, 명령, 경로, 파일 내용, 인자 값은 남기지 않는다. router 키와 일치하는 문자열은 메시지를 읽을 때와 줄을 쓰기 전에 두 번 가리며, 이름과 ID에도 적용한다([router 키 보호](router-key-security.md#출력-마스킹)).
- 꺼져 있으면 아무것도 쓰지 않고 파일과 폴더도 만들지 않는다. 켜고 끄는 값은 채팅마다 하나이고 이미 열린 연결도 설정이 바뀌면 바로 따른다(입력을 접수하거나 설정 파일이 바뀌어 새 설정 번호가 생길 때 맞춘다).
- 파일은 소유자만 읽고 쓰며(`0600`) 날짜마다 하나다. 30일이 지난 `provider-events-*.log`는 날짜가 바뀔 때 지운다. engine 로그와 같은 보관 기간이다.
- provider 고유 이름(Codex의 `method`, Claude의 `type`과 `subtype`)은 `providers/codex`, `providers/claude`가 읽어 방법 이름으로 바꾸고, 공통 기록기는 provider 이름을 모른다. 새 provider 어댑터는 받은 메시지마다 같은 기록기를 한 번 부르면 같은 형식으로 남는다.
- 쓰기에 실패해도 연결과 이벤트 처리는 막지 않는다.

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
- TUI가 `ListModels`(provider를 주면 그 provider만)로 요청하면 engine이 응답 `result`의 `Models`로 답한다. 연결과 조회가 끝난 뒤에 응답하며, 그동안 engine의 다른 요청 처리는 기다리지 않는다([provider 요청 작업](#provider-요청-작업)). provider 하나가 목록을 못 주면 그 provider를 빼고 로그만 남기고, 모두 못 주면 빈 목록을 돌려주고 로그만 남긴다. TUI 창이 끝없이 기다리지 않게 하기 위해서다.
- 고른 모델은 TUI가 `SetModel`로 알리면 engine이 채팅별로 기록 저장소에 저장하고(`chats.pinned_model`) 붙은 모든 TUI에 `ModelPinned`로 알린다. 고정한 채팅에 붙을 때도 같은 알림을 보낸다. 그래서 TUI를 다시 열거나 채팅을 옮겨도 다시 바꿀 때까지 유지된다. 입력은 모델을 싣지 않고, engine이 접수 때 채팅의 고정값을 읽어 입력에 남긴다. 모델이 provider를 정하고, 같은 이름이 두 provider에 있어도 구분되도록 `<provider>/<model>` 글로 저장한다.
- `sessions` 기록은 session을 열 때 고른 모델(`model`)을 남긴다. 고르지 않았으면 provider 기본값이라 비어 있다. 입력의 모델이 열린 메인의 모델과 다르면 그 메인을 쓰지 않고 새 메인 session을 열어 패킷을 넘기고, 떠나는 메인은 보관한다([provider 전환](#provider-전환)). 다른 모델로 연 보관 session은 되돌아갈 때도 재개하지 않는다. 열린 session의 모델을 provider 안에서 바꾸는 방식은 쓰지 않는다. session 기록의 모델과 실제 모델이 어긋나지 않게 하기 위해서다. 같은 provider 안에서 모델만 바뀐 교체는 provider 전환 안내 줄을 남기지 않는다.
- 맥락 정리로 여는 새 session은 이전 session의 모델을 이어 쓴다.
- provider의 `/model` 명령은 provider 명령 목록에서 빼고 Saturn `/model`로만 처리한다. provider가 몰래 모델을 바꿔 기록과 어긋나는 일을 막기 위해서다. 모델을 고르지 않은 채 provider 설정이나 환경으로 정해진 모델은 막지 않고 추적만 한다(위 provider 실행 절).
- router의 `target_model` 후보는 이 목록과 같다([router](router.md)). 오토 모드(실험 옵션, 기본은 매뉴얼)에서만 묻고, 기본 모델과 선택 방식은 [기본 모델과 선택 방식](#기본-모델과-선택-방식)에 있다. engine은 연결을 만들 때 받아 둔 목록을 후보로 넣고, router가 새 작업으로 판단한 입력에는 고른 모델을 그 입력의 모델로 적용하고, 이어 가기와 끼워 넣기는 현재 모델을 유지한다. 목록을 받기 전이거나 후보 밖 값이면 현재 모델로 보내고, 모델이 바뀌면 위 규칙대로 새 메인 session을 연다.

### 기본 모델과 선택 방식

새 작업을 어느 모델로 보낼지는 기본 모델(`model.default`)과 선택 방식(`model.mode`) 둘이 정한다([설정](settings.md#설정-키), [#338](https://github.com/woonyong-choi/saturn/issues/338) 결정). 기본 모델은 `<provider>/<model>` 글이고, 선택 방식은 매뉴얼(`manual`, 기본)과 오토(`auto`)다. 오토는 router가 새 작업의 모델을 고르는 실험 옵션이라 사용자가 명시해야 켜지고, 모델 선택 순효과 실험([#542](https://github.com/woonyong-choi/saturn/issues/542))을 통과하기 전에는 기본으로 켜지 않는다.

새 작업으로 판단된 입력의 모델은 아래 순서에서 처음 나오는 값이다. 이어 가기와 끼워 넣기는 이 순서를 타지 않고 현재 모델을 유지한다. 단 채팅에 이어 갈 메인 session이 없으면(router 판단이 실패한 첫 입력이 그렇다) 유지할 현재 모델이 없으므로 이어 가기와 끼워 넣기도 `/model` 고정, 없으면 기본 모델로 연다(router의 `target_model` 선택은 이어 가기에 쓰지 않는다). 이미 열린 메인이 있으면 판단이 실패해도 그 모델을 유지하고 기본 모델로 바꾸지 않는다([#506](https://github.com/woonyong-choi/saturn/issues/506)).

| 순서 | 값 | 오토 | 매뉴얼 |
|---|---|---|---|
| 1 | `/model`로 채팅에 고정한 모델 | 사용 | 사용 |
| 2 | router `target_model`이 고른 모델(확신도 0.6 이상, 후보 안, provider가 지금 알린 목록에 있는 모델) | 사용 | 묻지 않음 |
| 3 | 사용자 선호(`model.prefer`) 중 품질을 확정한 후보 | 사용 | 사용 안 함 |
| 4 | 기본 모델 | 사용 | 사용 |
| 5 | provider 기본값(`--model`을 넘기지 않음) | 기본 모델이 없을 때 | 기본 모델이 없을 때 |

- 명시 고정(`/model`)과 선호(`model.prefer`)는 다르다. 고정은 그 채팅의 모든 새 작업에 붙고 router, 선호, 품질 자료가 덮지 못한다. 선호는 강제가 아니라 [모델 평가 근거 목록](model-evidence.md)에서 품질을 확정한 후보 안의 우선순위이고, 오토 모드에서 router가 고르지 못했을 때만 기본 모델보다 앞선다. 확정하지 못한 선호(목록에 없음, 근거 부족, revision 불명)는 쓰지 않고 판단 기록에 건너뛴 이유를 남긴다. 지금은 provider가 모델의 revision을 알려 주지 않아 품질을 확정한 후보가 없으므로 선호는 기록만 남는다.
- router가 고른 모델이 적용 직전에 provider가 알린 모델 목록에 없으면 지원하지 않는 선택(`unsupported`)으로 기록하고 선호, 기본 모델 순으로 넘어간다. 어댑터가 지원 목록으로 알리는 것은 모델 이름뿐이라 추론 깊이는 고르지 않고 provider 기본값을 따른다. 추론 깊이 조합은 어댑터가 지원 조합을 알려 줄 때 더한다.
- 정한 규칙(`pinned`, `router`, `preference`, `default`, `current`)과 router 선택을 쓰지 못한 이유(`manual`, `no-candidates`, `router-failed`, `invalid`, `fallback`, `unsupported`), 건너뛴 선호, 후보 지문, 정책 지문을 판단 기록과 함께 남긴다([기록 저장과 보존](records.md#판단-기록)). 판단이 어긋나 적용하지 않았으면 적용하지 않은 것으로 남긴다.
- 매뉴얼은 `target_model` 질문만 뺀다. `keep_current` 같은 관계 판단은 오토와 똑같이 묻는다. 매뉴얼에서도 `/model` 뒤 입력이 모두 대기해 병렬 작업이 막히는 일을 피하기 위해서다([router](router.md)).
- 기본 모델과 선택 방식은 입력을 접수할 때 고정한 설정 번호의 값을 쓴다. 설정 파일을 직접 고치면 설정 파일 감시가 병합하고 붙은 TUI에 `ModelSettings`로 알린다. 알림은 접속마다 그 접속의 실행 `-c`를 얹은 설정으로 만들고, 같은 값은 그 접속에 다시 알리지 않는다. 같은 채팅의 다른 접속에는 그 접속의 설정으로 따로 알린다.
- engine은 채팅에 붙을 때 `ModelSettings`(`default`, `mode`)를 보낸다. `default`가 `None`이면 아직 고르지 않은 것이다.
- 기본 모델을 고르지 않았으면 TUI가 `기본 모델을 고르세요` 창을 연다. 실행마다 한 번만 열고, `Esc`로 닫으면 저장하지 않고 다음 실행에 다시 묻는다. 이미 고른 적이 있으면 묻지 않는다. 창은 `ListModels`를 provider 없이 보내 목록을 받는다.
- 이 목록은 설치돼 연결할 수 있는 provider 모두에서 받는다. engine은 연결이 아직 없는 provider도 연결을 만들어 목록을 받고, 어느 provider를 이미 연결했는지나 `/model` 창을 연 적이 있는지에 따라 목록이 달라지지 않는다. 목록을 못 받은 provider는 빼고 로그만 남긴다. 이 창이 모든 provider의 목록을 받아 두므로, 그 뒤 오토 모드의 `target_model` 후보도 같은 목록이다.
- 고른 모델은 TUI가 `SetDefaultModel`로 알리면 engine이 사용자 설정 파일(`~/.saturn/config.toml`)의 `model.default`에 쓴다([설정 파일 편집](settings.md#설정-파일-편집)). 폴더 설정이나 실행 `-c`가 같은 키를 정했으면 그 값이 이기고, 알림은 병합 결과를 보인다.
- `/model` 창에서 `m`은 선택 방식을 오토와 매뉴얼 사이에서 바꾸는 `SetModelMode`를 보내고 engine이 `model.mode`에 쓴다. `d`는 고른 줄을 기본 모델로 저장한다. `Enter`는 지금처럼 이 채팅에만 고정한다.
- 지금 선택 방식은 바닥줄(`모델 오토`, `모델 매뉴얼`)과 `/model` 창 첫 줄(`기본 모델 claude · opus · 선택 방식 오토`)에 보인다.
- 기본 모델은 사용자가 모델을 따로 고르지 않은 새 작업에 쓴다. 이미 열린 메인 session의 모델과 다르면 위 규칙대로 새 메인 session을 연다.
- Jev가 `target_model`을 맞게 고르는지는 이 기능 밖의 별도 실험으로 확인한다.

### provider 전환

한 채팅 안에서 Codex와 Claude를 바꿔 가도 채팅은 하나로 이어진다. 채팅마다 열린 메인 session은 하나다. `sessions`는 입력을 보내는 순간 대상 session을 정한다.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/provider-switch.ko.dark.svg">
  <img src="../assets/provider-switch.ko.light.svg" alt="provider를 바꿔도 채팅은 하나로 이어지고 새 session은 Saturn 기록에서 고른 패킷을 받는다" width="100%">
</picture>

session 교체는 같은 채팅·역할 안에서 턴이 끝난 경계에만 한다. 진행 중인 턴이 교체로 끊기는 일을 막기 위해서다. session ID는 재사용하지 않는다. 대기열은 에이전트 session이 아니라 채팅에 둔다. 교체 중 들어온 입력이 옛 session을 가리키는 일을 막기 위해서다. 교체 중 들어온 입력은 새 session에 들어온 순서대로 보낸다. 입력 순서를 교체와 무관하게 지키기 위해서다.

새 session에는 패킷을 넘기고, 그 뒤로는 받지 않은 기록 번호 뒤의 변경분만 넘긴다. 새 session의 전달 기록 번호는 이전 session이 받은 번호보다 작아지지 않는다. 패킷이 그 번호까지의 기록을 담으므로 같은 결과를 두 번 붙이지 않기 위해서다. 패킷을 어떻게 고르는지는 [맥락 정리](context-management.md)에 있다.

- 메인 에이전트 번호는 session을 바꿔도 같게 이어 간다. 대기열의 작업이 에이전트 번호로 session을 가리키므로 번호가 달라지면 이어 갈 입력이 보낼 곳을 잃기 때문이다.
- 각 session은 자기 provider가 낸 이벤트를 받은 것으로 친다. 이벤트를 기록할 때마다 그 session의 전달 기록 번호를 올리고 턴이 끝날 때 저장한다. 그래서 돌아온 session에는 자기가 낸 결과가 다시 붙지 않는다.
- 패킷은 provider를 바꾸는 입력이거나 이어 갈 메인이 없는 채팅의 첫 턴(provider에는 턴 하나)으로 보낸다. 그 턴의 완료 신호는 작업 끝이 아니다. 입력이 보낸 턴의 완료만 작업 끝으로 보기 위해서다. 그 턴의 답(완료 신호 전의 메인 글)은 `engine`이 `PacketReply`로 바꿔 입력이 연 실행에 함께 기록하고, 화면의 답과 다음 패킷의 답에는 넣지 않는다. 사용자 입력의 답이 아닌 "알겠습니다" 같은 글이 입력 턴의 답과 이어 붙으면 어디까지가 입력의 답인지 알 수 없고, 같은 글이 다음 패킷에 답으로 실리기 때문이다. 패킷 턴의 도구 호출은 실제로 한 일이라 그대로 보인다. 패킷 턴은 이미 끝난 입력의 작업을 다시 하지 않는다. 패킷 맨 앞의 지시문이 기록이 요청이 아니라고 알리고 도구 호출과 파일 변경 없이 기다리게 하며, 입력 항목마다 끝남, 진행 중, 결과 모름 상태를 적는다([맥락 정리](context-management.md#패킷-구성), [#383](https://github.com/woonyong-choi/saturn/issues/383)).
- provider는 진행 중인 턴에 새 턴 입력이 오면 그 턴에 합쳐 완료 신호를 하나만 보낸다(Claude Code 2.1.288과 codex-cli 0.158.0에서 실측, [#318](https://github.com/woonyong-choi/saturn/issues/318)). 합쳐지면 입력의 완료 신호가 없어 작업이 끝나지 않으므로, provider 연결은 진행 중인 턴이 있을 때 받은 새 턴 입력을 줄 세워 두었다가 앞 턴의 완료 신호를 `engine`에 전달한 뒤 하나씩 보낸다. 패킷 턴과 입력 턴이 각자 완료 신호를 내므로 패킷 몫으로 센 신호 하나가 입력의 완료를 가리지 않는다. 끼워 넣기(`steer`)는 합치는 것이 목적이라 줄 세우지 않고, 멈춤은 줄 선 입력을 버린다. 줄 선 입력을 보내지 못하면 그 에이전트에 `StreamLost`를 알린다.
- 취소 안전: 이벤트 수신은 이벤트를 꺼내기만 하고 응답을 기다리는 일을 하지 않는다. `engine`은 수신 Future를 `select`에서 언제든 버리므로, 응답을 기다리는 줄 선 입력의 전송은 `engine`이 완료 이벤트를 처리한 뒤 가지 본문에서 끝까지 기다리며 부른다. 버려진 수신이 이미 꺼낸 완료 이벤트와 전송을 함께 잃는 일([#324](https://github.com/woonyong-choi/saturn/issues/324))을 막기 위해서다.
- 떠나는 메인은 새 session이 열린 뒤에 보관한다. 새 session을 열지 못하면 떠나는 메인은 그대로 열려 있다. provider 둘 다 열리지 않은 채팅이 생기는 일을 막기 위해서다.
- 패킷의 고정 구역이 `P_hard`도 넘으면 새 session을 열지 않고 입력을 작업과 함께 보류하며 TUI에 `고정 제약이 길어 맥락 정리를 미룹니다`와 제약 목록을 보인다. 사용자가 `/continue`를 하면 다시 판정한다.
- 패킷을 provider가 맥락 한도 초과로 거절하면 경쟁 구역을 줄여 한 번만 다시 보낸다. 줄인 패킷도 거절되거나 고정 구역만으로 넘치면 입력을 보류하고 `맥락 한도 초과로 멈춤 · /continue로 다시 시도하세요`를 보인다([맥락 정리](context-management.md#패킷-구성)).
- 맥락 정리로 여는 새 session의 패킷도 같은 규칙으로 줄여 한 번만 다시 보낸다. 줄인 패킷도 거절되면 보류할 입력이 없어 옛 session을 그대로 두고 `맥락 한도 초과로 멈춤` 알림만 보인다.
- 열린 메인이 있으면 그 메인이 이어 갈 session이다. 보관 session이 더 나중에 등록돼도 열린 메인을 먼저 본다. provider를 오간 뒤 등록 순서와 열린 순서가 다르기 때문이다.
- 입력에 고정한 모델이 있으면 그 모델의 provider로 보낸다([모델 고르기](#모델-고르기)). 모델을 고르지 않은 입력의 provider는 `engine`의 내부 호출(`switch_provider`)로만 바꾼다.

### 그 provider로 돌아가기

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/provider-return.ko.dark.svg">
  <img src="../assets/provider-return.ko.light.svg" alt="보관 session은 캐시 유지 시간 안이면 재개하고, 지났으면 패킷이 마지막 활성 맥락보다 작을 때만 새 session을 연다" width="100%">
</picture>

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

- 닫는 일은 TUI가 붙어 있어도 한다. `engine`의 유휴 점검이 돌 때마다 `sessions`가 열린 메인 중 유휴가 된 지 5분이 지난 session을 `idle_expired`로 고른다. 메인과 모든 subagent가 유휴인 session만 센다. 새 턴이 시작되면 시계가 지워지고, 그 채팅에 보내기 전 입력이나 응답을 기다리는 전달이 있거나, 허가나 입력 답을 기다리거나, 멈추는 중이거나 보류한 작업이 있으면 닫지 않는다.
- 닫을 때 `engine`은 `set_state(ClosedResumable)`로 보관해 저장하고, 저장에 성공한 session의 provider session을 닫는다. 저장하지 못하면 상태와 유휴 시계를 닫기 전 값으로 되돌리고(`undo_idle_close`) provider session은 닫지 않으며, 다음 유휴 검사가 같은 session을 다시 골라 닫기를 다시 시도한다. provider session ID와 전달 기록 번호는 그대로 남아 `engine`을 다시 시작해도 같은 판정이 난다. 같은 연결을 쓰는 다른 session과 다른 채팅은 닫지 않고, 연결 창구는 유지한다.
- 이 닫기는 TUI가 모두 떨어졌을 때 `engine` 전체를 끝내는 유예 종료와 따로 돈다. 같은 시계 값(`idle_grace`)을 쓰지만 서로의 결과에 기대지 않는다.
- 닫은 메인에 같은 provider로 다시 입력이 오면 마지막 턴 뒤 경과 시간이 캐시 유지 시간을 넘었는지 [맥락 정리](context-management.md)의 유휴 복귀 조건으로 먼저 판정한다. 새 session이 나으면 닫은 session은 `Ended`로 두고, 아니면 보관한 ID로 재개한다. 닫는 것은 provider 쪽 캐시 유지 시간을 바꾸지 않으므로 캐시 유지 시간이 1시간인 Claude 구독도 5분에 닫는다. 닫았다 재개한 session의 캐시 적중은 실측 전이다.

`store`는 Saturn 밖에서 연 provider session을 채팅에 연결할 수 있다. Saturn이 관리하는 session은 이 연결 목록에서 뺀다. 닫았다 재개한 session의 재개 시간과 첫 턴 캐시 적중은 실측으로 확인한다([#10](https://github.com/woonyong-choi/saturn/issues/10)).

### 기록 번호로 결과 전달

1. `sessions`는 session마다 마지막으로 전달받은 기록 번호를 기록한다.
2. 다음 입력 때 `sessions`는 그 기록 번호 뒤에 쌓인 다른 에이전트의 결과 요약과 수정 파일 경로([수정 파일 목록](#수정-파일-목록))를 붙인다.
3. router가 쌓인 항목 전체에서 관련 항목을 고르고, 순서는 [맥락 고르기](context-selection.md)를 따른다.

에이전트끼리 직접 통신하지 않는다. 보조 에이전트 결과로 쉬는 메인 에이전트를 깨우지 않고, 메인 에이전트의 다음 입력 때 전달한다. 두 규칙 모두 맥락 전달을 Saturn 기록 번호 하나로 맞추기 위해서다.

### 수정 파일 목록

수정 파일 목록은 provider 이벤트가 아니라 실행 경계의 폴더 상태 차이로 센다. 이벤트만 보면 Codex exec의 자식과 Claude subagent가 셸 명령으로 고친 파일이 빠지기 때문이다. 이벤트는 누가 고쳤는지 알리는 보조 정보로만 쓴다.

1. 시작: `engine`은 실행을 기록한 뒤 provider에 턴을 보내기 전에 폴더 상태(스냅샷)를 찍어 메모리에 둔다.
2. 트리 유휴: 턴이 끝나 실행을 `Completed`로 닫을 때 지금 상태를 찍어 시작 상태와 비교한다.
3. 멈춤 확인: 멈춘 실행을 `Stopped`로 닫을 때와, 결과를 모르는 작업을 확인 입력으로 이으려고 닫을 때 같은 비교를 한다. 보내기 전에 실패한 실행은 비교하지 않고 시작 상태만 버린다.
4. `engine`은 비교 결과를 기록 저장소에 남기고(표 `run_changes`, [기록 저장과 보존](records.md#실행별-수정-파일)) 패킷과 확인 입력에 쓴다.

스냅샷을 읽는 일은 `engine`의 `workspace`가 하고, 두 스냅샷을 비교하고 수정 주체를 붙이고 글로 쓰는 규칙은 `core`의 `sessions::changes`가 가진 순수 함수다. `core`는 파일과 프로세스를 직접 다루지 않는다.

폴더 상태는 작업 폴더와 채팅에 더한 폴더([채팅 폴더와 이어 열기](engine-lifecycle.md#채팅-폴더와-이어-열기))를 링크를 푼 절대 경로로 읽는다. 더한 폴더가 다른 폴더 안에 있으면 바깥 폴더만 읽는다. 폴더마다 방식을 따로 고른다.

| 폴더 | 읽는 방식 | 같은지 보는 기준 |
|---|---|---|
| git 저장소 | `git status --porcelain=v1 -z --untracked-files=all -- .`가 알린 파일을 읽고, 파일마다 SHA-256을 계산한다. 저장소의 `HEAD`도 적어 둔다. | 해시가 같으면 같은 상태다. 실행 전에 이미 고쳐져 있던 파일은 실행 중에 다시 고쳐 해시가 달라질 때만 목록에 든다. `git`이 지워졌다고 알린 파일은 지워짐으로 센다. |
| git이 아닌 폴더 | 폴더를 이름순으로 훑는다. 링크는 따라가지 않는다. | 파일 크기와 수정 시각이 모두 같으면 같은 상태다. 한쪽에만 있는 파일은 새로 생김이나 지워짐이다. |

- 실행 사이에 `HEAD`가 바뀌었으면(에이전트가 커밋했으면) `git diff --name-only`로 그 사이 커밋에 든 파일을 목록에 더한다. 커밋하면 `git status`가 깨끗해져 상태 차이로는 보이지 않기 때문이다.
- 무시 목록은 git 저장소에서는 `.gitignore`다. `git`이 적용하므로 `engine`이 따로 읽지 않는다. git이 아닌 폴더는 `.git`, `target`, `node_modules`, `build`, `dist`, `out`, `.venv`, `__pycache__` 같은 빌드 산출물과 의존성 폴더 이름(`core`의 `IGNORED_NAMES`)을 건너뛴다. git이 아닌 폴더의 `.gitignore`는 읽지 않는다(초안).
- 큰 폴더의 비용은 스냅샷 하나당 파일 20,000개, 3초, 해시 파일 크기 8MiB(모두 초안)로 막는다. 파일 수나 시간이 상한에 닿으면 거기서 멈추고 그 스냅샷을 부분으로 표시한다. 해시 상한보다 큰 파일은 해시하지 않고 크기와 수정 시각으로 비교한다. `git`이 시간 안에 답하지 않으면 그 자식만 종료하고 부분으로 표시한다.
- 어느 쪽이든 부분이면 목록도 부분이다. git이 아닌 폴더에서 한쪽에만 있는 파일은 새로 생겼는지 훑지 못한 것인지 알 수 없어 목록에서 뺀다. 부분 목록은 기록에 부분 표시와 함께 남고, 글로 쓸 때 "폴더가 커서 다 훑지 못했으니 더 바뀌었을 수 있다"고 밝힌다. 상한에 닿아도 실행은 막지 않는다.
- 목록은 파일 50개(초안)까지 풀어 쓰고 나머지는 개수만 적는다.
- 바뀐 내용(diff)은 목록에 싣지 않는다. 스냅샷은 해시만 가져 이전 내용이 없고, 내용을 패킷에 실으면 예산을 크게 쓰기 때문이다. 받은 에이전트가 목록의 경로로 `git diff -- <경로>`를 실행하거나 파일을 읽어 필요할 때 확인한다. 목록을 넘길 때 이 확인을 안내하지는 않는다. 패킷 지시문이 이미 현재 상태를 확인하고 믿으라고 알린다.
- 수정 주체는 provider 이벤트의 파일 수정 도구 호출(`FileEdit`)이 알린 경로로 붙인다. 이벤트에서 메인 에이전트이면 `main agent`, subagent이면 `subagent <번호>`다. 이벤트에 없는 수정은 `by shell or child process`로 쓴다. 이벤트가 알렸어도 폴더 상태가 같으면 목록에 없다. 이벤트는 목록을 늘리거나 줄이지 않는다.

목록은 세 자리에 쓴다.

- 기록: 실행마다 파일 경로, 종류(생김, 바뀜, 지워짐), 수정 주체, 부분 여부를 남긴다.
- 패킷: 실행마다 항목 하나를 경쟁 구역 맨 앞에 둔다. 도구 호출 순위와 따로 먼저 넣고, 예산이 모자라면 축약본(파일 수), 경로 순으로 줄어든다. 다른 session이 낸 실행만 넣고, 보관 session을 재개할 때는 그 session이 받은 기록 번호 뒤의 실행만 넣는다.
- 멈춤 보류 표시: 멈춘 작업을 `/continue`로 이을 때 확인 입력의 결과 줄 뒤에 `Files changed during that turn: ...`를 한 줄 더한다. 에이전트가 처음부터 다시 훑지 않고 그 파일부터 확인하게 하기 위해서다. 바뀐 파일이 없으면 줄을 더하지 않는다.

시작 상태는 메모리에만 있다. `engine`이 죽으면 그 실행의 시작 상태를 잃으므로 크래시 복구의 확인 입력에는 목록이 없고 에이전트가 파일 상태를 직접 확인한다. 같은 폴더를 사용자나 다른 프로그램이 같은 시간에 고치면 구분하지 못하고 그 파일도 목록에 든다.

### 완료 검사 근거

작업이 끝났다는 상태(`Done`)와 마지막 수정 뒤 검사가 통과했는지는 따로 보인다. `Done`은 트리가 유휴가 됐다는 뜻일 뿐 결과가 맞다는 뜻이 아니기 때문이다. 실행을 `Completed`로 닫을 때 수정 파일 목록([수정 파일 목록](#수정-파일-목록))과 그 실행의 기록된 이벤트로 근거를 가려 `ChatNotice::CompletionEvidence`로 알리고 `runs`에 남긴다. 검사를 새로 실행하거나, 검사가 통과하지 않았다고 `Stop`을 거부하거나, 입력을 다시 보내지 않는다. 멈춘 실행과 보내기 전에 실패한 실행에는 붙이지 않는다.

판정은 `core`의 `sessions::completion`이 가진 순수 함수이고 위에서부터 먼저 걸리는 것이 답이다.

| 순서 | 조건 | 판정 |
|---|---|---|
| 1 | 실행 시작 때 폴더 상태가 없어 목록을 만들지 못했다 | 미확인, `Unmeasured` |
| 2 | 바뀐 파일이 없고 훑기가 부분이 아니다 | 해당 없음 |
| 3 | 훑기가 부분이다 | 미확인, `PartialSnapshot` |
| 4 | 끝나지 않았거나 끊긴 하위 에이전트가 이벤트에 남아 있다 | 미확인, `TreeNotIdle` |
| 5 | 인정되는 검사 명령이 설정에 없다 | 미확인, `NotChecked` |
| 6 | 이벤트에 없는 수정이 있는데 파일을 바꿀 수 있는 셸 명령 이벤트가 하나도 없어 수정과 검사의 순서를 알 수 없다 | 미확인, `OrderUnknown` |
| 7 | 설정한 검사마다 마지막 실행을 본다. 결과나 종료 코드가 없으면 `NotChecked`, 그 실행과 겹치는 수정 호출이 있으면 `EditedDuringCheck`, 그 뒤에 수정 호출이 있으면 `NotChecked`, 종료 코드가 0이 아니면 `CheckFailed` | 하나라도 걸리면 미확인(`CheckFailed`, `EditedDuringCheck`, `NotChecked` 순으로 앞선 까닭), 모두 통과하면 확인됨 |

- 검사 명령은 설정 `completion.checks`(문자열 목록)에 적는다. 도구 호출이 단순한 셸 명령 하나이고, 낱말 단위로 정리한 앞부분이 설정한 명령과 같을 때만 그 검사로 본다. 인자는 판정하지 않는다(`cargo test --workspace`는 `cargo test`로 본다). 파이프, `&&`, `||`, `;`, `&`, 리디렉션, 명령 치환, 괄호가 있으면 앞 명령의 실패를 숨길 수 있으므로 인정하지 않는다. 설정한 명령 자체가 이런 문법이거나 첫 낱말이 `echo`, `printf`, `true`, `false`, `:`, `exit`, `test`, `[`이면 설정이 없는 것으로 본다.
- 수정 호출은 파일 수정 도구(`FileEdit`)와, 인정한 검사와 읽기 전용 목록(`ls`, `cat`, `rg`, `grep`, `git status`, `git diff`, `git log`)이 아닌 모든 셸 명령이다. 호출의 시작과 끝은 기록 번호로 비교한다. 결과 이벤트가 없는 셸 명령은 끝나지 않은 것으로, 결과 이벤트가 없는 파일 수정 도구는 시작과 같은 때에 끝난 것으로 본다. 시각이 아니라 기록 번호만 순서의 근거로 쓰므로 시각을 몰라도 같은 답이 난다.
- 종료 코드는 정규화된 `ToolResult.exit_code`가 코드로 끝난 셸 명령에만 있다. 신호로 끝났거나 코드를 알 수 없으면 근거가 아니다. Claude와 Codex는 같은 `ToolCall`, `ToolResult` 값을 내므로 같은 규칙으로 판정한다.
- 확인됨이면 근거로 쓴 검사 결과 이벤트의 기록 번호(채팅 안의 `events.seq`)를 함께 알린다.
- 근거는 `runs`의 `evidence_state`, `evidence_reason`, `evidence_events`에 남고([기록 저장과 보존](records.md#실행별-완료-검사-근거)), TUI를 다시 붙이면 기록 조회가 같은 줄을 작업 끝 알림 뒤에 되살린다.
- 자식 에이전트 결과는 부모의 근거로 세지 않는다. 하위 에이전트가 남아 있으면 확인됨이 아니다.
- 알려진 한계: 검사 명령의 인자(`--help`, `--no-run` 같은 것)는 판정하지 않는다. 같은 폴더를 사용자나 다른 프로그램이 같은 시간에 고친 파일은 수정 목록에 들지만 순서를 가릴 수 없다.

### subagent 트리 추적

1. `agents`는 이벤트에서 subagent의 시작, 진행, 끝을 기록한다.
2. `agents`는 subagent를 부모 에이전트 아래에 등록한다.
3. `agents`는 부모 턴 완료만 작업 끝 후보로 본다.
4. 답이 먼저 나왔는데 subagent가 남아 있으면 `agents`는 `answered-tree-running`으로 둔다.
5. 에이전트와 모든 subagent가 끝나면 `agents`는 트리 유휴로 판정한다.
6. 입력 없이 provider가 시작한 턴은 `origin = provider-wake`로 기록한다.

작업 끝은 메인 에이전트와 모든 subagent가 끝난 때로 판정한다. 멈춤, 쓰기 잠금, compaction 경계는 subagent까지 끝났는지 알아야 정할 수 있기 때문이다. Codex 작업 끝은 부모 작업의 `turn/completed`로만 판정한다. 자식 작업의 끝을 작업 끝으로 잘못 보지 않기 위해서다. Claude subagent는 `system` 이벤트로 추적할 수 있다. `task_started`와 `task_notification`은 `tool_use_id`로 `Agent` 도구 호출 id를 가리키고, `background_tasks_changed`는 실행 중인 작업 전체 목록을 싣는다. 백그라운드 subagent는 `Agent` 도구 결과가 시작 직후(`async_launched`)에 오므로 도구 결과를 끝으로 보면 안 된다. 같은 작업이 끝난 뒤 다시 시작되기도 해서(subagent가 띄운 셸이 끝나면 같은 `task_id`가 다시 `task_started`) `task_notification`의 `completed`도 트리 끝이 아니다. 작업 목록이 비었다가 약 10밀리초 안에 다시 차는 경우가 있으므로, 빈 목록 뒤 1초(초안) 동안 새 작업이 없을 때 트리가 끝났다고 본다. 메인의 `result`는 백그라운드 subagent가 끝나기 전에 오고, 끝난 뒤 사용자 입력 없이 `origin.kind`가 `task-notification`인 `result`가 다시 온다(Claude Code 2.1.288, [실험](../experiments/claude-provider-behavior/report.md)). 구현은 전경 `Agent`의 도구 결과만 끝으로 보고, 시작 접수(`tool_use_result.status`가 `async_launched`)나 `task_started`의 `is_backgrounded`가 참이면 백그라운드로 올려 끝내지 않는다. 백그라운드 subagent는 `task_notification`의 `status`가 `completed`, `failed`, `stopped`, `killed`일 때 끝을 알린 것으로 보고, subagent가 소유한 백그라운드 작업(`task_started`의 `owned_by_subagent`와 `is_backgrounded`가 참)이 `background_tasks_changed`와 `task_notification`으로 모두 사라진 뒤 1초(초안) 동안 같은 `tool_use_id`의 `task_started`가 없을 때 `SubagentEnded`를 한 번 낸다. 그 사이 다시 시작하면 끝을 취소하고, 이미 낸 뒤 다시 시작하면 `SubagentStarted`를 다시 낸다. 알림이 중복되거나 도구 결과보다 먼저 와도 끝은 한 번이고, 모르는 `status`는 끝으로 보지 않으며, 흐름이 끊기면 남은 subagent는 `StreamLost`가 된다. 소유 작업을 어느 subagent의 것인지 가를 필드가 없어 소유 작업이 하나라도 남으면 끝을 알린 모든 백그라운드 subagent의 끝을 미룬다(늦게 끝낼 뿐 먼저 끝내지 않는다). 실제 Claude 실행으로 같은 순서를 다시 확인하는 일은 남아 있다(#424). Codex 자식 session 신호는 실측했다([실측](../experiments/codex-provider-behavior/report.md)). 부모의 `turn/completed`와 자식의 `turn/completed`는 각자의 `threadId`로 따로 오고, 부모가 자식을 기다리면 부모가 자식보다 늦게(3/3), 기다리지 않으면 부모가 먼저(16.9~18.8초 대 42.9~46.0초, 3/3) 끝난다. 자식 생성은 승인 요청을 만들지 않고 자식 명령의 승인 요청은 자식 `threadId`로 온다(3/3).

부모 턴이 끝난 뒤 메인 에이전트의 글이나 도구 호출이 오면 새 턴이 시작된 것으로 본다. 입력 없이 provider가 시작한 턴도 트리 유휴로 잘못 보지 않기 위해서다. 흐름이 완료 신호 없이 끝나면 트리를 끝난 것으로 두고 관찰 끊김으로 기록한다. 더 받을 이벤트가 없는 트리를 실행 중으로 남겨 두지 않기 위해서다. 멈춤 신호 순서는 깊은 subagent부터이고, 깊이가 같으면 subagent id 순서, 마지막이 메인이다.

### 사용량 보고와 턴 값

1. `agents`는 사용량 보고의 원값, 범위, 대상 에이전트, 모델을 한 행으로 기록한다.
2. 범위는 `main-turn`, `tree-total`, `thread-cumulative` 중 하나다.
3. `agents`는 턴 값을 저장하지 않고, 필요할 때 기록 순서로 계산한다.
4. 누적 범위면 같은 session의 이번 누적에서 직전 누적을 뺀다.
5. 턴 범위면 보고값을 그대로 턴 값으로 쓴다.
6. session이 바뀌면 누적 계산을 새로 시작한다.

누적 범위의 직전 누적은 같은 session에서 같은 에이전트와 subagent의 앞 누적 보고에서 칸마다 찾는다. 바로 앞 보고에 그 칸이 없었거나 두 보고 사이에 부모 턴이 둘 이상 끝났으면 차이가 여러 턴에 걸친다고 표시한다. 누적이 직전보다 작으면 그 칸의 턴 값은 NULL로 두고 여러 턴에 걸친다고 표시한다. 계산할 수 없는 턴 값을 지어내지 않기 위해서다. `tree-total`은 그 턴의 트리 합계이므로 턴 범위처럼 그대로 쓴다.

사용량 보고는 원값과 범위를 그대로 넘기고 0으로 채우지 않는다. Codex session 누적을 그대로 더하면 중복 계산되기 때문이다. provider가 보고하지 않은 값은 NULL로 둔다. 지어낸 값을 막기 위해서다. 캐시 토큰은 Claude `cache_read_input_tokens`와 `cache_creation_input_tokens`, Codex `cachedInputTokens`와 `cacheWriteInputTokens`를 각각 캐시 읽기와 캐시 쓰기 열에 기록한다. Claude `result.usage`는 그 턴의 메인 에이전트 사용량(`main-turn`)이다. 한 process의 두 번째 턴 값은 누적이 아니고 하위 에이전트 사용량도 들어 있지 않다. `result.modelUsage`와 `total_cost_usd`는 session 시작부터의 누적이면서 하위 에이전트를 포함한다. 메시지 줄의 `usage`는 입력 3칸이 `result.usage`와 맞지만 출력 토큰은 스트리밍 도중 값이라 맞지 않으므로 턴 값은 `result`에서만 읽는다(Claude Code 2.1.288, [실험](../experiments/claude-provider-behavior/report.md)). 현재 구현은 `claude/convert.rs` 78~90행에서 `result.usage`를 `main-turn`으로 기록하고 있어 범위가 맞다. 하위 에이전트 사용량은 `modelUsage` 누적의 차이로만 알 수 있고 아직 읽지 않는다.

### 트리 전체 중지

1. `agents`는 추적된 subagent부터 멈춤 신호 대상 순서를 정한다.
2. `providers`는 Codex에는 자식 session별 `turn/interrupt`를, Claude에는 멈춤 제어 신호를 보낸다.
3. 10초 뒤 남은 프로세스가 있으면 `processes`는 provider 프로세스 묶음에 중지 신호를 보낸다.
4. 중지 신호 뒤 5초(초안)가 지나도 남으면 `processes`는 강제 종료한다.
5. `processes`는 트리 전체의 종료를 확인한 뒤 완료를 보고한다.

- provider는 새 프로세스 묶음의 리더로 실행한다. 자식 환경은 비운 뒤 제외 목록 변수를 지운 환경과 중첩 표지, 채팅의 출입증과 소켓 경로(`SATURN_PASS`, `SATURN_ENGINE_SOCKET`)만 넣는다([하위 접속](child-sessions.md)). 실행 명세의 환경 값은 디버그 출력에 담지 않는다.
- `processes`는 1초(초안)마다 프로세스 표를 읽어 리더의 자손을 기억한다. 묶음 밖으로 빠져나간 자손에는 신호를 보내지 않고, 살아 있으면 남은 수로 센다.
- 리더를 남기는 중지(Codex app-server처럼 session을 이어 쓸 때)는 리더를 뺀 묶음 구성원에만 신호를 보낸다. 멈춘 session은 보류로 두고 provider에 열린 채 남겨, 이을 때 다시 열지 않는다. 지금은 두 provider 모두 이 방식이다.
- 완료는 세 조건을 모두 확인한 뒤에만 보고한다. 모든 에이전트가 멈춘 뒤의 완료 신호(`TurnCompleted` 또는 흐름 끊김)를 보냈거나 멈출 때 이미 답을 마쳤고, 트리가 유휴이고, 모든 프로세스 묶음의 중지 결과가 돌아왔다. 멈춤 직전에 보낸 턴의 낡은 유휴 표시를 완료로 읽지 않기 위해서다. 중지 결과가 하나라도 `남은 프로세스`이거나 확인하지 못하면 완료 대신 `멈춤 확인 안 됨 · N개 남음`을 보고한다.
- 멈춤 요청은 응답한 뒤 별도 작업에서 묶음을 중지한다. 중지를 기다리는 동안에도 다른 요청과 provider 이벤트를 처리하기 위해서다.

멈춤은 Saturn session의 모든 에이전트와 subagent에 닿는다. 에이전트 하나만 멈추는 기능은 취소와 모델 교체 같은 내부 처리에서만 쓰기 때문이다. 트리 전체 종료를 확인하기 전에는 완료라고 하지 않는다. subagent가 남은 채 멈췄다고 보이는 일을 막기 위해서다. Claude에서 `interrupt` 제어 요청은 실행 중인 백그라운드 subagent와 그 subagent가 띄운 셸 작업까지 멈추고(`task_notification`의 `stopped`, 3/3) process는 그대로 남는다. 입력을 닫으면 process가 8~10초 뒤 작업을 멈추고 종료 코드 0으로 끝난다. 리더에 SIGTERM을 보내면 1초 안에 종료 코드 143으로 끝나고 자손도 3초 안에 사라진다(3/3). 세 방법 모두 멈춘 뒤 40초 동안 작업이 끝났다는 marker가 생기지 않았다(Claude Code 2.1.288, [실험](../experiments/claude-provider-behavior/report.md)). `claude.rs` 507행이 `Subagent` 대상 멈춤을 보내지 않는 것은 `interrupt` 하나가 트리 전체에 닿으므로 맞다. Codex는 부모 `turn/interrupt`가 자식 작업을 멈추지 않았고(12초 뒤에도 자식 turn과 명령 프로세스가 남음, 3/3), 자식 thread에 보낸 `turn/interrupt`는 자식 turn을 `interrupted`로 끝냈지만 자식 명령 프로세스는 10초 뒤에도 남았다(3/3, [실측](../experiments/codex-provider-behavior/report.md)). 그래서 Codex 멈춤은 자식 thread마다 interrupt를 보내고 프로세스 묶음 중지로 마무리해야 한다. 멈춘 작업의 보류와 재개는 [입력 처리](input-handling.md)에 있다.

### session 상태

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/session-states.ko.dark.svg">
  <img src="../assets/session-states.ko.light.svg" alt="열린 session은 닫힘·재개 가능이나 보류로 갔다가 다시 열릴 수 있고, 교체되거나 밀리거나 보류 종료되면 종료가 된다" width="100%">
</picture>

| 상태 | 뜻 | 다음 상태 |
|---|---|---|
| `열림` | provider 대화에 연결 중인 session | `닫힘·재개 가능`, `보류`, `종료` |
| `닫힘·재개 가능` | 유예 뒤나 provider 전환 뒤 닫고 provider session ID를 보관한 session | `열림`, `종료` |
| `보류` | 멈춤이나 증명되지 않은 크래시로 보류한 session | `열림`, `종료` |
| `종료` | 새 session으로 교체됐거나, 같은 provider의 새 보관 session에 밀렸거나, 보류 종료로 끝난 session | 없음 |

크래시 뒤 session을 어떻게 나누는지는 [engine 수명과 복구](engine-lifecycle.md)에 있고, Codex 자식 session 정리와 provider 재개 실측은 [강제 종료 뒤 provider session 재개 결과](../experiments/crash-resume/report.md)에 기록했다.

### 크래시 뒤 끊긴 하위 에이전트

크래시 복구가 하위 에이전트를 `끊김`으로 기록하면 `engine`은 그 목록을 보류한 session을 다시 열 때 `SessionSpec`의 `interrupted_children`으로 provider에 넘기고, 열린 뒤에는 지운다. 증명 없이 같은 작업이 사용자 몰래 다시 실행되지 않게 하기 위해서다([결정](../decisions/2026-10-04-crash-recovery-blocks-provider-resume.md)).

| provider | 다시 열기 전 처리 |
|---|---|
| Codex | 부모 `thread/resume` 전에 목록의 자식 thread마다 `thread/archive`, `thread/unsubscribe`를 보낸다. 부모 thread는 보관하지 않는다. 요청이 거절되거나 연결이 끊겨도 경고만 남기고 부모를 연다. |
| Claude | 실행 환경에서 `CLAUDE_CODE_RESUME_INTERRUPTED_TURN`만 뺀다. 설치본(2.1.288)은 이 변수가 작업자 재시작으로 끊긴 턴을 자동으로 다시 실행하게 한다고 설명한다. 이 변수는 크래시 뒤가 아니어도 Claude 실행마다 뺀다. |

두 provider 모두 다시 연 뒤 끊긴 하위 에이전트의 이벤트가 오면 `engine`이 막는다([engine 수명과 복구](engine-lifecycle.md#크래시-뒤-복구)). Claude 변수의 효과는 실측했다. 변수가 있으면 `--resume`만으로 끊긴 턴이 3/3회 다시 실행됐고, 빼면 0/3회였다. Codex 정리 요청의 인과적 필요성은 raw 재개에서도 재실행이 없어 확인하지 못했다([결과](../experiments/crash-resume/report.md)).

### 무응답 표시

Saturn은 오래 조용한 작업을 자동으로 죽이지 않는다. 실행 중인 작업에서 마지막 provider 이벤트 뒤 5분(초안) 동안 이벤트가 없으면 TUI가 상태판 실행 줄에 `응답 없음 N분`을 보이고, 사용자가 기존 멈춤으로 멈출 수 있게 한다. 이벤트가 다시 오면 표시를 지우고 그 시각부터 다시 센다. 허가 요청이나 입력 요청을 기다리는 동안은 무응답으로 보지 않고, 답한 뒤 다시 센다. 오래 걸리는 빌드와 테스트를 잘못 끊지 않고 사용자 몰래 멈추지도 않기 위해서다. 시간은 TUI가 받은 이벤트 도착 시각으로 재므로 나중에 붙은 TUI는 붙은 때부터 센다.

기준 시간 5분은 두 provider의 기본 대기값에서 정했다(2026-10-03 확인, Claude Code 2.1.288, Codex CLI 0.158.0). 상수 하나이고 설정 키는 두지 않는다.

| provider | 값 | 기본값 | 출처 |
|---|---|---|---|
| Claude Code | 스트림 무응답 감시(`CLAUDE_STREAM_IDLE_TIMEOUT_MS`) | 5분(5분보다 짧게 설정할 수 없음) | 설치된 실행 파일의 문자열(`Math.max(값, 300000)`) |
| Claude Code | 요청 시간 제한(`API_TIMEOUT_MS`) | 10분 | [환경 변수 문서](https://code.claude.com/docs/en/env-vars), 실행 파일 문자열 |
| Claude Code | Bash 도구(`BASH_DEFAULT_TIMEOUT_MS`, `BASH_MAX_TIMEOUT_MS`) | 기본 2분, 모델이 늘려도 최대 10분 | 같은 문서, 실행 파일 문자열(`120000`, `600000`) |
| Codex | SSE 무응답(`stream_idle_timeout_ms`) | 5분(`300000`) | [설정 참조](https://learn.chatgpt.com/docs/config-file/config-reference), 실행 파일에 키 존재 확인 |
| Codex | MCP 도구(`tool_timeout_sec`) | 60초 | 같은 문서, 실행 파일에 키 존재 확인 |
| Codex | 셸 명령 기본 제한 | 확인 불가 | 문서와 실행 파일에서 값을 찾지 못했다 |

두 provider의 스트림 무응답 기본값이 모두 5분이라 그 값을 쓴다. Claude Code의 Bash 도구는 조용한 채로 10분까지 돌 수 있어 긴 명령은 5분에 표시가 뜰 수 있다. 표시만 하고 멈추지 않으므로 오래 걸리는 작업에서도 잃는 것이 없다.

### provider 요청 응답 제한

provider 요청이 끝없이 기다리지 않도록 응답 대기에 제한을 두되, 턴 실행은 자동으로 끊지 않는다(무응답 표시와 같은 결정). 제한은 요청 종류별 상수 둘이고 설정 키는 없다. 요청은 [연결 작업](#provider-요청-작업)이 실행하므로 이 시간 동안 engine의 다른 요청 처리는 기다리지 않는다.

| 종류 | 요청 | 제한 | 근거 |
|---|---|---|---|
| 바로 돌아와야 하는 요청 | `turn/start`, `turn/steer`, `turn/interrupt` 등 | 10초 | 응답은 요청을 받았다는 확인이다. 멈춤 신호 뒤 프로세스 묶음 중지 유예(10초)와 Claude interrupt 응답 대기와 맞춘다. |
| 시작·열기 요청 | `initialize`, `skills/list`, `thread/start`, `thread/resume`, `mcpServerStatus/list`, `model/list` | 60초 | 사용자 MCP 서버 7개 설정으로 `CODEX_HOME`을 만들어 codex-cli 0.158.0 `app-server`를 세 번 띄워 쟀다(모델 호출 없음). `initialize` 0.30~0.62초, `thread/start` 0.17~0.46초, `mcpServerStatus/list` 호출 하나 1.7~4.9초이고 MCP 서버가 준비되지 않은 동안 느려진다. 첫 턴 전 MCP 준비 대기(30초)와 그 마지막 호출을 덮고 측정 최댓값의 열 배인 60초로 둔다. `thread/resume`은 쓰기 없는 새 thread가 기록이 없어 오류로 즉시 돌아와 이어 가는 thread의 응답 시간은 재지 못했다. |

`turn/start`가 모델 응답 전에 돌아오는지는 모델 호출 없이 확인할 수 없어 재지 않았다. 설계상 턴 시작 확인만 기다린다.

### provider 요청 작업

engine의 요청 처리 루프는 provider 요청이 끝나기를 기다리지 않는다. 연결 시작, session 열기와 닫기, 턴 시작, 끼워 넣기, 멈춤 신호, 허가·입력 답, 모델 목록 조회 같은 provider 요청은 연결마다 하나인 연결 작업이 실행하고, 루프는 결과 메시지만 받아 다음 단계를 잇는다. 느리거나 막힌 요청 하나가 다른 채팅의 입력과 조회, 같은 채팅의 멈춤 요청 처리를 늦추지 않게 하기 위해서다.

- 연결 작업은 연결(`ProviderConnection`)을 혼자 쥐고 요청을 줄 선 순서대로 하나씩 끝까지 실행한다. 같은 연결로 가는 요청은 순서가 바뀌지 않는다(턴 시작 뒤 끼워 넣기, 앞 턴이 끝난 뒤 다음 턴). 시작한 요청은 취소하지 않으므로 stdin 쓰기가 줄 중간에 끊겨 줄이 반쯤 쓰이는 일이 없다.
- 이벤트와 요청 결과는 같은 메시지 통로로 일어난 순서대로 루프에 온다. 턴 시작 응답 뒤에 오는 `TurnStarted`가 그 응답보다 먼저 처리되지 않는다.
- 상태 변경(기록 저장, 대기열, 작업 상태)은 루프 한 곳에서만 한다. 작업은 결과를 돌려주기만 한다.
- 입력 전달은 채팅마다 하나만 진행한다. 연결을 시작하고 session을 열고 첫 턴을 보내는 동안 그 채팅의 다음 입력은 보내지 않고 대기열에 둔다. 끝나면 이어서 보낸다. 같은 채팅의 순서를 지키기 위해서다. 다른 채팅의 입력은 막지 않는다.
- 전달은 단계마다 루프가 상태를 바꾼 뒤 다음 요청을 맡긴다. 입력을 `전달 중`으로 기록한 뒤 연결을 맺고(루프에서 실행 설정을 만들고 연결과 모델 목록은 별도 작업), session을 열고(번호 할당과 열 값 준비는 루프, `open_session`은 연결 작업), 작업과 실행을 기록한 뒤 턴을 보낸다. 결과를 적용하는 자리는 이 단계들의 앞 절반과 같은 루프다.
- 멈춤은 진행 중인 요청을 기다리지 않는다. 멈춤 요청은 기록과 보류를 바로 처리하고 멈춤 신호만 연결 작업의 줄에 넣는다. 신호는 앞선 요청 뒤에 나가고, 프로세스 묶음 중지는 기다리지 않고 시작한다. 요청을 기다리던 전달은 `멈춤` 표시가 붙고, 결과가 와도 더 보내지 않는다.
  - 연결이나 session 열기를 기다리던 전달은 열린 session을 쓰지 않고 닫고, 입력을 `보류`로 둔다. 보내지 않음이 확정이기 때문이다. 이으면 처음부터 보낸다.
  - 턴 전송을 기다리던 전달은 결과가 오면 입력을 받은 것으로 기록하되 작업은 멈춘 채로 둔다. 응답이 없거나 결과를 모르는 경우도 같다. 이을 때 보내는 확인 입력이 파일 상태부터 확인하기 때문이다.
- 결과를 적용할 때 그사이 채팅이 멈췄는지는 `멈춤` 표시로 판단한다. 판단 결과는 기존대로 적용 직전에 채팅 revision을 비교하고, 전달 중인 입력은 다시 판단하지 않는다.
- 연결 작업은 stdin을 읽지 않는 provider 때문에 쓰기가 막히면 그 연결의 요청이 줄 서서 기다린다. 루프와 다른 채팅은 영향을 받지 않는다. 쓰기에는 제한 시간을 두지 않는다. 시간이 지나 쓰기를 포기하면 줄이 반쯤 쓰여 연결이 어긋나기 때문이다. 연결이 끊기면(프로세스 종료) 쓰기가 오류로 끝나고 연결 끊김으로 처리한다.
- 연결 작업은 만들 때 연결 번호를 받고, 이벤트, 명령 목록, 종료, 끊김, 응답 메시지마다 그 번호를 싣는다. 번호는 한 번 쓰면 다시 쓰지 않고, provider가 정하는 session 번호와는 다른 값이다(한 연결 번호 아래 session이 여럿 열린다). 루프는 지금 그 채팅과 provider의 연결 번호와 같은 메시지만 이벤트, 명령 목록, 종료로 적용한다. 설정 변경 재시작이나 재접속으로 교체된 옛 연결의 늦은 메시지는 버려, 새 연결을 지우거나 새 실행을 바꾸지 않는다. 전달과 요청은 맡긴 연결의 번호를 기억하고 그 연결의 응답과 끊김만 받는다. 교체돼 닫힌 연결이 이미 맡은 요청의 결과를 보내면 기다리던 전달이 그 결과로 이어 간다. 연결을 맺는 중인 요청의 결과는 아직 연결이 없어 번호 없이 오고, 번호 없는 요청이 받는다.
- 연결 작업이 패닉하거나 중단돼 끝나면 루프에 연결 작업 종료를 알린다. 기다리던 전달은 보내기 전 단계(연결, session 열기, 변경분)면 연결 끊김으로 거절하고, 보낸 뒤 단계(턴 전송, 끼워 넣기)면 결과를 모르는 것으로 보아 `NeedsCheck`로 둔다. 연결은 끊긴 것으로 처리해 열려 있던 session의 흐름 끊김을 알린다. 채팅이 응답 없는 전달에 묶인 채 남지 않는다.
- 입력 전달이 아닌 요청도 같은 방식이다. 루프는 요청마다 번호를 붙여 맡기고, 결과가 오면 그 번호의 요청이 남긴 값으로 이어 간다. 연결 작업이 끝나 결과가 오지 않으면 연결 끊김으로 이어 간다.
  - 허가·입력 답은 TUI의 응답을 provider가 받은 뒤에 한다. 받았으면 요청을 지우고 창을 닫고 작업을 다시 `실행 중`으로 보이며, 받지 못했으면 오류로 응답하고 요청을 그대로 두어 다시 답할 수 있다. 답이 가는 동안 같은 요청의 다른 답은 `UnexpectedAnswer`로 거절한다. 답이 가는 동안 턴이 끝나 요청이 이미 지워졌으면 창과 작업 상태는 건드리지 않는다. Saturn 규칙이 낸 답은 provider가 받지 못한 것이 결과로 오면 그때 사용자에게 올린다. 그사이 그 session이 닫혔으면 올리지 않는다.
  - session 닫기는 결과를 쓰지 않고 맡긴다. 실패는 연결 작업이 로그로 남기고, 같은 연결의 뒤따르는 요청은 줄 선 순서대로 나간다.
  - `/model` 목록은 provider마다 연결과 모델 조회를 맡기고, 모든 목록이 모이면 응답 `result`의 `Models`로 돌려준다. 알리는 순서는 기본 provider 순서다. 연결을 맺는 동안 다른 요청이 같은 연결을 먼저 맺었으면 그 연결을 쓰고 새 연결은 닫는다.
  - 맥락 정리의 새 session 열기를 맡긴 채팅은 열릴 때까지 다음 입력을 보내지 않고 대기열에 둔다. 열리면 옛 session을 바꿔 기록하고 닫은 뒤 기다리던 입력을 보낸다. 맥락 한도 초과는 패킷을 줄여 한 번만 다시 연다. 열지 못하면 옛 session을 그대로 두고 입력을 보낸다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| Codex `turn/steer`가 활성 턴 없음으로 실패 | 확정 미전달로 기록하고, 다시 판단하지 않고 같은 session에 `turn/start`로 보낸다. |
| Claude 끼워 넣기 중 턴 종료 | provider가 추가 메시지를 다음 턴에 처리하므로 따로 처리하지 않는다. |
| 멈춤 뒤 묶음 밖으로 빠져나간 프로세스 존재 | 완료라고 하지 않고 `멈춤 확인 안 됨 · N개 남음`을 보고한다. |
| 완료 신호 없는 흐름 끝, 읽는 중 종료, 끝이 없는 subagent | 관찰 끊김으로 보고 `effect_scope`를 `unobserved`로 기록한다. |
| provider 흐름의 관찰 중단 | `effect_scope`를 `unobserved`로 기록하고 자동으로 이어 가지 않는다. |
| 실행 중 작업의 provider 이벤트가 5분 동안 없음 | 죽이지 않고 상태판에 `응답 없음 N분`을 보인다. 사용자가 멈출 수 있고, 이벤트가 오면 지운다. |
| 중간 사용량 보고 누락 | 차이가 여러 턴에 걸친다고 표시하고 0으로 채우지 않는다. |
| provider 요청이 제한 시간 안에 응답하지 않음. 바로 돌아와야 하는 요청(`turn/start`, `turn/steer`, interrupt)은 10초, 시작·열기 요청(`initialize`, `thread/start`, `thread/resume`, MCP 상태·목록 조회)은 60초(둘 다 초안) | 그 요청만 응답 없음으로 돌려주고, 연결과 프로세스와 진행 중인 턴은 끊지 않는다. 보내던 입력은 결과를 모르는 것으로 보아 `NeedsCheck`로 두고, session 열기는 실패로 알리며, 멈춤 신호는 연결 끊김으로 보고 프로세스 묶음 중지로 넘어간다. 늦게 온 응답은 버린다. |
| provider 요청이 느리거나 막힌 동안 같은 채팅이나 다른 채팅의 요청이 옴 | 요청은 연결 작업이 실행하므로 루프는 다른 채팅의 입력과 조회, 같은 채팅의 멈춤을 바로 처리한다. 같은 채팅의 다음 입력은 앞선 전달이 끝날 때까지 대기열에 둔다. |
| 허가·입력 답이 가는 중에 같은 요청에 다른 답이 옴 | 기다리던 답의 결과가 올 때까지 `UnexpectedAnswer`로 거절한다. 앞선 답이 실패하면 요청이 남아 다시 답할 수 있다. |
| 맥락 정리의 새 session 열기를 기다리는 동안 입력이 옴 | 입력은 접수하고 대기열에 둔다. 열린 뒤 새 session으로 보낸다. 멈춤은 그대로 처리하고 대기 입력은 보류된다. |
| provider 요청을 기다리는 중 멈춤 | 멈춤은 기다리지 않고 처리한다. 열기 전의 전달은 입력을 보류하고, 턴 전송 중의 전달은 입력을 받은 것으로 기록한 채 작업을 멈춘 채로 둔다. |
| stdin을 읽지 않아 provider 쓰기가 막힘 | 그 연결의 연결 작업만 기다린다. 쓰기를 중간에 끊지 않고, 프로세스가 끝나면 연결 끊김으로 처리한다. |
| 교체되거나 끝난 옛 연결의 이벤트, 종료, 응답이 늦게 도착 | 연결 번호가 지금 연결과 다르면 버린다. 전달과 요청은 맡긴 연결의 응답만 받는다. 새 연결과 새 실행은 그대로다. |
| 연결 작업이 패닉하거나 중단돼 끝남 | 기다리던 전달은 보내기 전 단계면 거절하고 보낸 뒤 단계면 `NeedsCheck`로 둔다. 연결은 끊긴 것으로 처리한다. |

관찰 끊김을 `unobserved`로 두는 것은 관찰하지 못한 외부 효과가 있을 수 있기 때문이다.

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| `debug.provider_events`가 꺼져 있으면 관측 기록을 쓰지 않고, 켜면 방법 이름, 순서, ID, 필드 이름만 남고 값과 router 키 문자열은 없다. | `saturn-terminal/engine/src/providers/trace.rs`의 `off_writes_nothing_and_creates_no_file`, `on_records_order_ids_and_field_names_without_values`, `router_key_does_not_appear_even_in_names_and_ids`, `data_used_as_a_key_is_not_written_as_a_field_name`, `switching_the_flag_applies_to_open_connections_at_once`, 어댑터의 `trace_message`와 `trace_line`은 `providers/codex/tests.rs`와 `providers/claude/tests.rs`의 관측 시험 |
| 채팅마다 열린 메인 session은 하나이고, 보관 session은 provider마다 하나까지다. | `saturn-terminal/core/src/sessions/tests.rs`의 `provider_switches_keep_one_open_and_one_archive_per_provider`, `register_second_archive_ends_older_one_and_drops_its_last_turn` |
| 캐시 유지 시간 안인 보관 session으로 돌아가면 `A`와 무관하게 재개하고 변경분만 붙인다. 지났으면 `P < A`일 때만 새 session을 연다. | `saturn-terminal/core/src/sessions/context.rs`의 `decide_return_matches_rule_table`, `saturn-terminal/core/src/sessions/tests.rs`의 `target_for_send_decides_returning_to_the_archive_by_cache_and_packet_size` |
| Claude 캐시 유지 시간은 `apiKeySource`가 `none`이면 1시간, 아니면 5분이고, 저장한 값이 없으면 5분이다. | `saturn-terminal/engine/src/providers/claude/tests.rs`의 `cache_window_without_api_key_is_one_hour`, `cache_window_with_api_key_is_five_minutes_and_missing_source_is_unknown`, `saturn-terminal/engine/src/lifecycle/sessions.rs`의 `cache_window_without_provider_report_is_five_minutes`, `cache_window_reported_by_provider_survives_closed_connection_and_restart` |
| 재시작 뒤에도 보관 session의 재개 판정이 같다. | `saturn-terminal/engine/src/lifecycle/sessions.rs`의 `cache_window_inside_resumes_archived_session`, `cache_window_inside_resumes_even_when_context_is_large`, `cache_window_expired_opens_new_session`, `cache_window_expired_resumes_when_context_is_smaller_than_packet`, `restart_without_last_turn_resumes_archived_session` |
| provider를 바꿀 때 떠나는 메인은 보관하고, 보관과 재개 상태를 저장한다. | `saturn-terminal/engine/src/lifecycle/sessions.rs`의 `archive_main_ends_older_archive_and_saves_both_states`, `resume_main_opens_archive_and_returns_delivered_number` |
| session 교체 뒤 새 session에는 받지 않은 기록 번호 뒤의 변경분만 넘긴다. | `saturn-terminal/engine/src/lifecycle/switch_round_trip.rs`의 `codex_to_claude_to_codex_hands_over_without_duplicates_or_gaps`, `delivered_numbers_never_go_down_across_switches` |
| provider 전환에서 패킷을 만들고 session별 전달 기록 번호를 지킨다. | `saturn-terminal/engine/src/lifecycle/switch_round_trip.rs`의 `codex_to_claude_to_codex_hands_over_without_duplicates_or_gaps`, `switch_tells_the_user_which_provider_took_over`, `saturn-terminal/engine/src/handoff.rs`의 `packet_carries_input_answer_and_tool_result_with_session_title` |
| 패킷 턴은 끝난 입력의 작업을 다시 실행하지 않는다. | `saturn-terminal/engine/src/handoff.rs`의 `finished_input_is_marked_finished_and_never_an_open_item`, 실제 Codex와 Claude로 `scripts/e2e/README.md` 단계 j |
| 패킷으로 연 턴의 완료는 작업 끝이 아니다. | `saturn-terminal/engine/src/lifecycle/switch_round_trip.rs`의 `packet_turn_completion_does_not_end_the_task_on_the_new_provider`, `saturn-terminal/engine/src/lifecycle/turn_end.rs`의 `packet_turn_completion_is_not_the_end_of_the_task` |
| 패킷 턴에 대한 provider의 답은 기록에 `PacketReply`로 남기고, 화면의 답과 다음 패킷의 답에 넣지 않는다. | `saturn-terminal/engine/src/lifecycle/switch_round_trip.rs`의 `packet_turn_reply_is_not_shown_as_the_input_reply` |
| Claude 백그라운드 subagent는 시작 접수 도구 결과와 부모의 `result`로 끝나지 않고, 실제 `task_notification`과 소유 작업 종료 뒤 확정 시각에 한 번만 끝나며, 확정 전에 다시 시작하면 끝이 취소된다. | `saturn-terminal/engine/src/providers/claude/background/tests.rs`(실측 `bg_none` 1회차 순서 `fixtures/background-agent.jsonl`)의 `the_launch_result_does_not_end_the_background_subagent`, `the_subagent_stays_running_after_the_parent_result_and_ends_once_at_the_real_end`, `a_restart_inside_the_settle_window_cancels_the_end`, `duplicate_and_early_notices_end_the_subagent_once`, `saturn-terminal/engine/src/providers/claude/tests.rs`의 `a_background_subagent_ends_only_after_its_notice_and_the_settle_wait`. 실제 Claude 실행 확인은 남음(#424) |
| 진행 중인 턴에 보낸 새 턴 입력은 provider가 합치지 않게 앞 턴의 완료 뒤에 보내고, 멈추면 버린다. | `saturn-terminal/engine/src/providers/claude/tests.rs`의 `a_turn_sent_during_the_packet_turn_gets_its_own_completion`, `interrupt_drops_the_turns_waiting_behind_the_running_one`, `saturn-terminal/engine/src/providers/codex/tests.rs`의 같은 이름 두 시험 |
| 줄 선 입력의 전송이 늦어 수신 Future가 버려져도 완료 이벤트와 그 입력을 잃지 않는다. | `saturn-terminal/engine/src/providers/claude/tests.rs`와 `codex/tests.rs`의 `a_turn_sent_during_the_packet_turn_gets_its_own_completion`(`next_arrival`로 읽고, Codex는 응답이 늦고 Claude는 쓰는 중 막히는 입력) |
| 패킷의 고정 구역이 `P_hard`를 넘으면 새 session을 열지 않고 입력을 보류한다. | `saturn-terminal/engine/src/lifecycle/switch_round_trip.rs`의 `packet_over_the_hard_limit_is_not_sent_and_the_input_is_held` |
| 열린 메인이 있으면 보관 session이 더 나중에 등록돼도 열린 메인이 이어 갈 session이다. | `saturn-terminal/core/src/sessions/tests.rs`의 `live_main_prefers_the_open_main_over_a_later_registered_archive` |
| 이벤트는 처리 전에 기록하고, 기록하지 못하면 화면과 상태에 반영하지 않는다. | `saturn-terminal/engine/src/lifecycle/events.rs`의 `event_is_recorded_before_it_reaches_the_screen_and_the_state`, `events_get_one_number_each_in_arrival_order`, `event_from_the_provider_pump_reaches_the_engine_loop` |
| 입력 없이 시작한 턴과 늦은 이벤트, 끊긴 흐름을 기록한다. | `saturn-terminal/engine/src/lifecycle/events.rs`의 `output_after_the_turn_ended_starts_a_run_without_input`, `lost_stream_records_unobserved_and_needs_a_check`, `usage_is_recorded_as_a_usage_row_and_not_as_a_ledger_event` |
| 트리 유휴 뒤 5분 유예가 지난 열린 메인만 닫을 대상으로 고르고, 새 턴이 시작되면 시계가 지워지며, 실행 중인 subagent가 있으면 고르지 않는다. | `saturn-terminal/core/src/sessions/tests.rs`의 `idle_expired_counts_only_idle_open_mains_past_the_grace_with_an_idle_tree` |
| TUI가 붙은 채로도 5분 직전에는 session을 유지하고 5분이 지나면 닫아 `ClosedResumable`로 보관하며 engine은 끝나지 않는다. 유예 중 새 턴이 시작되면 시계가 멈추고, subagent 실행 중이거나 허가를 기다리면 닫지 않는다. | `saturn-terminal/engine/src/lifecycle/idle_close.rs`의 `attached_tui_keeps_the_session_until_the_grace_and_closes_it_after`, `a_new_turn_during_the_grace_cancels_the_clock`, `a_running_subagent_keeps_the_session`, `a_pending_permission_alone_keeps_an_idle_session_open` |
| 보관 상태를 저장하지 못하면 session은 열린 채 유휴 시계도 그대로 남고 provider session을 닫지 않으며, 저장이 되돌아오면 다음 검사가 닫는다. | `saturn-terminal/engine/src/lifecycle/idle_close.rs`의 `a_failed_archive_write_keeps_the_session_open_and_the_next_check_closes_it`, `saturn-terminal/core/src/sessions/tests.rs`의 `undoing_an_idle_close_restores_the_open_state_and_the_idle_clock` |
| 닫은 뒤 다음 입력은 보관한 provider session ID로 재개하고, engine을 다시 시작해도 ID와 전달 기록 번호를 보존하며, 캐시 유지 시간이 지난 뒤에는 열려 있을 때와 같은 유휴 복귀 판정을 받는다. | `saturn-terminal/engine/src/lifecycle/idle_close.rs`의 `the_next_input_resumes_the_closed_session_with_the_stored_id`, `closed_session_keeps_the_id_and_the_delivered_number_across_an_engine_restart`, `returning_to_a_closed_session_after_the_cache_window_still_judges_the_idle_return` |
| 턴 끝에서 마지막 턴 값을 기록하고 트리 유휴일 때만 작업을 끝낸다. | `saturn-terminal/engine/src/lifecycle/turn_end.rs`의 `turn_end_records_the_last_turn_value_and_finishes_the_task`, `answer_with_a_running_subagent_ends_the_turn_only_when_the_tree_is_idle`, `waiting_input_is_sent_after_the_turn_ends` |
| Claude 오류 결과(`is_error` 참, `success`가 아닌 `subtype`)는 성공한 턴 종료가 아니라 결과 확인 필요로 올라가고, 멈춤 요청 뒤의 결과는 완료로 남는다. | `saturn-terminal/engine/src/providers/claude/tests.rs`의 `error_result_is_not_a_completed_turn`, `steer_needs_active_turn_and_interrupt_waits_for_response`, `reopening_a_session_still_linked_closes_the_old_process` |
| 권한은 Saturn 규칙이 정본이고, 권한 외 Saturn 기본값은 사용자 provider 설정에 값이 없을 때만 실행 인자로 넘긴다. | [권한](permissions.md)의 요구사항을 확인하고, 사용자 설정에 값이 있는 안전망 항목의 인자가 실행 명령에 없는지 확인한다. |
| 작업 끝은 메인 에이전트와 모든 subagent가 끝난 때로 판정한다. | `saturn-core`의 `agents` 시험 `on_event_tracks_the_tree_status`, `saturn-terminal/engine/src/lifecycle/turn_end.rs`의 `answer_with_a_running_subagent_ends_the_turn_only_when_the_tree_is_idle` |
| 누적 범위 사용량의 턴 값은 같은 session의 직전 누적을 뺀 값이다. | Codex 누적 보고 두 개에서 턴 값이 차이로 나오는지 확인한다. |
| 멈춤 신호는 추적된 subagent까지 보낸다. | `saturn-terminal/engine/src/lifecycle/stop.rs`의 `stop_signals_the_deepest_subagent_first_and_finishes_only_when_the_tree_is_idle` |
| 멈춤 신호 10초 뒤 남은 프로세스 묶음에는 중지 신호를 보낸다. | `saturn-terminal/engine/src/processes/mod.rs`의 `stop_sends_term_after_grace` |
| provider 요청 하나가 응답하지 않아도 그 요청만 제한 시간 뒤 실패하고 연결은 계속 쓸 수 있다. 응답 없는 멈춤 신호는 기다리지 않고 연결 끊김으로 돌려준다. 바로 돌아와야 하는 요청의 제한(10초)보다 느린 시작·열기 응답은 기다려 성공한다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `a_silent_request_fails_alone_and_leaves_the_connection_usable`, `a_silent_interrupt_reports_the_lost_connection_instead_of_waiting`, `a_slow_open_reply_is_waited_for_longer_than_a_quick_one` |
| 허가·입력 답, session 닫기, `/model` 목록의 연결과 조회, 맥락 정리의 새 session 열기가 느려도 다른 채팅의 붙기와 입력, 같은 채팅의 멈춤과 조회를 바로 처리하고, 풀리면 응답한다. 답이 가는 중인 요청에 겹친 답은 거절한다. | `saturn-terminal/engine/src/lifecycle/provider_stall.rs`의 `a_slow_permission_answer_does_not_stall_other_chats_or_stop`, `a_slow_input_answer_does_not_stall_other_chats_or_stop`, `a_slow_session_close_does_not_stall_other_chats_or_stop`, `a_slow_model_list_does_not_stall_other_chats_or_stop`, `a_slow_connection_for_the_model_list_does_not_stall_other_chats_or_stop`, `a_slow_context_restart_does_not_stall_other_chats_and_holds_the_next_input`, `a_second_answer_to_a_request_already_being_answered_is_refused` |
| 트리 유휴와 프로세스 중지를 모두 확인한 뒤에만 멈춤 완료를 보고한다. | `saturn-terminal/engine/src/lifecycle/stop.rs`의 `stop_is_not_complete_until_the_process_group_is_confirmed_stopped`, `stop_without_a_finished_turn_signal_is_not_complete`, `processes_left_outside_the_group_are_reported_instead_of_done` |
| 끼워 넣기와 멈춤 신호가 문서대로 provider에 전달된다. | Codex는 [실측](../experiments/codex-provider-behavior/report.md), Claude는 [#5](https://github.com/woonyong-choi/saturn/issues/5) |
| 닫은 session을 보관한 ID로 재개한다. | [#10](https://github.com/woonyong-choi/saturn/issues/10) |
| 채팅의 고정 모델이 저장되고 TUI가 붙을 때 알려진다. 고정 입력도 router 관계 판단을 받고 고정 모델로 전송된다. | `saturn-terminal/engine/src/lifecycle/model.rs`의 `pinned_model_is_saved_and_told_to_every_tui_that_attaches`, `pinned_input_gets_the_relation_judgment_and_the_pinned_model`, `saturn-terminal/engine/src/lifecycle/decision.rs`의 `pinned_model_input_still_gets_the_relation_judgment_while_task_runs` |
| 고정한 모델이 session을 여는 모델과 provider를 정하고 session 기록에 남는다. | `saturn-terminal/engine/src/lifecycle/model.rs`의 `pinned_model_opens_the_session_with_that_model_and_records_it`, `pinned_model_decides_the_provider`, `saturn-terminal/engine/src/store/sessions.rs`의 `session_model_round_trips_through_live_mains` |
| 모델이 바뀌면 새 메인 session을 열고, 같은 모델이면 열린 session을 쓴다. | `saturn-terminal/engine/src/lifecycle/model.rs`의 `changing_the_model_opens_a_new_main_session_with_a_packet`, `same_model_keeps_using_the_open_session` |
| router가 후보를 받아 고른 모델로 보내고 새 작업으로 판단되면 그 모델의 session을 연다(메인이면 새 메인 session). 고정 모델이면 묻지 않고, 목록을 받기 전이거나 후보 밖 값이면 현재 모델을 쓴다. | `saturn-terminal/engine/src/lifecycle/target_model.rs`의 `target_model_candidates_are_the_model_list_in_provider_order`, `target_model_chosen_by_the_router_is_applied`, `target_model_picks_the_provider_of_the_chosen_model`, `target_model_on_a_second_new_task_opens_a_session_with_that_model`, `target_model_is_not_asked_when_the_model_is_pinned`, `target_model_is_not_asked_before_the_model_list_arrives`, `target_model_is_ignored_when_the_input_continues_current_work`, `target_model_other_keeps_the_default_model`, `target_model_outside_the_candidates_is_ignored` |
| 기본 설정은 매뉴얼이라 router에 모델을 묻지 않는다. 새 작업의 모델은 채팅 고정, 오토의 router 선택, 선호, 기본 모델, provider 기본값 순으로 정하고 명시 고정, 매뉴얼, 후보 없음, 무효 답, router가 고르지 않음, 장애, 지원하지 않는 선택에서 같은 대체가 동작하며 정한 규칙과 이유가 판단 기록에 남는다([`model_selection.rs`](../../saturn-terminal/engine/src/lifecycle/model_selection.rs)의 `model_selection_falls_back_in_the_fixed_order_and_records_the_rule`, `a_choice_the_provider_no_longer_lists_is_recorded_as_unsupported`, `a_preference_without_confirmed_quality_never_overrides_the_default`). 매뉴얼은 `target_model`을 묻지 않는다. 기본 모델과 방식은 TUI가 붙을 때 알려지고 바꾸면 사용자 설정 파일에 저장된다. | `saturn-terminal/engine/src/lifecycle/model_mode.rs`의 `default_model_receives_a_new_task_the_router_did_not_place`, `no_default_model_keeps_the_provider_default`, `auto_mode_router_choice_beats_the_default_model`, `auto_mode_falls_back_to_the_default_model_on_other`, `manual_mode_does_not_ask_target_model_and_uses_the_default_model`, `manual_mode_still_judges_the_relation_between_inputs`, `manual_mode_sends_to_the_pinned_model_instead_of_the_default`, `default_model_decides_the_provider`, `default_model_of_an_unregistered_provider_is_ignored`, `attaching_tells_the_tui_that_no_default_model_is_chosen_yet`, `attaching_tells_the_tui_the_configured_default_and_mode`, `choosing_the_default_model_writes_the_user_config_and_tells_the_tui`, `changing_the_mode_writes_the_user_config_and_applies_to_the_next_input`, `setting_the_default_from_a_client_that_is_not_attached_is_refused` |
| 모델 목록은 설치된 provider 순서로 오고 provider로 거를 수 있다. Codex는 숨긴 모델을 빼고, Claude 기본은 `--model`을 넘기지 않는다. | `saturn-terminal/engine/src/lifecycle/model.rs`의 `model_list_comes_in_provider_order`, `model_list_can_be_limited_to_one_provider`, `saturn-terminal/engine/src/providers/codex/tests.rs`의 `model_list_entries_skip_hidden_models_and_fall_back_to_the_id`, `saturn-terminal/engine/src/providers/claude/tests.rs`의 `default_model_is_not_passed_to_claude` |
| subagent의 시작과 끝을 이벤트로 추적한다. | Claude는 [실험](../experiments/claude-provider-behavior/report.md), Codex는 [실측](../experiments/codex-provider-behavior/report.md) |
| Claude 백그라운드 subagent까지 멈춘다. | [실험](../experiments/claude-provider-behavior/report.md)에서 `interrupt`, 입력 닫기, SIGTERM 모두 3/3 확인 |
| Claude 사용량 보고의 범위를 올바르게 표시한다. | [실험](../experiments/claude-provider-behavior/report.md)에서 `usage`는 `main-turn`, `modelUsage`는 하위 포함 누적임을 확인 |
| Claude 스트림에서 provider 명령 결과와 허가 요청을 받는다. | [실험](../experiments/claude-provider-behavior/report.md)에서 프로젝트 명령의 `can_use_tool` 왕복과 로컬 명령의 `result`를 확인 |
| Codex 자식 thread는 `thread/started` 없이 부모의 `spawnAgent` 완료 항목으로 등록하고, 완료보다 먼저 온 자식 알림은 쥐어 두었다가 등록 직후 한 번만 순서대로 처리하며, 처음 보는 thread나 보낸 thread가 다른 항목은 자식으로 붙이지 않는다. 부모의 멈춤은 자식을 끝내지 않고 자식 멈춤은 자식 thread로 간다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `a_child_is_registered_from_the_spawn_completion_without_thread_started`, `interrupting_the_parent_does_not_end_a_child_and_the_child_is_interrupted_by_its_own_thread`, `a_nested_child_is_registered_under_the_child_that_spawned_it`, `a_duplicate_spawn_completion_registers_the_child_once`, `a_child_of_an_unknown_parent_is_not_attached_and_its_events_are_not_applied`, `a_spawn_item_sent_by_another_thread_does_not_register_a_child`, `events_that_arrive_before_the_spawn_completion_are_applied_once_in_order`, `held_notifications_stay_inside_the_limits`, `a_child_closed_before_its_registration_is_ended_and_forgotten_once_registered`, `a_duplicate_close_before_registration_ends_the_child_once`, `a_nested_child_closed_before_registration_is_ended_under_its_parent`, `a_child_closed_without_any_turn_is_started_and_ended_once`, `a_child_whose_turn_start_was_evicted_from_the_hold_is_still_ended`, `closing_a_child_that_already_ended_does_not_end_it_again`, `closed_notifications_of_unregistered_threads_stay_inside_the_hold_limits`, `an_approval_asked_before_the_child_is_registered_is_surfaced_once_after_registration`, `an_approval_pushed_out_of_the_hold_limit_is_declined_instead_of_left_unanswered`, `an_input_request_pushed_out_of_the_hold_limit_gets_an_error_reply` |
| 자식이 살아 있는 동안 트리 유휴가 아니다. | `saturn-terminal/engine/src/lifecycle/child_sessions.rs`와 `saturn-terminal/core/src/agents/mod.rs`의 트리 유휴 시험(어댑터가 `SubagentStarted`와 `SubagentEnded`를 올리는 것은 위 시험) |
| 채팅에 더한 폴더가 session을 열 때 provider 실행 인자로 간다. | `saturn-terminal/engine/src/providers/claude/tests.rs`의 `add_dir_follows_the_model_as_one_variadic_flag_before_the_permission_args`, `saturn-terminal/engine/src/providers/codex/tests.rs`의 `add_dir_goes_to_the_thread_config_when_a_session_opens` |
| 허가 답이 없는 동안 턴이 멈춰 있고, 답한 뒤 이어진다. 답은 Codex에는 요청과 같은 JSON-RPC 번호로, Claude에는 `control_response`로 나간다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `command_approval_is_answered_with_the_same_numeric_request_id`, `file_change_approval_keeps_a_string_request_id`, `saturn-terminal/engine/src/providers/claude/tests.rs`의 `allow_once_answers_can_use_tool_with_the_request_input` |
| 허가 요청을 TUI에 올리고 사용자 답을 provider에 넘기며, 받지 못했으면 다시 답할 수 있다. | `saturn-terminal/engine/src/lifecycle/events.rs`의 `permission_request_reaches_the_tui_and_the_answer_reaches_the_provider`, `answer_for_a_request_nobody_asked_is_refused`, `answer_the_provider_did_not_take_keeps_the_request_for_another_try`, `turn_end_withdraws_requests_nobody_answered` |
| 이미 답했거나 모르는 허가 요청에 답하면 보내지 않는다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `answering_an_unknown_or_answered_request_is_not_sent`, `saturn-terminal/engine/src/providers/claude/tests.rs`의 `answering_an_unknown_or_answered_request_is_not_sent` |
| Claude Code 도구 호출의 도구 종류, 경로, 읽은 줄 범위, 바뀐 줄 수와 셸 종료 코드를 이벤트에 싣는다. | `saturn-terminal/engine/src/providers/claude/tests.rs`의 `detail_of_edit_counts_changed_lines_and_keeps_path`, `detail_of_multi_edit_sums_every_edit`, `detail_of_write_has_no_line_change`, `detail_of_read_range_needs_offset_and_limit`, `detail_of_test_command_is_test_run`, `shell_exit_code_reads_prefix_only_for_errors` |
| 명령이 테스트 실행인지 셸인지 가르고, 바뀐 줄 수에서 앞뒤 공통 줄을 뺀다. | `saturn-terminal/engine/src/providers/tool_detail.rs`의 `classify_command_test_runners_are_test_runs`, `classify_command_other_commands_are_shell`, `line_change_counts_only_the_lines_that_differ` |
| Codex 명령의 경로와 종료 코드, 파일 수정의 경로와 바뀐 줄 수와 수정 내용을 같은 칸에 싣는다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `detail_of_read_command_keeps_action_paths`, `detail_of_test_command_is_test_run_without_paths`, `detail_of_file_change_counts_diff_lines`, `detail_of_file_change_added_file_counts_whole_text`, `tool_output_file_change_is_path_and_diff`, `exit_code_of_command_reads_code_and_ignores_other_items`, `turn_events_are_converted_in_order` |
| Codex 한 턴의 여러 응답 메시지는 항목이 바뀌는 자리에 빈 줄이 들어가 구분되어 보이고, 턴 맨 앞과 `itemId` 없는 조각에는 들어가지 않는다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `messages_of_one_turn_stay_apart_when_the_message_item_changes`, `a_new_turn_does_not_start_with_a_separator`, `a_delta_without_an_item_id_is_passed_through_unchanged`. 실제 Codex 화면에서 해설과 최종 답이 나뉘어 보이는지는 실제 provider로 확인한다 |
| Codex 추론 항목은 `Reasoning` 종류로 남기고 도구 결과 후보에서 뺀다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `detail_of_reasoning_is_not_a_candidate`, `saturn-protocol/src/event.rs`의 `is_candidate_only_reasoning_is_excluded` |
| 수정 파일 목록은 provider 이벤트 없이 셸이 바꾼 파일도 담고, git 저장소와 git이 아닌 폴더 모두 세며, 무시 목록의 파일은 담지 않는다. 이벤트는 수정 주체만 붙인다. | `saturn-terminal/engine/src/lifecycle/changed_files.rs`의 `shell_edits_without_provider_events_are_listed_at_tree_idle`, `git_repository_lists_shell_edits_and_skips_ignored_files`, `provider_events_only_name_who_edited`, `saturn-terminal/engine/src/workspace.rs`의 `plain_folder_sees_files_changed_without_any_event`, `git_folder_sees_shell_edits_by_content_and_respects_gitignore`, `git_folder_keeps_files_the_run_committed`, `added_folder_is_scanned_with_its_own_mode`, `saturn-terminal/core/src/sessions/changes.rs`의 시험 |
| 큰 폴더는 파일 수와 시간 상한에서 멈추고 부분 목록으로 표시한다. | `saturn-terminal/engine/src/workspace.rs`의 `file_count_limit_stops_the_scan_and_marks_it_partial`, `expired_time_limit_marks_the_scan_partial`, `large_files_are_compared_without_a_hash`, `saturn-terminal/engine/src/lifecycle/changed_files.rs`의 `too_large_folder_is_cut_at_the_limit_and_marked_partial` |
| 수정 파일 목록은 패킷에 들어가고, 이미 받은 session의 실행은 다시 넣지 않는다. | `saturn-terminal/engine/src/handoff.rs`의 `packet_lists_files_changed_by_shell_even_without_tool_events`, `changes_of_others_drops_the_sessions_own_and_already_delivered_runs` |
| 셸이 감싼 명령은 안쪽 명령으로 기록한다. | `saturn-terminal/engine/src/providers/tool_detail.rs`의 `unwrap_shell_gives_the_inner_command_or_leaves_the_input`, `saturn-terminal/engine/src/providers/codex/tests.rs`의 `activity_of_wrapped_command_is_inner_command` |
| Codex와 Claude Code의 도구 결과를 같은 충실도로 기록으로 바꾼다. | [변환 수정 뒤 기록 전환 품질 측정](../experiments/record-fidelity-stage2/report.md): 같은 받는 쪽에서 Codex 기록 패킷과 Claude Code 기록 패킷의 정답률 차이 0.0%p [0.0, 0.0]로 채택. 경로 120/120 대 120/120, 메모 필드 192/192 대 191/192 |
| 시작 요청이 느려도 다른 채팅의 입력과 조회, 같은 채팅의 멈춤을 바로 처리한다. 같은 연결의 요청 순서는 바뀌지 않는다. | `saturn-terminal/engine/src/lifecycle/provider_stall.rs`의 `a_slow_start_request_does_not_stall_other_chats_or_stop`, `a_provider_that_stops_reading_input_does_not_stall_other_chats_or_stop`, `a_silent_turn_start_asks_the_user_to_check_while_other_chats_keep_working`, `saturn-terminal/core/src/queue/tests.rs`의 `next_to_send_except_skips_busy_chats_but_not_others`, `saturn-terminal/engine/src/lifecycle/deliver.rs`와 `steer_rejected.rs`의 전달 순서 시험 |
| 교체된 옛 연결의 늦은 종료, 이벤트, 끊김은 새 연결과 새 실행에 적용하지 않고, 현재 연결의 종료는 계속 처리한다. 대기 중인 전달은 맡긴 연결 밖의 응답으로 이어지지 않는다. | `saturn-terminal/engine/src/lifecycle/intake.rs`의 `late_close_from_replaced_connection_does_not_remove_new_connection`, `close_of_the_current_connection_still_removes_it`, `late_event_and_loss_from_a_replaced_connection_do_not_touch_the_new_run`, `reply_from_another_connection_does_not_advance_a_waiting_delivery` |
| 연결 작업이 패닉해도 그 채팅이 멈춘 채 남지 않고 알림이 나간다. | `saturn-terminal/engine/src/lifecycle/provider_stall.rs`의 `a_connection_task_that_panics_while_sending_leaves_the_task_to_check`, `a_connection_task_that_panics_while_opening_rejects_the_input` |
| 시작 요청을 기다리는 중 멈추면 연 session을 쓰지 않고 입력을 보류하고, 턴 시작을 보내지 않는다. | `saturn-terminal/engine/src/lifecycle/provider_stall.rs`의 `a_stop_during_a_slow_start_holds_the_input_instead_of_sending_it` |
| 실행 중 작업이 5분 동안 provider 이벤트가 없으면 `응답 없음 N분`을 보이고, 이벤트가 오면 지우며, 허가나 입력 요청을 기다리는 동안은 보이지 않는다. 자동으로 멈추지 않는다. | `saturn-terminal/tui/src/view/status_board.rs`의 `no_response_shows_in_minutes_after_the_threshold_without_events`, `no_response_clears_when_an_event_arrives`, `no_response_does_not_show_while_waiting_for_permission_or_input`, `no_response_text_is_translated` |
| Codex는 끊긴 자식 thread를 부모를 다시 열기 전에 보관하고 구독을 끊으며 부모는 건드리지 않는다. 끊긴 자식이 없으면 정리하지 않는다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `interrupted_children_are_cleaned_before_the_parent_is_resumed`, `resume_without_interrupted_children_cleans_nothing` |
| Claude 실행 환경에서 `CLAUDE_CODE_RESUME_INTERRUPTED_TURN`만 빠진다. | `saturn-terminal/engine/src/providers/claude/tests.rs`의 `resume_interrupted_turn_variable_is_not_passed_to_claude` |
| provider는 열린 id로 식별하고 어댑터 밖 공통 코드는 provider 이름으로 분기하지 않는다. | `providers/codex*`, `providers/claude*`, 시험 코드, 어댑터 등록 파일 `providers/builtin.rs`를 뺀 `saturn-terminal`과 `saturn-protocol`의 비테스트 코드에서 `rg -i "codex\|claude"`로 이름을 찾고, 남은 것이 설명 주석, 제품 소개 글, 개발용 예제(`core/examples/packet`)뿐인지 확인한다 |
| 끝난 실행은 마지막 수정 뒤 설정한 검사가 종료 코드 0으로 끝났을 때만 확인됨이고, 수정 없음, 설정 없음, 누락, 실패, 검사 중 수정, 부분 스냅샷, 순서 불명, 하위 에이전트 잔류를 구분하며 echo와 실패를 숨기는 복합 셸은 근거가 아니다. | `saturn-terminal/core/src/sessions/completion/tests.rs`의 `evidence_is_decided_only_by_a_real_check_after_the_last_edit`, `commands_that_hide_a_failure_or_do_not_run_the_check_are_not_evidence`, `a_subagent_left_running_or_interrupted_is_never_verified` |
| 완료 검사 근거는 `Done`과 따로 알리고 다시 붙어도 같은 줄이 보이며, 입력을 다시 보내지 않는다. | `saturn-terminal/engine/src/lifecycle/completion_evidence.rs`의 `a_check_after_the_last_edit_is_shown_with_its_event_and_survives_reattaching`, `an_edit_without_a_configured_check_is_unverified_and_a_clean_run_is_not_applicable`, `saturn-terminal/tui/src/view/transcript.rs`의 `lines_completion_evidence_names_the_state_events_and_reason_in_both_languages` |
| 어댑터 파일만 더해 가짜 provider를 붙일 수 있다. | `saturn-terminal/engine/src/lifecycle/fake_provider.rs`의 `a_registered_adapter_runs_an_input_from_open_to_turn_end` |
| 어댑터 설명자의 표시명, 실행 파일, 기본 순서, 지시 문서 이름, 맥락 기본값, 기능을 화면과 첫 입력 기본 provider, 패킷, 예산, 끼워 넣기가 쓴다. | `saturn-terminal/engine/src/providers/registry.rs`의 `descriptors_come_back_in_the_default_order`, `installed_means_an_executable_file_of_the_descriptor_on_the_given_path`, `saturn-terminal/engine/src/lifecycle/fake_provider.rs`의 `descriptor_values_reach_the_common_code`, `an_adapter_without_the_steer_feature_never_gets_a_steer`, `an_adapter_with_the_steer_feature_gets_the_steer`, `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_skips_provider_docs`, `saturn-terminal/tui/src/i18n.rs`의 `provider_names_come_from_what_engine_announced`, `saturn-terminal/tui/src/app/tests.rs`의 `model_command_values_are_the_provider_ids_engine_announced` |
| 기록 저장소의 옛 provider 값 `Codex`, `Claude`와 모델 고정 글 `codex/<model>`, `claude/<model>`이 옛 값 그대로 읽힌다. | `saturn-terminal/engine/src/store/records/tests.rs`의 `old_provider_values_read_as_open_ids`, `saturn-protocol/src/ids.rs`의 `old_stored_values_read_as_the_same_id`, `saturn-terminal/engine/src/providers/mod.rs`의 `pinned_text_keeps_the_old_provider_prefix` |
| 인터페이스 판이 지원 범위 밖인 어댑터는 등록하지 않는다. | `saturn-terminal/engine/src/providers/registry.rs`의 `duplicate_ids_and_other_interface_versions_are_not_registered` |
| engine 시작 때 provider CLI 버전이 마지막으로 확인한 버전과 다르면 알린다. | `saturn-terminal/engine/src/lifecycle/versions.rs`의 `the_first_check_records_the_version_without_a_notice`, `a_changed_version_is_told_once_to_the_first_tui_and_recorded`, `the_same_version_is_not_told`, `a_cli_that_cannot_be_read_keeps_the_last_checked_version`, `start_info_carries_the_detected_versions`, `saturn-terminal/engine/src/providers/adapter.rs`의 `version_is_the_first_word_that_starts_with_a_digit`, `saturn-terminal/tui/src/view/status_board.rs`의 `alert_text_shows_the_version_or_count_the_alert_carries` |
| 버전이 바뀌면 실제 provider로 입력, 전환, 다시 열기만 도는 빠른 확인 절차가 `scripts/e2e`에 있다. | 구현 전(#412). 실제 Codex와 Claude로 빠른 확인 절차를 실행한다. |

## 단점

- provider별 연결 규약을 구현하고, 규약이 바뀌면 계속 따라가야 한다.
- subagent 추적을 직접 구현해야 한다.
- 설정으로 증명하지 못한 실행은 크래시 뒤 자동으로 이어 가지 못한다.
- 실행마다 폴더 상태를 두 번 읽어 매우 큰 폴더에서는 턴 시작이 최대 3초 늦어진다. 상한을 넘으면 목록이 부분이 된다.
- 계약에 판 번호를 붙여 유지하므로 provider가 늘어도 한 판의 뜻을 바꾸지 못한다.

## 대안

- ACP를 Saturn 계약으로 삼고 Codex와 Claude도 ACP로 연결하는 방식은 끼워 넣기와 전달 실패 구분 같은 깊은 기능을 잃어 버렸다([결정 기록](../decisions/2026-10-04-direct-adapters-and-acp-for-new-providers.md)).
- 자체 계약을 두고 ACP 어댑터를 지금 구현해 확장으로 보태는 방식은 세 겹을 따라가야 해 버렸다([결정 기록](../decisions/2026-10-04-direct-adapters-and-acp-for-new-providers.md)).
- provider를 닫힌 enum으로 두는 방식은 provider를 더할 때마다 공통 코드를 고쳐야 해 버렸다([결정 기록](../decisions/2026-10-04-open-providers-and-saturn-extensions.md)).
- 수정 파일을 provider 이벤트로만 세는 방식은 자식 프로세스와 셸 명령이 고친 파일이 빠져 버렸다([이슈 #65](https://github.com/woonyong-choi/saturn/issues/65)).
- 한 번 실행 방식(`codex exec`, `claude -p`)은 끼워 넣기와 Codex 맥락 크기 관찰이 불가능해 버렸다([결정 기록](../decisions/2026-09-29-persistent-provider-connections.md)).
- 실행 인자로 subagent와 네트워크를 고정하는 방식은 사용자 설정을 무시해 버렸다([결정 기록](../decisions/2026-09-29-minimal-provider-control.md)).

## 미해결 질문

- router 상태에 subagent 목록을 넣을지, 개수만 넣을지, 넣지 않을지 ([#63](https://github.com/woonyong-choi/saturn/issues/63))
