# provider 연결과 session

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [provider마다 입력을 계속 받는 상시 연결을 만든다](../decisions/2026-09-29-persistent-provider-connections.md), [provider 설정과 subagent 사용은 사용자 설정을 따르고 Saturn은 추적만 한다](../decisions/2026-09-29-minimal-provider-control.md) |

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
5. 인수가 끝나면 engine은 이전 Claude 메인 에이전트를 종료한다.
6. 대기열과 기록은 채팅에 있으므로, 사용자는 같은 채팅에서 Codex로 작업을 이어 간다.

### 보조 에이전트 결과를 메인 에이전트가 받기

1. 메인 에이전트가 작업하는 동안 사용자는 하던 일과 무관한 질문을 보낸다.
2. judge가 무관한 작업으로 판단하면 `queue`는 채팅에 속한 보조 에이전트를 시작한다.
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

engine은 provider마다 켜 둔 채 입력을 받는 연결을 둔다. Codex는 app-server로, Claude Code는 stream-json 입력으로 연결한다. 한 번 실행 방식으로는 끼워 넣기가 불가능하기 때문이다.

`core`는 provider 연결 공통 규격인 `ProviderClient` trait을 정의하고, engine의 `providers` 모듈이 `CodexClient`와 `ClaudeClient`로 구현한다. 구현은 끼워 넣기, 멈춤 신호, compaction, 사용량 보고를 Saturn 용어로 넘긴다. provider 고유 이름은 `providers/codex`, `providers/claude` 안에서만 쓴다. TUI와 앱이 provider를 몰라도 화면을 그리게 하기 위해서다.

| 동작 | Codex | Claude Code |
|---|---|---|
| 새 턴 | `turn/start` | 스트림 입력 |
| 끼워 넣기 | `turn/steer` | 스트림 입력 추가 |
| 멈춤 신호 | `turn/interrupt` | interrupt 제어 요청 |
| compaction 요청 | `thread/compact/start` | `/compact` 전송 |
| session 닫기 | app-server 하나를 창구로 정리 | 프로그램 종료 |
| session 재개 | app-server 하나를 창구로 재개 | stream-json 방식 `--resume` |
| 프로세스 수명 | 연결 창구로 유지, session은 턴 끝 뒤 5분 유예에 정리 | 턴 진행 중과 턴 끝 뒤 5분 유예까지 |

Codex app-server 규약은 codex-cli 0.158.0의 `codex app-server generate-json-schema` 결과로 확인했다.

- 메시지는 한 줄에 JSON 하나이고 `jsonrpc` 필드가 없다. 초기화는 `initialize` 요청 뒤 `initialized` 알림이다.
- session 닫기는 `thread/unsubscribe`다. 기록을 지우는 `thread/archive`, `thread/delete`는 쓰지 않는다.
- 자식 작업은 `thread/started` 알림의 `thread.parentThreadId`로 부모 thread에 잇는다.
- 활성 턴 없음은 오류 코드가 따로 없어 `turn/steer` 오류 문구로 판정한다(초안).
- 맥락 크기는 `thread/tokenUsage/updated`의 `last.totalTokens`이고 메인 턴 끝에 보낸다. 누적 사용량의 새 입력은 `total.inputTokens - cachedInputTokens`다.
- 명령 대응표는 `compact` → `thread/compact/start`, `review` → `review/start`(대상 `uncommittedChanges`)이고, 명령 목록에서 `new`, `resume`, `fork`, `quit`, `exit`를 뺀다(초안). 스킬은 `turn/start` 입력에 `{"type":"skill","name","path"}` 항목으로 넣는다.
- 권한 기본값 인자는 `-c sandbox_mode="workspace-write"`이고(초안), 사용자 설정은 `$CODEX_HOME/config.toml`(기본 `~/.codex/config.toml`)의 루트와 선택된 프로필에서 `approval_policy`, `sandbox_mode`, `model_auto_compact_token_limit` 키가 있는지만 본다.

Claude Code 실행 인자는 Claude Code 2.1.285의 `--help`로 확인했다.

