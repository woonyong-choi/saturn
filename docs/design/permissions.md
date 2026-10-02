# 권한

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [권한 판단의 정본은 Saturn 설정의 `permission` 규칙 하나로 둔다](../decisions/2026-10-02-saturn-permission-authority.md), [provider 설정과 subagent 사용은 사용자 설정을 따르고 Saturn은 추적만 한다](../decisions/2026-09-29-minimal-provider-control.md) |

## 요약

권한은 provider의 셸 명령, 파일 편집, MCP 도구, subagent 실행을 허용, 묻기, 거부 중 무엇으로 처리할지 정하는 기능이다. 정본은 Saturn 설정의 `permission` 규칙 하나다. 권한 모드는 기본 규칙 묶음이고 개별 규칙이 그 위에 덧붙는다. engine은 규칙을 Codex와 Claude Code 각각의 방식으로 바꿔 넘기고, 묻기로 판정된 호출은 TUI 허가 요청 창으로 올린다. provider 설정 파일은 고치지 않는다.

## 동기

권한을 사용자의 provider 설정에 맡기면 Saturn이 묻지 못하는 실행이 생긴다. 2026-10-02 실측에서 Codex 0.158.0은 사용자나 폴더의 허용 규칙에 걸린 명령을 묻지 않고 실행했다. app-server에는 그 규칙을 무시하는 인자가 없었다. Claude Code 2.1.285는 사용자 설정이 `bypassPermissions`여도 묻기를 강제하는 실행 인자를 주면 요청이 호스트로 왔다. 허가 규칙이 provider 파일에 흩어지면 사용자는 허용한 것을 한곳에서 볼 수 없다. 같은 작업이 provider에 따라 다르게 허가되기도 한다. 이 기능은 규칙을 Saturn 설정 한곳에 두어 두 provider를 같은 규칙으로 판단한다. 근거는 [이슈 194](https://github.com/woonyong-choi/saturn/issues/194)에 있다.

## 예시

### 규칙으로 명령을 허용하고 거부하기

1. 사용자가 기본 모드 `edit`에서 `permission.shell`에 `"git status"`는 `allow`, `"rm *"`는 `deny`로 적는다.
2. Codex가 `git status`를 실행하려 하면 engine은 묻지 않고 허용한다.
3. Codex가 `rm -rf build`를 실행하려 하면 engine은 허가 창 없이 거부하고, 명령은 실행되지 않는다.
4. Codex가 `touch a.txt`를 실행하려 하면 `edit` 모드의 셸 기본값이 `ask`이므로 TUI가 허가 요청 창을 띄운다.

### 항상 허용하기

1. Claude가 `cargo test`를 실행하려 하고 셸 기본값이 `ask`라서 TUI가 허가 요청 창을 띄운다.
2. 사용자가 `a`(항상 허용)를 누른다.
3. engine은 허용 답을 Claude에 보내고, `cargo test`에 대한 항상 허용을 기록 저장소에 저장한다.
4. 다음에 같은 명령이 오면 engine은 묻지 않고 허용한다.
5. 사용자의 `~/.claude`와 `~/.codex` 설정 파일은 바뀌지 않는다.

### 작업 중 모드 바꾸기

1. 사용자가 기본 모드 `edit`로 작업하고, Codex가 작업 폴더 안의 파일을 고치면 허가 창 없이 적용된다.
2. 사용자가 `/permissions read-only`(초안)를 입력한다.
3. engine은 다음 허가 요청부터 편집과 실행을 거부한다.
4. 사용자가 `/permissions edit`를 입력하면 작업 폴더 안 편집이 다시 허용된다.

## 상세 설계

### 권한 규칙

규칙 대상은 셸 명령, 파일 편집, MCP 도구, subagent 실행 네 가지다. 값은 `allow`, `ask`, `deny` 셋이다. 키 이름과 형식은 [설정](settings.md)의 `permission` 키 표에 있다.

1. `settings`가 현재 권한 모드의 기본 규칙을 앞에 둔다.
2. `settings`가 사용자 설정의 개별 규칙을 그 뒤에 잇는다.
3. `settings`가 폴더 설정의 개별 규칙을 그 뒤에 잇는다.
4. engine이 호출마다 이어 붙인 규칙에서 마지막으로 일치한 규칙의 값을 쓴다.
5. 사용자와 폴더의 개별 규칙 중 `deny`가 하나라도 일치하면 순서와 상관없이 `deny`를 쓴다.

- 마지막 일치가 이긴다. OpenCode의 규칙 방식을 따른다.
- `deny`는 예외다. 폴더 설정이 사용자 설정의 `deny`를 뒤집어 저장소가 사용자의 금지를 풀지 못하게 하기 위해서다. 모드의 기본 규칙에는 이 예외가 없고, 개별 `allow`가 모드의 기본 `deny`를 덮을 수 있다.
- 패턴은 `*`를 포함할 수 있는 글자 일치다(초안).
- 셸 명령이 `&&`, `;`, `|`로 이어져 있으면 나뉜 부분마다 판정하고 가장 엄한 값(`deny`, `ask`, `allow` 순)을 쓴다(초안). 허용된 명령 뒤에 막힌 명령을 붙이는 우회를 막기 위해서다.
- 폴더 설정의 `permission`은 폴더 설정 신뢰 창의 적용되는 항목에 보인다. 저장소가 사용자 모르게 허용 규칙을 넣는 일을 막기 위해서다.
- 개별 규칙은 입력 접수 때 고정한 설정 번호의 값을 쓴다. 한 입력을 한 규칙으로 끝까지 판단하기 위해서다. 모드는 예외로 다음 허가 요청부터 바뀐 값을 쓴다(초안). 모드를 낮추면 진행 중인 작업에도 바로 적용하기 위해서다.

### 권한 모드

`permission.mode`(초안 이름)는 기본 규칙 묶음을 고른다. 기본값은 `edit`다. 모드 이름과 값은 초안이다.

| 모드 | 기본 규칙 | Claude의 같은 모드 | Codex의 같은 모드 |
|---|---|---|---|
| `ask` | 모든 실행을 묻는다 | `default` | 해당 없음 |
| `edit` | 작업 폴더 안 편집은 `allow`, 그 밖은 `ask` | `acceptEdits` | `auto` |
| `read-only` | 읽기만 `allow`, 편집과 실행은 `deny` | `plan` | `read-only` |
| `full` | 모두 `allow`, 개별 규칙의 `deny`만 적용 | `bypassPermissions` | `full-access` |

- `edit`에서 작업 폴더 밖 편집, 셸 명령, MCP 도구, subagent 실행은 `ask`다. 작업 폴더 밖 편집은 사용자가 확인한 경로만 열기 위해서다(초안).
- `permission.shell` 같은 개별 규칙은 모드 기본 규칙 위에 덧붙는다.
- 모드마다 provider 구성은 같다. Codex는 `untrusted`와 읽기 전용 샌드박스, Claude는 모든 대상 도구의 `ask` 목록을 쓴다. 모드에 따라 달라지는 것은 engine이 허가 요청에 하는 답이다. `deny` 패턴이 모든 요청에 걸리게 하고, 모드를 바꿔도 provider를 다시 시작하지 않기 위해서다(초안).
- Codex execpolicy에는 개별 셸 규칙만 번역한다. `"*"` 패턴은 어떤 명령도 매치하지 않았기 때문이다. 모드 기본 규칙과 번역하지 않은 요청은 engine이 승인 요청에 규칙으로 답한다.
- 세션 중에는 TUI 명령 `/permissions`(초안)로 모드를 본다. `/permissions {모드}`는 채팅 층의 `permission.mode`를 바꾼다. 화면은 [TUI](tui.md)에 있다.
- 모드의 순서는 낮은 쪽부터 `read-only`, `ask`, `edit`, `full`이다. `ask`는 묻기만 하고 거부하지 않으므로 `read-only`보다 높다.
- 폴더 설정의 `permission.mode`는 사용자 층까지 합친 모드보다 낮은 값만 적용하고, 같거나 높은 값은 무시한다. 저장소가 모두 허용을 켜지 못하게 하기 위해서다. 무시한 사실은 폴더 설정 신뢰 창의 무시되는 항목에 보인다.
- 모두 허용(`full`)은 사용자 설정이나 `/permissions`로만 켠다. Claude Code는 v2.1.257부터 프로젝트·로컬 설정의 `bypassPermissions`와 `auto`를 무시한다([설정 문서](https://code.claude.com/docs/en/settings), 2026-10-02 확인). Codex는 신뢰한 프로젝트 설정이 승인 정책을 정할 수 있다.

### 항상 허용 저장

사용자가 허가 요청 창에서 `항상`을 고르면 engine은 그 호출의 도구 종류와 패턴을 허용 규칙으로 기록 저장소에 저장한다. provider 설정 파일에는 쓰지 않는다.

- 항상 허용은 설정 규칙이 `ask`로 판정한 호출에만 적용한다. 개별 규칙의 `deny`는 항상 허용보다 앞선다(초안). 사용자가 나중에 넣은 거부 규칙이 옛 허용에 가려지지 않게 하기 위해서다.
- 항상 허용의 범위는 작업 폴더 단위이고 Saturn 기록 저장소에만 저장한다(초안).
- 저장 표의 이름과 열은 [기록 저장과 보존](records.md)의 규칙에 따라 구현 이슈에서 정한다.

### 판정 흐름

1. provider가 도구 호출 허가를 요청한다.
2. `providers`가 요청을 도구 종류와 패턴으로 바꾼다.
3. `permission`이 규칙과 저장된 항상 허용으로 값을 정한다.
4. `allow`면 `providers`가 허용 답을 바로 보낸다.
5. `deny`면 `providers`가 거부 답을 바로 보낸다.
6. `ask`면 engine이 TUI에 허가 요청을 올리고, 답이 올 때까지 요청을 보관한다.

Codex에서는 execpolicy가 판정한 명령은 provider 안에서 끝나고, 나머지는 승인 요청으로 와서 위 흐름을 탄다.

### Codex 구성

engine은 Saturn 전용 `CODEX_HOME`으로 app-server를 시작한다(2026-10-02 확인, codex-cli 0.158.0). 사용자의 `~/.codex`는 쓰지 않는다.

- 전용 폴더는 `~/.saturn/codex-home` 아래에 규칙 집합마다 하나를 둔다(초안).
- 로그인은 원본 `~/.codex/auth.json`을 가리키는 심볼릭 링크로 공유한다. 인증 파일은 복사하지 않는다.
- `config.toml`은 사용자 `~/.codex/config.toml`에서 권한 관련 키를 뺀 뒤 Saturn이 매 실행마다 생성한다.
- 뺄 키는 `approval_policy`, `sandbox_mode`, `sandbox_workspace_write.*`, `approvals_reviewer`, `hooks`, `hooks.state`, rules, `projects.*.trust_level`, `shell_environment_policy.*`, `cli_auth_credentials_store` 등이다.
- 모델과 MCP 서버 같은 권한 외 설정은 사용자 설정을 그대로 옮기고 추적한다.
- 생성한 설정에 `approvals_reviewer="user"`를 명시한다. 자동 검토자가 Saturn 앞에서 판단하는 일을 막기 위해서다.
- Saturn의 개별 셸 규칙은 execpolicy 규칙 파일로 번역한다. `allow`는 `allow`, `ask`는 `prompt`, `deny`는 `forbidden`이다.
- `thread/start`에 `approvalPolicy="untrusted"`와 읽기 전용 샌드박스를 준다. 파일 편집도 `item/fileChange/requestApproval`로 받기 위해서다. `untrusted`는 설정 키로는 쓸 수 없고 `thread/start` 인자로만 줄 수 있다.
- 자식 thread는 부모의 승인 정책과 규칙을 이어받아, 자식이 실행한 명령도 같은 규칙으로 승인 요청이 왔다(5/5 관측).
- 세션을 시작할 때 Codex 버전과 실제 적용된 승인 정책과 샌드박스를 확인하고, 기대와 다르면 첫 턴을 보내지 않는다(초안).
- 규칙이 다른 채팅은 규칙마다 app-server 프로세스와 `CODEX_HOME`을 따로 둔다. app-server 하나가 thread별로 execpolicy 파일을 고르게 하는 인자가 없기 때문이다.

MCP 도구는 규칙을 다음처럼 번역한다.

| Saturn 규칙 | Codex MCP 설정 |
|---|---|
| `allow` | 서버 기본 `default_tools_approval_mode="approve"` 또는 도구별 `approval_mode="approve"` |
| `ask` | 도구별 `approval_mode="prompt"`. 승인 요청은 `mcpServer/elicitation/request`로 온다 |
| `deny` | `disabled_tools`. 목록에서 빠지고 직접 호출해도 오류가 난다 |

- 패턴으로 쓴 MCP 규칙은 준비 확인 때 받은 도구 목록에 적용해 도구별 값으로 펼친다(초안).
- 첫 턴을 보내기 전에 대상 서버마다 `mcpServerStatus/list`를 호출해 `runtimeStatus`, `tools`, `toolsError`를 확인한다. 준비가 안 됐거나 도구 목록이 예상과 다르면 첫 턴을 보내지 않는다.
- `mcp_optional_startup_grace_ms`는 기본 1000 ms라 늦게 뜨는 서버의 도구가 첫 턴에서 빠질 수 있다. 예상 준비 시간만큼 늘린다. 실험은 12000 ms(초안)로 확인했다.
- 호스트가 직접 부르는 `mcpServer/tool/call`은 `prompt` 설정을 우회해 실행되므로 쓰지 않는다. 모든 MCP 승인은 모델 경로의 요청으로만 받는다.

### Claude 구성

engine은 Claude Code를 실행할 때 `--permission-prompt-tool stdio`와 `--settings '{"permissions":{"ask":[...]}}'`를 함께 준다(2026-10-02 확인, Claude Code 2.1.285).

- `ask` 목록에는 규칙 대상인 도구 이름을 모두 나열한다. 도구를 나열해야 요청이 오기 때문이다.
- 사용자 설정이 `bypassPermissions`이고 폴더 허용 목록이 있어도 `Bash` 호출은 모두 `can_use_tool`로 왔다.
- 요청은 `control_request`의 `can_use_tool`로 오고, 답은 `control_response`로 보낸다. 허용은 `{"behavior":"allow","updatedInput":<요청 input>}`, 거부는 `{"behavior":"deny","message":"..."}`다.
- `--permission-prompt-tool` 없이 `ask`만 주면 요청이 호스트로 오지 않고 자동 거부된다.
- judge 키 보호 훅의 실행별 설정은 같은 `--settings` 값에 합쳐 넘긴다(초안). 훅이 막는 호출은 규칙이 `allow`여도 막는 것이 설계다(초안, [judge 키 보호](judge-key-security.md)).

Claude는 `Bash`만 실측했다. `Edit`, `Write`, MCP 도구, subagent 도구가 같은 방식으로 오는지는 [이슈 232](https://github.com/woonyong-choi/saturn/issues/232)에서 실측한다.

### 허가 요청 창과 답

허가 요청 창의 선택지는 `이번만 허용`, `항상 허용`, `거부` 셋이다. Claude와 Codex CLI의 허가 창과 같은 모양이다. 화면과 키는 [TUI](tui.md)에 있다.

| 사용자 답 | engine 동작 |
|---|---|
| `이번만 허용` | provider에 허용을 보내고 저장하지 않는다 |
| `항상 허용` | provider에 허용을 보내고 항상 허용을 기록 저장소에 저장한다 |
| `거부` | provider에 거부를 보낸다. 다르게 하라는 말을 함께 남길 수 있다 |

거부와 함께 남기는 말의 입력 방식과 처리는 정해지지 않았다([#56](https://github.com/woonyong-choi/saturn/issues/56)). 정해지기 전에는 말 없이 거부만 보낸다.

사용자 답은 provider에 아래 값으로 나간다(초안). 답이 없는 동안 provider는 그 호출에서 멈춰 있고, 답이 나가면 이어진다.

| 요청 | `이번만 허용` | `항상 허용` | `거부` |
|---|---|---|---|
| Codex 셸 명령, 파일 편집 (`item/commandExecution/requestApproval`, `item/fileChange/requestApproval`) | `{"decision":"accept"}` | `{"decision":"acceptForSession"}`. 요청의 `availableDecisions`에 없으면 `accept` | `{"decision":"decline"}` |
| Codex MCP 도구 (`mcpServer/elicitation/request`) | `{"action":"accept","content":{}}` | `이번만 허용`과 같음 | `{"action":"decline"}` |
| Codex 권한 요청 (`item/permissions/requestApproval`) | 요청한 권한을 `scope: "turn"`으로 | 요청한 권한을 `scope: "session"`으로 | 빈 권한 |
| Codex 옛 이름 (`execCommandApproval`, `applyPatchApproval`) | `{"decision":"approved"}` | `{"decision":"approved_for_session"}` | `{"decision":"denied"}` |
| Claude `can_use_tool` | `{"behavior":"allow","updatedInput":<요청 input>}` | `이번만 허용`과 같음 | `{"behavior":"deny","message":"<고정 문구>"}` |

- 응답은 Codex에는 요청과 같은 JSON-RPC 번호(숫자와 문자열 그대로)로, Claude에는 `control_response`로 보낸다. 모르는 요청이나 이미 답한 요청에는 보내지 않는다.
- `항상 허용`은 이번에는 provider에 보내는 값만 정한다. 사용자의 허용 규칙을 기록 저장소에 저장하는 일과 provider에 맞는 값이 없을 때의 처리는 [#232](https://github.com/woonyong-choi/saturn/issues/232)에서 정한다. 그 전에는 provider에 값이 없는 요청(Codex MCP, Claude)에서 `이번만 허용`으로 보낸다. 같은 호출이 다시 오면 다시 묻는다.
- Codex에서 실측한 응답은 셸 명령의 `decline`과 MCP의 `decline`이다. 허용 응답과 권한 요청, 옛 이름의 값은 schema에서 가져온 것이라 실측하지 않았다([#232](https://github.com/woonyong-choi/saturn/issues/232)).
- 거부 응답에 붙는 고정 문구(`The user denied this tool call in Saturn.`)는 초안이다. 모델에 전달된다.
- `_meta.codex_approval_kind`가 없는 `mcpServer/elicitation/request`는 승인이 아니라서 허가 요청으로 올리지도 응답하지도 않는다(초안).

### 허가 대기 중 피드백

도구 호출이 시작되고 3초 안에 허가 요청이나 진행 이벤트가 오지 않으면 TUI는 계속 기다린다. 이때 상태판 실행 줄에 `도구 사용 허가 준비 중 · {provider}`(문구 초안)를 보인다. MCP 승인 요청이 도구 호출 시작 뒤 약 0~145초에 도착한 관측이 있고 원인은 확인하지 못했기 때문이다. 허가 요청이 오면 표시를 지우고 허가 요청 창을 띄운다. 구현은 [#146](https://github.com/woonyong-choi/saturn/issues/146)에 있다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| 규칙을 provider 설정으로 번역하는 데 실패 | 첫 턴을 보내지 않고 입력을 오류 상태로 둔다. |
| Codex 버전이나 적용 정책이 기대와 다름 | 첫 턴을 보내지 않고 오류를 보인다. |
| MCP 서버가 준비되지 않았거나 도구 목록이 예상과 다름 | 첫 턴을 보내지 않고 재시도하거나 오류 상태로 둔다. |
| TUI가 붙어 있지 않을 때 `ask` 요청 도착 | 요청을 보관하고 TUI가 붙으면 가장 먼저 보인다([engine 수명과 복구](engine-lifecycle.md)). |
| 이미 답했거나 모르는 요청에 답함 | provider에 보내지 않고 `NotSent`로 돌려준다. 쓰기 전에 실패하면 요청을 되돌려 다시 답할 수 있다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 규칙은 모드, 사용자, 폴더 순서로 이어 붙이고 마지막 일치가 이긴다. | 사용자 `allow`와 폴더 `ask`가 겹칠 때 값이 폴더 `ask`인지 확인한다. |
| 사용자나 폴더의 개별 `deny`가 하나라도 일치하면 거부한다. | 사용자 `deny` 뒤에 폴더 `allow`를 두어도 거부되는지 확인한다. |
| 폴더 설정의 모드가 사용자 층 모드보다 높거나 같으면 무시하고 신뢰 창에 보인다. | 사용자 `edit`에 폴더 `full`을 두면 `edit`가 유지되고 무시되는 항목에 `full`이 보이는지 확인한다. |
| 모드의 기본 규칙이 표대로 판정되고, 기본 모드는 `edit`다. | 모드마다 작업 폴더 안 편집, 밖 편집, 셸, MCP, subagent 호출의 값을 확인한다. |
| 모드를 바꾸면 다음 허가 요청부터 새 모드로 판정하고 provider를 다시 시작하지 않는다. | `/permissions read-only` 뒤 같은 작업의 다음 요청이 거부되는지 확인한다. |
| Codex 셸, 파일 편집, subagent, MCP가 Saturn 규칙대로 허용, 묻기, 거부로 처리된다. | 가짜 app-server로 규칙별 응답을 확인한다. |
| 사용자 Codex 규칙, 훅, 자동 검토자가 Saturn 판단에 끼어들지 않는다. | 사용자 `allow`와 Saturn `prompt`가 충돌하는 가짜 home에서 요청이 오는지 확인한다. |
| MCP 준비를 확인하기 전에는 첫 턴을 보내지 않는다. | 준비가 늦은 가짜 MCP 서버로 첫 턴 전송 시점을 확인한다. |
| Claude 규칙 대상 도구의 호출이 모두 `can_use_tool`로 온다. | [#232](https://github.com/woonyong-choi/saturn/issues/232) |
| 허가 답이 provider에 요청 번호와 같은 번호로 나가고, 답이 없는 동안 턴이 멈춰 있다가 답한 뒤 이어진다. | `saturn-terminal/engine/src/providers/codex.rs`의 `command_approval_is_answered_with_the_same_numeric_request_id`, `command_decisions_follow_the_answer_and_the_available_list`, `mcp_tool_approval_is_answered_with_an_elicitation_action`, `saturn-terminal/engine/src/providers/claude.rs`의 `allow_once_answers_can_use_tool_with_the_request_input`, `deny_answers_can_use_tool_without_the_note` |
| 허가 창은 세 선택지이고, 도구 호출 뒤 3초 안에 허가 요청이나 진행 이벤트가 없으면 상태판에 준비 중을 보인다. | `saturn-terminal/tui/src/keys.rs`의 `permission_keys`, `saturn-terminal/tui/src/view/status_board.rs`의 `approval_pending_shows_after_three_seconds_without_events`, `approval_pending_clears_when_permission_request_or_progress_arrives` |
| 항상 허용은 기록 저장소에 저장되고 provider 설정 파일은 바뀌지 않는다. | 항상 허용 뒤 기록 저장소 행과 provider 설정 파일 지문을 확인한다. |
| 개별 규칙의 `deny`는 항상 허용보다 앞선다. | 항상 허용이 있는 패턴에 `deny`를 넣어 거부되는지 확인한다. |

## 단점

- provider마다 구성을 구현하고, Codex와 Claude Code가 바뀌면 계속 따라가야 한다.
- 사용자가 provider 설정에 둔 권한은 Saturn 실행에서 적용되지 않는다.
- Codex는 읽기 전용 샌드박스로 실행해 일반 작업이 승인 요청으로 몰릴 수 있다. 불편 정도는 측정 전이다.
- 규칙이 다른 채팅마다 Codex app-server 프로세스가 늘어난다.
- `deny`를 뺀 규칙은 마지막 일치가 이기므로 폴더 설정이 사용자 설정의 `allow`를 바꿀 수 있다. 모드는 폴더 설정이 낮추기만 할 수 있다. 폴더 설정 신뢰 창이 이를 사용자에게 보인다.
- 모든 모드가 같은 provider 구성을 쓰므로 `full`에서도 Codex는 승인 요청을 거치고, 요청마다 engine 응답을 기다린다.

## 대안

- 권한을 사용자 설정에 맡기고 추적만 하는 방식은 Codex 허용 규칙 명령을 묻지 못해 버렸다([결정 기록](../decisions/2026-10-02-saturn-permission-authority.md)).
- Saturn 설정을 provider 설정에 번역해 사용자 설정과 병합하는 방식은 사용자 허용 규칙이 이겨 묻기를 강제하지 못해 버렸다.
- provider 설정 파일에 규칙을 기록하는 방식은 사용자가 정한 설정을 덮어써 버렸다.
- Codex PreToolUse 훅으로 묻는 방식은 훅이 `ask`를 지원하지 않고(openai/codex#28437, 2026-10-02 확인), 시간 초과와 오류에서 도구를 실행하며, subagent 생성과 파일 편집에 걸리지 않아 버렸다.

## 미해결 질문

- Claude `Edit`, `Write`, MCP 도구, subagent 도구가 모두 `can_use_tool`로 오는지 ([#232](https://github.com/woonyong-choi/saturn/issues/232))
- Claude 사용자와 폴더의 `deny` 규칙과 훅이 Saturn 판정 앞에서 호출을 막는지 ([#232](https://github.com/woonyong-choi/saturn/issues/232))
- Codex 읽기 전용 샌드박스에서 일반 읽기, 빌드, 테스트 작업이 얼마나 막히는지 ([#232](https://github.com/woonyong-choi/saturn/issues/232))
- 키체인 로그인(`cli_auth_credentials_store=keyring`)에서 전용 `CODEX_HOME`이 로그인을 공유하는지 ([#232](https://github.com/woonyong-choi/saturn/issues/232))
- Codex가 승인된 편집과 명령을 읽기 전용 샌드박스에 막히지 않고 실행하는지 ([#232](https://github.com/woonyong-choi/saturn/issues/232))
- Codex subagent 실행 자체를 허용, 묻기, 거부로 처리하는 방법 ([#232](https://github.com/woonyong-choi/saturn/issues/232))
- Codex MCP `prompt`가 도구를 시도한 모든 호출에서 요청으로 오는지(마지막 실측은 31/31, 앞선 실측은 불안정) ([#232](https://github.com/woonyong-choi/saturn/issues/232))
- `항상 허용`을 provider 값으로 보낼 방법: Claude 세션 규칙(`updatedPermissions`), Codex MCP의 `_meta.persist`, Codex 허용 응답과 권한 요청, 옛 이름 값의 실측 ([#232](https://github.com/woonyong-choi/saturn/issues/232))
- Codex 자식 thread의 승인 요청에 Saturn이 부모 규칙으로 응답할지, 사용자에게 따로 보일지 ([#61](https://github.com/woonyong-choi/saturn/issues/61))
- 허가 거절 뒤 다르게 하라는 입력을 어떻게 받을지 ([#56](https://github.com/woonyong-choi/saturn/issues/56))
