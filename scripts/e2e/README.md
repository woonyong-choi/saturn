# 실제 provider 통합 확인

실제 Codex와 Claude Code 로그인 상태로 `saturn`의 첫 대화 흐름(Codex 답, 양방향 전환, 제약 유지, router 판단, 종료와 다시 열기)을 한 번에 확인하는 수동 절차다. 가짜 provider 테스트가 보지 못하는 provider 응답 모양과 프로세스 수명을 본다.

## 준비

- 두 CLI(`codex`, `claude`)가 이미 로그인된 상태여야 한다(`claude auth status`는 모델을 부르지 않는다). 이 절차는 새 로그인을 하지 않고 `~/.codex`, `~/.claude`의 파일을 바꾸지 않는다.
- `cargo build --release`로 `saturn`과 `saturn-engine`을 만든다.
- 연습 폴더를 저장소의 git 무시 폴더 아래에 만든다(예: `.local/e2e/practice`). 임시 폴더와 사본은 만들지 않는다.
- `tmux`로 TUI를 띄우고 단계마다 `tmux capture-pane -p`로 화면을 기록한다. 세션 이름은 `saturn-e2e-N`.
- 모델 비용을 줄이려고 Codex는 `gpt-5.6-luna`, Claude는 `haiku`로 고정한다. 모델 호출은 Codex와 Claude 각각 열 번 안쪽이다. router는 입력마다 한 번 부른다.

### Saturn 홈

`saturn`은 `~/.saturn`만 본다(`--home`은 `saturn-engine`에만 있다). 시작 전에 `~/.saturn`이 있는지 기록하고, 없었다면 끝난 뒤 실험이 만든 파일을 목록으로 남기고 지운다. 전용 `CODEX_HOME`의 `auth.json` 심볼릭 링크는 링크만 지운다.

### router 키

router 판단까지 실제로 쓰는 절차다. 키는 키체인 서비스 `saturn-experiments-judge`에 있는 것을 쓰고, 프로세스 환경으로만 넘긴다. 키를 화면, 로그, 파일에 남기지 않고 키체인에 새로 저장하지 않으며 `router.skip_check`로 확인을 건너뛰지 않는다. tmux 세션의 시작 명령 안에서 키를 읽어 인자로 드러나지 않게 한다.

```sh
tmux new-session -d -s saturn-e2e-1 -c .local/e2e/<실행>/practice \
  "zsh -c 'export SATURN_KEY=\$(security find-generic-password -s saturn-experiments-judge -w); export PATH=<저장소 경로>/target/release:\$PATH; exec zsh -f'"
```

세션 안에서 `saturn`을 실행하면 engine이 `SATURN_KEY`로 router 확인을 통과하고 시작 화면에 `Router jev · <버전>`이 보인다.

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
| k | 기록 저장소 `~/.saturn/saturn.db`를 읽기 전용으로 열어 표 `judgments`를 본다. | 입력마다 한 줄이 있고 `router`가 `jev`, `outcome`이 `Ok`이며 `answers`에 질문별 확률이 있다. `/usage`의 `router` 토큰과 맞는다. |

단계 e, f, j의 제약 유지는 파일 내용으로 본다. 인계 패킷에 사용자 제약을 담는 기능은 [#297](https://github.com/woonyong-choi/saturn/issues/297)에서 만들었다.

## 끝낸 뒤 정리

- 자기가 띄운 `saturn-engine`만 PID로 종료한다. 이름 패턴으로 일괄 종료하지 않는다.
- 자기가 만든 tmux 세션만 닫는다.
- `~/.saturn`이 시작 전에 없었다면 끝난 뒤 만든 파일 목록을 남기고 지운다. 전용 `CODEX_HOME`(`~/.saturn/codex-home/*`)의 `auth.json`은 심볼릭 링크이므로 링크만 먼저 지우고 `~/.codex/auth.json`이 그대로인지 확인한 뒤 폴더를 지운다.
- 종료한 engine의 자식(`codex app-server`, `claude`)이 남지 않았는지 `ps -eo pid,ppid,command`로 본다.
- 화면 원문은 저장소의 git 무시 폴더(`.local/e2e/`)에만 두고 공개하지 않는다. 토큰과 키는 기록하지 않는다.
