# router 키 보호

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [engine만 router를 부르고 자식 프로세스 환경에서 router 키를 지운다](../decisions/2026-09-29-engine-as-router-proxy.md), [판단 규격은 Saturn이 정하고 router는 중립 이름과 출처로 기록한다](../decisions/2026-09-29-vendor-neutral-router-spec.md) |

## 요약

router 키 보호는 외부 router API 키를 provider와 subagent가 어떤 경로로도 받거나 읽지 못하게 하는 기능이다. 키는 정해진 세 방법으로만 받고, macOS 키체인에 OS API로 직접 저장한다. `engine`은 자식 프로세스 환경에서 키 변수를 지운다. Saturn 소유 PreToolUse 훅은 저장된 키를 찾아가는 도구 호출을 막고, provider 명령 샌드박스는 훅이 놓치는 형태까지 OS 수준에서 막는다. 키는 HTTPS로 허용 호스트에만 보내고, 로그와 오류와 기록에서는 가린다.

## 동기

기준 router는 외부 API라 키가 필요하다. 키를 환경 변수로 주면 provider 자식 프로세스가 같은 환경을 그대로 받는다. provider와 subagent는 router를 부를 일이 없으므로 키를 받을 이유도 없다. macOS `security` 명령으로 키체인에 저장하면 그 명령이 신뢰 앱이 된다(2026-09-29 확인). 그러면 다른 프로세스도 같은 명령으로 확인 창 없이 키를 읽을 수 있다(Silverfort 보고). Saturn은 provider 설정을 막지 않고 추적만 하지만, router 키 보안만은 그 예외로 둔다.

## 예시

### 처음 실행 때 키를 입력할 때

1. 사용자가 저장된 키 없이 `saturn`을 실행한다.
2. `routers`의 시작 확인이 실패하고, `engine`은 소켓을 연 채 router 키를 기다린다.
3. TUI가 붙으면 `engine`이 `RouterKeyRequired`를 보내고, TUI가 숨김 입력으로 router 키를 요청한다.
4. 사용자가 키를 붙여 넣으면 TUI가 `SubmitRouterKey`로 보내고, `routers`가 `GET /v1/models`로 키를 확인하고 모델 버전을 고정한다.
5. `secrets`가 키를 macOS 키체인에 OS API로 직접 저장한다.
6. `settings`는 키의 출처와 끝 4자리만 설정에 적고, `engine`이 일반 요청을 받기 시작한다.

### CI에서 환경 변수로 키를 줄 때

1. 사용자가 파이프로 입력을 넘기는 CI에서 키 없이 `saturn`을 실행한다.
2. 화면이 없으므로 `cli`는 `RouterKeyRequired`를 받으면 묻지 않고 `SATURN_KEY` 환경 변수와 `router.key.command` 설정 방법을 안내하고 끝낸다.
3. 사용자가 `SATURN_KEY` 환경 변수로 키를 주고 다시 실행한다.
4. engine이 provider를 실행할 때 자식 환경에서 제외 목록의 변수를 지운다.
5. provider와 그 아래 subagent는 키 변수가 없는 환경에서 작업한다.

### subagent가 저장된 키를 찾을 때

1. Claude subagent가 도구 호출로 키 저장소 조회 명령을 실행하려 한다.
2. engine이 실행별 설정으로 넘긴 Saturn 소유 PreToolUse 훅이 그 호출을 막는다.
3. 사용자가 따로 둔 훅은 그대로 동작한다.

## 상세 설계

### 키 입력

1. `secrets`가 숨김 입력, `SATURN_KEY` 환경 변수, 비밀번호 관리자 명령 설정(`router.key.command`) 중 하나로 키를 받는다.
2. 비밀번호 관리자 명령으로 받은 키는 메모리에만 둔다.
3. `routers`가 입력 직후 `GET /v1/models`로 키를 확인하고 모델 버전을 고정한다.
4. 확인에 성공하면 `secrets`가 키를 저장한다.

