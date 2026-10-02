# Saturn 이슈 #194 MCP `approval_mode=prompt` 실험

## 방법

- 기준: Codex CLI 0.158.0, app-server stdio JSON-RPC.
- 전용 `CODEX_HOME`만 사용하고 `auth.json`은 `/Users/woonyong/.codex/auth.json`을 가리키는 심볼릭 링크로 두었다. 원본 인증 파일은 복사·이동·수정하지 않았다.
- `thread/start`에 `approvalPolicy="untrusted"`를 전달했다.
- 각 회차 첫 턴 전에 `mcpServerStatus/list`로 `saturn_prompt_fixture/write_like_tool` 목록을 확인했다. `mcp_optional_startup_grace_ms=12000`, 서버 `startup_timeout_sec=30`을 사용했다.
- 사용자 MCP 서버는 설정하지 않았다. `mcpServer/tool/call` 직접 호출도 하지 않았다.
- fixture는 `write_like_tool` 하나만 제공하고, 실행될 때만 worktree 안 `experiment-logs/tool-calls.jsonl`에 메시지를 기록했다.

## 회차별 관측

| 회차 | 시도 여부 | 승인 요청 | 답 | 실행 기록 | 판정 |
|---|---|---|---|---|---|
| decline 1 | 있음 | 있음 | decline | 없음 | 요청 후 미실행 |
| decline 2 | 있음 | 없음 | 없음(timeout) | 없음 | 승인 경계 관측 불가 |
| decline 3 | 있음 | 있음 | decline | 없음 | 요청 후 미실행 |
| decline 4 | 있음 | 없음 | 없음(timeout) | 없음 | 승인 경계 관측 불가 |
| decline 5 | 있음 | 있음 | decline | 없음 | 요청 후 미실행 |
| accept 1 | 있음 | 없음 | 없음(timeout) | 없음 | 승인 경계 관측 불가 |
| accept 2 | 있음 | 없음 | 없음(timeout) | 없음 | 승인 경계 관측 불가 |
| accept 3 | 있음 | 있음 | accept | 있음: `saturn-probe-accept-3` | 실행 |
| accept 4 | 있음 | 있음 | accept | 있음: `saturn-probe-accept-4` | 실행 |
| accept 5 | 있음 | 있음 | accept | 있음: `saturn-probe-accept-5` | 실행 |

모델이 도구를 시도하지 않은 회차는 0회다. 시도한 회차 기준으로 decline은 승인 요청 3/5회, accept는 승인 요청 3/5회였다. 승인 요청이 실제로 도착한 경우에는 decline 3/3회 미실행, accept 3/3회 실행이었다.

실제 모델 호출은 11회다. decline 1–5 5회, decline 2 보충 재시도 1회, accept 1–5 5회다. decline 2 보충 재시도도 도구 시도 후 승인 요청 없이 timeout됐다. 설정 위치·상태 응답 형태를 고치는 준비 실행 2회는 모델 호출 전에 끝나 실제 호출 수에 포함하지 않았다.

## 결론

**불안정** — 도구 시도는 10/10회였지만 `mcpServer/elicitation/request`가 decline 3/5회, accept 3/5회에만 도착했다. 요청이 온 경우의 decline 미실행과 accept 실행은 각각 3/3회 확인했다.
