# Saturn 이슈 #194 실험: Saturn 전용 CODEX_HOME + Saturn 규칙을 Codex 실행 규칙으로 번역

저장소: `~/workspace/oss/saturn` (GitHub `woonyong-choi/saturn`). 먼저 `gh issue view 194 --comments`로 앞 실험 네 개를 읽어라(app-server 실행법, `thread/start`의 `approvalPolicy`, 승인 요청 메시지 형식이 있다).

## 목표
Saturn이 Codex를 띄울 때 `CODEX_HOME`을 Saturn 전용 폴더로 주고, 그 폴더에 Saturn 권한 규칙을 Codex 실행 규칙(execpolicy, `allow`/`prompt`/`forbidden`)으로 써 두면, 사용자 허용 규칙과 상관없이 Saturn 규칙대로만 Codex가 명령을 실행·질문·거부하는지 확인한다.

## 확인할 것 (각 조건 3회 반복. 3회 모두 같아야 "확인", 다르면 "불안정", 못 하면 "확인 못 함". 추정으로 채우지 마라)
1. 로그인 공유: 전용 폴더에서 사용자 Codex 로그인을 쓸 수 있는가. 로그인 파일을 복사하지 말고 원본을 가리키는 심볼릭 링크로 시험한다(사용자 승인됨). 키체인 저장 방식(`cli_auth_credentials_store`)이면 그 경로도 확인. 토큰 갱신이 원본에 반영되는 구조인지 소스로 확인
2. 모두 묻기: 전용 폴더 규칙에 "모든 명령은 prompt"를 표현할 수 있는가(문법을 소스·문서로 확인). `approvalPolicy`를 무엇으로 줘야 prompt가 호스트 승인 요청으로 오는가. Codex가 원래 안전하다고 보고 바로 실행하는 명령(`ls`, `cat`, `node -e "console.log(1)"`)도 prompt 규칙을 따르는가
3. 번역: `allow` 규칙 명령은 묻지 않고 실행, `forbidden` 규칙 명령은 막힘
4. 사용자 규칙 무시: 사용자 `~/.codex/rules/`의 allow 명령(`sort` 등, 읽어서 확인)이 전용 폴더 사용 시 적용되지 않고 Saturn 규칙(prompt)을 따르는가. 프로젝트 폴더 `.codex/rules`의 allow도 무시되는가(신뢰 목록이 전용 폴더 config에 없을 때)
5. 승인 대기: prompt로 온 승인 요청에 90초 이상 답하지 않는 동안 실행되지 않고, 거부로 답하면 끝까지 실행되지 않는가
6. 파일 편집과 MCP: 파일 편집(`apply_patch`)과 MCP 도구 호출도 Saturn이 판단(승인 요청)할 수 있는가. 실행 규칙이 아니라 승인 정책·샌드박스 쪽이면 그 설정을 찾는다
7. subagent: subagent(자식 thread)가 실행하는 명령에도 같은 전용 폴더 규칙이 적용되는가
8. 사용자 다른 설정 옮기기: 사용자 `~/.codex/config.toml`에서 권한 관련 키(approval_policy, sandbox_mode, rules 관련, hooks 등)만 빼고 나머지(모델, MCP 서버, 프로필 등)를 전용 폴더 `config.toml`로 만들어 쓰면 정상 동작하는가. 어떤 키를 빼야 하는지 목록
9. 채팅마다 규칙이 다를 때: (a) 규칙이 다른 두 전용 폴더로 app-server 두 개를 띄워 각각 규칙대로 동작하는가, (b) app-server 하나에서 thread(세션)마다 규칙을 바꾸는 방법(`thread/start` config 덮어쓰기 등)이 있는가. 둘 다 시험

## 장소와 안전 규칙 (반드시)
- `git -C ~/workspace/oss/saturn worktree add ../saturn.wt/experiment-194-codex-home -b experiment/194-codex-home origin/main`. 전용 폴더(`codex-home-a/`, `codex-home-b/`), 규칙, 로그, 테스트 파일은 모두 이 worktree 안. 커밋하지 않는다
- 로그인 파일: 원본(`~/.codex/auth.json` 등)을 복사·이동·수정하지 않는다. 전용 폴더 안에 원본을 가리키는 심볼릭 링크만 만든다. 토큰 값은 절대 출력하지 않는다
- 사용자 전역 설정(`~/.codex/config.toml`, `~/.codex/rules/`, `~/.codex/hooks.json`)은 읽기만
- 금지: `/tmp`, `$TMPDIR`, `mktemp`, 저장소 사본, `--dangerously-*` 옵션, 파일 삭제·네트워크·설치 명령. Codex에 실행시키는 명령은 worktree 안 파일에 대한 `touch`, `echo`, `sort`, `ls`, `cat`, `node -e "console.log(1)"`만
- 실험이 띄운 app-server와 codex 프로세스는 끝날 때 모두 종료한다(남기지 마라). 다른 세션의 codex 프로세스는 건드리지 마라
- Codex 모델 호출은 40회 안쪽
- 커밋, PR, 이슈 닫기 금지. AI 흔적을 남기지 마라

## 보고
이슈 #194에 댓글로 남긴다(한국어, 표): 확인할 것 / 회차별 관측 / 결과 / 근거(소스 경로·줄, 문서 URL, 실제로 열어 본 것만). 마지막에 결론: 이 방식으로 Saturn이 Codex의 모든 실행을 판단할 수 있는가, 남는 구멍과 조건, 채팅마다 규칙이 다를 때 권장 방식(9번 a/b). 같은 내용을 `~/workspace/woon/.local/orchestration/saturn-build/codex-194-home-report.md`에도 쓴다.

끝나면 worktree의 실험 파일과 심볼릭 링크를 지우고 `git -C ~/workspace/oss/saturn worktree remove --force ../saturn.wt/experiment-194-codex-home`, `git -C ~/workspace/oss/saturn branch -D experiment/194-codex-home`로 정리한다.
