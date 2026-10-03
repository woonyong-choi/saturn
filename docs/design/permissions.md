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
5. 읽기 전용으로 접수한 작업이 실행 중일 때 모드를 `edit`로 올리면, 그 작업은 접수 때 권한대로 읽기 전용으로 남아 편집 요청을 거부하고 알림을 남긴다. 사용자가 새 입력을 보내면 쓰기 권한으로 접수해 쓰기 잠금 규칙을 따르고, 읽기 전용 작업에는 끼워 넣지 않고 그 작업이 끝난 뒤 새 턴으로 보낸다.

## 상세 설계

### 권한 규칙

규칙 대상은 셸 명령, 파일 편집, MCP 도구, subagent 실행 네 가지다. 값은 `allow`, `ask`, `deny` 셋이다. 키 이름과 형식은 [설정](settings.md)의 `permission` 키 표에 있다.

1. `settings`가 현재 권한 모드의 기본 규칙을 앞에 둔다.
2. `settings`가 사용자 설정의 개별 규칙을 그 뒤에 잇는다.
3. `settings`가 폴더 설정의 개별 규칙을 그 뒤에 잇는다.
4. engine이 호출마다 이어 붙인 규칙에서 마지막으로 일치한 규칙의 값을 쓴다.
5. 개별 규칙 중 `deny`가 하나라도 일치하면 순서와 상관없이 `deny`를 쓴다.

- 마지막 일치가 이긴다. OpenCode의 규칙 방식을 따른다.
- `deny`는 예외다. 폴더 설정이 사용자 설정의 `deny`를 뒤집어 저장소가 사용자의 금지를 풀지 못하게 하기 위해서다. 채팅 층과 실행 층의 개별 `deny`도 같게 본다(초안). 거부가 순서 때문에 풀리는 일을 없애기 위해서다. 모드의 기본 규칙에는 이 예외가 없고, 개별 `allow`가 모드의 기본 `deny`를 덮을 수 있다.
- 패턴은 `*`를 포함할 수 있는 글자 일치다(초안). `*`는 `/`와 공백을 포함한 아무 글자열이고 `\*`는 글자 `*` 그대로다. 끝이 ` *`인 패턴(`git status *`)은 인자가 없는 명령(`git status`)에도 일치한다.
- 셸 명령은 `&&`, `||`, `;`, `|`, `&`, 줄바꿈, 괄호, 역따옴표로 나뉜 부분마다 판정하고 가장 엄한 값(`deny`, `ask`, `allow` 순)을 쓴다(초안). 따옴표 안의 구분 글자는 나누지 않는다. 허용된 명령 뒤에 막힌 명령을 붙이는 우회를 막기 위해서다.
- 명령 치환(`$(...)`, 역따옴표)이 들어 있거나 따옴표가 닫히지 않은 셸 명령은 실제 실행을 글자만으로 알 수 없어, 거부가 아니면 규칙이 `allow`여도 `ask`로 판정한다(초안). 명령 전체가 항상 허용과 일치할 때만 `allow`다.
- 편집은 경로마다 판정하고 가장 엄한 값을 쓴다. 상대 경로는 작업 폴더 기준으로 읽고 링크를 풀어 판단하므로, 작업 폴더 안의 링크가 밖을 가리키면 밖의 편집이다. 패턴은 절대 경로와 작업 폴더 안일 때의 상대 경로 둘 다에 맞춰 본다(초안). 경로를 알 수 없는 편집 요청(Codex 승인 요청이 경로를 싣지 않고 앞선 `fileChange` 항목에서도 얻지 못한 경우)은 작업 폴더 밖으로 본다.
- MCP 도구의 대상 이름은 `mcp__{서버}__{도구}`이고(Claude의 도구 이름과 같다. 초안), Codex 요청에서 도구 이름을 읽지 못하면 도구 자리를 `?`로 둔다. subagent의 대상 이름은 종류 이름이다(Claude `Task`의 `subagent_type`).
- 도구 전체에 한 값을 준 규칙(`permission.shell = "ask"`)은 패턴 `*` 규칙과 같다.
- 폴더 설정의 `permission`은 폴더 설정 신뢰 창의 적용되는 항목에 보인다. 저장소가 사용자 모르게 허용 규칙을 넣는 일을 막기 위해서다.
- 개별 규칙은 입력 접수 때 고정한 설정 번호의 값을 쓴다. 한 입력을 한 규칙으로 끝까지 판단하기 위해서다. 모드는 예외로 다음 허가 요청부터 바뀐 값을 쓴다(초안). 모드를 낮추면 진행 중인 작업에도 바로 적용하기 위해서다. 읽기 전용으로 접수한 실행은 쓰기 잠금 없이 도는 중이므로 모드를 올려도 그 실행의 허가 요청은 읽기 전용 모드로 판정하고 항상 허용도 보지 않는다(초안). 접수 때 읽기 전용이던 실행이 잠금 없이 다른 쓰기 작업과 같은 폴더에 함께 쓰는 일을 막기 위해서다. 이 때문에 거부나 묻기가 되면 `ReadOnlyRunKept` 알림을 대화 기록에 남기고(`읽기 전용으로 접수한 작업의 쓰기를 거부함 · 쓰려면 새 입력으로 보내세요`), 새 모드는 그다음 입력부터 쓰기 권한으로 접수해 적용한다. 실행 중에 쓰기 잠금을 얻어 올리는 방식은 잠금을 얻지 못할 때 허가 요청을 붙들고 있어야 해 쓰지 않는다.

### 권한 모드

`permission.mode`(초안 이름)는 기본 규칙 묶음을 고른다. 기본값은 `edit`다. 모드 이름과 값은 초안이다.

