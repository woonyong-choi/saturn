# Saturn 이슈 #194 짧은 실험: MCP "묻기"(approval_mode=prompt) 경로 5회 확인

저장소: `~/workspace/oss/saturn`. 먼저 `gh issue view 194 --comments`의 마지막 댓글(MCP 준비 시점 실험)을 읽고 같은 방법을 쓴다: 전용 `CODEX_HOME`(원본 `~/.codex/auth.json` 심볼릭 링크만), app-server stdio, `thread/start`의 `approvalPolicy="untrusted"`, 첫 턴 전 `mcpServerStatus/list`로 테스트 서버 도구 준비 확인, `mcp_optional_startup_grace_ms` 넉넉히.

## 확인할 것
테스트 MCP 서버(worktree 안 로컬 스크립트, 무해한 도구 `write_like_tool` 하나: 메시지만 돌려주고 호출 기록을 worktree 안 로그 파일에 남김)에 `approval_mode="prompt"`를 둔다. 사용자 MCP 서버는 이번에는 넣지 않는다.
1. 모델이 그 도구를 반드시 부르도록 지시를 좁힌다(예: "write_like_tool을 message=\"saturn-probe-N\"으로 정확히 한 번 호출하라. 다른 도구나 셸은 쓰지 마라"). 5회 반복
2. 회차마다 기록: 모델이 도구 호출을 시도했는가, `mcpServer/elicitation/request`(또는 소스상 MCP 승인 요청)가 호스트에 왔는가, 거부(decline)로 답했을 때 테스트 서버 호출 기록에 실행이 없는가
3. 추가 5회: 같은 지시로 승인(accept)했을 때 실행되는가(호출 기록 있음)
4. 모델이 도구를 시도하지 않은 회차는 "시도 없음"으로 따로 세고, 시도한 회차 기준으로 판정한다. 시도 없음이 2회 이상이면 지시를 고쳐 그만큼 다시 돌린다(상한 안에서)
판정: 시도한 회차가 각 5회 이상이고 모두 승인 요청 → 거부 시 미실행 / 승인 시 실행이면 "확인", 하나라도 다르면 "불안정"

## 장소와 안전 규칙
- `git -C ~/workspace/oss/saturn worktree add ../saturn.wt/experiment-194-mcp-prompt -b experiment/194-mcp-prompt origin/main`. 모든 파일은 이 안. 커밋하지 않는다
- 로그인 원본 복사·이동·수정 금지, 토큰 출력 금지. 사용자 전역 설정은 읽기만
- 금지: `/tmp`, `$TMPDIR`, `mktemp`, 저장소 사본, `--dangerously-*`, 파일 삭제·네트워크·설치 명령, `mcpServer/tool/call` 직접 호출(승인 경계를 우회하므로 이번 판정에 쓰지 마라)
- 실험이 띄운 프로세스는 끝날 때 모두 종료. 다른 세션 프로세스는 건드리지 마라
- Codex 모델 호출 상한 20회. 닿으면 멈추고 거기까지로 보고
- 커밋, PR, 이슈 닫기 금지. AI 흔적 금지

## 보고
이슈 #194에 댓글(한국어, 표): 회차 / 시도 여부 / 승인 요청 / 답 / 실행 기록 / 판정. 결론 한 줄. 같은 내용을 `<비공개 경로>/codex-194-prompt-path-report.md`에. 실제 모델 호출 횟수를 적는다.
끝나면 실험 파일과 링크를 지우고 `git -C ~/workspace/oss/saturn worktree remove --force ../saturn.wt/experiment-194-mcp-prompt`, `git -C ~/workspace/oss/saturn branch -D experiment/194-mcp-prompt`로 정리한다.
