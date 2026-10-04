# 실제 provider 통합 확인

실제 Codex와 Claude Code 로그인 상태로 `saturn`의 첫 대화 흐름(Codex 답, 양방향 전환, 제약 유지, router 판단, 종료와 다시 열기)을 한 번에 확인하는 수동 절차다. 가짜 provider 테스트가 보지 못하는 provider 응답 모양과 프로세스 수명을 본다.

## 준비

- 두 CLI(`codex`, `claude`)가 이미 로그인된 상태여야 한다(`claude auth status`는 모델을 부르지 않는다). 이 절차는 새 로그인을 하지 않고 `~/.codex`, `~/.claude`의 파일을 바꾸지 않는다.
- `cargo build --release`로 `saturn`과 `saturn-engine`을 만든다.
- 연습 폴더를 저장소의 git 무시 폴더 아래에 만든다(예: `.local/e2e/practice`). 임시 폴더와 사본은 만들지 않는다.
- `tmux`로 TUI를 띄우고 단계마다 `tmux capture-pane -p`로 화면을 기록한다. 세션 이름은 `saturn-e2e-N`.
- 모델 비용을 줄이려고 Codex는 `gpt-5.6-luna`, Claude는 `haiku`로 고정한다. 모델 호출은 Codex와 Claude 각각 열 번 안쪽이다. router는 입력마다 한 번 부른다.

### Saturn 홈

`saturn`과 `saturn-engine`은 환경 변수 `SATURN_HOME`이 있고 비어 있지 않으면 그 폴더를, 없으면 `~/.saturn`을 홈으로 쓴다(`saturn-engine`은 `--home`도 받는다). 실제 `~/.saturn`을 건드리지 않도록 실행마다 `SATURN_HOME`을 저장소의 git 무시 폴더(예: `.local/e2e/<실행>/home`)로 지정하되 tmux를 시작하기 전에 절대 경로로 확정하고(상대 경로는 tmux의 작업 폴더 기준으로 달라진다), tmux 세션의 시작 환경에 넣는다. 그 폴더는 시작 전에 비어 있어야 한다. 끝난 뒤 실험이 만든 파일을 목록으로 남기고 지운다. 전용 `CODEX_HOME`(`$SATURN_HOME/codex-home/*`)의 `auth.json` 심볼릭 링크는 링크만 지운다.

### router 키

router 판단까지 실제로 쓰는 절차다. 키는 새로 만들지 않고, 사용이 승인된 기존 자격증명(사용자가 준 시험용 키나 승인한 `SATURN_KEY` 환경 변수)을 쓴다. 키체인 항목을 조회하는 명령은 이 절차에 넣지 않는다. 키는 tmux를 띄우는 셸의 환경 변수 `SATURN_KEY`로만 넘기고, 화면, 로그, 파일, 명령 인자에 남기지 않는다. 키체인에 새로 저장하지 않으며 `router.skip_check`나 가짜 router로 확인을 건너뛴 결과를 정상 router 검증으로 쓰지 않는다. Saturn은 시작할 때 `SATURN_KEY`를 저장된 키보다 먼저 읽는다.

```sh
E2E_RUN="$(pwd)/.local/e2e/<실행>"   # 저장소 루트에서 실행, 절대 경로로 확정
mkdir -p "$E2E_RUN/home" "$E2E_RUN/practice"
SATURN_KEY=<승인된 키를 이미 환경에 둔 셸에서> \
SATURN_HOME="$E2E_RUN/home" \
tmux new-session -d -s saturn-e2e-1 -c "$E2E_RUN/practice" \
  "zsh -c 'export PATH=<저장소 경로>/target/release:\$PATH; exec zsh -f'"
```

환경 변수는 tmux 서버가 이미 떠 있으면 세션에 전달되지 않을 수 있으므로, 세션 안에서 `env | grep -c '^SATURN_KEY='`로 키 값을 드러내지 않고 있는지만 확인하고, `tmux send-keys -t saturn-e2e-1 'printf %s "$SATURN_HOME"' Enter` 뒤 `tmux capture-pane -p`로 `SATURN_HOME`이 `$E2E_RUN/home`과 정확히 같은 절대 경로인지 확인한다. 다르면 세션을 닫고 새로 시작한다. 세션 안에서 `saturn`을 실행하면 engine이 `SATURN_KEY`로 router 확인을 통과하고 시작 화면에 `Router jev · <버전>`이 보인다.

## 절차와 기대 결과

