# 권한

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [권한 판단의 정본은 Saturn 설정의 `permission` 규칙 하나로 둔다](../decisions/2026-10-02-saturn-permission-authority.md), [provider 설정과 subagent 사용은 사용자 설정을 따르고 Saturn은 추적만 한다](../decisions/2026-09-29-minimal-provider-control.md) |

## 요약

권한은 provider의 셸 명령, 파일 편집, MCP 도구, subagent 실행을 허용, 묻기, 거부 중 무엇으로 처리할지 정하는 기능이다. 정본은 Saturn 설정의 `permission` 규칙 하나다. engine은 규칙을 Codex와 Claude Code 각각의 방식으로 바꿔 넘기고, 묻기로 판정된 호출은 TUI 허가 요청 창으로 올린다. provider 설정 파일은 고치지 않는다.

## 동기

권한을 사용자의 provider 설정에 맡기면 Saturn이 묻지 못하는 실행이 생긴다. 2026-10-02 실측에서 Codex 0.158.0은 사용자나 폴더의 허용 규칙에 걸린 명령을 묻지 않고 실행했다. app-server에는 그 규칙을 무시하는 인자가 없었다. Claude Code 2.1.285는 사용자 설정이 `bypassPermissions`여도 묻기를 강제하는 실행 인자를 주면 요청이 호스트로 왔다. 허가 규칙이 provider 파일에 흩어지면 사용자는 허용한 것을 한곳에서 볼 수 없다. 같은 작업이 provider에 따라 다르게 허가되기도 한다. 이 기능은 규칙을 Saturn 설정 한곳에 두어 두 provider를 같은 규칙으로 판단한다. 근거는 [이슈 194](https://github.com/woonyong-choi/saturn/issues/194)에 있다.

## 예시

### 규칙으로 명령을 허용하고 거부하기

1. 사용자가 `permission.shell`에 `"*"`는 `ask`, `"git status"`는 `allow`, `"rm *"`는 `deny`로 적는다.
2. Codex가 `git status`를 실행하려 하면 engine은 묻지 않고 허용한다.
3. Codex가 `rm -rf build`를 실행하려 하면 engine은 허가 창 없이 거부하고, 명령은 실행되지 않는다.
4. Codex가 `touch a.txt`를 실행하려 하면 규칙이 `ask`이므로 TUI가 허가 요청 창을 띄운다.

### 항상 허용하기

1. Claude가 `cargo test`를 실행하려 하고 규칙이 `ask`라서 TUI가 허가 요청 창을 띄운다.
2. 사용자가 `a`(항상)를 누른다.
3. engine은 허용 답을 Claude에 보내고, `cargo test`에 대한 항상 허용을 기록 저장소에 저장한다.
4. 다음에 같은 명령이 오면 engine은 묻지 않고 허용한다.
5. 사용자의 `~/.claude`와 `~/.codex` 설정 파일은 바뀌지 않는다.

### 허가 준비 중 표시

1. Codex가 MCP 도구 호출을 시작한다.
2. 3초 안에 허가 요청도 진행 이벤트도 오지 않는다.
3. 상태판 실행 줄에 `도구 사용 허가 준비 중 · codex`(초안)가 보인다.
4. 허가 요청이 오면 표시를 지우고 허가 요청 창을 띄운다.

## 상세 설계

### 권한 규칙

규칙 대상은 셸 명령, 파일 편집, MCP 도구, subagent 실행 네 가지다. 값은 `allow`, `ask`, `deny` 셋이다. 키 이름과 형식은 [설정](settings.md)의 `permission` 키 표에 있다.

1. `settings`가 사용자 설정의 규칙을 읽는다.
2. `settings`가 폴더 설정의 규칙을 그 뒤에 잇는다.
3. engine이 호출마다 이어 붙인 규칙에서 마지막으로 일치한 규칙의 값을 쓴다.
4. 일치한 규칙이 없으면 기본값을 쓴다. 기본값은 `ask`다(초안).

- 마지막 일치가 이긴다. OpenCode의 규칙 방식을 따른다.
- 패턴은 `*`를 포함할 수 있는 글자 일치다(초안).
- 셸 명령이 `&&`, `;`, `|`로 이어져 있으면 나뉜 부분마다 판정하고 가장 엄한 값(`deny`, `ask`, `allow` 순)을 쓴다(초안). 허용된 명령 뒤에 막힌 명령을 붙이는 우회를 막기 위해서다.
- 폴더 설정의 `permission`은 폴더 설정 신뢰 창의 적용되는 항목에 보인다. 저장소가 사용자 모르게 허용 규칙을 넣는 일을 막기 위해서다.
- 규칙은 입력 접수 때 고정한 설정 번호의 값을 쓴다. 한 입력을 한 규칙으로 끝까지 판단하기 위해서다.

### 항상 허용 저장

사용자가 허가 요청 창에서 `항상`을 고르면 engine은 그 호출의 도구 종류와 패턴을 허용 규칙으로 기록 저장소에 저장한다. provider 설정 파일에는 쓰지 않는다.

- 항상 허용은 설정 규칙이 `ask`로 판정한 호출에만 적용한다. 설정의 `deny`는 항상 허용보다 앞선다(초안). 사용자가 나중에 넣은 거부 규칙이 옛 허용에 가려지지 않게 하기 위해서다.
- 항상 허용의 범위는 작업 폴더 단위다(초안).
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
- Saturn 규칙은 execpolicy 규칙 파일로 번역한다. `allow`는 `allow`, `ask`는 `prompt`, `deny`는 `forbidden`이다.
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

허가 요청 창의 선택지는 `이번만`, `항상`, `거부`이고 `Esc`로 `다르게 하라고 말하기`를 고른다. 화면과 키는 [TUI](tui.md)에 있다.

| 사용자 답 | engine 동작 |
|---|---|
| `이번만` | provider에 허용을 보내고 저장하지 않는다 |
| `항상` | provider에 허용을 보내고 항상 허용을 기록 저장소에 저장한다 |
| `거부` | provider에 거부를 보낸다 |
| `다르게 하라고 말하기` | 입력 처리 방식이 정해지기 전까지 거부로 보낸다([#56](https://github.com/woonyong-choi/saturn/issues/56)) |

### 허가 대기 중 피드백

도구 호출이 시작되고 3초 안에 허가 요청이나 진행 이벤트가 오지 않으면 TUI는 계속 기다린다. 이때 상태판 실행 줄에 `도구 사용 허가 준비 중 · {provider}`(문구 초안)를 보인다. MCP 승인 요청이 도구 호출 시작 뒤 약 0~145초에 도착한 관측이 있고 원인은 확인하지 못했기 때문이다. 허가 요청이 오면 표시를 지우고 허가 요청 창을 띄운다. 구현은 [#146](https://github.com/woonyong-choi/saturn/issues/146)에 있다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| 규칙을 provider 설정으로 번역하는 데 실패 | 첫 턴을 보내지 않고 입력을 오류 상태로 둔다. |
| Codex 버전이나 적용 정책이 기대와 다름 | 첫 턴을 보내지 않고 오류를 보인다. |
| MCP 서버가 준비되지 않았거나 도구 목록이 예상과 다름 | 첫 턴을 보내지 않고 재시도하거나 오류 상태로 둔다. |
| TUI가 붙어 있지 않을 때 `ask` 요청 도착 | 요청을 보관하고 TUI가 붙으면 가장 먼저 보인다([engine 수명과 복구](engine-lifecycle.md)). |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 권한 규칙은 사용자, 폴더 순서로 이어 붙이고 마지막 일치가 이긴다. | 사용자 `deny`와 폴더 `allow`가 겹칠 때 값이 폴더 `allow`인지 확인한다. |
| Codex 셸, 파일 편집, subagent, MCP가 Saturn 규칙대로 허용, 묻기, 거부로 처리된다. | 가짜 app-server로 규칙별 응답을 확인한다. |
| 사용자 Codex 규칙, 훅, 자동 검토자가 Saturn 판단에 끼어들지 않는다. | 사용자 `allow`와 Saturn `prompt`가 충돌하는 가짜 home에서 요청이 오는지 확인한다. |
| MCP 준비를 확인하기 전에는 첫 턴을 보내지 않는다. | 준비가 늦은 가짜 MCP 서버로 첫 턴 전송 시점을 확인한다. |
| Claude 규칙 대상 도구의 호출이 모두 `can_use_tool`로 온다. | [#232](https://github.com/woonyong-choi/saturn/issues/232) |
| 항상 허용은 기록 저장소에 저장되고 provider 설정 파일은 바뀌지 않는다. | 항상 허용 뒤 기록 저장소 행과 provider 설정 파일 지문을 확인한다. |
| 설정의 `deny`는 항상 허용보다 앞선다. | 항상 허용이 있는 패턴에 `deny`를 넣어 거부되는지 확인한다. |

## 단점

- provider마다 구성을 구현하고, Codex와 Claude Code가 바뀌면 계속 따라가야 한다.
- 사용자가 provider 설정에 둔 권한은 Saturn 실행에서 적용되지 않는다.
- Codex는 읽기 전용 샌드박스로 실행해 일반 작업이 승인 요청으로 몰릴 수 있다. 불편 정도는 측정 전이다.
- 규칙이 다른 채팅마다 Codex app-server 프로세스가 늘어난다.
- 마지막 일치가 이기므로 폴더 설정의 `allow`가 사용자 설정의 `deny` 뒤에 오면 이긴다. 폴더 설정 신뢰 창이 이를 사용자에게 보인다.

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
- Codex subagent 실행 자체를 허용, 묻기, 거부로 처리하는 방법 ([#232](https://github.com/woonyong-choi/saturn/issues/232))
- Codex MCP `prompt`가 도구를 시도한 모든 호출에서 요청으로 오는지(마지막 실측은 31/31, 앞선 실측은 불안정) ([#232](https://github.com/woonyong-choi/saturn/issues/232))
- Codex 자식 thread의 승인 요청에 Saturn이 부모 규칙으로 응답할지, 사용자에게 따로 보일지 ([#61](https://github.com/woonyong-choi/saturn/issues/61))
- 허가 거절 뒤 다르게 하라는 입력을 어떻게 받을지 ([#56](https://github.com/woonyong-choi/saturn/issues/56))
