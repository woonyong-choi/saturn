<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/assets/logo-dark.png">
    <img src="docs/assets/logo-light.png" alt="Saturn logo" width="160">
  </picture>
</p>

<h1 align="center">Saturn</h1>

<p align="center">
  Codex와 Claude Code를 하나의 대화로 이어 쓰는 터미널 도구입니다.
</p>

<p align="center">
  <a href="README.md">English</a> | 한국어<br>
  <a href="#설치">설치</a> · <a href="#사용법">사용법</a> · <a href="#상태">상태</a> · <a href="#문서">문서</a>
</p>

Codex와 Claude Code를 함께 쓰는 개발자는 provider마다 session과 압축 방식이 달라서 도구를 바꿀 때마다 맥락을 잃습니다. Saturn은 모든 입력을 보내기 전에 로컬에 기록하고, 그 기록에서 각 provider session에 필요한 맥락만 골라 넘깁니다. 도구마다 터미널을 따로 띄우는 방식과 달리, provider를 바꾸거나 작업을 병렬로 돌리거나 새 session을 열어도 한 채팅의 기록과 작업 상태가 이어집니다.

> [!NOTE]
> 개발 중입니다. 배포판은 없고 소스에서 빌드해 실행합니다.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/overview.ko.dark.svg">
  <img src="docs/assets/overview.ko.light.svg" alt="설계: 모든 입력이 Saturn 기록에 먼저 접수되고 새 provider session은 그 기록에서 만든 packet만 받습니다" width="100%">
</picture>

## 작동 방식

1, 2, 4, 5, 6단계는 `main`에서 동작합니다. 3단계에서 패킷으로 새 session을 여는 것까지는 되지만, 패킷에 무엇을 담을지를 router가 고르지 않고 provider에 압축을 요청하지도 않습니다. 사용자가 정한 제약은 기록하지만 제약 해제와 패킷에 제약을 싣는 것은 설계만 되어 있습니다(상태 참고).

1. 저장소에서 `saturn`을 실행하고 요청을 입력합니다. 뒤에서 도는 engine 프로세스가 Codex나 Claude Code에 보내기 전에 입력을 로컬 SQLite 데이터베이스에 저장합니다.
2. 에이전트가 일하는 동안 이어지는 요청을 입력합니다. 입력에 대한 예·아니요, 선택형, 등급형 질문에 답하는 작은 모델인 router가 진행 중인 턴에 더할지, 별도 작업으로 시작할지, 대기열에 둘지 정합니다.
3. session의 맥락이 정해 둔 토큰 기준을 넘고 실행 중인 작업이 없으면, Saturn은 provider에 압축을 맡기거나 비용이 더 적을 때 새 session을 엽니다. 새 session은 Saturn 기록에서 고른 목표, 최근 턴, 끝나지 않은 항목을 패킷으로 받습니다.
4. 채팅을 Claude Code에서 Codex로 바꿉니다. 채팅은 사용자가 보는 대화이고, provider session은 그 뒤에서 열리고 닫힙니다. 새 session은 그 채팅을 마지막으로 본 뒤 바뀐 내용만 받습니다.
5. 터미널을 닫습니다. engine은 이미 보낸 입력을 계속 처리하고, 나중에 다시 붙을 수 있습니다.
6. provider가 명령 실행이나 파일 수정을 요청합니다. Saturn은 provider 설정 대신 자체 권한 규칙을 적용하고, 규칙이 묻기로 정한 경우에만 허가 창을 보입니다.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/provider-switch.ko.dark.svg">
  <img src="docs/assets/provider-switch.ko.light.svg" alt="채팅을 Claude Code에서 Codex로 바꾸면 새 Codex session이 Saturn 기록으로 만든 패킷을 받고, 두 결과가 한 채팅에 남습니다" width="100%">
</picture>