- 새 session은 Saturn이 만든 UUID를 `--session-id`로 넘긴다. stream-json은 첫 입력 전에 `system/init`을 내지 않으므로 session id를 미리 알기 위해서다. 재개는 `--resume <id>`이고, 500ms(초안) 안에 프로그램이 끝나면 재개 실패로 본다.
- 권한 기본값 인자는 `--permission-mode acceptEdits`이고(초안), 안전망 `--autocompact` 값은 허용 범위 100000~1000000으로 맞춘다.
- 사용자 설정은 `~/.claude/settings.json`, `<작업 폴더>/.claude/settings.json`, `settings.local.json`의 `permissions.defaultMode`, `autoCompactEnabled`와 환경 변수 `CLAUDE_CODE_AUTO_COMPACT_WINDOW`, `DISABLE_COMPACT`가 있는지만 본다(초안).
- 명령 목록에서 `clear`, `resume`, `exit`, `quit`를 뺀다(초안). 허가 요청은 `control_request`의 `can_use_tool`로 본다(초안, [#26](https://github.com/woonyong-choi/saturn/issues/26) 실측 전).
- 맥락 크기는 마지막 메인 `assistant` 메시지 `usage`의 입력, 캐시 읽기, 캐시 쓰기 합이다(초안).
- interrupt 제어 응답은 10초, session 닫기 뒤 종료는 5초까지 기다리고, 넘으면 프로세스 묶음 중지로 넘어간다(초안).

끼워 넣기 실측을 통과하기 전의 provider에서는 끼워 넣기를 대기로 바꿔 처리한다. 끼워 넣기 경로가 문서대로 동작하는지 실측으로 확인해야 하기 때문이다([#5](https://github.com/woonyong-choi/saturn/issues/5), [#27](https://github.com/woonyong-choi/saturn/issues/27)). 이때 TUI는 `바로 반영: 준비 중`을 보인다. 사용자가 바로 반영되지 않는 이유를 알게 하기 위해서다. 입력을 어디로 보낼지는 [입력 처리](input-handling.md)가 정한다.

### provider 실행과 기본값 인자

1. `settings`는 입력 접수 때 고정한 설정 번호의 값을 읽는다.
2. `secrets`는 자식 환경에서 제외 목록에 있는 변수를 지운다.
3. `providers`는 사용자 provider 설정에 값이 없는 항목에만 Saturn 기본값 인자를 더한다.
4. `providers`는 Claude 실행에 Saturn 소유 PreToolUse 훅을 실행별 설정으로 넘긴다.
5. `processes`는 provider 프로세스를 실행하고 감시를 시작한다.
6. `providers`는 provider 명령 목록을 모은다.

Saturn은 provider의 샌드박스, 네트워크, 권한, subagent 설정을 막거나 바꾸지 않는다. provider 설정은 사용자에게 맡기고 Saturn은 추적만 하기 때문이다. provider 설정을 바꾸는 provider 명령도 막지 않고, 실제 적용된 값을 읽어 기록한다. 막지 않으면서 추적은 하기 위해서다.

Saturn 기본값은 권한(수정 허용)과 자동 압축 안전망 값 두 가지다. 사용자 설정이 없을 때도 동작을 정해 두기 위해서다. 기본값은 사용자 provider 설정에 값이 없을 때만 실행 인자로 넘긴다. 사용자가 정한 provider 설정을 덮어쓰지 않기 위해서다.

| provider | 안전망 인자 |
|---|---|
| Claude Code | `--autocompact` |
| Codex | `-c model_auto_compact_token_limit` |

안전망은 실행 인자로만 넘기고 사용자 설정 파일은 건드리지 않는다. 안전망 값의 계산은 [맥락 정리](context-management.md)에 있다.

추적만 하는 규칙의 예외는 judge 키 보안뿐이다. judge 키가 provider 자식 프로세스로 새는 일을 막기 위해서다. 2단계의 환경 변수 제거와 4단계의 훅은 [judge 키 보호](judge-key-security.md)에 있다.

### provider 명령과 스킬 전달

| 항목 | Codex | Claude Code |
|---|---|---|
| 명령 목록 수집 | 명령 대응표와 `skills/list` | `system/init`의 `slash_commands` |
| 명령과 스킬 전달 | 명령 이름과 app-server 메서드 대응표로 호출 | 프롬프트에 `/이름`을 그대로 넣어 전송 |

provider 명령 목록에서 TUI 전용 명령과 Saturn session 명령이 대신하는 명령은 뺀다. 뒤에서 연결할 때 쓸 수 없거나 Saturn session 기록과 어긋나기 때문이다. Claude 스트림에서 provider 명령의 결과와 허가 요청이 어떻게 오는지는 실측으로 확인한다([#26](https://github.com/woonyong-choi/saturn/issues/26)).

### 이벤트 수신과 변환

1. `providers`는 provider 이벤트를 Saturn 용어의 이벤트로 바꾼다.
2. `providers`는 Codex 신호를 provider session id(`thread_id`)별로 나눈다.
3. `providers`는 Codex 자식 작업을 부모 작업 아래에 등록한다.
4. `providers`는 Claude subagent를 Task/Agent 도구 호출 id와 `parent_tool_use_id`로 부모에 잇는다.
5. `providers`는 Codex `tokenUsage`에 session 누적 범위를, Claude 사용량에 턴 값 범위를 표시한다.
6. `providers`는 입력 없이 provider가 시작한 턴에 `origin = provider-wake`를 표시한다.
7. `store`는 이벤트, subagent, 사용량 보고 원값을 기록한다.

### 메인 에이전트와 보조 에이전트

1. `sessions`는 채팅마다 메인 에이전트 하나를 유지한다.
2. 모델이나 provider가 바뀌면 `sessions`는 새 메인 에이전트를 시작하고, 이전 메인 에이전트는 인수 뒤 종료한다.
3. judge가 무관한 작업으로 판단하면 `queue`는 채팅에 속한 보조 에이전트를 시작한다.
4. 보조 에이전트는 끝나면 결과를 전달한 뒤 바로 종료한다.

보조 에이전트가 물려받는 설정 층은 [설정](settings.md)에 있다. judge가 무엇을 묻는지는 [judge](judge.md)에 있다.

### provider 전환

한 채팅 안에서 Codex와 Claude를 바꿔 가도 채팅은 하나로 이어진다. 채팅마다 살아 있는 session은 하나다. `sessions`는 입력을 보내는 순간 대상 session을 정한다.

session 교체는 턴이 끝난 경계에서만 한다. 진행 중인 턴이 교체로 끊기는 일을 막기 위해서다. 대기열은 에이전트 session이 아니라 채팅에 둔다. 교체 중 들어온 입력이 옛 session을 가리키는 일을 막기 위해서다. 교체 중 들어온 입력은 새 session에 들어온 순서대로 보낸다. 입력 순서를 교체와 무관하게 지키기 위해서다.

새 session에는 패킷을 넘기고, 그 뒤로는 받지 않은 기록 번호 뒤의 변경분만 넘긴다. 패킷을 어떻게 고르는지는 [맥락 정리](context-management.md)에 있다.

### session 닫기와 재개

1. 트리 유휴 뒤 5분 유예가 지나면 `sessions`는 session을 닫는다.
2. `store`는 닫은 session의 provider session ID를 보관한다.
3. 새 입력이 오면 `sessions`는 보관한 ID로 session을 재개한다.

`store`는 Saturn 밖에서 연 provider session을 채팅에 연결할 수 있다. Saturn이 관리하는 session은 이 연결 목록에서 뺀다. 닫았다 재개한 session의 재개 시간과 첫 턴 캐시 적중은 실측으로 확인한다([#10](https://github.com/woonyong-choi/saturn/issues/10)).

### 기록 번호로 결과 전달

1. `sessions`는 session마다 마지막으로 전달받은 기록 번호를 기록한다.
2. 다음 입력 때 `sessions`는 그 기록 번호 뒤에 쌓인 다른 에이전트의 결과 요약과 수정 파일 경로를 붙인다.
3. 쌓인 양이 많을 때만 judge가 관련 항목을 고른다.

에이전트끼리 직접 통신하지 않는다. 보조 에이전트 결과로 쉬는 메인 에이전트를 깨우지 않고, 메인 에이전트의 다음 입력 때 전달한다. 두 규칙 모두 맥락 전달을 Saturn 기록 번호 하나로 맞추기 위해서다.

### subagent 트리 추적

1. `agents`는 이벤트에서 subagent의 시작, 진행, 끝을 기록한다.
2. `agents`는 subagent를 부모 에이전트 아래에 등록한다.
3. `agents`는 부모 턴 완료만 작업 끝 후보로 본다.
4. 답이 먼저 나왔는데 subagent가 남아 있으면 `agents`는 `answered-tree-running`으로 둔다.
5. 에이전트와 모든 subagent가 끝나면 `agents`는 트리 유휴로 판정한다.
6. 입력 없이 provider가 시작한 턴은 `origin = provider-wake`로 기록한다.

작업 끝은 메인 에이전트와 모든 subagent가 끝난 때로 판정한다. 멈춤, 쓰기 잠금, compaction 경계는 subagent까지 끝났는지 알아야 정할 수 있기 때문이다. Codex 작업 끝은 부모 작업의 `turn/completed`로만 판정한다. 자식 작업의 끝을 작업 끝으로 잘못 보지 않기 위해서다. Claude subagent 이벤트와 Codex 자식 session 신호는 실측으로 확인한다([#17](https://github.com/woonyong-choi/saturn/issues/17), [#20](https://github.com/woonyong-choi/saturn/issues/20)).

### 사용량 보고와 턴 값

1. `agents`는 사용량 보고의 원값, 범위, 대상 에이전트, 모델을 한 행으로 기록한다.
2. 범위는 `main-turn`, `tree-total`, `thread-cumulative` 중 하나다.
3. `agents`는 턴 값을 저장하지 않고, 필요할 때 기록 순서로 계산한다.
4. 누적 범위면 같은 session의 이번 누적에서 직전 누적을 뺀다.
5. 턴 범위면 보고값을 그대로 턴 값으로 쓴다.
6. session이 바뀌면 누적 계산을 새로 시작한다.

사용량 보고는 원값과 범위를 그대로 넘기고 0으로 채우지 않는다. Codex session 누적을 그대로 더하면 중복 계산되기 때문이다. provider가 보고하지 않은 값은 NULL로 둔다. 지어낸 값을 막기 위해서다. Claude 사용량 보고의 범위는 실측으로 확인한다([#19](https://github.com/woonyong-choi/saturn/issues/19)).

### 트리 전체 중지

1. `agents`는 추적된 subagent부터 멈춤 신호 대상 순서를 정한다.
2. `providers`는 Codex에는 자식 session별 `turn/interrupt`를, Claude에는 멈춤 제어 신호를 보낸다.
3. 10초 뒤 남은 프로세스가 있으면 `processes`는 provider 프로세스 묶음에 중지 신호를 보낸다.
4. 중지 신호 뒤 5초(초안)가 지나도 남으면 `processes`는 강제 종료한다.
5. `processes`는 트리 전체의 종료를 확인한 뒤 완료를 보고한다.

- provider는 새 프로세스 묶음의 리더로 실행한다. 자식 환경은 비운 뒤 제외 목록 변수를 지운 환경과 중첩 표지만 넣는다.
- `processes`는 1초(초안)마다 프로세스 표를 읽어 리더의 자손을 기억한다. 묶음 밖으로 빠져나간 자손에는 신호를 보내지 않고, 살아 있으면 남은 수로 센다.
- 리더를 남기는 중지(Codex app-server처럼 session을 이어 쓸 때)는 리더를 뺀 묶음 구성원에만 신호를 보낸다.

멈춤은 Saturn session의 모든 에이전트와 subagent에 닿는다. 에이전트 하나만 멈추는 기능은 취소와 모델 교체 같은 내부 처리에서만 쓰기 때문이다. 트리 전체 종료를 확인하기 전에는 완료라고 하지 않는다. subagent가 남은 채 멈췄다고 보이는 일을 막기 위해서다. Claude 백그라운드 subagent의 중지는 실측으로 확인한다([#18](https://github.com/woonyong-choi/saturn/issues/18)). 멈춘 작업의 보류와 재개는 [입력 처리](input-handling.md)에 있다.

### session 상태

| 상태 | 뜻 | 다음 상태 |
|---|---|---|
| `열림` | provider 대화에 연결 중인 session | `닫힘·재개 가능`, `보류`, `종료` |
| `닫힘·재개 가능` | 유예 뒤 닫고 provider session ID를 보관한 session | `열림`, `종료` |
| `보류` | 멈춤이나 증명되지 않은 크래시로 보류한 session | `열림`, `종료` |
| `종료` | 교체나 보류 종료로 끝난 session | 없음 |

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
| 채팅마다 살아 있는 session은 하나다. | provider를 바꾼 뒤 한 채팅에 열린 session이 하나뿐인지 확인한다. |
| session 교체 뒤 새 session에는 받지 않은 기록 번호 뒤의 변경분만 넘긴다. | 교체 뒤 첫 입력에 이미 받은 기록 번호의 결과가 다시 붙지 않는지 확인한다. |
| Saturn 기본값은 사용자 provider 설정에 값이 없을 때만 실행 인자로 넘긴다. | 사용자 설정에 값이 있는 항목의 인자가 실행 명령에 없는지 확인한다. |
| 작업 끝은 메인 에이전트와 모든 subagent가 끝난 때로 판정한다. | 답이 먼저 나오고 subagent가 남은 경우 트리 유휴가 되지 않는지 확인한다. |
| 누적 범위 사용량의 턴 값은 같은 session의 직전 누적을 뺀 값이다. | Codex 누적 보고 두 개에서 턴 값이 차이로 나오는지 확인한다. |
| 멈춤 신호는 추적된 subagent까지 보낸다. | 멈춤 요청 뒤 모든 추적 subagent가 멈춤 신호를 받는지 확인한다. |
| 멈춤 신호 10초 뒤 남은 프로세스 묶음에는 중지 신호를 보낸다. | 멈춤 신호를 무시하는 프로세스가 10초 뒤 중지 신호를 받는지 확인한다. |
| 끼워 넣기와 멈춤 신호가 문서대로 provider에 전달된다. | [#5](https://github.com/woonyong-choi/saturn/issues/5), [#27](https://github.com/woonyong-choi/saturn/issues/27) |
| 닫은 session을 보관한 ID로 재개한다. | [#10](https://github.com/woonyong-choi/saturn/issues/10) |
| subagent의 시작과 끝을 이벤트로 추적한다. | [#17](https://github.com/woonyong-choi/saturn/issues/17), [#20](https://github.com/woonyong-choi/saturn/issues/20) |
| Claude 백그라운드 subagent까지 멈춘다. | [#18](https://github.com/woonyong-choi/saturn/issues/18) |
| Claude 사용량 보고의 범위를 올바르게 표시한다. | [#19](https://github.com/woonyong-choi/saturn/issues/19) |
| Claude 스트림에서 provider 명령 결과와 허가 요청을 받는다. | [#26](https://github.com/woonyong-choi/saturn/issues/26) |

## 단점

- provider별 연결 규약을 구현하고, 규약이 바뀌면 계속 따라가야 한다.
- subagent 추적을 직접 구현해야 한다.
- 설정으로 증명하지 못한 실행은 크래시 뒤 자동으로 이어 가지 못한다.

## 대안

- 한 번 실행 방식(`codex exec`, `claude -p`)은 끼워 넣기와 Codex 맥락 크기 관찰이 불가능해 버렸다([결정 기록](../decisions/2026-09-29-persistent-provider-connections.md)).
- 실행 인자로 subagent와 네트워크를 고정하는 방식은 사용자 설정을 무시해 버렸다([결정 기록](../decisions/2026-09-29-minimal-provider-control.md)).

## 미해결 질문

- 한 번 실행 경로를 상시 연결의 대체 경로로 구현할지, 경로 하나만 유지할지 ([#34](https://github.com/woonyong-choi/saturn/issues/34))
- 메인이 아닌 provider의 명령을 고르면 그 provider session을 새로 열지, 메인 전환을 물을지, 거절할지 ([#41](https://github.com/woonyong-choi/saturn/issues/41))
- Codex 자식 session의 승인 요청에 Saturn이 부모 정책으로 응답할지, 사용자에게 따로 보일지, 모두 거절할지 ([#61](https://github.com/woonyong-choi/saturn/issues/61))
- judge 상태에 subagent 목록을 넣을지, 개수만 넣을지, 넣지 않을지 ([#63](https://github.com/woonyong-choi/saturn/issues/63))
- 수정 파일 목록을 실행 경계의 파일 상태 차이로 계산할지, provider 이벤트로 계산할지 ([#65](https://github.com/woonyong-choi/saturn/issues/65))
