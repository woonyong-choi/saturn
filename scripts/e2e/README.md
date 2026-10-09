# 실제 provider 통합 확인

실제 Codex와 Claude Code 로그인 상태로 `saturn`의 수동 입력 경로(Codex 답, 양방향 전환, 종료와 다시 열기)를 확인하는 절차다. 가짜 provider 테스트가 보지 못하는 provider 응답 모양과 프로세스 수명을 본다.

## 준비

- 두 CLI(`codex`, `claude`)가 이미 로그인된 상태여야 한다(`claude auth status`는 모델을 부르지 않는다). 이 절차는 새 로그인을 하지 않고 `~/.codex`, `~/.claude`의 파일을 바꾸지 않는다.
- `cargo build --workspace --locked`로 `saturn`과 `saturn-engine`을 만든다.
- 연습 폴더를 저장소의 git 무시 폴더 아래에 만든다(예: `.local/e2e/practice`). 임시 폴더와 사본은 만들지 않는다.
- `tmux`로 TUI를 띄우고 단계마다 `tmux capture-pane -p`로 화면을 기록한다. 세션 이름은 `saturn-e2e-manual`.
- 모델 비용을 줄이려고 Codex는 `gpt-5.6-luna`, Claude는 `haiku`로 고정한다. 모델 호출은 Codex와 Claude 각각 열 번 안쪽이다. 기본 수동 모드에서는 router를 부르지 않는다.

### Saturn 홈

`saturn`과 `saturn-engine`은 환경 변수 `SATURN_HOME`이 있고 비어 있지 않으면 그 폴더를, 없으면 `~/.saturn`을 홈으로 쓴다(`saturn-engine`은 `--home`도 받는다). 실제 `~/.saturn`을 건드리지 않도록 실행마다 짧은 절대 경로의 전용 홈(예: `/tmp/saturn-e2e-<실행>`)을 지정하고 tmux 세션의 시작 환경에 넣는다. 깊은 저장소 경로는 `engine.sock`의 Unix 소켓 경로 길이 제한을 넘을 수 있다. 홈은 시작 전에 비어 있어야 한다. 화면 기록은 저장소의 git 무시 폴더에 보관한다. 끝난 뒤 실험이 만든 파일을 목록으로 남기고 지운다. 전용 `CODEX_HOME`(`$SATURN_HOME/codex-home/*`)의 `auth.json` 심볼릭 링크는 링크만 지운다.

### 수동 모드

기본 `router.mode = "manual"`은 Jev 키가 없어도 시작한다. 별도 키 파일이나 환경 변수는 넣지 않는다. `SATURN_HOME`을 시험 폴더로 지정하고, 화면에 `Router manual`이 표시되는지 확인한다.

```sh
E2E_RUN="$(pwd)/.local/e2e/manual-<실행>"
SATURN_E2E_HOME="/tmp/saturn-e2e-<실행>"
mkdir -p "$E2E_RUN/practice" "$SATURN_E2E_HOME"
tmux new-session -d -s saturn-e2e-manual -c "$E2E_RUN/practice" \
  -e "SATURN_HOME=$SATURN_E2E_HOME" \
  "zsh -c 'export PATH=<저장소 경로>/target/debug:\$PATH; exec zsh -f'"
```

`tmux` 서버가 이미 떠 있으면 시작 환경이 세션에 전달되지 않을 수 있다. 세션 안에서 `SATURN_HOME`이 `$SATURN_E2E_HOME`과 같은 절대 경로인지 확인하고 다르면 세션을 닫아 다시 시작한다.

## 절차와 기대 결과