provider 전환: 새 session이 기록으로 만든 패킷을 받습니다. [provider 연결과 session](docs/design/providers-and-sessions.md)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/permission-decision.ko.dark.svg">
  <img src="docs/assets/permission-decision.ko.light.svg" alt="허가 요청은 Saturn 규칙이 allow나 deny로 답하고 ask일 때만 TUI에 올라옵니다" width="100%">
</picture>

권한: 규칙이 답하고 ask일 때만 묻습니다. [권한](docs/design/permissions.md)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/constraint-lifecycle.ko.dark.svg">
  <img src="docs/assets/constraint-lifecycle.ko.light.svg" alt="제약은 자동 등록, 묻고 등록, 등록 안 함으로 갈립니다" width="100%">
</picture>

제약: 앞으로도 지키라고 한 말입니다. [제약](docs/design/constraints.md)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/extension-store.ko.dark.svg">
  <img src="docs/assets/extension-store.ko.light.svg" alt="확장은 Saturn 저장소에 복사되어 provider별로 판정된 뒤 provider 형식으로 주입됩니다" width="100%">
</picture>

확장: 저장소 하나에서 provider별로 주입합니다. [기능 목록과 확장](docs/design/extensions.md)

전체 설계는 [설계 문서](docs/README.md)에 있고, 설계 문서는 한국어로 씁니다.

## 설치

Saturn은 Apple Silicon의 macOS에서 실행합니다. CI가 빌드에 쓰는 Rust 1.95.0이 필요합니다(2026-10-04 확인). 로그인을 마친 Codex CLI(`codex`), Claude Code(`claude`) 중 하나 이상도 필요합니다. Saturn은 대신 로그인하지 않으므로 각 CLI에 먼저 로그인하세요. 로그인 상태는 `codex login status`, `claude auth status`로 확인합니다.

`saturn`이 `saturn-engine`을 시작하므로 두 실행 파일을 함께 설치하세요.

```sh
cargo install --locked --git https://github.com/woonyong-choi/saturn saturn-cli saturn-engine
```

Cargo는 두 실행 파일을 bin 폴더(기본 `~/.cargo/bin`)에 넣습니다. 이 폴더가 `PATH`에 있는지 확인하세요. `saturn`은 `saturn-engine`을 자기 폴더에서 먼저 찾고, 없으면 `PATH`에서 찾습니다.

클론에서 직접 빌드하려면 저장소 루트에서 `cargo build --release`를 실행하세요. `target/release/saturn`과 `target/release/saturn-engine`이 만들어집니다. 두 파일은 같은 폴더에 두세요.

## 사용법

> [!NOTE]
> 아래 단계는 실제 Codex와 Claude Code로 요청 전송과 `/model` 전환까지 확인했습니다(상태 참고).

### saturn 시작

저장소에서 `saturn`을 실행하세요. engine을 뒤에서 시작하고 전체 화면 채팅을 엽니다. 처음 시작하면 `~/.saturn`이 만들어집니다.

```sh
saturn
```

router 키가 없으면 Router key 창이 숨김 입력으로 키를 묻습니다. `Enter`는 확정하고 `Esc`는 `saturn`을 끝냅니다. `saturn`이 끝나도 engine은 계속 실행됩니다.

### router 키 넣기

router는 API 키가 필요합니다. Saturn은 engine을 시작할 때 아래 방법을 순서대로 시도하고, 처음 성공한 방법에서 멈춥니다.

1. engine을 시작하는 셸의 `SATURN_KEY` 환경 변수. 설정돼 있으면 저장된 키보다 먼저 읽습니다.
2. 저장된 키. macOS에서는 계정 이름이 `saturn-key`인 키체인 항목이고, Router key 창에 키를 입력하면 Saturn이 만듭니다. 다른 시스템에서는 Saturn 홈의 `router.key` 파일(권한 0600)입니다.
3. `router.key.command` 설정의 명령. `~/.saturn/config.toml`의 `[router.key]` 아래에 `command = ["op", "read", "<항목>"]`처럼 정합니다. Saturn은 셸 없이 명령을 실행하고 출력의 첫 줄을 키로 읽습니다.

