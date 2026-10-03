# 실제 provider 통합 확인

실제 Codex와 Claude Code 로그인 상태로 `saturn`의 첫 대화 흐름(Codex 답, Claude 전환, 제약 유지, 종료와 다시 열기)을 한 번에 확인하는 수동 절차다. 가짜 provider 테스트가 보지 못하는 provider 응답 모양과 프로세스 수명을 본다.

## 준비

- 두 CLI(`codex`, `claude`)가 이미 로그인된 상태여야 한다. 이 절차는 새 로그인을 하지 않고 `~/.codex`, `~/.claude`의 파일을 바꾸지 않는다.
- `cargo build --release`로 `saturn`과 `saturn-engine`을 만든다.
- 연습 폴더를 저장소의 git 무시 폴더 아래에 만든다(예: `.local/e2e/practice`). 임시 폴더와 사본은 만들지 않는다.
- `tmux`로 TUI를 띄우고 단계마다 `tmux capture-pane -p`로 화면을 기록한다. 세션 이름은 `saturn-e2e-N`.
- 모델 비용을 줄이려고 Codex는 `gpt-5.6-luna`, Claude는 `haiku`로 고정한다. 모델 호출은 Codex와 Claude 각각 열 번 안쪽이다.

### Saturn 홈

`saturn`은 `~/.saturn`만 본다(`--home`은 `saturn-engine`에만 있다). 시작 전에 `~/.saturn`이 있는지 기록하고, 없었다면 끝난 뒤 실험이 만든 파일을 목록으로 남기고 지운다. 전용 `CODEX_HOME`의 `auth.json` 심볼릭 링크는 링크만 지운다.

### router 키 없이 실행

router 키(`SATURN_KEY`, 키체인)는 쓰지 않는다. 키가 없으면 engine은 시작 확인에 실패해 TUI에 키를 묻는다. 확인을 건너뛰도록 engine을 직접 띄운 뒤 `saturn`을 실행한다.

```sh
saturn-engine --home ~/.saturn -c router.skip_check=true
```

이때 router 판단은 모두 실패하고(`/usage`의 Router calls, 화면의 `Auto judgment paused`) 입력은 현재 에이전트와 현재 모델로 간다. 판단 방식의 동작은 이 절차에서 보지 않는다.

사용자 Codex 설정에 MCP 서버가 있으면 [#317](https://github.com/woonyong-choi/saturn/issues/317)이 닫히기 전까지 첫 입력이 거절된다. 그동안은 engine을 `-c 'permission.mcp="deny"'`로 더 띄워 우회한다.

## 절차와 기대 결과

| 단계 | 할 일 | 기대 결과 |
|---|---|---|
| a | 연습 폴더에서 `saturn`을 실행한다. | 상단에 버전과 폴더가 보이고 입력창이 열린다. |
| b | `/model codex`를 입력하고 `Enter`를 두 번 눌러(첫 `Enter`는 명령 완성) 모델 창에서 `gpt-5.6-luna`를 고른다. 이어 `현재 폴더에 notes.md 파일을 만들고 세 줄을 써 줘. 규칙: 모든 줄은 '- '로 시작한다. 다른 설명은 하지 마.`를 보낸다. | `Next inputs go to codex · gpt-5.6-luna`가 보이고, 입력이 `Delivering`에서 `Applied`로 바뀐다. |
| c | 허가 요청 창이 뜨면 이번만 허용한다. | 기본 권한 모드 `edit`는 작업 폴더 안 편집을 묻지 않고 허용하므로 창이 뜨지 않을 수 있다. 이 경우 기록 저장소에 허가 요청 이벤트가 남는다. |
| d | 실행이 끝난 뒤 `notes.md`를 확인한다. | 파일이 있고 세 줄이며 모든 줄이 `- `로 시작한다. 실행 줄이 `codex · <시간> · Token <수>`로 끝난다. |
| e | `/model claude`로 `haiku`를 고르고 `notes.md에 두 줄 더 추가해 줘. 다른 설명은 하지 마.`를 보낸다(규칙을 다시 말하지 않는다). | `switched codex → claude`가 보이고 `Applied`가 된다. |
| f | 실행이 끝난 뒤 `notes.md`를 확인한다. | 다섯 줄이고 새 두 줄도 `- `로 시작한다. 실행 줄이 `claude · <모델> · <시간>`으로 끝나고 Claude의 답이 채팅에 보인다. |
| g | `/usage`를 연다. | `claude`, `codex`, `router` 줄이 보이고 두 provider의 토큰이 0보다 크다. |
| h | `Ctrl+D`로 TUI를 닫는다(`on_exit` 기본 `background`). | 실행 중인 작업이 없으면 바로 닫히고, 있으면 `Tasks still running: N · Reopen with saturn`이 남는다. |
| i | 같은 폴더에서 `saturn --continue`를 실행한다. | 같은 채팅과 두 입력이 열린다. 아직 지원하지 않으면 [#319](https://github.com/woonyong-choi/saturn/issues/319)의 오류가 나오고 `saturn --resume <채팅 id>`로 같은 채팅을 연다. |

단계 e와 f의 기대 결과 중 제약 유지는 파일 내용으로만 본다. 인계 패킷에 사용자 제약을 담는 기능은 [#297](https://github.com/woonyong-choi/saturn/issues/297)에서 만든다.

## 끝낸 뒤 정리

- 자기가 띄운 `saturn-engine`만 PID로 종료한다. 이름 패턴으로 일괄 종료하지 않는다.
- 자기가 만든 tmux 세션만 닫는다.
- 종료한 engine의 자식(`codex app-server`, `claude`)이 남지 않았는지 `ps -eo pid,ppid,command`로 본다.
- 화면 원문은 저장소의 git 무시 폴더(`.local/e2e/`)에만 두고 공개하지 않는다. 토큰과 키는 기록하지 않는다.