- router 키는 명령 인자와 표준 입력으로 받지 않는다. 키 입력 방법을 위 세 가지로 한정하기 위해서다.
- 등록된 키가 확인되면 화면이 있어도 묻지 않는다. 키가 없거나 확인에 실패했을 때만 묻는다.
- 키를 받는 순서는 `SATURN_KEY` 환경 변수 → 저장된 키(키체인, 키체인이 없으면 0600 파일) → 비밀번호 관리자 명령 → 숨김 입력이다(초안). 시작할 때 환경 변수가 있으면 저장된 키보다 먼저 읽고, 환경 변수 키가 거절돼도 저장된 키로 넘어가지 않는다. 앞의 방법이 모두 실패하고 화면이 있으면 TUI가 숨김 입력 창으로 묻고, 화면이 없으면(스크립트, CI) 묻지 않고 `SATURN_KEY` 환경 변수와 `router.key.command` 설정 방법을 안내하고 끝낸다.
- 숨김 입력은 TUI의 router 키 입력 창이 받아 `SubmitRouterKey`로 `engine`에 보낸다. `engine`은 터미널에서 직접 숨김 입력을 받지 않는다. 사용자당 하나인 상주 프로세스라 키를 물을 터미널을 갖지 않기 때문이다.
- `SubmitRouterKey` 메시지는 기록 저장소와 로그에 남기지 않는다. `engine`이 키를 기다리지 않을 때 온 `SubmitRouterKey`는 거절한다. 확인된 키를 틀린 키로 덮어 잃지 않기 위해서다.
- 비밀번호 관리자 명령은 셸 없이 실행하고 stdout 첫 줄을 키로 쓴다. 실패하면 종료 코드만 보인다.
- 받은 값은 앞뒤 공백과 끝 줄바꿈을 지우고, 버릴 때 메모리를 0으로 덮는다.
- 시작 확인이 실패하면 `engine`은 소켓을 열고, router를 확인하기 전에는 `SubmitRouterKey`, `Attach`, `Detach`만 받는다. 붙는 TUI에 `RouterKeyRequired`로 키를 요청한다. 시작 확인 절차는 [router](router.md)에, 소켓 요청 규칙은 [engine 수명](engine-lifecycle.md)에 있다.

### 키 저장

| 조건 | 저장 위치와 방식 |
|---|---|
| 기본 | macOS 키체인에 OS API로 직접 저장 |
| OS 저장소가 없음 | 권한 0600 파일에 저장 |
| 강화 방식 | 신뢰 앱 없는 키체인 항목으로 저장 |