| 모드 | 기본 규칙 | Claude의 같은 모드 | Codex의 같은 모드 |
|---|---|---|---|
| `ask` | 모든 실행을 묻는다 | `default` | 해당 없음 |
| `edit` | 작업 폴더 안 편집과 읽기 전용 셸 명령은 `allow`, 그 밖은 `ask` | `acceptEdits` | `auto` |
| `read-only` | 읽기만 `allow`, 편집과 실행은 `deny` | `plan` | `read-only` |
| `full` | 모두 `allow`, 개별 규칙의 `deny`만 적용 | `bypassPermissions` | `full-access` |

- `read-only`에서도 개별 `allow`나 `ask` 규칙이 하나라도 있으면 입력을 쓰기 권한으로 접수해 쓰기 잠금 규칙을 따른다([입력 처리](input-handling.md#판단-차례와-적용)). 규칙이 모드의 기본 `deny`를 풀어 승인한 쓰기가 잠금 없이 나가는 일을 막기 위해서다(초안). `deny` 규칙만 있으면 읽기 전용이다.
- `--add-dir`와 `/add-dir`로 더한 폴더([engine 수명과 복구](engine-lifecycle.md#채팅-폴더와-이어-열기)) 안의 편집은 `edit`에서 작업 폴더 안의 편집처럼 `allow`다(사용자 결정). 읽기 전용과 `deny` 규칙은 더한 폴더에도 그대로 적용한다. 더한 폴더는 쓰기 잠금의 범위에도 들어가, 같은 폴더를 더한 채팅끼리는 쓰기 작업이 한 번에 하나만 실행된다([입력 처리](input-handling.md#쓰기-규칙)).
- `edit`에서 작업 폴더 밖 편집, 읽기 전용 목록 밖의 셸 명령, MCP 도구, subagent 실행은 `ask`다. 작업 폴더 밖 편집은 사용자가 확인한 경로만 열기 위해서다(초안).
- `edit`에서도 작업 폴더와 더한 폴더 안의 `.git` 아래 경로(경로 구성 요소가 `.git`인 모든 경로, worktree의 `.git` 파일 포함) 편집은 `ask`다(사용자 결정). 에이전트가 `.git/config`에 `diff.external`, `core.fsmonitor`, `core.pager`, textconv 드라이버나 `.git/hooks`를 써 넣어, 자동 허용된 `git status`, `git diff`, `git log`로 외부 명령을 묻지 않고 실행하는 일을 막기 위해서다. 파일 편집 도구 경로만 해당하고 셸 명령의 편집은 셸 규칙을 따른다. `.gitignore`, `.github/`는 `.git`이 아니므로 그대로 `allow`다. 개별 편집 규칙 `allow`는 일반 우선순위대로 이 기본값을 덮는다.
- `edit`는 읽기 전용 셸 명령 `ls`, `cat`, `rg`, `grep`, `git status`, `git diff`, `git log`를 인자와 함께 `allow`한다(사용자 결정). Codex 읽기 전용 샌드박스가 `ls`, `cat`까지 승인 요청으로 올리는 불편을 줄이기 위해서다. 목록은 코드 상수 하나이고 설정 키는 없다. `|`, `<`, `>`, `;`, `&&`, `||`, `&`, 줄바꿈, 괄호, 명령 치환이 섞인 명령은 목록에 들지 않고 `ask`다. `--pre`, `--hostname-bin`, `--output`, `--ext-diff`, `--textconv`(`옵션=` 형태 포함)가 붙으면 파일을 쓰거나 외부 명령을 실행할 수 있어 목록에 들지 않고 `ask`다. `git`은 긴 옵션의 앞부분만 써도 받으므로 이 옵션의 앞부분도 같게 본다. 개별 셸 규칙이 일치하면 그 값이 목록을 덮고 `deny`는 항상 이긴다. `full`은 목록을 보지 않고 `read-only`와 `ask`는 그대로다. 목록은 모드 기본 규칙이므로 Codex execpolicy에 번역하지 않는다.
- `permission.shell` 같은 개별 규칙은 모드 기본 규칙 위에 덧붙는다.
- 모드마다 provider 구성은 같다. Codex는 `untrusted`와 읽기 전용 샌드박스, Claude는 모든 대상 도구의 `ask` 목록을 쓴다. 모드에 따라 달라지는 것은 engine이 허가 요청에 하는 답이다. `deny` 패턴이 모든 요청에 걸리게 하고, 모드를 바꿔도 provider를 다시 시작하지 않기 위해서다(초안).
- Codex execpolicy에는 개별 셸 규칙만 번역한다. `"*"` 패턴은 어떤 명령도 매치하지 않았기 때문이다. 모드 기본 규칙과 번역하지 않은 요청은 engine이 승인 요청에 규칙으로 답한다.
- 세션 중에는 TUI 명령 `/permissions`(초안)로 모드를 본다. `/permissions {모드}`는 채팅 층의 `permission.mode`를 바꾼다. engine은 채팅 층 원문에 `permission.mode`만 고쳐 쓰고 요청마다 그 값을 먼저 읽는다. 값 없이 실행해 현재 모드를 보이는 동작은 조회 결과를 돌려주는 방식([#177](https://github.com/woonyong-choi/saturn/issues/177))이 정해진 뒤에 넣는다. 화면은 [TUI](tui.md)에 있다.
- 모드의 순서는 낮은 쪽부터 `read-only`, `ask`, `edit`, `full`이다. `ask`는 묻기만 하고 거부하지 않으므로 `read-only`보다 높다.
- 폴더 설정의 `permission.mode`는 사용자 층까지 합친 모드보다 낮은 값만 적용하고, 같거나 높은 값은 무시한다. 저장소가 모두 허용을 켜지 못하게 하기 위해서다. 무시한 사실은 폴더 설정 신뢰 창의 무시되는 항목에 보인다.
- 모두 허용(`full`)은 사용자 설정이나 `/permissions`로만 켠다. Claude Code는 v2.1.257부터 프로젝트·로컬 설정의 `bypassPermissions`와 `auto`를 무시한다([설정 문서](https://code.claude.com/docs/en/settings), 2026-10-02 확인). Codex는 신뢰한 프로젝트 설정이 승인 정책을 정할 수 있다.

### 항상 허용 저장

사용자가 허가 요청 창에서 `항상`을 고르면 engine은 그 호출의 도구 종류와 패턴을 허용 규칙으로 기록 저장소에 저장한다. provider 설정 파일에는 쓰지 않는다.

- 항상 허용은 설정 규칙이 `ask`로 판정한 호출에만 적용한다. 개별 규칙의 `deny`는 항상 허용보다 앞선다(초안). 사용자가 나중에 넣은 거부 규칙이 옛 허용에 가려지지 않게 하기 위해서다.
- 항상 허용의 범위는 작업 폴더 단위이고 Saturn 기록 저장소에만 저장한다(초안). 표 `permission_allows`(작업 폴더, 도구, 패턴)에 한 행씩 두고, 같은 행은 한 번만 저장한다([기록 저장과 보존](records.md)).
- 저장하는 패턴은 호출을 글자 그대로 일치시키는 패턴이다(`*`와 `\`를 앞에 `\`를 붙여 적는다). 셸 명령은 규칙이 `ask`로 판정한 조각마다, 편집은 `ask`였던 경로마다(링크를 푼 절대 경로) 저장한다. 명령 치환이 든 명령은 명령 전체를 저장한다.
- 규칙으로 읽은 호출의 `항상 허용` 답은 provider에 이번만 허용으로 보내고 저장은 Saturn이 한다. provider가 세션 동안 기억해 뒤에 바뀐 규칙이나 모드를 우회하는 일을 막기 위해서다(초안). 규칙으로 읽지 못한 요청(권한 요청 등)은 저장할 수 없어 provider 값으로 그대로 보낸다.

### 판정 흐름

![허가 요청은 규칙이 allow나 deny로 답하면 바로 답하고, ask일 때만 TUI에 올려 사용자에게 묻는다](../assets/permission-decision.svg)

1. provider가 도구 호출 허가를 요청한다.
2. `providers`가 요청을 도구 종류와 패턴으로 바꾼다.
3. `permission`이 규칙과 저장된 항상 허용으로 값을 정한다.
4. `allow`면 `providers`가 허용 답을 바로 보낸다.
5. `deny`면 `providers`가 거부 답을 바로 보낸다.
6. `ask`면 engine이 TUI에 허가 요청을 올리고, 답이 올 때까지 요청을 보관한다.

- 규칙으로 읽을 수 없는 요청(`call`이 없는 요청: Codex 권한 요청, 명령이 없는 요청, 규칙 대상이 아닌 Claude 도구)은 모드와 상관없이 `ask`다. 규칙이나 기록을 읽지 못해도 `ask`다.
- 규칙이 `allow`나 `deny`로 답했는데 provider가 그 답을 받지 못했으면 사용자에게 묻는다.
- 규칙은 에이전트가 가장 나중에 시작한 입력의 설정 번호 값을 쓰고, 모드는 채팅 층에 쓴 값이 있으면 그것을 먼저 쓴다.

Codex에서는 execpolicy가 판정한 명령은 provider 안에서 끝나고, 나머지는 승인 요청으로 와서 위 흐름을 탄다.

### Codex 구성

engine은 Saturn 전용 `CODEX_HOME`으로 app-server를 시작한다(2026-10-02 확인, codex-cli 0.158.0). 사용자의 `~/.codex`는 쓰지 않는다.

- 전용 폴더는 `~/.saturn/codex-home/{규칙 지문}` 아래에 규칙 집합마다 하나를 둔다(초안). 규칙 지문은 모든 개별 규칙(도구, 값, 패턴)을 적은 글의 SHA-256 앞 16자다. 규칙이 같은 채팅은 같은 폴더를 쓰고 app-server 프로세스는 채팅마다 따로 둔다.
- 폴더는 0700, 안의 `config.toml`과 규칙 파일은 0600이다. 파일은 같은 폴더의 임시 파일에 쓴 뒤 이름을 바꿔 갈아 끼운다.
- 로그인은 원본 `~/.codex/auth.json`을 가리키는 심볼릭 링크로 공유한다. 인증 파일은 복사하지 않는다. 원본이 없으면(로그인 전) 링크를 만들지 않는다. 원본 폴더는 환경의 `CODEX_HOME`, 없으면 `HOME/.codex`다.
- `config.toml`은 사용자 `~/.codex/config.toml`에서 권한 관련 키를 뺀 뒤 Saturn이 매 실행마다 생성한다. 사용자 설정을 해석하지 못하면 번역 실패로 보고 첫 턴을 보내지 않는다.
- 뺄 키는 `approval_policy`, `sandbox_mode`, `sandbox_workspace_write.*`, `approvals_reviewer`, `hooks`, `hooks.state`, rules, `projects.*.trust_level`, `shell_environment_policy.*`, `cli_auth_credentials_store` 등이다. 구현은 `default_permissions`와 `permissions` 표도 빼고, 프로필(`profiles.*`)과 프로젝트(`projects.*`) 표 안의 같은 키도 뺀다(초안).
- 모델과 MCP 서버 같은 권한 외 설정은 사용자 설정을 그대로 옮기고 추적한다.
- 생성한 설정에 `approvals_reviewer="user"`를 명시한다. 자동 검토자가 Saturn 앞에서 판단하는 일을 막기 위해서다.
- Saturn의 개별 셸 규칙은 `rules/default.rules`의 `prefix_rule`로 번역한다. `allow`는 `allow`, `ask`는 `prompt`, `deny`는 `forbidden`이다. execpolicy는 접두사 일치만 하므로 `명령 *` 모양의 규칙만 `allow`로 옮기고, 글자 그대로 일치하는 `allow`와 `*`가 중간에 있거나 셸 문법이 든 패턴은 옮기지 않는다. 글자 그대로 일치하는 `ask`와 `deny`는 접두사 일치로 옮겨 더 엄하게 한다(초안). 옮기지 않은 규칙은 engine이 승인 요청에 답한다.
- 모드는 규칙 번역에 넣지 않는다. 모드를 바꿔도 provider를 다시 시작하지 않기 위해서다. 단 에이전트 질문 기능만 모드 `full`일 때 끄고, 이는 연결을 다시 시작해 적용한다([아래](#provider-설정과-질문-기능)).
- 채팅 중에 설정이 바뀌면(규칙 지문이나 에이전트 질문 설정이 연결을 시작할 때와 달라지면) 그 채팅의 app-server를 다시 시작한다(사용자 결정, [#289](https://github.com/woonyong-choi/saturn/issues/289)). 바뀐 것은 `/permissions`로 모드를 바꿀 때와 새 입력을 접수하며 설정을 다시 읽을 때 알아챈다. 설정 파일 변경을 알려 주는 감시는 두지 않는다. 채팅에 실행 중인 작업이 없으면 바로 다시 시작하고, 입력 접수에서 알아챘으면 그 입력을 보내기 전에 다시 시작해 새 설정으로 보낸다. 작업 중이면 지금 턴이 끝난 뒤 바로 다시 시작한다. 연결은 통째로 닫고, 열려 있던 session은 기록에 남겨 다음 입력이 새 설정의 `CODEX_HOME`으로 연결을 만들고 보관한 provider session id로 이어 연다. 바로 다시 시작하면 `다시 시작함 · 변경된 권한 설정을 적용했습니다` 안내만, 턴 끝으로 미뤘으면 미룬 것을 처음 알아챌 때 `다음 요청부터 적용됩니다` 안내와 다시 시작할 때 `다시 시작함` 안내를 대화 기록에 한 줄씩 남긴다. engine은 문구를 만들지 않고 알림 종류(`PermissionsChanged`, `ProviderRestarted`)만 보내며, 문구는 TUI가 시스템 언어로 고른다([TUI](tui.md#화면-언어와-출력-방식)).
- `thread/start`와 `thread/resume`에 `approvalPolicy="untrusted"`와 `sandbox="read-only"`(읽기 전용 샌드박스)를 준다. 파일 편집도 `item/fileChange/requestApproval`로 받기 위해서다. `untrusted`는 설정 키로는 쓸 수 없고 `thread/start` 인자로만 줄 수 있다.
- 자식 thread는 부모의 승인 정책과 규칙을 이어받아, 자식이 실행한 명령도 같은 규칙으로 승인 요청이 왔다(5/5 관측). Saturn은 자식 요청도 부모 에이전트의 요청으로 올려 같은 규칙으로 판정한다([#61](https://github.com/woonyong-choi/saturn/issues/61) 결정). 승인 요청에는 편집 경로가 없어 앞선 `item/started`의 `fileChange` 항목 경로를 기억해 쓴다.
- Codex 버전으로 열기를 막지 않는다. 버전은 `initialize` 응답의 `userAgent`를 로그에만 남긴다. 세션을 열 때 실제 적용된 승인 정책과 샌드박스를 확인하고, 기대와 다르면 첫 턴을 보내지 않는다. 정책은 `thread/start`와 `thread/resume` 응답의 `approvalPolicy`가 `untrusted`, `sandbox`가 읽기 전용, `approvalsReviewer`가 있으면 `user`인지 본다. 다르면 그 thread를 구독에서 빼고 `NotSent` 오류로 알린다.
- 규칙이 다른 채팅은 규칙마다 app-server 프로세스와 `CODEX_HOME`을 따로 둔다. app-server 하나가 thread별로 execpolicy 파일을 고르게 하는 인자가 없기 때문이다.

MCP 도구는 규칙을 다음처럼 번역한다.

| Saturn 규칙 | Codex MCP 설정 |
|---|---|
| `allow` | 서버 기본 `default_tools_approval_mode="approve"` 또는 도구별 `approval_mode="approve"` |
| `ask` | 도구별 `approval_mode="prompt"`. 승인 요청은 `mcpServer/elicitation/request`로 온다 |
| `deny` | `disabled_tools`. 목록에서 빠지고 직접 호출해도 오류가 난다 |

- 패턴으로 쓴 MCP 규칙은 준비 확인 때 받은 도구 목록에 적용해 도구별 값으로 펼치지 않고, 서버 기본값과 이름이 정해진 도구의 값으로만 옮긴다(초안). 사용자 설정의 서버마다 개별 규칙만으로 정한다: 서버 전체가 `allow`이고 겹치는 규칙이 모두 `allow`면 `default_tools_approval_mode="approve"`, 서버 전체가 `deny`이고 겹치는 `allow`가 없으면 `enabled=false`, 그 밖에는 `"prompt"`다. 규칙 패턴이 `mcp__{서버}__{도구}`로 이름을 정한 도구는 `allow`면 도구별 `approval_mode="approve"`, `ask`면 `"prompt"`, `deny`면 `disabled_tools`에 넣는다. `prompt`로 온 요청은 engine이 규칙으로 답한다. 모드를 `full`로 바꿔도 `prompt`는 그대로이고 engine이 허용으로 답한다.
- 첫 턴을 보내기 전에 대상 서버(사용자가 끄지 않았고 `enabled=false`로 옮기지 않은 서버)마다 `mcpServerStatus/list`를 호출해 시작이 끝났는지 확인한다. codex-cli 0.158.0은 `runtimeStatus`를 항상 `null`로 주고, 이 호출은 서버 시작이 모두 끝난 뒤에 응답한다(실측, [#317](https://github.com/woonyong-choi/saturn/issues/317)). 준비된 서버는 `serverInfo`가 차 있고 `toolsError`가 `null`이며, 시작에 실패한 서버(실행 파일 없음, 시작 시간 초과)는 `serverInfo`가 `null`이고 `toolsError`가 차 있다(빈 `tools`가 함께 온다). `runtimeStatus`가 문자열로 오는 버전은 그 값을 먼저 본다(`ready`, `failed`, `cancelled`, 그 밖은 기다림). 준비를 알 수 없는 서버(`serverInfo`와 `toolsError`가 모두 비었거나 목록에 없음)는 250 ms 간격으로 최대 30초(초안) 다시 묻는다. 시작에 실패한 서버와 30초 안에 준비를 알 수 없던 서버는 그 서버의 도구만 쓸 수 없는 것으로 보고 이유를 한 줄 로그로 남긴 채 첫 턴을 보낸다. 서버 하나가 시작에 실패했다고 모든 입력을 막지 않기 위해서다. 쓸 수 없는 서버의 도구를 모델이 부르면 승인 요청으로 오고 Saturn 규칙이 판정한다. 확인은 연결마다 한 번이다. 도구 목록이 예상과 같은지 이름까지 대조하는 일은 하지 않는다.
- `mcp_optional_startup_grace_ms`는 기본 1000 ms라 늦게 뜨는 서버의 도구가 첫 턴에서 빠질 수 있다. 예상 준비 시간만큼 늘린다. 실험은 12000 ms(초안)로 확인했고 생성한 설정에 그 값을 쓴다.
- Codex는 subagent 실행 자체를 승인 요청으로 올리지 않아 `permission.subagent`를 적용하지 못한다. subagent가 실행하는 명령, 편집, MCP 도구는 부모와 같은 규칙으로 판정한다(초안). `permission.subagent`는 Claude의 `Task`, `Agent` 도구에만 적용한다.
- Codex execpolicy와 MCP 설정은 app-server를 시작할 때 읽으므로, 채팅 도중 바뀐 개별 규칙은 engine이 답하는 요청에만 새 값이 쓰인다. 시작 때 정한 `forbidden`, `allow`, MCP `approve`는 새 설정 번호로 바뀌지 않는다. [Codex 실행 중 설정 다시 읽기 실측](../experiments/codex-live-reload/report.md)에서도 `rules/default.rules` 변경과 `config/batchWrite(reloadUserConfig=true)`는 같은 process에서 반영되지 않고 새 process에서만 반영됐다.
- `prompt`로 설정한 MCP 도구는 모델이 시도한 모든 호출에서 승인 요청이 왔다(실험 9에서 31/31, 고친 드라이버의 실험 10에서 10/10. [실험](../experiments/provider-permission-gating/report.md)). 도구를 시도하지 않고 끝난 회차는 요청이 없는 것이 맞다.
- 호스트가 직접 부르는 `mcpServer/tool/call`은 `prompt` 설정을 우회해 실행되므로 쓰지 않는다. 모든 MCP 승인은 모델 경로의 요청으로만 받는다.

### Claude 구성

engine은 Claude Code를 실행할 때 `--permission-prompt-tool stdio`와 `--settings '{"permissions":{"ask":[...]}}'`를 함께 준다(2026-10-02 확인, Claude Code 2.1.285).

- `ask` 목록에는 규칙 대상인 도구 이름을 모두 나열한다. 도구를 나열해야 요청이 오기 때문이다. 목록은 `Bash`, `Edit`, `MultiEdit`, `Write`, `NotebookEdit`, `Task`, `Agent`, `mcp__*`이다(초안). MCP는 서버를 알 수 없어 `mcp__*` 하나로 두고, 이 패턴이 실제로 통하는지는 실측한다.
- Saturn 기본 `--permission-mode`는 넣지 않는다. 이전 기본값 `acceptEdits`는 권한을 provider 모드에 맡기는 것이라 규칙과 어긋난다. 규칙 대상이 아닌 도구(`Read`, `WebFetch` 등)의 요청은 `call` 없이 올라와 사용자에게 묻는다.
- 요청의 도구를 `Bash`는 셸 명령(`input.command`), `Edit`, `MultiEdit`, `Write`는 편집(`input.file_path`), `NotebookEdit`는 편집(`input.notebook_path`), `Task`와 `Agent`는 subagent(`input.subagent_type`), `mcp__`로 시작하는 이름은 MCP 도구(도구 이름 그대로)로 읽는다.
- 사용자 설정이 `bypassPermissions`이고 폴더 허용 목록이 있어도 `Bash` 호출은 모두 `can_use_tool`로 왔다.
- 요청은 `control_request`의 `can_use_tool`로 오고, 답은 `control_response`로 보낸다. 허용은 `{"behavior":"allow","updatedInput":<요청 input>}`, 거부는 `{"behavior":"deny","message":"..."}`다.
- `--permission-prompt-tool` 없이 `ask`만 주면 요청이 호스트로 오지 않고 자동 거부된다.
- router 키 보호 훅의 실행별 설정은 같은 `--settings` 값에 합쳐 넘긴다(초안). 훅이 막는 호출은 규칙이 `allow`여도 막는 것이 설계다(초안, [router 키 보호](router-key-security.md)).

Claude는 `Bash`만 실측했다. `Edit`, `Write`, MCP 도구, subagent 도구가 같은 방식으로 오는지는 [#301](https://github.com/woonyong-choi/saturn/issues/301)에서 실측한다.

### provider 설정과 질문 기능

provider 설정은 추적만 하는 원칙([최소 provider 제어](../decisions/2026-09-29-minimal-provider-control.md))에는 예외가 하나 있다. 에이전트 질문 기능(Codex `default_mode_request_user_input`, Claude `AskUserQuestion`)은 사용자 provider 설정이 아니라 Saturn 설정이 정하고, 두 provider에 같게 적용한다(사용자 결정, [#284](https://github.com/woonyong-choi/saturn/issues/284)). 기본은 켜고, 권한 모드가 `full`이면 끈다. Codex의 실행 중 `experimentalFeature/enablement/set`은 codex-cli 0.158.0 실측에서 효과가 없어 쓰지 않는다. Codex는 생성 설정의 `[features]` 값이 달라지고, Claude는 `--disallowedTools AskUserQuestion` 인자가 달라지므로, 둘 다 규칙 변경과 같은 경로로 연결을 다시 시작해 적용한다. Claude의 권한 모드 변경 자체는 engine이 요청마다 판정하므로 다시 시작이 필요 없다. 적용 전에 온 질문은 사용자에게 보이고 engine은 대신 답하지 않는다. 자세한 동작은 [입력 요청](input-requests.md#에이전트-질문-설정)에 있다.

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
- engine은 허가 요청 이벤트를 기록한 뒤 TUI에 올리고 작업을 `허가 기다림`으로 보인다. engine은 요청마다 자체 고유 요청 번호를 발급해 TUI에 보내고, 안에서는 `(채팅, provider 연결, provider 요청 번호)`에 대응시켜 보관한다. 사용자 답은 TUI가 돌려준 engine 요청 번호로 찾은 요청의 원래 연결과 provider 요청 번호로 넘기고, 보낸 뒤 다른 TUI의 창을 지우고 작업을 다시 `실행 중`으로 보인다. 묻지 않은 요청의 답은 거절한다. 답한 TUI가 그 요청의 채팅에 붙어 있지 않아도 거절한다(`ChatNotAttached`). 허가 요청의 답은 그 요청을 보낸 에이전트에만 가야 하기 때문이다. provider가 받지 못했으면 요청을 그대로 두어 다시 답할 수 있다.
- 답이 오기 전에 턴이 끝나거나 흐름이 끊기면 그 요청의 창을 모든 TUI에서 지운다. provider 요청 번호는 채팅 연결마다 따로 매겨져 서로 겹칠 수 있으므로 TUI와 주고받는 번호로 쓰지 않는다. 같은 provider 요청 번호가 두 채팅에서 동시에 와도 요청은 따로 보관되고 답이 섞이지 않는다.
- 규칙으로 읽은 호출의 `항상 허용`은 engine이 기록 저장소에 저장하고 provider에는 `이번만 허용` 값으로 보낸다. 위 표의 `항상 허용` 열은 규칙으로 읽지 못한 요청에만 쓰인다. 그 요청에서 provider에 맞는 값이 없으면(Codex MCP, Claude) `이번만 허용`으로 보내고 같은 호출이 다시 오면 다시 묻는다.
- Codex에서 실측한 응답은 셸 명령의 `decline`과 MCP의 `decline`이다. 허용 응답과 권한 요청, 옛 이름의 값은 schema에서 가져온 것이라 실측하지 않았다([#301](https://github.com/woonyong-choi/saturn/issues/301)).
- 거부 응답에 붙는 고정 문구(`The user denied this tool call in Saturn.`)는 초안이다. 모델에 전달된다.
- `_meta.codex_approval_kind`가 없는 `mcpServer/elicitation/request`와 Claude `AskUserQuestion`은 승인이 아니라 사용자에게 묻는 입력 요청이라서 허가 요청이 아니다. 이 요청은 [입력 요청](input-requests.md)으로 올린다.

### 허가 대기 중 피드백

도구 호출이 시작되고 3초 안에 허가 요청이나 진행 이벤트가 오지 않으면 TUI는 계속 기다린다. 이때 상태판 실행 줄에 `도구 사용 허가 준비 중 · {provider}`(문구 초안)를 보인다. MCP 승인 요청이 도구 호출 시작 뒤 약 0~145초에 도착한 관측이 있고 원인은 확인하지 못했기 때문이다. 허가 요청이 오면 표시를 지우고 허가 요청 창을 띄운다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| 규칙을 provider 설정으로 번역하는 데 실패 | 첫 턴을 보내지 않고 입력을 오류 상태로 둔다. |
| Codex가 적용한 정책이 기대와 다름 | 첫 턴을 보내지 않고 오류를 보인다. |
| MCP 서버가 준비되지 않았거나 도구 목록이 예상과 다름 | 첫 턴을 보내지 않고 재시도하거나 오류 상태로 둔다. |
| TUI가 붙어 있지 않을 때 `ask` 요청 도착 | 요청을 보관하고 TUI가 붙으면 가장 먼저 보인다([engine 수명과 복구](engine-lifecycle.md)). |
| 이미 답했거나 모르는 요청에 답함 | provider에 보내지 않고 `NotSent`로 돌려준다. 쓰기 전에 실패하면 요청을 되돌려 다시 답할 수 있다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 규칙은 모드, 사용자, 폴더 순서로 이어 붙이고 마지막 일치가 이긴다. | `saturn-terminal/core/src/permission/tests.rs`의 `last_matching_rule_wins`, `rule_allow_overrides_mode_default_deny`, `saturn-terminal/engine/src/settings/layers.rs`의 `permission_rules_follow_layer_order_then_file_order` |
| 개별 `deny`가 하나라도 일치하면 거부한다. | `saturn-terminal/core/src/permission/tests.rs`의 `deny_is_sticky`, `deny_beats_always_allow`, `compound_command_takes_the_strictest_part` |
| 폴더 설정의 모드가 사용자 층 모드보다 높거나 같으면 무시하고 신뢰 창에 보인다. | `saturn-terminal/core/src/permission/tests.rs`의 `folder_mode_cannot_raise`, `saturn-terminal/engine/src/settings/layers.rs`의 `folder_permission_mode_cannot_raise_and_is_reported_as_ignored`, `saturn-terminal/engine/src/settings/manager.rs`의 `folder_permission_mode_that_does_not_lower_is_listed_as_ignored_in_the_trust_prompt` |
| 모드의 기본 규칙이 표대로 판정되고, 기본 모드는 `edit`다. | `saturn-terminal/core/src/permission/tests.rs`의 `mode_default_rules`, `mode_edit_treats_dotdot_escape_as_outside` |
| `edit`는 읽기 전용 셸 명령 목록을 허용하고, 셸 문법이 섞이면 묻고, 개별 규칙으로 덮을 수 있고 `deny`가 이기며, 다른 모드는 그대로다. | `saturn-terminal/core/src/permission/tests.rs`의 `mode_edit_allows_read_only_shell_commands_with_arguments`, `mode_edit_asks_when_shell_syntax_is_mixed_into_a_read_only_command`, `mode_edit_asks_when_a_read_only_command_gets_a_write_or_exec_option`, `mode_edit_asks_for_edits_under_git_internals_but_not_other_dot_files`, `read_only_shell_list_yields_to_individual_rules_and_deny_wins`, `read_only_shell_list_does_not_change_other_modes` |
| 더한 폴더 안의 편집은 `edit`에서 작업 폴더처럼 허용하고, 읽기 전용과 `deny`는 그대로다. | `saturn-terminal/core/src/permission/tests.rs`의 `mode_edit_allows_edits_inside_added_folders_like_the_workdir`, `added_folders_do_not_widen_read_only_or_deny_rules`, `saturn-terminal/engine/src/lifecycle/permissions.rs`의 `edit_inside_an_added_folder_is_allowed_like_the_workdir_in_edit_mode` |
| 에이전트 질문 기능은 Saturn 권한 모드가 정하고, `full`이면 두 provider에서 뺀다(둘 다 다시 시작으로 적용). | `saturn-terminal/engine/src/lifecycle/agent_questions.rs`의 테스트 전체([입력 요청](input-requests.md#요구사항) 표 참고) |
| 모드를 바꾸면 다음 허가 요청부터 새 모드로 판정하고 provider를 다시 시작하지 않는다. | `saturn-terminal/engine/src/lifecycle/permissions.rs`의 `mode_change_applies_next_request`, `saturn-terminal/tui/src/commands.rs`의 `parse_permissions_reads_one_known_mode` |
| 규칙의 `allow`와 `deny`는 사용자에게 묻지 않고 provider에 답하고, `ask`와 규칙으로 읽을 수 없는 요청만 TUI로 올린다. | `saturn-terminal/engine/src/lifecycle/permissions.rs`의 `rule_allow_answers_the_provider_without_asking_the_user`, `rule_deny_answers_the_provider_without_asking_the_user`, `rule_ask_goes_to_the_tui_and_waits_for_the_answer`, `request_without_a_readable_call_goes_to_the_tui_even_in_full_mode`, `rule_answer_that_the_provider_does_not_take_falls_back_to_the_user` |
| Codex 셸, 파일 편집, subagent 명령, MCP가 Saturn 규칙대로 허용, 묻기, 거부로 처리된다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `permission_shell`, `permission_edit`, `permission_subagent`, `permission_mcp`, 가짜 app-server |
| 사용자 Codex 규칙, 훅, 자동 검토자가 Saturn 판단에 끼어들지 않는다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `codex_home_ignores_user_rules`, `saturn-terminal/engine/src/providers/codex_home/tests.rs`의 `generated_config_has_no_permission_keys`, `home_links_login_without_copying_and_leaves_user_files_alone` |
| 규칙이 execpolicy와 MCP 설정으로 번역된다. | `saturn-terminal/engine/src/providers/codex_home/tests.rs`의 `shell_rules_translate_to_execpolicy_decisions`, `mcp_rules_translate_to_approval_modes_and_disabled_tools`, `mcp_allow_for_a_whole_server_is_approve_and_default_is_prompt` |
| MCP 준비를 확인하기 전에는 첫 턴을 보내지 않고, 시작에 실패했거나 준비를 알 수 없는 서버는 그 서버의 도구만 쓸 수 없는 것으로 보고 첫 턴을 보낸다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `first_turn_waits_for_mcp_ready`, `first_turn_is_sent_when_a_server_failed_to_start`, `first_turn_is_sent_when_a_server_stays_unknown_past_the_limit`, `saturn-terminal/engine/src/providers/codex_permission.rs`의 `null_runtime_status_with_server_info_is_ready`, `tools_error_is_unavailable_not_waiting`, `missing_or_empty_entries_are_waiting`, `string_runtime_status_is_read_first` |
| 버전과 상관없이 적용된 정책을 확인하고 다르면 첫 턴을 보내지 않는다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `startup_checks_applied_policy_whatever_the_version`, `saturn-terminal/engine/src/providers/codex_permission.rs`의 `applied_policy_must_be_untrusted_read_only` |
| Claude `can_use_tool`에 Saturn 규칙대로 `allow`, `deny`를 답한다. | `saturn-terminal/engine/src/providers/claude/tests.rs`의 `permission_rules`, `launch_args_add_defaults_and_hook_settings` |
| Claude 규칙 대상 도구의 호출이 모두 `can_use_tool`로 온다. | [#301](https://github.com/woonyong-choi/saturn/issues/301) 실측 |
| 허가 답이 provider에 요청 번호와 같은 번호로 나가고, 답이 없는 동안 턴이 멈춰 있다가 답한 뒤 이어진다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `command_approval_is_answered_with_the_same_numeric_request_id`, `command_decisions_follow_the_answer_and_the_available_list`, `mcp_tool_approval_is_answered_with_an_elicitation_action`, `saturn-terminal/engine/src/providers/claude/tests.rs`의 `allow_once_answers_can_use_tool_with_the_request_input`, `deny_answers_can_use_tool_without_the_note` |
| 두 채팅이 같은 provider 요청 번호로 허가 요청을 보내도 각각 보관되고 답이 섞이지 않으며, 다른 채팅에 붙은 TUI의 답은 거절한다. | `saturn-terminal/engine/src/lifecycle/permissions.rs`의 `same_provider_request_id_in_two_chats_keeps_both_and_answers_stay_apart`, `permission_answer_from_a_tui_attached_to_another_chat_is_refused` |
| 허가 요청이 TUI에 오르고 사용자 답이 provider에 넘어가며 턴이 끝나면 답 없는 요청이 지워진다. | `saturn-terminal/engine/src/lifecycle/events.rs`의 `permission_request_reaches_the_tui_and_the_answer_reaches_the_provider`, `answer_for_a_request_nobody_asked_is_refused`, `answer_the_provider_did_not_take_keeps_the_request_for_another_try`, `turn_end_withdraws_requests_nobody_answered` |
| 허가 창은 세 선택지이고, 도구 호출 뒤 3초 안에 허가 요청이나 진행 이벤트가 없으면 상태판에 준비 중을 보인다. | `saturn-terminal/tui/src/keys.rs`의 `permission_keys`, `saturn-terminal/tui/src/view/status_board.rs`의 `approval_pending_shows_after_three_seconds_without_events`, `approval_pending_clears_when_permission_request_or_progress_arrives` |
| 항상 허용은 기록 저장소에 저장되고 provider 설정 파일은 바뀌지 않는다. | `saturn-terminal/engine/src/lifecycle/permissions.rs`의 `always_allow_is_stored_in_records`, `always_allow_is_kept_per_workdir`, `always_allow_for_an_unreadable_request_goes_to_the_provider_as_given`, `saturn-terminal/engine/src/store/permissions.rs`의 `allows_are_kept_per_workdir_in_saved_order`, `saving_the_same_allow_twice_keeps_one_row` |
| 읽기 전용 모드의 입력은 읽기 전용 권한으로 접수한다. 쓰기를 열 수 있는 규칙(`allow`, `ask`)이 있으면 쓰기다. 승인한 쓰기도 쓰기 잠금을 얻은 실행만 한다. | `saturn-terminal/engine/src/lifecycle/permissions.rs`의 `read_only_mode_accepts_inputs_as_read_only`, `read_only_mode_with_an_allow_rule_keeps_inputs_as_write`, `read_only_mode_with_an_ask_rule_keeps_inputs_as_write`, `read_only_mode_with_only_a_deny_rule_keeps_inputs_as_read_only`, `write_approved_through_an_ask_rule_in_read_only_mode_holds_the_write_lock`, `chat_layer_mode_decides_the_input_permission` |
| 읽기 전용으로 접수한 실행은 모드를 올려도 읽기 전용으로 판정하고, 달라지면 `ReadOnlyRunKept`를 알린다. | `saturn-terminal/engine/src/lifecycle/permissions.rs`의 `running_read_only_task_keeps_its_acceptance_permission_after_a_mode_change` |
| 채팅 중 바뀐 설정(Codex 규칙·질문 설정, Claude 도구 목록)은 작업 중이 아니면 바로, 입력 접수에서 알아챘으면 그 입력을 보내기 전에, 작업 중이면 턴이 끝난 뒤 연결을 다시 시작해 적용한다. | `saturn-terminal/engine/src/lifecycle/live_settings.rs`의 테스트 전체, `saturn-terminal/engine/src/lifecycle/permissions.rs`의 `changed_codex_rules_of_a_running_chat_mark_the_connection_stale_until_they_match_again`, `stale_codex_connection_restarts_after_the_turn_ends_and_reopens_the_session`, `stale_codex_connection_waits_while_the_chat_is_running` |
| 바로 다시 시작하면 `ProviderRestarted`만, 턴 끝으로 미루면 미룬 것을 알아챌 때 `PermissionsChanged`를 한 번과 다시 시작할 때 `ProviderRestarted`를 문구 없이 알린다. | `saturn-terminal/engine/src/lifecycle/live_settings.rs`의 `live_settings_idle_codex_restarts_at_once_and_the_next_input_uses_the_new_settings`, `live_settings_running_codex_restarts_after_the_turn_and_tells_both_notices`, `live_settings_notice_is_not_repeated_while_the_restart_waits`, `saturn-terminal/engine/src/lifecycle/permissions.rs`의 `stale_codex_connection_restarts_after_the_turn_ends_and_reopens_the_session`, `saturn-terminal/tui/src/view/transcript.rs`의 `lines_permission_notices_follow_language` |
| 개별 규칙의 `deny`는 항상 허용보다 앞선다. | `saturn-terminal/engine/src/lifecycle/permissions.rs`의 `deny_rule_beats_a_stored_always_allow` |
| 스키마 V4 이관은 채팅 행을 보존하고 항상 허용 표를 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 `v3_file_migrates_to_permission_allows_keeping_chats` |

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

- Claude `Edit`, `Write`, MCP 도구, subagent 도구가 모두 `can_use_tool`로 오는지 ([#301](https://github.com/woonyong-choi/saturn/issues/301))
- Claude 사용자와 폴더의 `deny` 규칙과 훅이 Saturn 판정 앞에서 호출을 막는지 ([#301](https://github.com/woonyong-choi/saturn/issues/301))
- Codex 읽기 전용 샌드박스에서 일반 읽기, 빌드, 테스트 작업이 얼마나 막히는지 ([#301](https://github.com/woonyong-choi/saturn/issues/301))
- 키체인 로그인(`cli_auth_credentials_store=keyring`)에서 전용 `CODEX_HOME`이 로그인을 공유하는지 ([#301](https://github.com/woonyong-choi/saturn/issues/301))
- Codex가 승인된 편집과 명령을 읽기 전용 샌드박스에 막히지 않고 실행하는지 ([#301](https://github.com/woonyong-choi/saturn/issues/301))
- Codex subagent 실행 자체를 막을 방법. 지금은 승인 요청이 없어 안의 명령만 판정한다 ([#301](https://github.com/woonyong-choi/saturn/issues/301))
- 규칙으로 읽지 못한 요청의 `항상 허용`을 provider 값으로 보낼 방법: Claude 세션 규칙(`updatedPermissions`), Codex MCP의 `_meta.persist`, Codex 허용 응답과 권한 요청, 옛 이름 값의 실측 ([#301](https://github.com/woonyong-choi/saturn/issues/301))
- Codex MCP 승인 요청에서 도구 이름을 읽는 필드(`_meta`의 이름 키)가 실제로 있는지. 지금은 `_meta.tool_name`, `_meta.tool`, 요청 문구의 `tool "이름"` 순으로 읽는다 ([#301](https://github.com/woonyong-choi/saturn/issues/301))
- 허가 거절 뒤 다르게 하라는 입력을 어떻게 받을지 ([#56](https://github.com/woonyong-choi/saturn/issues/56))