`SATURN_KEY`가 설정돼 있는데 router가 거절하면 저장된 키로 넘어가지 않고 명령, Router key 창 순서로 갑니다. Saturn은 명령줄 인자나 표준 입력으로는 키를 받지 않습니다. 이미 실행 중인 engine은 `SATURN_KEY`를 다시 읽지 않습니다.

```sh
export SATURN_KEY=<your key>
saturn
```

### 화면 없이 실행

파이프나 CI처럼 화면이 없으면 Saturn은 키를 물을 수 없습니다. 키를 정하는 방법을 출력하고 종료 코드 77로 끝납니다.

```sh
saturn usage
```

```text
Error: Router key required (router key required: router rejected the key): set the SATURN_KEY environment variable or the router.key.command setting, then run again
```

### 종료 코드

| 코드 | 뜻 |
|---|---|
| 0 | 성공 |
| 1 | 그 밖의 실패. 확인 질문에 아니라고 답한 경우와 plain 모드의 작업 실패 포함 |
| 2 | 사용법 오류. 호출을 바꿔야 풀림. 예: 선택 창에 필요한 터미널 없음, 출입증 없는 에이전트 안 중첩 실행 |
| 66 | 대상 없음. 이어 열 채팅 없음, 없는 폴더, 없는 router 버전 |
| 69 | engine을 쓸 수 없음. 시작 실패, 응답 없음, 교체 실패, 연결 끊김 |
| 70 | engine 내부 오류 |
| 75 | 지금은 안 되고 나중에 가능. 예: 학습 표본 부족 |
| 77 | router 키 없음이나 확인 실패 |
| 78 | 설정 오류 |
| 130 | 창에서 `Esc`나 `Ctrl+C`로 중단 |

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/exit-codes.ko.dark.svg">
  <img src="docs/assets/exit-codes.ko.light.svg" alt="종료 코드는 호출을 바꿀 것, 다시 시도할 것, 설정을 고칠 것, 그 밖의 실패, 정상과 중단으로 갈립니다" width="100%">
</picture>

### 단순 화면 쓰기

`saturn --plain`은 박스와 움직임 없이 줄마다 말한 쪽을 적고 선택지를 번호 목록으로 보여 줍니다. `NO_COLOR` 환경 변수나 `tui.screen` 설정도 같은 효과이고, 채팅 중에는 `/plain`으로 바꿉니다. `--plain=false`는 둘보다 앞섭니다. 터미널이 없으면 `saturn`이 알아서 단순 화면을 씁니다.

```sh
saturn --plain
```

### 모델 바꾸기

입력창에 `/model`을 입력하면 다음 입력부터 쓸 모델을 고를 수 있습니다. `/model codex`나 `/model claude`는 그 provider의 모델만 보여 줍니다. 목록에는 설치된 provider가 나옵니다.

```text
/model codex
```

`Enter`는 명령 완성을 받아들이고, 한 번 더 누르면 명령을 실행합니다. 방향키로 움직이고 `Enter`로 고르며 `Esc`로 취소합니다. 고른 모델은 다시 고를 때까지 그 채팅의 이후 모든 입력에 적용됩니다.

### 채팅 이어 열기

같은 폴더에서 `saturn --continue`를 실행하면 그 폴더에서 가장 최근 채팅을 엽니다. `saturn --resume`은 그 폴더의 채팅 목록에서 골라 엽니다.

```sh
saturn --continue
```

## 상태