- 키체인은 `keyring`으로 OS API를 직접 쓴다.
- 강화 방식이면 session 시작 때 키체인 암호를 한 번 요청한다.
- 강화 방식이면 비활성 10분이나 최대 12시간 뒤 키를 잠금 상태로 바꾼다.
- 설정에는 키의 출처와 끝 4자리만 적는다.
- 키체인 항목의 서비스 이름은 `saturn`, 계정 이름은 `saturn-key`다.
- 대체 파일은 `~/.saturn/router.key`이고(초안), 권한이 0600이 아니면 읽지 않는다.
- 대체 파일은 같은 폴더에 새 임시 파일을 0600으로 만든 뒤 교체한다.
- 임시 파일은 고유 이름으로 만들고 기존 `.partial` 경로의 심볼릭 링크를 따라가지 않는다.
- engine은 강화 방식의 잠금 조건을 1분마다 확인한다(초안).
- `keyring`은 항목의 신뢰 앱 목록을 정하지 못한다. 강화 방식 항목을 어떤 API로 만들지는 [#102](https://github.com/woonyong-choi/saturn/issues/102)에서 정하고, 그 전에는 표준 방식 항목에 잠금 시계만 더한다.
- router 주소와 키 참조는 폴더 층에서 바꿀 수 없다. 사용자 전용 항목이기 때문이다.

### `security` 명령을 쓰지 않는 이유

키를 키체인에 저장할 때 `security` 명령을 쓰지 않는다. 그 명령으로 저장하면 명령 자체가 항목의 신뢰 앱이 된다. 그러면 누구든 같은 명령으로 확인 창 없이 키를 읽을 수 있다. OS API로 직접 저장한 항목을 확인 창 없이 읽는 경로가 있는지는 [#2](https://github.com/woonyong-choi/saturn/issues/2) 실험으로 확인한다.

### 키를 저장하지 않는 곳

- router 키는 SQLite 기록 저장소, 로그, 영수증, 설정 파일에 저장하지 않는다. 저장된 곳을 에이전트가 찾아가 읽는 위험을 줄이기 위해서다.
- Authorization 헤더는 기록하지 않는다.

### 자식 환경의 변수 제거

하위 접속으로 만든 채팅의 환경도 같은 규칙을 따르고, 권한 모드 `full`이어도 훅과 샌드박스는 같다. 하위 접속의 출입증은 키가 아니며 [하위 접속](child-sessions.md#출입증)이 다룬다.

1. engine이 `Supervisor`로 넘길 때 제외 목록의 변수를 지운다.
2. `Supervisor`가 provider로 넘길 때 제외 목록의 변수를 다시 지운다.

- 제외 목록은 `secrets` 모듈 한 곳에 두고 Saturn 내부 비밀 변수도 같은 목록으로 처리한다. 제거 대상을 한 곳에서 테스트로 고정하기 위해서다.
- 두 단계 모두에서 지운다. router 키 환경 변수가 자식 프로세스에 그대로 전달되는 일을 막기 위해서다.
- 프로세스 표를 읽는 `/bin/ps`에도 같은 제외 목록을 적용한다.

### Saturn 소유 PreToolUse 훅

engine은 Claude를 실행할 때 Saturn 소유 PreToolUse 훅을 실행별 설정으로 넘긴다.

- 훅은 키 저장소 조회 명령, 대체 파일 읽기, Saturn 비밀 파일 접근을 막는다. subagent가 저장된 키를 찾아가 읽는 일을 막기 위해서다.
- 사용자의 기존 훅은 감싸거나 지우지 않는다. provider 설정 파일을 고치지 않기 위해서다.
- 훅은 권한 규칙과 따로 동작한다. 규칙이 `allow`여도 키 저장소 접근은 훅이 막는다(초안). 실행별 설정은 권한의 `--settings` 값에 합쳐 넘긴다([권한](permissions.md)).
- 훅 명령은 engine 실행 파일의 `hook pre-tool-use --home <Saturn 홈>`이고 모든 도구에 건다(초안). 이 명령은 engine 잠금과 소켓을 열지 않고 판정만 하고 끝난다. 훅은 provider 자식 프로세스에서 짧게 실행되기 때문이다. 막는 명령은 `security`의 `find-generic-password`, `find-internet-password`, `dump-keychain`, `export`이고, 막는 경로는 `~/.saturn/router.key`, `~/Library/Keychains/`, `/Library/Keychains/` 아래다(초안). 셸 연결 기호로 나뉜 부분마다 보고, 경로는 심볼릭 링크를 푼 뒤 비교한다.
- 훅 입력은 Claude Code PreToolUse 규격의 stdin JSON(`tool_name`, `tool_input`, `cwd`)이다. `Bash`는 `command`를 명령으로, `Read`·`Edit`·`MultiEdit`·`Write`·`NotebookRead`·`NotebookEdit`·`Glob`·`Grep`·`LS`는 경로 필드를 경로로 판정하고, 상대 경로는 `cwd` 기준으로 바꾼다. 그 밖의 도구는 판정하지 않는다.
- 훅 출력은 막을 때 stdout에 `hookSpecificOutput`(`permissionDecision: "deny"`와 이유 한 줄)을 쓰고 종료 코드 0으로 끝난다. 허용할 때는 아무것도 쓰지 않고 종료 코드 0으로 끝내 사용자의 다른 훅이 이어서 판정하게 한다. 입력이 JSON이 아니거나 `tool_name`이 없으면 stderr에 이유를 쓰고 종료 코드 2로 끝낸다(초안). 판정할 수 없는 호출을 통과시키지 않기 위해서다.
- 명령 판정은 낱말을 따옴표와 이스케이프를 푼 뒤 본다. `sec"ur"ity`, `s\ecurity`처럼 섞은 형태도 같은 낱말로 읽는다. `sudo`, `env`, `xargs`, `timeout` 같은 감싸는 명령은 옵션을 알 수 없어 뒤의 낱말마다 명령 시작 자리로 본다.
- 셸(`sh`, `bash`, `zsh`, `dash`, `ksh`, `fish`, `ash`, `csh`, `tcsh`)의 `-c` 인자와 `eval` 인자는 같은 판정기로 다시 해석한다. 표준 입력으로 받는 경우(`echo ... | sh`, `sh <<< ...`, `sh <<EOF`)도 넘어가는 문자열을 다시 해석한다. 재귀 깊이는 4단계까지이고 넘으면 막는다(초안).
- 인터프리터(`python*`, `perl`, `ruby`, `node`, `php`, `lua`, `osascript`, `deno`, `bun`, `swift`, `awk`)의 인자와 heredoc 본문은 해석하지 않고 문자열만 본다. `security`와 막는 하위 명령이 함께 나오거나 키 저장소 경로(`Library/Keychains`, `router.key`, 훅이 아는 차단 경로)가 나오면 막는다. 막는 쪽으로 치우친 검사라 스크립트가 그 낱말을 일반 글로 담아도 막힌다.
- `security -i`(표준 입력으로 하위 명령을 받는 모드)는 막는다.
- 해석할 수 없는 명령은 막는다. 따옴표 짝이 안 맞는 경우, `<<` 뒤에 구분자가 없는 경우, 재귀 깊이를 넘은 경우다. `#` 뒤 주석과 heredoc 본문의 따옴표는 짝을 보지 않는다.
- 이 판정은 차단 목록 방식이라 막지 못하는 형태가 남는다. 문자열을 조립하거나 인코딩해 실행하는 명령(`base64 -d | sh`), 내려받은 스크립트나 사용자가 만든 스크립트 파일 실행, 인터프리터가 이름을 조립해 부르는 `security`, `find -exec`처럼 목록에 없는 실행기, 작업 폴더를 옮긴 뒤의 상대 경로가 그렇다. 키 항목 자체에 접근 제어를 거는 근본 대책은 [#2](https://github.com/woonyong-choi/saturn/issues/2)와 [#102](https://github.com/woonyong-choi/saturn/issues/102)에서 정한다. 훅은 그 앞의 한 겹이다.
- Claude Code 2.1.288에서 Saturn 훅은 Bash로 쓴 `security find-generic-password` 조회를 요청이 호스트에 오기 전에 막았고, 전경과 백그라운드 subagent의 같은 조회도 막았다(각 3/3). 훅 입력에는 subagent 호출에 `agent_id`와 `agent_type`이 실린다. 중첩 subagent의 조회도 훅 기록에서 3/3 막혔지만 그 호출이 스트림에는 1/3만 보였다. 반면 `sh -c "/usr/bin/security find-generic-password ..."`는 3/3 막지 못했고 값이 도구 결과에 나왔다. 원인은 첫 낱말과 첫 비옵션 낱말만 보는 판정이었고, 셸·`eval`·인터프리터 인자를 해석하는 수정은 위 항목대로 단위 테스트로만 확인했다. 실제 provider로 다시 재지 않았다. Codex 훅은 재지 않았다([#3](https://github.com/woonyong-choi/saturn/issues/3), [#23](https://github.com/woonyong-choi/saturn/issues/23), [실험](../experiments/claude-provider-behavior/report.md)).
- Codex 0.158.0은 전용 `CODEX_HOME`의 `hooks.json`에 둔 `PreToolUse` 훅을 부른다([실측](../experiments/codex-provider-behavior/report.md)). 훅은 `hooks/list`의 `currentHash`를 `config.toml`의 `hooks.state`에 `trusted_hash`로 써야 불리고, 신뢰 값이 없으면(`trustStatus=untrusted`) 불리지 않았다(0/3). 입력은 Claude와 같은 키(`tool_name`, `tool_input.command`, `cwd`)였고 Saturn 훅 명령이 그대로 판정했다. 신뢰한 훅은 `security find-generic-password`와 가짜 키 파일의 `cat`을 승인 요청이 오기 전에 막았고(각 3/3, 명령 실행 없음, Codex가 `hook/completed`에 `blocked`와 이유를 알림), 샌드박스 없이 실행한 경우도 막았다(3/3). 훅이 없고 샌드박스도 없으면 가짜 값이 출력됐고(3/3), 읽기 전용 샌드박스는 훅 없이도 키체인 조회를 막았다(3/3, 항목을 찾지 못함).
- Codex 파일 편집(`apply_patch`)에도 훅이 불리지만(`tool_input.command`에 `*** Add File: 경로`가 든 패치 글) Saturn 훅은 이 도구 이름을 몰라 허용한다(`saturn-terminal/engine/src/providers/claude/hook.rs`의 `tool_call`이 알지 못하는 도구를 `ToolCall::Other`로 돌려준다). 가짜 키 파일 편집이 3/3 적용됐다. 지금 engine은 Codex에 훅을 넘기지 않는다.

### 겹 구성

키 보호는 한 겹으로 막지 않는다. 훅은 차단 목록이라 막지 못하는 형태가 남으므로, OS가 강제하는 겹을 그 아래에 둔다. 모든 겹은 권한 모드와 무관하게 켜 둔다. 권한은 Saturn 규칙이 정본이지만 router 키 보호는 그 예외이므로 모드 `full`도 이 겹을 끄지 못하고, `/permissions`나 설정에도 끄는 값이 없다.

| 겹 | 막는 것 | 근거 |
|---|---|---|
| 환경 격리 | 자식 환경의 키 변수 | `secrets` 모듈 단위 테스트, [실험](../experiments/router-key-defense/report.md)의 환경 항목 |
| Saturn 소유 PreToolUse 훅 | 이름이 보이는 키 저장소 조회와 파일 도구의 키 저장소 경로 | [Claude provider 실측](../experiments/claude-provider-behavior/report.md), 위 단위 테스트 |
| provider 명령 샌드박스 | 훅이 못 읽는 형태(인코딩, 스크립트 파일, 이름 조립)를 포함한 명령의 키체인 접근 | [OS 수준 방어 실측](../experiments/router-key-defense/report.md) |
| 키체인 접근 제어 | 보조층. 확인 창은 클릭 한 번으로 열려 단독 방어로 쓰지 않는다 | 같은 실측, 구현은 [#102](https://github.com/woonyong-choi/saturn/issues/102) |

### provider 명령 샌드박스

- Claude: engine은 Claude를 띄울 때마다 실행별 `--settings`에 `sandbox`를 합쳐 넘긴다. 모든 session이 대상이고, 새 session과 재개 모두 같다. 값은 `enabled: true`, `allowUnsandboxedCommands: false`, `autoAllowBashIfSandboxed: false`, `failIfUnavailable: true`, `filesystem.denyRead`이다([Claude Code 샌드박스 문서](https://code.claude.com/docs/en/sandboxing), 2026-10-04 확인). `allowUnsandboxedCommands: false`는 `dangerouslyDisableSandbox`로 샌드박스 밖에서 다시 실행하는 길을 막고, `failIfUnavailable: true`는 샌드박스를 쓸 수 없을 때 명령을 샌드박스 없이 돌리지 않고 실패하게 한다. `autoAllowBashIfSandboxed: false`는 샌드박스를 켜면 Bash가 허가 요청 없이 자동 허용되는 Claude Code 기본값을 꺼서, 모든 Bash 호출이 호스트로 와 Saturn 권한 규칙의 판정을 받게 한다([#508](https://github.com/woonyong-choi/saturn/issues/508)).
- `denyRead` 경로는 훅이 막는 키 저장소 경로와 같은 목록이다. 사용자 `~/Library/Keychains`, `/Library/Keychains`, Saturn 홈의 `router.key`이고 링크를 푼 경로가 다르면 그것도 넣는다. 절대 경로로 적는다. 목록은 훅 정책 한 곳에서 만들어 훅과 샌드박스가 어긋나지 않게 한다.
- 합치는 규칙: 사용자의 Claude 설정 파일은 읽지도 고치지도 않는다. 실행별 `--settings`는 훅, `permissions.ask`와 같은 값에 `sandbox`를 더하고, 설정 층끼리 배열은 합쳐지므로 사용자의 `denyRead`와 `allowWrite`는 그대로 남는다. 작업 폴더와 더한 폴더(`--add-dir`)의 쓰기는 Claude 샌드박스 기본 허용이라 Saturn 권한 규칙과 겹치는 `allowWrite`를 따로 넣지 않는다.
- 이 겹이 막지 못하는 것: 사용자 설정의 `sandbox.excludedCommands`로 샌드박스 밖에서 도는 명령, Bash 샌드박스가 덮지 않는 Claude의 파일 도구와 MCP 서버(파일 도구는 훅이 본다). `denyRead`는 Read 도구를 막지 않는다. `excludedCommands`를 실행별 설정으로 비울 수는 없다(배열은 합쳐진다). 그래서 Claude session을 열기 전에 사용자, 프로젝트, 프로젝트 로컬 설정 파일에 비어 있지 않은 `sandbox.excludedCommands`가 있는지 보고, 있으면 session을 열지 않고 `NotSent`로 그 파일 경로와 이유를 알린다(목록 값은 알리지 않는다). 프로젝트 층 설정의 제외는 실측에서 적용되지 않았지만 어느 층에서든 보장하지 않는다. 제외를 지워야 Saturn에서 Claude를 쓸 수 있다.
- Codex: 모든 권한 모드가 같은 구성으로 `thread/start`와 `thread/resume`에 샌드박스 `workspace-write`(작업 폴더와 더한 폴더만 쓰기, 네트워크 없음)를 주고, 응답이 다른 샌드박스를 적용했으면 첫 턴을 보내지 않는다([권한](permissions.md#codex-구성)). 사용자 Codex 설정의 `sandbox_mode`와 `sandbox_workspace_write`는 생성 설정으로 옮기지 않는다. 그래서 Saturn에서 고를 수 있는 모든 모드에서 명령은 Codex 샌드박스 안에서 돌고, 샌드박스를 끄는 `danger-full-access`는 Saturn이 고르지도 받아들이지도 않는다. 실측에서 이 샌드박스는 `:read-only`, `:workspace` 모두 키체인 조회를 막았다. 남은 경로는 샌드박스 밖 실행 승인이고 engine이 허가 판정에서 막는다. 명령 승인 요청에는 실행 범위를 직접 알리는 값이 없고, 샌드박스 밖 실행을 요청한 호출(`sandbox_permissions=require_escalated`)만 이유(`reason`)를 붙여 온다. Codex 어댑터는 `reason`이 비어 있지 않거나 `additionalPermissions`, `networkApprovalContext`가 있으면 샌드박스 밖 실행(`PermissionCall.outside_sandbox`)으로 표시하고 허가 요청 줄에 `run command outside sandbox:`로 밝힌다. 범위를 확신할 수 없는 요청은 밖으로 본다. engine의 `router_permission`은 호출마다 모드, 규칙, 항상 허용보다 먼저 키 저장소 판정(위 훅과 같은 `HookPolicy`)을 하고, 걸리면 `full`과 `allow` 규칙이 있어도 거부한다. 파일 편집과 읽기의 키 저장소 경로도 같다. 그다음 샌드박스 밖 실행 요청은 허용으로 판정돼도 사용자에게 묻는다(`full` 자동 허용, 모드 기본 규칙, 개별 `allow`, 항상 허용 모두 대신 허용하지 못한다). 하위 채팅은 묻지 않고 거부한다. 일반 작업 권한은 기존 규칙이 그대로 정하고, 키 저장소 접근은 이 예외로 분리한다. 명령 해석 한계(스크립트 파일, 이름 조립)는 샌드박스 밖에서는 훅 판정이 못 막으므로 사용자가 허가 창의 표시를 보고 정한다.
- 샌드박스 지원 여부: macOS에서는 기본 도구로 돈다. 샌드박스를 쓸 수 없는 환경(Linux의 필수 도구 부재 등)에서 Claude는 명령 대신 시작이 실패하고, Saturn은 그 오류를 session 시작 실패로 보인다.

### 전송

- router는 `engine`만 부른다. router 키가 자식 프로세스나 다른 호스트로 새는 일을 막기 위해서다.
- router 전송은 HTTPS와 허용 호스트 `api.typesafe.ai`만 쓴다. 키가 다른 호스트로 가는 일을 막기 위해서다.
- 리다이렉트 때 인증 헤더를 지운다. 같은 이유다.
- TLS 검증을 끄는 설정은 두지 않는다. 같은 이유다.

### 출력 마스킹

- router 키와 일치하는 문자열은 로그, 오류, 디버그 출력에서 가린다. 키가 provider 기록이나 TUI로 새는 일을 막기 위해서다.
- 하위 접속의 출입증 토큰(`saturn-pass-`로 시작하는 16진수 글자)도 키와 같은 방식으로 가린다([하위 접속](child-sessions.md#출입증)).
- Codex app-server stdout의 JSON 문자열 값은 오류와 이벤트로 바꾸기 전에 가린다.
- 판단 기록을 저장하기 전에 `secrets`가 보낸 원문과 받은 원문의 비밀값을 가린다.
- 가린 자리는 `[redacted]`로 바꾸고 끝 4자리도 남기지 않는다. `Authorization`, `Proxy-Authorization`, `X-Api-Key` 헤더 줄은 이름만 남기고 값을 가린다(초안).
- 출력 가림 버퍼는 줄바꿈 전의 조각을 `flush`나 `Debug`로 내보내지 않는다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| 키 입력 거부 | TUI가 원인 한 줄을 보이고 끝낸다. `engine`은 계속 키를 기다린다. |
| TUI가 보낸 키의 확인 실패 | 오류 응답과 함께 `RouterKeyRequired`를 다시 보내고 키를 저장하지 않는다. |
| 화면이 없는 환경(파이프, CI)의 router 확인 실패 | `cli`가 묻지 않고 끝내며 `SATURN_KEY` 환경 변수와 `router.key.command` 설정 방법을 안내한다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| provider 자식 프로세스 환경에는 router 키 변수가 없다. | `secrets` 모듈 테스트로 자식 환경에 제외 목록의 이름이 없는지 확인한다. |
| router 키는 기록 저장소, 로그, 오류 출력에 남지 않는다. | 키를 넣은 호출과 오류를 만든 뒤 저장소와 출력에 키 문자열이 없는지 확인한다. `saturn-terminal/engine/src/lifecycle/requests.rs`의 `requests_wait_for_router_key_and_key_is_not_recorded`는 홈 폴더 전체(`logs/` 포함)를, `engine_log_file_never_holds_the_router_key`는 실제 `EngineLog` 파일을 본다. |
| router 호출은 TLS 인증서 검증을 끄지 않는다. | `saturn-terminal/engine/src/routers/remote/tests.rs`의 `real_client_rejects_an_untrusted_certificate_and_sends_nothing`이 실제 reqwest 클라이언트로 자체 서명 인증서 서버를 거부하는지 확인한다. |
| 키체인에 직접 저장한 키는 확인 창 없이 읽히지 않는다. | [#2](https://github.com/woonyong-choi/saturn/issues/2) 실험으로 확인 창 없이 읽는 경로를 확인한다. |
| engine 실행 파일은 생성한 훅 명령(`hook pre-tool-use`)을 받아 허용과 거부를 훅 규격의 출력과 종료 코드로 돌려준다. | `saturn-terminal/engine/tests/key_hook.rs`의 `hook_command_denies_key_store_access`, `hook_command_allows_ordinary_calls_without_output`, `hook_command_blocks_unreadable_input_with_exit_code_2`, `hook_command_leaves_saturn_home_untouched` |
| 훅은 셸·`eval`·인터프리터로 감싼 키 저장소 조회도 막고, 해석할 수 없는 명령은 막으며, 목록 밖 하위 명령은 막지 않는다. | `saturn-terminal/engine/src/secrets/hook.rs`의 `shell_wrapped_lookups_are_denied`, `quoting_and_escapes_inside_shell_strings_do_not_hide_lookups`, `separators_inside_shell_strings_are_split`, `eval_strings_are_judged_again`, `nested_shells_are_judged_down_to_the_limit`, `nesting_beyond_the_limit_is_denied`, `unparseable_commands_are_denied`, `interpreter_one_liners_naming_key_stores_are_denied`, `shells_fed_by_pipe_here_string_or_here_document_are_judged`, `interactive_security_is_denied`, `wrapped_commands_outside_the_list_are_allowed`, `quotes_comments_and_here_documents_in_ordinary_commands_are_allowed` |
| Saturn 소유 PreToolUse 훅은 키 저장소 접근을 막는다. | Claude 직접 명령은 [실험](../experiments/claude-provider-behavior/report.md)에서 3/3 막혔고 `sh -c` 감싼 명령은 수정 전에 막지 못했다(수정 뒤 실제 provider 재측정은 아직). Codex는 [실측](../experiments/codex-provider-behavior/report.md)에서 명령 차단 3/3을 확인했고 파일 편집 도구(`apply_patch`)는 막지 못했다. |
| Claude를 띄울 때마다(모든 모드 포함 `full`) 실행별 `--settings`에 Bash 샌드박스(`enabled`, `allowUnsandboxedCommands: false`, `autoAllowBashIfSandboxed: false`, `failIfUnavailable: true`)와 키 저장소 경로의 `filesystem.denyRead`가 들어간다. | `saturn-terminal/engine/src/providers/claude/tests.rs`의 `launch_args_enable_the_bash_sandbox_and_deny_reading_key_stores`, `the_key_sandbox_stays_on_in_full_mode_and_keeps_the_other_settings`, `the_bash_sandbox_does_not_auto_allow_shell_commands`, `launch_args_add_defaults_and_hook_settings`, `saturn-terminal/engine/src/lifecycle/intake.rs`의 launch 명세 테스트 |
| 사용자·프로젝트 Claude 설정에 비어 있지 않은 `sandbox.excludedCommands`가 있으면 session을 열지 않고 `NotSent`로 알린다. | `saturn-terminal/engine/src/providers/claude/tests.rs`의 `session_does_not_open_when_a_settings_layer_excludes_commands_from_the_sandbox`, `empty_or_missing_sandbox_exclusions_do_not_stop_the_session` |
| Codex가 올린 샌드박스 밖 실행 승인은 `full` 자동 허용보다 먼저 키 저장소 판정을 받아 거부되고, 그 밖의 샌드박스 밖 실행은 허용으로 판정돼도 묻는다. | `saturn-terminal/engine/src/lifecycle/permissions.rs`의 `key_store_lookup_is_denied_before_the_mode_in_every_mode`, `key_file_access_is_denied_even_when_a_rule_allows_it`, `outside_sandbox_command_is_asked_even_in_full_mode_and_with_an_allow_rule`, `saturn-terminal/engine/src/providers/codex/permission.rs`의 `command_request_with_a_reason_or_extra_permissions_runs_outside_the_sandbox`. 실제 Codex 재확인은 #423에서 한다. |
| Claude 샌드박스에 키체인 폴더 읽기 금지를 더하면 키체인 조회가 막히고 로그인은 유지된다. | [실험](../experiments/router-key-defense/report.md)에서 직접 조회 0/3 접근(샌드박스만 켠 경우 3/3). Saturn이 띄운 실제 session에서 거부되는지는 #423을 닫기 전에 확인한다. |
| Codex 명령은 모든 모드에서 작업 폴더 쓰기 샌드박스 안에서 돌고, 다른 샌드박스가 적용되면 첫 턴을 보내지 않는다. | `saturn-terminal/engine/src/providers/codex/permission.rs`의 `applied_policy_must_be_untrusted_workspace_write_without_network`, [실험](../experiments/router-key-defense/report.md)에서 `:read-only`와 `:workspace` 모두 0/3 접근. 샌드박스 밖 실행 승인은 위 줄이 막는다. |
| 훅은 subagent의 도구 호출에도 적용된다. | Claude 전경, 백그라운드, 중첩 subagent의 조회가 [실험](../experiments/claude-provider-behavior/report.md)에서 훅에 막혔다. Codex는 [#23](https://github.com/woonyong-choi/saturn/issues/23)에서 확인한다. |

## 단점

- 제외 목록과 훅 검사를 provider 변화에 맞춰 계속 유지한다.
- 훅의 명령 판정은 차단 목록이라 위의 남은 한계를 근본적으로 없애지 못한다. 그 한계는 provider 명령 샌드박스가 맡는다.
- 샌드박스를 쓸 수 없는 환경에서는 Claude session이 시작되지 않는다.
- 강화 방식에서는 session을 시작할 때마다 키체인 암호를 입력한다.

## 대안

- 환경 변수로 키를 그대로 넘기는 방식은 provider와 subagent에 키가 드러나 버렸다([engine만 router를 부르고 자식 프로세스 환경에서 router 키를 지운다](../decisions/2026-09-29-engine-as-router-proxy.md)).
- 암호문 파일에 저장하고 실행마다 암호를 확인하는 방식은 실행마다 암호를 입력해야 해 버렸다(같은 결정 기록).