| 단계 | 할 일 | 기대 결과 |
|---|---|---|
| a | 연습 폴더에서 `saturn`을 실행한다. | 상단에 버전과 폴더가 보이고 `Router jev · <버전>`이 보이며 입력창이 열린다. |
| b | `/model codex`를 입력하고 `Enter`를 두 번 눌러(첫 `Enter`는 명령 완성) 모델 창에서 `gpt-5.6-luna`를 고른다. 이어 `현재 폴더에 notes.md 파일을 만들고 세 줄을 써 줘. 각 줄은 서로 다른 구체적인 한국어 문장(예: 장보기 품목, 오늘 할 일)으로 채워. 규칙: 모든 줄은 '- '로 시작한다. 다른 설명은 하지 마.`를 보낸다. | `Next inputs go to codex · gpt-5.6-luna`가 보이고, 입력이 `Applied`가 된다. |
| c | 허가 요청 창이 뜨면 이번만 허용한다(`y`). | 기본 권한 모드 `edit`는 작업 폴더 안 편집을 묻지 않고 허용하므로 이 단계에서는 창이 뜨지 않을 수 있다. 쉘 명령(`sed`, `tail` 같은 읽기)은 묻는다. 허가 요청 이벤트는 기록 저장소에 남는다. |
| d | 실행이 끝난 뒤 `notes.md`를 확인한다. | 파일이 있고 세 줄이며 모든 줄이 `- `로 시작하고 내용이 비지 않았다. 실행 줄이 `codex · <시간> · Token <수>`로 끝난다. |
| e | `/model claude`로 `haiku`를 고르고 `notes.md에 두 줄 더 추가해 줘. 다른 설명은 하지 마.`를 보낸다(규칙을 다시 말하지 않는다). | `switched codex → claude`가 보이고 `Applied`가 된다. |
| f | 실행이 끝난 뒤 `notes.md`를 확인한다. | 다섯 줄이고 새 두 줄도 `- `로 시작한다. 작업이 5분 안에 끝나 실행 줄이 `claude · <모델> · <시간>`으로 끝나고 Claude의 답이 채팅에 보인다. |
| g | `/usage`를 연다. | `claude`, `codex`, `router` 줄이 보이고 세 줄의 토큰이 모두 0보다 크며 `Router calls`가 입력 수와 같다. |
| h | `Ctrl+D`로 TUI를 닫는다(`on_exit` 기본 `background`). | 실행 중인 작업이 없으면 바로 닫히고, 있으면 `Tasks still running: N · Reopen with saturn`이 남는다. |
| i | 같은 폴더에서 `saturn --continue`를 실행한다. | 같은 채팅이 열리고 두 입력과 답이 보인다. |
| j | `/model codex`로 `gpt-5.6-luna`를 다시 고르고 `notes.md 맨 끝에 한 줄만 더 추가해 줘. 다른 설명은 하지 마.`를 보낸다. 쉘 명령 허가가 뜨면 허용한다. | `switched claude → codex`가 보이고 `Applied`가 된다. 끝난 뒤 `notes.md`는 여섯 줄이고 새 줄이 `- `로 시작하며, 앞 입력의 작업이 다시 실행되지 않는다. |
| k | 기록 저장소 `$SATURN_HOME/saturn.db`를 읽기 전용으로 열어 표 `judgments`를 본다. | 입력마다 한 줄이 있고 `router`가 `jev`, `outcome`이 `Ok`이며 `answers`에 질문별 확률이 있다. `/usage`의 `router` 토큰과 맞는다. |

단계 e, f, j의 제약 유지는 파일 내용으로 본다. 다만 인계 패킷의 제약 구역은 아직 비어 있어(제약 식별과 저장은 들어갔지만 패킷에 넣는 연결은 [#380](https://github.com/woonyong-choi/saturn/issues/380)에서 만든다), 이 단계의 통과는 패킷의 제약 구역이 아니라 최근 입력과 목표 발췌에 남은 문장 덕일 수 있다. 제약 구역을 보려면 제약이 최근 입력에서 밀려나는 긴 대화가 필요하다.

## 끝낸 뒤 정리

- 자기가 띄운 `saturn-engine`만 PID로 종료한다. 이름 패턴으로 일괄 종료하지 않는다.
- 자기가 만든 tmux 세션만 닫는다.
- 실행마다 지정한 `SATURN_HOME` 폴더에 만들어진 파일 목록을 남기고 지운다. 전용 `CODEX_HOME`(`$SATURN_HOME/codex-home/*`)의 `auth.json`은 심볼릭 링크이므로 링크만 먼저 지우고 `~/.codex/auth.json`이 그대로인지 확인한 뒤 폴더를 지운다.
- 종료한 engine의 자식(`codex app-server`, `claude`)이 남지 않았는지 `ps -eo pid,ppid,command`로 본다.
- 화면 원문은 저장소의 git 무시 폴더(`.local/e2e/`)에만 두고 공개하지 않는다. 토큰과 키는 기록하지 않는다.
