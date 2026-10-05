# 기능 목록과 확장

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [provider는 열린 id의 어댑터로 붙이고 확장은 Saturn 저장소에 설치해 session을 열 때 주입한다](../decisions/2026-10-04-open-providers-and-saturn-extensions.md) |

이 문서의 동작 중 기능 목록을 TUI에 보내는 것과 확장 저장소(설치, 제거, 목록, 부분 판정, 설치 알림)는 구현했고, 스킬과 명령과 MCP 서버 주입의 형식과 연결 구성(실제 provider에서의 동작은 실측 전)도 구현했다. provider 전환 알림도 구현했다. 훅 주입(Claude만)과 직접 설치 추적(읽기, 한 번만 묻기, 옮기기)도 구현했고 실제 provider에서의 동작은 실측 전이다. 권한 규칙의 새 종류는 구현 전이다. 구현 범위는 [#412](https://github.com/woonyong-choi/saturn/issues/412)이고, 요구사항 표의 행은 같은 표시를 쓴다. provider 계층과 어댑터는 [provider 연결과 session](providers-and-sessions.md#provider-계층과-어댑터)에 있다.

## 요약

어댑터는 provider가 알리는 명령, 스킬, MCP 서버, 플러그인, 모드, 훅, 하위 에이전트를 기능 목록으로 engine에 올린다. engine은 이 목록을 이름과 종류와 함께 TUI의 `/`와 `$` 팝업과 권한 규칙에 올린다. 사용자가 확장을 설치해 달라고 하면 Saturn 확장 저장소에 원본을 두고, session을 열 때 어댑터가 provider 형식으로 주입한다. provider가 받을 수 없는 부분은 설치할 때와 전환할 때 대화 기록에 한 줄로 알린다.

## 동기

Codex와 Claude Code는 스킬, MCP 서버, 명령, 플러그인을 각자의 방식으로 설치한다. 사용자가 Claude에 설치한 스킬은 같은 채팅에서 Codex로 전환하면 보이지 않는다. Saturn이 TUI에 보내는 목록은 명령과 스킬의 이름과 설명뿐이고, 플러그인, MCP 서버, 훅은 목록에 오르지 않으며, 권한 규칙의 대상은 닫힌 다섯 종류다. 이 기능이 없으면 사용자는 provider마다 같은 확장을 따로 설치하고, 전환 뒤에 쓰던 도구가 사라진 이유를 알 수 없다.

## 예시

### 스킬을 설치하고 두 provider에서 쓰기

1. 사용자가 입력창에 "이 스킬 설치해 줘"와 스킬 위치를 보낸다.
2. engine이 원본을 `~/.saturn/extensions/`에 복사하고 부분마다 provider별 사용 가능 여부를 판정한다.
3. 대화 기록에 `commit-helper 설치 · Claude, Codex에서 사용 가능`이 한 줄로 남는다.
4. 사용자가 Claude 메인 에이전트로 작업하면 session을 열 때 Claude 어댑터가 스킬을 Claude 형식으로 주입한다.
5. 사용자가 Codex로 전환하면 Codex 어댑터가 같은 원본을 Codex 형식으로 주입한다. `$` 팝업에는 두 session 모두에서 `commit-helper`가 보인다.

### 한쪽에서만 쓸 수 있는 부분이 있는 플러그인

1. 사용자가 스킬 하나, MCP 서버 하나, 훅 하나로 이루어진 플러그인을 설치해 달라고 한다.
2. engine이 플러그인을 세 부분으로 나눠 판정한다. 스킬과 MCP 서버는 두 provider에서 쓸 수 있고, 훅은 Claude에서만 쓸 수 있다.
3. 대화 기록에 `review-kit 설치 · 훅은 Claude 전용이라 Codex에서 쓰지 못함`이 남는다.
4. 사용자가 Claude에서 Codex로 전환하면 전환 안내 줄 바로 뒤에 `review-kit의 훅은 Codex에 적용되지 않음`이 한 줄 더 남는다.

### 훅이 필요한 작업이 Claude로 가기

1. 사용자가 Codex 메인 에이전트와 작업하다가 Claude 전용 훅이 필요한 작업을 보낸다.
2. 채팅에 고정 모델이 없으면 router가 필요한 기능을 가진 provider가 Claude 하나뿐임을 반영해 Claude를 고른다.
3. 채팅의 모델이 Codex로 고정돼 있으면 engine은 전환하기 전에 `이 작업에는 Claude 전용 기능이 필요함 · Claude로 전환할까요`를 사용자에게 묻는다.

### provider에 직접 설치한 것을 알아채기

1. 사용자가 Claude Code 안에서 직접 플러그인을 설치한다.
2. Claude 어댑터가 다음 session을 열 때 기능 목록에 새 항목을 `provider에 직접 설치`로 올린다.
3. engine은 Saturn 기록에 그 항목을 추적 대상으로 남긴다.
4. 항목을 Codex로도 옮길 수 있으면 `plugin-x를 Saturn에 설치해 Codex에서도 쓸까요`를 한 번 묻는다. 사용자가 거절하면 같은 항목을 다시 묻지 않는다.

## 상세 설계

### 기능 목록

어댑터는 session을 열 때와 기능이 바뀐 것을 알아챌 때 기능 목록을 engine에 올린다. 항목 하나는 아래 값을 가진다.

| 값 | 뜻 |
|---|---|
| 종류 | 아래 표의 일곱 종류 중 하나 |
| 이름 | provider 안에서 그 항목을 부르는 이름 |
| provider | 항목을 알린 어댑터의 id |
| 출처 | `provider 내장`, `Saturn 설치`, `provider에 직접 설치` 중 하나 |
| 이동성 | `옮길 수 있음` 또는 `전용` |

| 종류 | 예 |
|---|---|
| 명령 | `/review`, `/compact` |
| 스킬 | `$commit-helper` |
| MCP 서버 | 외부 도구 서버 |
| 플러그인 | 스킬, MCP 서버, 명령, 훅을 묶은 배포 단위 |
| 모드 | provider가 제공하는 계획 모드 같은 동작 방식 |
| 훅 | 도구 호출 전후에 실행하는 스크립트 |
| 하위 에이전트 | provider가 띄울 수 있는 subagent 종류 |

- `옮길 수 있음`은 다른 provider에 같은 효과로 주입할 수 있다는 뜻이다. `전용`은 그 provider 안에서만 의미가 있다는 뜻이다. provider 내장 항목은 모두 `전용`이다.
- 플러그인은 목록에 항목으로 오르지만 이동성은 부분마다 따로 판정한다. 부분 판정은 [설치](#설치)에 있다.
- 목록에 없는 종류를 어댑터가 올리면 engine은 그 항목을 `기능` 종류로 보이고 이름만 쓴다. 어댑터가 새 종류를 알려도 공통 코드를 고치지 않게 하기 위해서다.
- 어댑터가 목록을 주지 못하면 그 provider의 기능 목록은 비어 있는 것으로 보고 로그만 남긴다. 목록 실패가 session 열기를 막지 않게 하기 위해서다.

### TUI와 권한 규칙에 노출

engine은 어댑터가 올린 목록을 붙은 모든 TUI에 `Commands` 알림으로 보내고, 나중에 붙는 TUI에는 붙을 때 마지막 목록을 보낸다. 지금 알림의 항목은 이름, 설명, 스킬 여부만 가진다. 종류, 출처, 이동성 값은 구현 전이다.

- `/` 팝업은 명령 종류 항목을 보이고 오른쪽에 알린 provider의 표시명을 출처로 붙인다. TUI 전용 명령과 Saturn session 명령이 대신하는 명령은 어댑터가 올리기 전에 뺀다([provider 명령과 스킬 전달](providers-and-sessions.md#provider-명령과-스킬-전달)).
- `$` 팝업은 스킬 종류 항목을 보인다. 메인 에이전트 provider의 스킬이 먼저 오르고 다른 provider의 스킬은 `$<provider id> 이름`으로 부른다([TUI](tui.md)).
- 이동성이 `전용`인 항목에는 팝업에서 provider 표시명을 붙여 어느 provider에서만 쓰는지 보이게 한다.
- 권한 규칙의 대상 종류는 지금의 다섯 가지(셸 명령, 파일 편집, 파일 읽기, MCP 도구, subagent 실행)에 어댑터가 올린 종류를 더한 열린 목록이다. 새 종류는 `permission.<종류>` 키로 규칙을 쓰고, 대상 이름은 항목의 이름이다. 형식은 [권한](permissions.md#권한-규칙)에 있다.

### 확장 저장소

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/extension-store.ko.dark.svg">
  <img src="../assets/extension-store.ko.light.svg" alt="설치한 확장은 Saturn 확장 저장소에 원본으로 남고 session을 열 때 provider별로 주입된다" width="100%">
</picture>

확장 원본은 `~/.saturn/extensions/<이름>/` 폴더에 둔다. 원본 파일은 이 폴더에만 있고 provider 형식 파일은 여기에 만들지 않는다. 설치한 확장의 이름, 출처, 설치 시각, 부분별 판정은 기록 저장소의 `extensions` 표에 둔다([기록 저장과 보존](records.md#기록-저장소)).

- 쓰는 쪽은 engine 하나다. 기록 저장소와 같다.
- 확장 저장소는 `saturn prune`과 `/prune`의 정리 대상이 아니다. 채팅 기록이 아니라 사용자가 설치한 자원이기 때문이다.
- 설치 범위는 사용자 하나다. 폴더에 두는 확장은 [미해결 질문](#미해결-질문)에 있다.
- 확장 이름은 폴더 이름 한 칸으로 쓰므로 영문, 숫자, `-`, `_`, `.`만 쓰고 `.`으로 시작하지 않으며 64자까지다. 이름은 원천에서 얻는다. 폴더는 폴더 이름, git 주소는 `.git`을 뗀 저장소 이름이다.
- `extensions` 표의 부분별 판정은 JSON 글이고 부분마다 종류, 이름, 확장 폴더 안의 위치, provider별 판정을 가진다. 시작할 때 `모름`으로 남은 판정은 어댑터에 다시 묻고 바뀌었으면 저장한다.
- 설치와 제거, 목록은 TUI의 `/extensions`(`/extensions install <원천>`, `/extensions remove <이름>`)로 요청하고 `InstallExtension`, `RemoveExtension`, `ListExtensions`로 engine에 간다. 설치와 제거 결과는 요청한 채팅의 대화 기록에 한 줄로 남고, 목록은 `QueryResult::ExtensionList` 응답으로 요청한 TUI에만 간다. 어느 채팅에도 붙지 않은 접속은 요청하지 못한다.

### 설치

설치는 사용자가 설치해 달라고 말한 입력에서만 시작한다. engine이 스스로 provider 확장을 Saturn으로 옮기지 않는다. 입력에서 설치 요청을 알아보는 방식은 [미해결 질문](#미해결-질문)에 있다.

1. engine이 사용자가 가리킨 원본을 `~/.saturn/extensions/<이름>/`에 복사한다. 원천은 절대 경로 폴더나 git 저장소 주소(`https://`, `ssh://`, `file://`, `git@`로 시작)다. 폴더는 `.git`과 심볼릭 링크를 빼고 복사하고 64 MiB를 넘으면 거절하며, 확장 저장소를 품은 폴더도 거절한다. git 주소는 engine이 `git clone --depth 1`로 내려받고 `.git`을 지운다. 자식 프로세스에는 `PATH`, `HOME`, `SSH_AUTH_SOCK`만 주고 60초를 넘으면 실패로 본다. 네트워크는 engine만 쓴다. 내려받기는 요청 처리 루프를 막지 않도록 별도 작업으로 돌리고, 끝나면 결과를 루프에 돌려줘 나누기, 판정, 기록, 알림을 마무리한다. 내려받는 동안 다른 요청과 입력은 그대로 처리하고, 같은 이름의 설치는 거절한다.
2. engine이 묶음이면 스킬, MCP 서버, 명령, 훅으로 나눈다([묶음 나누기](#묶음-나누기)).
3. engine이 부분마다 등록된 어댑터에 `주입 가능` 여부를 묻는다.
4. engine이 부분별 판정을 `extensions` 표에 저장한다.
5. engine이 판정을 대화 기록에 한 줄로 남긴다. 옮길 수 없는 부분이 있으면 부분 이름과 provider를 적는다.

판정은 provider마다 따로 한다. 기본 답은 어댑터 설명자의 주입 형식(`extensions`)을 따른다. 형식이 그 종류를 받으면 `주입 가능`, 받지 않으면 `불가`이고, 형식만으로 답할 수 없는 어댑터는 판정 함수를 바꿔 `모름`을 돌려준다. 어댑터가 `주입 가능`, `불가`, `모름` 셋 중 하나로 답하고, `모름`은 `불가`와 같게 알리되 어댑터를 고치면 다시 판정한다.

### 묶음 나누기

설치한 확장 폴더를 아래 규칙으로 부분으로 나눈다(초안). 아무 부분도 없으면 설치하지 않는다.

| 부분 | 찾는 위치 | 이름 |
|---|---|---|
| 스킬 | 루트의 `SKILL.md`(확장 전체가 스킬 하나), `skills/<폴더>/SKILL.md` | 확장 이름, 폴더 이름 |
| 명령 | `commands/<이름>.md`(프롬프트 파일) | 파일 이름 |
| MCP 서버 | 루트의 `.mcp.json` 또는 `mcp.json`의 `mcpServers` 키 | 키 |
| 훅 | `hooks/hooks.json`의 `hooks` 키 | 키 |

- JSON 정의가 깨졌으면 설치하지 않고 파일 이름과 이유를 알린다.
- 부분 정의의 내용은 설치할 때 해석하지 않는다. 키와 파일 이름만 읽고, provider 형식으로 바꾸는 일은 어댑터가 주입할 때 한다.

### 주입

session을 열 때 engine이 그 provider의 어댑터에 설치된 확장 중 주입 가능한 부분을 넘기고, 어댑터가 provider 형식으로 session에 넣는다. 사용자 provider 설정 파일은 고치지 않는다([결정 기록](../decisions/2026-10-02-saturn-permission-authority.md)). 어디에 무엇을 놓을지는 어댑터가 정한다.

- engine은 연결을 시작할 때 설치한 확장 중 그 어댑터가 지금 `주입 가능`이라고 답한 부분만 어댑터에 넘긴다(기록에 저장한 판정이 아니라 지금의 답이다). 부분마다 확장 이름, 종류, 이름, 원본의 절대 경로를 준다. 어댑터는 원본을 읽기만 한다.
- 구현한 통로(초안, 실제 provider에서 쓰이는지는 실측 전이다).

| provider | 스킬 | 명령 | MCP 서버 | 훅 |
|---|---|---|---|---|
| Claude | Saturn이 만든 `~/.saturn/claude-extensions/<지문>/`을 플러그인 폴더(`.claude-plugin/plugin.json`, `skills/`)로 두고 `--plugin-dir`로 넘긴다 | 같은 플러그인 폴더의 `commands/` | 같은 폴더의 `mcp.json`을 `--mcp-config`로 넘긴다 | 같은 플러그인 폴더의 `hooks/hooks.json`. 이벤트마다 확장들의 정의를 이어 쓰고, 명령의 `${CLAUDE_PLUGIN_ROOT}`는 확장 원본 폴더로 바꾼다 |
| Codex | 전용 `CODEX_HOME`의 `skills/<이름>/` | 전용 `CODEX_HOME`의 `prompts/<이름>.md` | 전용 `CODEX_HOME`의 `config.toml` `[mcp_servers.<이름>]`. 정의의 `command`, `args`, `env`, `cwd`, `url`만 옮기고, 사용자 서버와 같은 규칙 번역을 받는다 | 주입하지 않는다. 훅은 `hooks/list`의 `currentHash`를 `config.toml`에 신뢰값으로 써야 불리는데 Saturn 훅도 아직 Codex에 넘기지 않아 판정 순서를 보장할 수 없다 |

- Codex 폴더 이름은 규칙 지문 뒤에 `-x<확장 지문>`을 붙인다. 확장 묶음마다 폴더가 따로이고, 규칙 지문 읽기에는 영향이 없다.
- 훅은 Claude만 주입하고 Codex는 주입하지 않으므로 훅 부분의 판정은 Claude `주입 가능`, Codex `불가`다. Codex로 전환하면 기존 전환 알림이 훅이 적용되지 않음을 알린다.
- Claude 훅은 Saturn 소유 PreToolUse 훅이 든 실행별 `--settings`에 섞지 않고 플러그인 폴더로 따로 넘긴다. Saturn 훅의 설정은 어떤 확장이 있어도 바뀌지 않고, Claude는 같은 이벤트의 훅을 모두 부르며 한 훅이라도 막으면 막으므로 확장 훅이 키 저장소 접근 거부를 풀지 못한다([router 키 보호](router-key-security.md#saturn-소유-pretooluse-훅)). 훅 정의가 이벤트별 목록이 아니면 그 부분만 주입하지 않고 한 줄 남긴다. 확장끼리 같은 이벤트 이름은 겹침으로 보지 않고 모두 이어 쓴다.
- 이름이 겹치는 부분은 먼저 놓은 것을 남기고 나중 것을 주입하지 않는다. 사용자 Codex 설정에 같은 이름의 MCP 서버가 있어도 같다. 주입하지 못한 부분과 provider와 이유를 대화 기록에 한 줄로 남긴다. 나머지 부분은 주입한 채 연결을 시작한다.
- 주입한 MCP 서버와 훅 스크립트도 provider 자식 프로세스이므로 router 키를 제외한 환경으로 실행한다([router 키 보호](router-key-security.md)).
- 주입한 MCP 도구 호출은 Saturn `permission.mcp` 규칙으로 판정한다. 확장이 권한 판정을 우회하지 않게 하기 위해서다.
- 설치와 제거는 다음 session을 열 때 적용한다. 열린 session의 구성은 바꾸지 않는다. 한 session 안에서 기능 목록이 바뀌는 일을 막기 위해서다. 구현은 연결을 시작할 때 쓴 확장 지문을 기억하고, 설치나 제거로 어떤 연결의 주입 부분이 바뀌면 그 연결만 권한 설정이 바뀔 때와 같은 방식으로 다시 시작한다. 채팅에 작업이 없으면 바로, 있으면 턴 끝에 시작하고, 열려 있던 session은 보관한 provider session id로 이어 연다. 주입할 부분이 바뀌지 않은 연결은 그대로 둔다.

### 새 provider에 확장 주입을 붙일 때

공통 동작은 trait의 기본 메서드와 공용 도우미가 하고, 어댑터는 provider 형식만 정한다. 어댑터가 하는 일은 아래뿐이다.

1. 설명자의 `extensions`(주입 형식)에 스킬 폴더 위치, 명령 파일 위치와 확장자, MCP 서버와 훅을 받는지를 적는다. 받지 않는 종류는 비워 둔다. `주입 가능` 판정의 기본 답과 engine이 넘기는 부분이 이 값을 따르므로 판정 코드는 쓰지 않는다.
2. 연결을 시작할 때 부르는 `translate_permission`의 입력에서 `extensions`(주입할 부분과 지문)를 받아 공용 도우미 두 개를 쓴다. `place_files`는 스킬과 명령을 주입 폴더에 놓고 이름이 겹치는 부분을 실패로 돌려주며, `collect_definitions`는 MCP 서버와 훅의 정의 JSON을 읽어 `Definition`으로 돌려준다.
3. 정의를 provider 형식으로 바꿔 쓰고(Claude는 `mcp.json`, Codex는 `config.toml`), 실행 인자나 환경에 주입 폴더를 가리키게 한다. 결과로 `PermissionLaunch`의 `extra_args`, `env`, `injection_failures`를 채운다.

판정 기본 구현, 원본 경로 계산, 지문, 연결 다시 시작, 실패 알림은 공통 코드가 한다. 어댑터가 `주입 가능` 판정을 바꾸는 것은 `Unknown`이 필요할 때만이다.

### 옮길 수 없는 부분 알림

옮길 수 없는 부분은 두 시점에 대화 기록에 한 줄씩 남긴다.

| 시점 | 줄 |
|---|---|
| 설치 | `{확장} 설치 · {부분}은 {provider} 전용이라 {다른 provider}에서 쓰지 못함` |
| provider 전환 | `{확장}의 {부분}은 {새 provider}에 적용되지 않음` |

전환 줄은 provider가 바뀌는 전환에만 남기고, 같은 provider 안에서 모델만 바꾼 교체에는 남기지 않는다([모델 고르기](providers-and-sessions.md#모델-고르기)). 문구는 초안이다.

- 전환 줄 대상은 바뀌기 전 provider에 주입했고 새 provider는 받지 못하는 부분이다. 처음부터 어느 쪽도 받지 못하던 부분은 설치할 때 알렸으므로 다시 알리지 않는다. 부분마다 한 줄이고, 대상이 없으면 줄을 더하지 않는다.
- 이 줄은 전환하는 순간에만 보내고 기록에는 저장하지 않는다. 채팅을 다시 열어 기록에서 되살리는 전환 줄에는 붙지 않는다. 당시의 설치 상태를 저장하지 않아 지금 상태로 다시 계산하면 사실과 달라질 수 있기 때문이다.

### 전용 기능이 필요한 작업

입력이 특정 provider 전용 기능을 필요로 하면 두 경로 중 하나를 따른다.

- 모델이 고정돼 있지 않으면 router가 필요한 기능을 가진 provider를 대상으로 고른다([router](router.md)).
- 모델이 고정돼 있거나 이어 가기가 전환을 필요로 하면 전환하기 전에 사용자에게 묻는다. 거절하면 현재 provider에서 기능 없이 진행한다.

필요한 기능이 무엇인지 입력에서 어떻게 알아내는지와 router 판단 질문에 어떻게 넣는지는 [미해결 질문](#미해결-질문)에 있다.

### provider에 직접 설치한 것

사용자가 provider에 직접 설치한 항목은 어댑터가 사용자 폴더를 읽기만 해서 올린다. Saturn은 항목을 지우거나 바꾸지 않고 추적만 한다([결정 기록](../decisions/2026-09-29-minimal-provider-control.md)). 운영 `~/.claude`, `~/.codex`는 읽기만 하고, 설정 파일의 내용과 토큰은 로그, 화면, 알림에 내지 않는다.

- 읽는 곳(초안). Claude는 `~/.claude/skills/<이름>/SKILL.md`, `~/.claude/commands/<이름>.md`, `~/.claude.json`의 `mcpServers` 키, `~/.claude/plugins/installed_plugins.json`의 플러그인 이름이다. Codex는 `CODEX_HOME`(없으면 `~/.codex`)의 `skills/<이름>/SKILL.md`(`.`으로 시작하는 내장 폴더 제외), `prompts/<이름>.md`, `config.toml`의 `[mcp_servers.<이름>]`이다. 홈은 채팅에 붙은 TUI가 넘긴 환경에서 찾는다. 한 provider에서 읽는 항목은 500개까지다.
- 연결을 시작할 때 새로 찾은 항목을 기록 저장소 `direct_installs`에 `asked`로 남기고, 그 채팅의 대화 기록에 `{provider}에 직접 설치된 항목 N개 추적 · ...`을 한 줄로 알린다. 다른 provider가 같은 종류를 주입할 수 있는 항목이 있으면 `/extensions move <provider> <이름>`으로 옮길 수 있다고 한 줄 더 알린다. 한 번 물은 항목은 다시 묻지 않고, 답하지 않으면 거절로 본다. 플러그인은 부분이 아니라 추적만 하고 옮기지 않는다.
- `/extensions` 목록은 설치한 확장 아래에 `provider에 직접 설치됨` 항목을 provider, 종류, 이름, `옮길 수 있음`, `옮김`, `추적만`과 함께 보인다. 목록은 요청한 접속이 붙은 채팅의 환경으로 읽는다.
- 옮기면 engine이 항목을 `~/.saturn/extensions/<이름>/`에 확장 하나로 복사한다(스킬은 폴더 그대로, 명령은 `commands/<이름>.md`, MCP 서버는 정의를 `.mcp.json`에 쓰고 소유자만 읽게 한다). 그다음 [설치](#설치)와 같은 부분 나누기와 판정과 알림을 하고 `moved`로 기록한다. 이름이 이미 설치돼 있거나 항목이 사라졌으면 옮기지 않고 이유를 남긴다. provider의 원본은 그대로 둔다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| 설치 원본을 읽지 못하거나 이름이 이미 있는 경우 | 설치하지 않고 이유를 대화 기록에 남긴다. 기존 설치는 바꾸지 않는다. |
| 어댑터 하나가 부분 주입에 실패 | 그 provider의 그 부분만 빼고 session을 연다. 대화 기록에 한 줄 남긴다. |
| 어댑터가 기능 목록을 주지 못하는 경우 | 목록을 비워 두고 로그만 남긴다. session 열기는 막지 않는다. |
| 설치한 확장의 원본이 저장소에서 사라짐 | 주입하지 않고 대화 기록에 한 줄 남긴다. `extensions` 표의 행은 지우지 않는다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 어댑터가 올린 기능 목록이 `Commands` 알림으로 TUI에 가고 `/`와 `$` 팝업에 보인다. | `saturn-terminal/engine/src/lifecycle/commands.rs`의 `a_list_the_connection_reports_later_reaches_the_attached_tui`, `an_unchanged_list_is_not_sent_again`, `a_changed_list_replaces_the_earlier_one`, `a_tui_that_attaches_later_gets_the_latest_list`, `saturn-terminal/tui/src/app/tests.rs`의 `commands_notification_fills_the_slash_and_dollar_popups`. 실제 Codex와 Claude의 목록이 오르는지는 구현 전(#412)이고 실제 provider로 확인한다 |
| 목록 항목은 종류, 이름, provider, 출처, 이동성을 가지고, 모르는 종류는 `기능` 종류로 표시한다. | 구현 전(#412). 모르는 종류를 올리는 가짜 어댑터로 확인한다. |
| 권한 규칙이 어댑터가 올린 종류를 대상으로 쓸 수 있고, 규칙이 없으면 모드 기본을 따른다. | 구현 전(#412). 새 종류 항목에 `allow`, `ask`, `deny` 규칙을 걸어 판정을 확인한다. |
| 설치 요청이 원본을 `~/.saturn/extensions/`에 두고 부분별 판정을 저장하며 판정을 대화 기록에 한 줄로 남긴다. | `saturn-terminal/engine/src/lifecycle/extensions.rs`의 `a_folder_is_copied_to_the_store_and_each_part_is_judged_per_provider`, `a_folder_with_a_skill_file_at_its_root_is_one_skill_named_after_the_folder`, `a_git_address_is_cloned_by_the_engine_without_the_git_folder`, `a_slow_clone_does_not_hold_up_other_requests_and_is_finished_when_it_ends`, `the_same_name_cannot_be_installed_again_while_it_is_being_downloaded`, `saturn-terminal/tui/src/view/extensions.rs`의 `install_line_lists_the_providers_that_can_use_every_part`, `install_line_names_only_the_parts_that_cannot_move` |
| 설치할 수 없는 원본은 이유를 대화 기록에 남기고 기존 설치와 저장소를 바꾸지 않는다. 제거는 원본과 행을 지우고 저장소 밖 경로를 건드리지 않는다. | `saturn-terminal/engine/src/lifecycle/extensions.rs`의 `an_install_that_cannot_proceed_says_why_and_keeps_the_earlier_install`, `a_failed_clone_leaves_nothing_behind`, `a_source_that_contains_the_store_is_refused`, `a_folder_named_like_a_hidden_file_is_refused`, `remove_deletes_the_original_and_the_row_and_refuses_names_outside_the_store`, `remove_clears_the_row_even_when_the_original_is_already_gone` |
| `모름` 판정은 시작할 때 어댑터에 다시 묻는다. | `saturn-terminal/engine/src/lifecycle/extensions.rs`의 `only_unknown_judgments_are_asked_again_at_start` |
| `/extensions`가 목록, 설치, 제거를 요청하고 목록을 보인다. | `saturn-terminal/tui/src/commands.rs`의 `parse_extensions_reads_list_install_and_remove`, `saturn-terminal/tui/src/app/tests.rs`의 `extensions_install_sends_a_folder_as_an_absolute_path_and_a_git_address_as_it_is`, `saturn-terminal/engine/src/lifecycle/extensions.rs`의 `list_answers_the_requesting_client_in_install_order`, `saturn-terminal/tui/src/view/extensions.rs`의 `list_shows_each_part_with_a_verdict_per_provider` |
| session을 열 때 어댑터가 주입 가능한 부분만 provider 형식으로 주입하고 사용자 provider 설정 파일은 바뀌지 않는다. | `saturn-terminal/engine/src/lifecycle/extension_inject.rs`의 `an_adapter_receives_only_the_parts_it_can_inject`, `without_installed_extensions_the_adapter_gets_nothing`, `saturn-terminal/engine/src/providers/claude/extensions.rs`의 `skills_and_commands_go_to_a_plugin_folder_and_servers_to_a_config_file`, `nothing_is_made_without_parts`, `saturn-terminal/engine/src/providers/claude/tests.rs`의 `extension_arguments_come_after_the_defaults_and_before_the_settings`, `saturn-terminal/engine/src/providers/codex/home/tests.rs`의 `extension_parts::skills_and_commands_are_copied_and_servers_follow_the_permission_rules`, `extension_parts::the_user_codex_folder_is_left_as_it_was`. 실제 Codex와 Claude session에서 스킬과 MCP가 쓰이는지는 실측 전이다 |
| 이름이 겹치거나 정의가 깨진 부분은 그 부분만 주입하지 않고, 원본이 사라진 확장은 주입하지 않으며, 둘 다 대화 기록에 한 줄 남긴다. | `saturn-terminal/engine/src/lifecycle/extension_inject.rs`의 `a_missing_original_is_told_and_the_other_extensions_are_still_injected`, `a_part_the_adapter_could_not_inject_is_told_with_its_provider`, `saturn-terminal/engine/src/providers/claude/extensions.rs`의 `a_name_used_twice_keeps_the_first_and_fails_the_second_and_hooks_are_refused`, `a_server_missing_from_its_definition_file_fails_alone`, `a_server_name_used_by_two_extensions_keeps_the_first`, `saturn-terminal/tui/src/view/extensions.rs`의 `inject_failure_line_names_the_provider_and_the_part` |
| 설치와 제거는 주입 부분이 바뀐 연결만 다시 시작한다. | `saturn-terminal/engine/src/lifecycle/extension_inject.rs`의 `an_install_restarts_only_the_connections_whose_parts_changed`, `a_remove_restarts_the_connection_that_had_the_parts`, `an_extension_with_nothing_for_a_provider_leaves_its_connection_alone` |
| Codex 전용 폴더에 확장 부분이 들어가고 폴더 이름이 확장 지문을 담는다. | `saturn-terminal/engine/src/providers/codex/home/tests.rs`의 `extension_parts::the_folder_name_carries_the_extension_fingerprint_after_the_rules_fingerprint` |
| Claude는 확장의 훅을 Saturn 훅이 든 `--settings`와 따로 플러그인 폴더로 주입하고, Codex는 훅을 받지 않는다. | `saturn-terminal/engine/src/providers/claude/extensions.rs`의 `hooks_of_every_extension_go_to_the_plugin_hook_file_and_not_to_the_saturn_settings`, `a_hook_that_is_not_a_list_fails_alone`, `saturn-terminal/engine/src/lifecycle/extension_inject.rs`의 `an_extension_with_nothing_for_a_provider_leaves_its_connection_alone`. 실제 Claude에서 확장 훅이 불리고 Saturn 훅이 먼저 거부하는지는 실측 전이다 |
| 옮길 수 없는 부분을 설치 때와 provider 전환 때 대화 기록에 한 줄씩 알린다. | 설치 줄은 위 설치 행의 시험이다. 전환 줄은 `saturn-terminal/engine/src/lifecycle/extension_switch.rs`의 `a_switch_tells_only_the_parts_the_new_provider_loses`, `saturn-terminal/tui/src/view/extensions.rs`의 `switch_lines_name_each_part_the_new_provider_does_not_take` |
| provider에 직접 설치한 항목은 읽기만 해 추적하고 옮길 수 있으면 항목마다 한 번만 묻는다. 옮기면 확장 저장소에 복사하고 provider 폴더는 그대로다. | `saturn-terminal/engine/src/lifecycle/extensions.rs`의 `items_installed_directly_in_a_provider_are_listed_and_asked_about_once`, `moving_a_direct_item_copies_it_to_the_store_and_leaves_the_provider_folder_alone`, `saturn-terminal/engine/src/providers/claude/direct.rs`의 `skills_commands_servers_and_plugins_are_read_from_a_fake_home`, `nothing_is_found_without_a_home_or_when_the_files_are_broken`, `saturn-terminal/engine/src/providers/codex/direct.rs`의 `skills_prompts_and_servers_are_read_and_builtin_skills_are_skipped`, `the_codex_home_variable_wins_over_the_home_folder`, `saturn-terminal/tui/src/view/extensions.rs`의 `direct_items_are_listed_with_their_state_and_asked_about_with_the_move_command`. 시험은 테스트가 만든 가짜 홈으로만 한다. 실제 `~/.claude`와 `~/.codex`를 읽는 확인은 실측 전이다 |
| 전용 기능이 필요한 작업은 router가 그 provider를 고르거나 전환 전에 사용자에게 묻는다. | 구현 전(#412). 고정 모델과 고정 없음 두 경우로 확인한다. |

## 단점

- 확장 원본과 provider 형식 사이의 번역을 어댑터마다 구현하고, provider의 확장 형식이 바뀌면 따라가야 한다.
- 설치한 확장이 provider마다 다르게 동작할 수 있다. 같은 스킬이라도 provider가 읽는 방식이 다르기 때문이다.
- 사용자가 provider에 직접 설치한 항목과 Saturn에 설치한 항목이 두 곳에 있어 어디에 있는지 헷갈릴 수 있다. 출처 표시로 구분한다.

## 대안

- provider에만 설치하는 방식은 전환하면 확장이 사라져 버렸다([결정 기록](../decisions/2026-10-04-open-providers-and-saturn-extensions.md)).
- 설치할 때 사용자가 Saturn과 provider 중 고르게 하는 방식은 매번 묻고 확장의 위치가 두 곳으로 갈라져 버렸다([결정 기록](../decisions/2026-10-04-open-providers-and-saturn-extensions.md)).

## 미해결 질문

- 폴더에 둔 확장(저장소의 `.saturn/`)을 폴더 설정 신뢰 창과 어떻게 묶을지, 아니면 사용자 범위만 허용할지 ([#412](https://github.com/woonyong-choi/saturn/issues/412))
- 위 표의 통로(Claude의 `--plugin-dir`과 `--mcp-config`, Codex의 전용 `CODEX_HOME`의 `skills/`, `prompts/`, `[mcp_servers]`)로 실제 session에서 스킬, 명령, MCP 서버가 쓰이는지, 주입한 MCP 도구 이름이 `permission.mcp` 규칙 패턴(`mcp__서버__도구`)과 맞는지 실측하는 일 ([#412](https://github.com/woonyong-choi/saturn/issues/412))
- 입력이 전용 기능을 필요로 한다는 것을 router가 판단할지, 입력에 쓰인 이름(`$이름`, `/이름`)에서 읽을지 ([#412](https://github.com/woonyong-choi/saturn/issues/412))
- 자연어 입력에서 설치 요청을 알아보는 방식. 입력을 router가 판단할지 정하지 못했다. 전용 명령 `/extensions install`은 구현했다 ([#412](https://github.com/woonyong-choi/saturn/issues/412))
- 주입한 확장 훅이 실제 Claude에서 불리는지, 같은 이벤트의 Saturn 훅 거부가 확장 훅의 `allow`보다 우선하는지, Codex에 훅을 신뢰값과 함께 넣을 방법 ([#412](https://github.com/woonyong-choi/saturn/issues/412))
- 새 권한 종류의 모드별 기본 규칙. 지금 초안은 `full`이면 `allow`, 그 밖에는 규칙이 없으면 `ask`다 ([#412](https://github.com/woonyong-choi/saturn/issues/412))
