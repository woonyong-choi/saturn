# 기능 목록과 확장

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [provider는 열린 id의 어댑터로 붙이고 확장은 Saturn 저장소에 설치해 session을 열 때 주입한다](../decisions/2026-10-04-open-providers-and-saturn-extensions.md) |

이 문서의 동작은 기능 목록을 TUI에 보내는 것 외에는 구현 전이다. 구현 범위는 [#412](https://github.com/woonyong-choi/saturn/issues/412)이고, 요구사항 표의 행은 같은 표시를 쓴다. provider 계층과 어댑터는 [provider 연결과 session](providers-and-sessions.md#provider-계층과-어댑터)에 있다.

## 요약

어댑터는 provider가 알리는 명령, 스킬, MCP 서버, 플러그인, 모드, 훅, 하위 에이전트를 기능 목록으로 engine에 올린다. engine은 이 목록을 이름과 종류와 함께 TUI의 `/`와 `$` 팝업과 권한 규칙에 올린다. 사용자가 확장을 설치해 달라고 하면 Saturn 확장 저장소에 원본을 두고, session을 열 때 어댑터가 provider 형식으로 주입한다. provider가 받을 수 없는 부분은 설치할 때와 전환할 때 대화 기록에 한 줄로 알린다.

## 동기

Codex와 Claude Code는 스킬, MCP 서버, 명령, 플러그인을 각자의 방식으로 설치한다. 사용자가 Claude에 설치한 스킬은 같은 채팅에서 Codex로 전환하면 보이지 않는다. 지금 Saturn은 provider 명령 목록을 TUI에 보내지 않고, 플러그인이나 확장이라는 개념이 없으며, 권한 규칙의 대상은 닫힌 다섯 종류다. 이 기능이 없으면 사용자는 provider마다 같은 확장을 따로 설치하고, 전환 뒤에 쓰던 도구가 사라진 이유를 알 수 없다.

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

engine은 어댑터가 올린 목록을 붙은 모든 TUI에 `Commands` 알림으로 보낸다. 지금은 TUI가 이 알림을 받는 코드는 있지만 engine이 보내는 코드가 없어 provider 명령이 팝업에 오르지 않는다(main 67a6dad 확인). 이 연결을 만드는 것이 구현 3단계다.

- `/` 팝업은 명령 종류 항목을 보이고 오른쪽에 알린 provider의 표시명을 출처로 붙인다. TUI 전용 명령과 Saturn session 명령이 대신하는 명령은 어댑터가 올리기 전에 뺀다([provider 명령과 스킬 전달](providers-and-sessions.md#provider-명령과-스킬-전달)).
- `$` 팝업은 스킬 종류 항목을 보인다. 메인 에이전트 provider의 스킬이 먼저 오르고 다른 provider의 스킬은 `$<provider id> 이름`으로 부른다([TUI](tui.md)).
- 이동성이 `전용`인 항목에는 팝업에서 provider 표시명을 붙여 어느 provider에서만 쓰는지 보이게 한다.
- 권한 규칙의 대상 종류는 지금의 다섯 가지(셸 명령, 파일 편집, 파일 읽기, MCP 도구, subagent 실행)에 어댑터가 올린 종류를 더한 열린 목록이다. 새 종류는 `permission.<종류>` 키로 규칙을 쓰고, 대상 이름은 항목의 이름이다. 형식은 [권한](permissions.md#권한-규칙)에 있다.

### 확장 저장소

확장 원본은 `~/.saturn/extensions/<이름>/` 폴더에 둔다. 원본 파일은 이 폴더에만 있고 provider 형식 파일은 여기에 만들지 않는다. 설치한 확장의 이름, 출처, 설치 시각, 부분별 판정은 기록 저장소의 `extensions` 표에 둔다([기록 저장과 보존](records.md#기록-저장소)).

- 쓰는 쪽은 engine 하나다. 기록 저장소와 같다.
- 확장 저장소는 `saturn prune`과 `/prune`의 정리 대상이 아니다. 채팅 기록이 아니라 사용자가 설치한 자원이기 때문이다.
- 설치 범위는 사용자 하나다. 폴더에 두는 확장은 [미해결 질문](#미해결-질문)에 있다.

### 설치

설치는 사용자가 설치해 달라고 말한 입력에서만 시작한다. engine이 스스로 provider 확장을 Saturn으로 옮기지 않는다. 입력에서 설치 요청을 알아보는 방식은 [미해결 질문](#미해결-질문)에 있다.

1. engine이 사용자가 가리킨 원본을 `~/.saturn/extensions/<이름>/`에 복사한다.
2. engine이 묶음이면 스킬, MCP 서버, 명령, 훅으로 나눈다.
3. engine이 부분마다 등록된 어댑터에 `주입 가능` 여부를 묻는다.
4. engine이 부분별 판정을 `extensions` 표에 저장한다.
5. engine이 판정을 대화 기록에 한 줄로 남긴다. 옮길 수 없는 부분이 있으면 부분 이름과 provider를 적는다.

판정은 provider마다 따로 한다. 어댑터가 `주입 가능`, `불가`, `모름` 셋 중 하나로 답하고, `모름`은 `불가`와 같게 알리되 어댑터를 고치면 다시 판정한다.

### 주입

session을 열 때 engine이 그 provider의 어댑터에 설치된 확장 중 주입 가능한 부분을 넘기고, 어댑터가 provider 형식으로 session에 넣는다. 사용자 provider 설정 파일은 고치지 않는다([결정 기록](../decisions/2026-10-02-saturn-permission-authority.md)). 어디에 무엇을 놓을지는 어댑터가 정한다.

- 지금 Saturn이 provider 실행에 쓰는 통로는 Codex의 채팅별 전용 `CODEX_HOME`과 Claude의 실행별 `--settings`다([권한](permissions.md)). 스킬, MCP 서버, 명령, 훅을 이 통로로 주입할 수 있는지는 어댑터 구현 때 실측한다. Codex의 훅은 전용 `CODEX_HOME`의 `hooks.json`과 `config.toml`의 `hooks.state` 신뢰 값으로 주입할 수 있었다([실측](../experiments/codex-provider-behavior/report.md)).
- 주입한 MCP 서버와 훅 스크립트도 provider 자식 프로세스이므로 router 키를 제외한 환경으로 실행한다([router 키 보호](router-key-security.md)).
- 주입한 MCP 도구 호출은 Saturn `permission.mcp` 규칙으로 판정한다. 확장이 권한 판정을 우회하지 않게 하기 위해서다.
- 설치와 제거는 다음 session을 열 때 적용한다. 열린 session의 구성은 바꾸지 않는다. 한 session 안에서 기능 목록이 바뀌는 일을 막기 위해서다.

### 옮길 수 없는 부분 알림

옮길 수 없는 부분은 두 시점에 대화 기록에 한 줄씩 남긴다.

| 시점 | 줄 |
|---|---|
| 설치 | `{확장} 설치 · {부분}은 {provider} 전용이라 {다른 provider}에서 쓰지 못함` |
| provider 전환 | `{확장}의 {부분}은 {새 provider}에 적용되지 않음` |

전환 줄은 provider가 바뀌는 전환에만 남기고, 같은 provider 안에서 모델만 바꾼 교체에는 남기지 않는다([모델 고르기](providers-and-sessions.md#모델-고르기)). 문구는 초안이다.

### 전용 기능이 필요한 작업

입력이 특정 provider 전용 기능을 필요로 하면 두 경로 중 하나를 따른다.

- 모델이 고정돼 있지 않으면 router가 필요한 기능을 가진 provider를 대상으로 고른다([router](router.md)).
- 모델이 고정돼 있거나 이어 가기가 전환을 필요로 하면 전환하기 전에 사용자에게 묻는다. 거절하면 현재 provider에서 기능 없이 진행한다.

필요한 기능이 무엇인지 입력에서 어떻게 알아내는지와 router 판단 질문에 어떻게 넣는지는 [미해결 질문](#미해결-질문)에 있다.

### provider에 직접 설치한 것

사용자가 provider에 직접 설치한 항목은 어댑터가 기능 목록에 `provider에 직접 설치`로 올린다. Saturn은 항목을 지우거나 바꾸지 않고 추적만 한다([결정 기록](../decisions/2026-09-29-minimal-provider-control.md)). 항목을 다른 provider로 옮길 수 있다고 어댑터가 판정하면 항목마다 한 번만 묻는다. 사용자가 수락하면 provider의 원본을 읽어 확장 저장소에 복사하고 [설치](#설치)와 같은 판정을 한다. 거절은 기록 저장소에 남겨 다시 묻지 않는다.

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
| 설치 요청이 원본을 `~/.saturn/extensions/`에 두고 부분별 판정을 저장하며 판정을 대화 기록에 한 줄로 남긴다. | 구현 전(#412). 부분마다 판정이 다른 묶음을 설치해 확인한다. |
| session을 열 때 어댑터가 주입 가능한 부분만 provider 형식으로 주입하고 사용자 provider 설정 파일은 바뀌지 않는다. | 구현 전(#412). 주입 전후 provider 설정 파일의 지문이 같은지, 실제 Codex와 Claude session에서 스킬과 MCP가 쓰이는지 확인한다. |
| 옮길 수 없는 부분을 설치 때와 provider 전환 때 대화 기록에 한 줄씩 알린다. | 구현 전(#412). 한쪽 전용 부분이 있는 확장으로 설치와 전환 줄을 확인한다. |
| provider에 직접 설치한 항목은 추적하고 옮길 수 있으면 항목마다 한 번만 묻는다. | 구현 전(#412). 거절한 항목이 다음 session에서 다시 묻지 않는지 확인한다. |
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
- 스킬, MCP 서버, 명령, 훅을 Codex의 전용 `CODEX_HOME`과 Claude의 `--settings`로 어디까지 주입할 수 있는지 실측하는 일 ([#412](https://github.com/woonyong-choi/saturn/issues/412))
- 입력이 전용 기능을 필요로 한다는 것을 router가 판단할지, 입력에 쓰인 이름(`$이름`, `/이름`)에서 읽을지 ([#412](https://github.com/woonyong-choi/saturn/issues/412))
- 설치 요청을 알아보는 방식. 입력을 router가 판단할지, 전용 명령을 둘지 ([#412](https://github.com/woonyong-choi/saturn/issues/412))
- 주입한 훅이 Saturn의 권한 판정에 끼어들 수 있는지와 막는 방법 ([#412](https://github.com/woonyong-choi/saturn/issues/412))
- 새 권한 종류의 모드별 기본 규칙. 지금 초안은 `full`이면 `allow`, 그 밖에는 규칙이 없으면 `ask`다 ([#412](https://github.com/woonyong-choi/saturn/issues/412))