Saturn은 개발 중입니다. 메시지 타입, core 규칙, engine, TUI, `saturn` 명령은 `main`에 있습니다. 가짜 provider를 쓴 테스트에서 동작하는 것은 입력 접수부터 provider 전송까지의 입력 흐름, 멈춤과 재개, Saturn 권한 규칙, `/model`, `/usage`, 파이프와 `NO_COLOR`를 위한 단순 화면 방식, TUI를 닫아도 작업 계속 실행, engine 크래시 뒤 복구, 확장을 Saturn 저장소에 설치하고 연결을 시작할 때 주입하는 것, 그리고 Saturn 안의 에이전트나 바깥의 Claude Code, Codex가 떠 있는 engine에 일을 부탁하는 하위 접속입니다. `scripts/e2e/README.md`의 확인 절차(단계 a~k)는 2026-10-04에 실제 Codex, Claude Code, router로 통과했고, 나머지는 실제 provider로 실행하지 않았습니다. 전환해 돌아올 때 오래된 사용자 제약을 인계 패킷에 담는 것은 만들지 않았습니다. 지금 패킷에는 제약이 실리지 않습니다([#296](https://github.com/woonyong-choi/saturn/issues/296) 완료 조건 2). 설계만 된 것은 router 답으로 패킷 순서 정하기, 인계 compact([#380](https://github.com/woonyong-choi/saturn/issues/380)), 제약 해제와 `/constraints` 화면([#379](https://github.com/woonyong-choi/saturn/issues/379), [#381](https://github.com/woonyong-choi/saturn/issues/381)), 로컬 router 모델의 채점과 학습, 서버와 데이터 공유입니다. 설계 문서, 결정 기록, 실험 보고서는 공개되어 있습니다. Apple Silicon macOS를 대상으로 하고 Codex CLI나 Claude Code가 필요합니다. 1.0 전까지 명령, 파일 형식, 동작이 예고 없이 바뀔 수 있습니다. 열린 설계 질문과 실험 계획은 [GitHub 이슈](https://github.com/woonyong-choi/saturn/issues)에 있고, 의견은 이슈 댓글로 받습니다.

## 비교

- Codex CLI나 Claude Code 단독 사용: 각자 session과 압축을 관리하고 별도 계층이 필요 없습니다. provider 하나만 쓴다면 그 도구를 바로 쓰는 편이 낫습니다.
- 두 도구를 터미널 두 개에서 실행: 병렬로 돌릴 수 있지만 맥락은 도구마다 따로이고, 도구 사이 맥락은 사람이 직접 옮깁니다. Saturn은 기록 하나를 두고 필요한 맥락을 provider 사이에 넘깁니다.

## 로드맵

첫 대화 이후의 순서는 아직 정하지 않았습니다.

1. 첫 대화: 실제 Codex와 Claude Code로 처음부터 끝까지 실행, 사용자 제약을 둔 뒤 다시 전환. (진행 중)
2. 제약과 패킷: 제약 해제와 예외, 인계 패킷의 제약, router 답으로 정하는 패킷 순서. (다음)
3. 채팅 관리: 채팅 이름과 묶음, 작업 완료 알림. (다음)
4. 로컬 router 모델: 판단 기록 채점, 개인 router 모델 학습, 같은 평가 세트에서 현재 router보다 나쁘지 않을 때만 교체. (나중)
5. 서비스: 동의 기반 데이터 수집, 원격 API, 인증, 인프라. (나중)

## 문서

설계 문서는 한국어로 씁니다.

- [아키텍처](docs/architecture.md): 코드 지도와 불변 조건
- [입력 처리](docs/design/input-handling.md): 입력 접수, 대기, 보류와 재개
- [provider 연결과 session](docs/design/providers-and-sessions.md): provider 연결, session 전환, subagent 추적
- [맥락 정리](docs/design/context-management.md): 맥락 측정과 새 session으로 이어 가기
- [결정 기록](docs/decisions/README.md): 설계를 정한 이유
- [전체 문서](docs/README.md)

## 개발

저장소 루트에서 다음 명령을 실행하세요. CI가 모든 PR에서 `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`와 `saturn-protocol/generated` 최신 여부를 검사합니다.

```sh
cargo build --workspace
cargo test --workspace
```

## 라이선스

[MIT](LICENSE)