| 단계 | 할 일 | 기대 결과 |
|---|---|---|
| a | 연습 폴더에서 `saturn`을 실행한다. | 상단에 버전과 폴더가 보이고 `Router manual`이 보이며 입력창이 열린다. |
| b | `/model codex`를 입력하고 `Enter`를 두 번 눌러(첫 `Enter`는 명령 완성) 모델 창에서 `gpt-5.6-luna`를 고른다. 이어 `현재 폴더에 notes.md 파일을 만들고 세 줄을 써 줘. 각 줄은 서로 다른 구체적인 한국어 문장(예: 장보기 품목, 오늘 할 일)으로 채워. 규칙: 모든 줄은 '- '로 시작한다. 다른 설명은 하지 마.`를 보낸다. | `Next inputs go to codex · gpt-5.6-luna`가 보이고, 입력이 `Applied`가 된다. |
| c | 허가 요청 창이 뜨면 이번만 허용한다(`y`). | 기본 권한 모드 `edit`는 작업 폴더 안 편집을 묻지 않고 허용하므로 이 단계에서는 창이 뜨지 않을 수 있다. 쉘 명령(`sed`, `tail` 같은 읽기)은 묻는다. 허가 요청 이벤트는 기록 저장소에 남는다. |
| d | 실행이 끝난 뒤 `notes.md`를 확인한다. | 파일이 있고 세 줄이며 모든 줄이 `- `로 시작하고 내용이 비지 않았다. 실행 줄이 `codex · <시간> · Token <수>`로 끝난다. |
| e | `/model claude`로 `haiku`를 고르고 `notes.md에 두 줄 더 추가해 줘. 다른 설명은 하지 마.`를 보낸다(규칙을 다시 말하지 않는다). | `switched codex → claude`가 보이고 `Applied`가 된다. |
| f | 실행이 끝난 뒤 `notes.md`를 확인한다. | 다섯 줄이고 새 두 줄도 `- `로 시작한다. 작업이 5분 안에 끝나 실행 줄이 `claude · <모델> · <시간>`으로 끝나고 Claude의 답이 채팅에 보인다. |
| g | `/usage`를 연다. | `claude`, `codex` 사용량이 보이고 `Router calls`는 0이다. |
| h | `Ctrl+D`로 TUI를 닫는다(`on_exit` 기본 `background`). | 실행 중인 작업이 없으면 바로 닫히고, 있으면 `Tasks still running: N · Reopen with saturn`이 남는다. |
| i | 같은 폴더에서 `saturn --continue`를 실행한다. | 같은 채팅이 열리고 두 입력과 답이 보인다. |
| j | `/model codex`로 `gpt-5.6-luna`를 다시 고르고 `notes.md 맨 끝에 한 줄만 더 추가해 줘. 다른 설명은 하지 마.`를 보낸다. 쉘 명령 허가가 뜨면 허용한다. | `switched claude → codex`가 보이고 `Applied`가 된다. 끝난 뒤 `notes.md`는 여섯 줄이고 새 줄이 `- `로 시작하며, 앞 입력의 작업이 다시 실행되지 않는다. |
| k | 기록 저장소 `$SATURN_HOME/saturn.db`를 읽기 전용으로 열어 표 `judgments`를 본다. | 판단 행이 0개이고 `/usage`의 `Router calls`도 0이다. |

단계 e, f, j는 이전 입력의 문장 형식과 수정 파일을 기억해 이어 쓰는지 확인한다. 수동 모드에서는 암묵적 제약을 자동 등록하지 않는다. 인계 패킷의 원문 보존은 별도 해시 검사와 실제 수신 화면으로 확인한다.

## 끝낸 뒤 정리

- 자기가 띄운 `saturn-engine`만 PID로 종료한다. 이름 패턴으로 일괄 종료하지 않는다.
- 자기가 만든 `saturn-e2e-manual` tmux 세션만 닫는다.
- 실행마다 지정한 `SATURN_HOME` 폴더에 만들어진 파일 목록을 남기고 지운다. 전용 `CODEX_HOME`(`$SATURN_HOME/codex-home/*`)의 `auth.json`은 심볼릭 링크이므로 링크만 먼저 지우고 `~/.codex/auth.json`이 그대로인지 확인한 뒤 폴더를 지운다.
- 종료한 engine의 자식(`codex app-server`, `claude`)이 남지 않았는지 `ps -eo pid,ppid,command`로 본다.
- 화면 원문은 저장소의 git 무시 폴더(`.local/e2e/`)에만 두고 공개하지 않는다. 토큰과 키는 기록하지 않는다.
