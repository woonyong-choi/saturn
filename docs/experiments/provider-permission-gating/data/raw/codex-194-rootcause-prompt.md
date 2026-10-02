# Saturn 이슈 #194 원인 찾기: MCP 승인 요청이 가끔 오지 않는 이유

저장소: `~/workspace/oss/saturn`. 먼저 `gh issue view 194 --comments`의 마지막 두 댓글을 읽어라. 직전 실험: `approval_mode="prompt"` MCP 도구를 모델이 10/10회 호출 시도했지만 `mcpServer/elicitation/request`는 6/10회만 호스트에 왔고, 나머지 4회는 승인 요청 없이 시간 초과로 끝났다(실행은 안 됨). 같은 방법(전용 `CODEX_HOME`, 원본 `~/.codex/auth.json` 심볼릭 링크만, app-server stdio, `thread/start`의 `approvalPolicy="untrusted"`, 첫 턴 전 `mcpServerStatus/list` 준비 확인, `mcp_optional_startup_grace_ms=12000`)을 쓴다.

## 할 일
1. **전부 기록**: app-server와 주고받는 모든 JSON-RPC 메시지(서버→호스트 요청·알림, 호스트→서버, 응답, stderr)를 회차마다 빠짐없이 원문 그대로 기록한다(토큰·인증값은 가림). 메서드 이름을 거르지 말고 모든 서버→호스트 요청에 응답 여부를 기록한다
2. **재현**: 직전과 같은 지시로 10회 돌려 승인 요청이 오지 않는 회차를 잡는다
3. **원인 분석**: 요청이 온 회차와 안 온 회차의 메시지 순서를 비교한다. Codex 소스(`codex-rs/core/src/mcp_tool_call.rs`, `codex-rs/codex-mcp/`, app-server 승인·elicitation 처리 코드)와 대조해 어디서 멈췄는지 찾는다(소스 경로·줄, 실제로 열어 본 것만). 다른 이름의 승인 요청(예: 도구 호출 승인, `item/*/requestApproval`)이 온 것을 놓쳤는지, 호스트가 응답하지 않은 다른 요청 때문에 막혔는지, 타이밍 문제인지 가린다
4. **해결 확인**: 원인을 찾으면 고치는 방법(드라이버의 처리 방식, 설정, 호출 순서 등)을 적용해 거부 5회, 승인 5회를 다시 돌린다. 판정: 시도한 회차 모두 승인 요청이 오고 거부 시 미실행·승인 시 실행이면 "확인"
5. 원인을 못 찾으면 "확인 못 함"과 남은 가설을 쓴다. 추정으로 채우지 마라

## 원자료 보존 (중요, 사용자가 나중에 블로그에 쓴다)
worktree를 지우기 전에 다음을 `<비공개 경로>/experiments/194-mcp-rootcause/`로 옮겨 보존한다: 드라이버·테스트 서버 스크립트, 전용 home의 `config.toml`과 rules(인증 링크와 토큰 제외), 회차별 원문 메시지 로그, 결과 요약 JSON. 인증 파일과 토큰 값은 절대 넣지 않는다.

## 장소와 안전 규칙
- `git -C ~/workspace/oss/saturn worktree add ../saturn.wt/experiment-194-mcp-rootcause -b experiment/194-mcp-rootcause origin/main`. 실험 파일은 이 안. 커밋하지 않는다
- 로그인 원본 복사·이동·수정 금지, 토큰 출력 금지. 사용자 전역 설정은 읽기만. 사용자 MCP 서버는 넣지 않는다
- 금지: `/tmp`, `$TMPDIR`, `mktemp`, 저장소 사본, `--dangerously-*`, 파일 삭제(정리 단계 제외)·네트워크·설치 명령, `mcpServer/tool/call` 직접 호출
- 실험이 띄운 프로세스는 끝날 때 모두 종료. 다른 세션 프로세스는 건드리지 마라
- Codex 모델 호출 상한 30회. 닿으면 멈추고 거기까지로 보고
- 커밋, PR, 이슈 닫기 금지. AI 흔적 금지

## 보고
이슈 #194에 댓글(한국어, 표): 회차별 관측(요청 도착, 메시지 순서 차이), 원인, 근거, 해결 방법과 재확인 결과, 결론. 같은 내용을 `<비공개 경로>/experiments/194-mcp-rootcause/report.md`에. 실제 모델 호출 횟수를 적는다.
끝나면 원자료를 옮긴 뒤 worktree 실험 파일과 링크를 지우고 `git -C ~/workspace/oss/saturn worktree remove --force ../saturn.wt/experiment-194-mcp-rootcause`, `git -C ~/workspace/oss/saturn branch -D experiment/194-mcp-rootcause`로 정리한다.
