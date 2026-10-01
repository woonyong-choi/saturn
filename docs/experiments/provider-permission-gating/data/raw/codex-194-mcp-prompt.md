# Saturn 이슈 #194 후속 실험: Codex MCP 도구 불안정 해결

저장소: `~/workspace/oss/saturn` (GitHub `woonyong-choi/saturn`). 먼저 `gh issue view 194 --comments`로 앞 실험 여섯 개를 읽어라. 마지막 댓글(구멍 막기 실험)에서 MCP 도구 호출이 3번 중 2번은 `mcpServer/elicitation/request`로 승인 요청이 왔지만 1번은 "도구 접근 불가"로 끝났다. 서버 준비 시점 문제로 보인다. 같은 방법(전용 `CODEX_HOME`, 원본 `~/.codex/auth.json` 심볼릭 링크, 권한 키를 뺀 `config.toml`, app-server stdio, `thread/start`의 `approvalPolicy="untrusted"`, MCP `approval_mode`)을 쓴다.

## 목표
Saturn이 띄운 Codex에서 MCP 도구가 매번 준비된 상태로 쓰이고, Saturn 규칙대로(허용/묻기/거부) 동작하게 하는 방법을 찾고 확인한다.

## 조사 (근거는 소스 경로·줄, 문서 URL, 실제로 열어 본 것만)
1. MCP 서버 시작과 도구 목록 확정 시점: app-server가 MCP 서버를 언제 띄우고, 도구 목록을 언제 모델에 넘기는지(thread 시작, 턴 시작, 지연 로딩). 서버가 아직 준비 안 됐을 때 도구가 빠지는 코드 경로
2. 준비 완료를 호스트가 알 수 있는 신호: app-server 알림(예: MCP 서버 시작 상태)이나 조회 메서드(예: MCP 서버 상태 목록). 이름과 형식
3. 설정: 서버별 `startup_timeout_sec`, `tool_timeout_sec`, 도구별 `approval_mode`(Auto/Prompt/Writes/Approve), `enabled_tools`/`disabled_tools` 등
4. Saturn 규칙 번역표: 허용 → 어떤 `approval_mode`, 묻기 → `prompt`, 거부 → 도구 끄기(`disabled_tools` 등). 소스로 각 값의 동작 확인

## 실측 (각 조건 5회 반복. 5회 모두 같아야 "확인", 다르면 "불안정", 못 하면 "확인 못 함". 추정으로 채우지 마라)
- 테스트 MCP 서버: worktree 안 로컬 스크립트로 무해한 도구 둘을 가진 서버를 만든다(예: 입력을 그대로 돌려주는 `echo_tool`, 파일을 만드는 대신 메시지만 돌려주는 `write_like_tool`). 시작이 느린 경우를 재현하려고 시작 지연(예 3초, 10초)을 줄 수 있게 한다
- 실제 환경과 비슷하게, 사용자 `~/.codex/config.toml`의 `mcp_servers` 항목들도 전용 config에 함께 넣어 여러 서버가 동시에 뜨게 한다. **단, 사용자 MCP 서버의 도구는 절대 호출하지 않는다**(모델 프롬프트에서 테스트 서버 도구만 쓰라고 지시하고, 사용자 서버 도구는 `disabled_tools`나 `approval_mode=prompt`로 묶은 뒤 요청이 오면 무조건 거부한다). 사용자 서버가 외부 네트워크에 붙는 것은 서버 기동까지만 허용
1. 기준선: 지금처럼 바로 첫 턴을 보낼 때 테스트 도구가 준비돼 있는 비율(5회)
2. 해결 후보 A: 준비 완료 신호(알림·조회)를 확인한 뒤 첫 턴을 보낸다(5회, 시작 지연 3초·10초 각각)
3. 해결 후보 B: `startup_timeout_sec` 등 설정 조정(5회)
4. 해결 후보 C: 조사에서 찾은 다른 방법이 있으면 시험
5. 번역표 확인(가장 좋은 해결 방법을 켠 상태에서 5회씩): `echo_tool` 허용이면 승인 요청 없이 실행, `write_like_tool` 묻기면 승인 요청이 오고 거부하면 실행 안 됨(테스트 서버 쪽 호출 기록으로 확인), 거부(도구 끄기)면 도구가 보이지 않거나 호출이 막힘
6. 승인 없이 실행된 MCP 호출이 한 번이라도 있었는지 테스트 서버 호출 기록과 대조해 따로 표시

## 장소와 안전 규칙 (반드시)
- `git -C ~/workspace/oss/saturn worktree add ../saturn.wt/experiment-194-codex-mcp -b experiment/194-codex-mcp origin/main`. 전용 home, 테스트 MCP 서버, 로그는 모두 이 worktree 안. 커밋하지 않는다
- 로그인 원본 복사·이동·수정 금지, 심볼릭 링크만, 토큰 출력 금지. 사용자 전역 설정(`~/.codex/config.toml`, `~/.codex/rules/`, `~/.codex/hooks.json`)은 읽기만. 사용자 MCP 서버 설정을 전용 config에 옮길 때 그 안의 비밀값(토큰, 키)은 출력·기록하지 않는다
- 금지: `/tmp`, `$TMPDIR`, `mktemp`, 저장소 사본, `--dangerously-*` 옵션, 파일 삭제·설치 명령, 사용자 MCP 도구 호출
- 실험이 띄운 app-server, MCP 서버, codex 프로세스는 끝날 때 모두 종료. 다른 세션의 codex 프로세스는 건드리지 마라
- Codex 모델 호출 상한 70회. 닿으면 멈추고 거기까지로 보고
- 커밋, PR, 이슈 닫기 금지. AI 흔적을 남기지 마라

## 보고
이슈 #194에 댓글(한국어, 표): 확인할 것 / 회차별 관측 / 결과 / 근거. 결론: 불안정의 원인, 권장 해결 방법과 조건, Saturn 규칙→MCP 설정 번역표, 승인 없이 실행된 MCP 호출 유무. 같은 내용을 `~/workspace/woon/.local/orchestration/saturn-build/codex-194-mcp-report.md`에도 쓴다. 실제 모델 호출 횟수를 적는다.

끝나면 worktree 실험 파일과 심볼릭 링크를 지우고 `git -C ~/workspace/oss/saturn worktree remove --force ../saturn.wt/experiment-194-codex-mcp`, `git -C ~/workspace/oss/saturn branch -D experiment/194-codex-mcp`로 정리한다.
