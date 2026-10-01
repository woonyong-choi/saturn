# Saturn 이슈 #194 실험 10: MCP 승인 요청 지연 원인

## 결론

**원인은 Codex가 아니라 실험 9 드라이버의 읽기 버그로 확인했다.** 기존 드라이버는 `selectors.select()` 뒤 텍스트 `readline()`을 사용했다. `readline()`이 한 번의 OS read에서 여러 JSONL을 파이썬 버퍼로 미리 읽으면, 다음 줄은 이미 도착했어도 fd가 다시 readable이 아니어서 `select()`가 깨어나지 않는다. 실험 9의 늦은 16회는 모두 드라이버가 150초 마감에 보낸 `turn/interrupt`보다 1–4ms 뒤에 도착했다.

바이트 단위 비차단 읽기와 줄 큐를 적용한 드라이버에서는 MCP 승인 요청이 10/10회 도구 시작 직후 도착했다. 거부 5/5회는 실행되지 않았고, 승인 5/5회는 fixture가 실행됐다. 셸 명령 승인도 3/3회 요청 후 실행됐으며 지연은 0, 0, 0.288ms였다.

## 소스 근거

| 경로와 줄 | 확인 내용 |
|---|---|
| [`core/src/mcp_tool_call.rs:263-292`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/mcp_tool_call.rs#L263-L292) | `mcpToolCall started`를 보낸 뒤 `maybe_request_mcp_tool_approval`로 바로 승인 경로에 진입한다. |
| [`core/src/mcp_tool_call.rs:1476-1525`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/mcp_tool_call.rs#L1476-L1525) | MCP 승인 reviewer를 계산하고, 자동 승인 조건이 아니면 승인 요청을 계속한다. |
| [`core/src/mcp_tool_call.rs:1542-1585`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/mcp_tool_call.rs#L1542-L1585) | `ApprovalAction::McpToolCall`을 만든 뒤 `sess.request_approval`을 기다린다. |
| [`core/src/mcp_tool_call.rs:1653-1693`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/mcp_tool_call.rs#L1653-L1693) | MCP tool approval elicitation을 구성하고 `request_mcp_server_elicitation`을 호출한다. |
| [`core/src/tools/approvals.rs:505-569`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/tools/approvals.rs#L505-L569) | 승인 순서는 hook → Guardian 조건부 검토 → 사용자 승인이다. Guardian이 결과를 반환하지 않을 때만 사용자가 처리한다. |
| [`core/src/tools/approvals.rs:572-676`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/tools/approvals.rs#L572-L676) | Guardian 경로는 `decide_approval`을 완료할 때까지 기다린다. |
| [`ext/guardian-reviewer/src/routing.rs:54-62`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/ext/guardian-reviewer/src/routing.rs#L54-L62) | `on-request`/granular와 `approvals_reviewer=auto_review`일 때만 Guardian으로 라우팅한다. |
| [`ext/guardian-reviewer/src/lib.rs:40-42`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/ext/guardian-reviewer/src/lib.rs#L40-L42) | Guardian review deadline은 90초다. |
| [`core/src/session/mcp.rs:557-628`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/session/mcp.rs#L557-L628) | app-server 이벤트를 보내고 응답 channel을 기다린다. |
| [`app-server/src/outgoing_message.rs:330-444`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/app-server/src/outgoing_message.rs#L330-L444) | 연결 대상에 server request를 전송하는 경로에는 사용자 응답 전의 140초 대기가 없다. |
| [`codex-mcp/src/rmcp_client.rs:339-397`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/codex-mcp/src/rmcp_client.rs#L339-L397) | MCP server startup은 `startup_timeout_sec`로 제한된다. |
| [`codex-mcp/src/rmcp_client.rs:656-680`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/codex-mcp/src/rmcp_client.rs#L656-L680) | tool catalog 조회는 별도 `tools/list` 경로다. 승인 요청 이후 지연의 근거는 아니다. |

설정에서 `approvals_reviewer`를 지정하지 않은 이번 fixture의 기본값은 upstream 테스트가 확인하는 `user`다([`core/src/config/config_tests.rs:11840-11852`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/config/config_tests.rs#L11840-L11852)). 따라서 H-a의 자동 검토 경로는 이번 기준선에 적용되지 않았다.

## 가설별 판정

| 가설 | 판정 | 근거 |
|---|---|---|
| H-a 자동 검토자가 먼저 검토한 뒤 사용자에게 전달 | 기준선 원인 아님 | 소스상 `auto_review` 설정이면 가능한 경로지만 이번 설정의 기본 reviewer는 `user`이고, 새 측정에서 10/10회 즉시 elicitation이 왔다. 90초 Guardian timeout도 관측되지 않았다. |
| H-b MCP server 응답·`tool_timeout_sec`·연결 확인 대기 | 기준선 원인 아님 | startup/catalog 경로는 소스에 별도 timeout으로 존재하지만 `mcpToolCall started` 뒤 승인 요청의 10회 지연은 모두 0.228–8.839ms였다. fixture는 매회 준비됐고, 실행 5/5회도 정상이다. |
| H-c 모델 turn/stream 순서 | 기준선 원인 아님 | 소스 호출 순서는 `mcpToolCall started` → 승인 경로 → elicitation이며, 고친 원시 읽기에서 같은 조건의 요청이 매회 즉시 도착했다. |
| 드라이버의 `select()` + 텍스트 `readline()` 버퍼링 | **확인** | 실험 9 늦은 16회가 모두 `turn/interrupt` 후 1–4ms에 도착했고, 바이트 단위 비차단 읽기에서 지연이 사라졌다. |

## 회차별 실측

원시 시각은 `rawByteReceivedAtNs`이며, 지연은 `item/started`의 `mcpToolCall`에서 `mcpServer/elicitation/request`까지다.

### MCP 승인, 고친 드라이버 기준선/해결 후보

| 조건 | 회차별 지연(ms) | 승인 요청 | 실행 | 판정 |
|---|---:|---:|---:|---|
| 거부 | 8.839, 0.253, 0.228, 0.308, 0.240 | 5/5 | 0/5 | 5회 모두 즉시 요청 후 미실행 |
| 승인 | 0.583, 0.312, 0.270, 0.357, 0.738 | 5/5 | 5/5 | 5회 모두 즉시 요청 후 실행 |

모든 회차의 unanswered server request는 0건이고, 각 실행기 호출은 app-server thread 하나만 만들었다. 원래 드라이버는 `experiment/run_trial_original.py`로 보존했고, 실험 9 원문은 이번 원자료와 별도로 보존했다.

### 셸 명령 승인

| 회차 | 요청 종류 | `commandExecution` 시작 → 승인 요청(ms) | 응답 | 실행 |
|---:|---|---:|---|---|
| 1 | `item/commandExecution/requestApproval` | 0.000 | accept | 실행 |
| 2 | `item/commandExecution/requestApproval` | 0.000 | accept | 실행 |
| 3 | `item/commandExecution/requestApproval` | 0.288 | accept | 실행 |

## 해결 방법과 조건

- 해결: 드라이버 stdout/stderr를 텍스트 stream의 `readline()`으로 읽지 않고, fd를 비차단으로 설정한 뒤 `os.read()`로 바이트를 drain하고 완성된 줄을 큐에 넣는다. 각 줄에 원시 바이트 수신 시각을 기록한다.
- 대기 제한: 회차 deadline을 300초로 늘렸다.
- Codex 설정: 원인 확인 후 변경하지 않았다. 이번 fixture 설정은 `approval_mode="prompt"`, `startup_timeout_sec=30`, `tool_timeout_sec=30`, `mcp_optional_startup_grace_ms=12000`이었다.
- 승인 동작: 거부 시 fixture 기록 없음, 승인 시 `tool-calls.jsonl` 기록 있음. 셸 3회는 승인 후 worktree의 `command-probe-*.txt`가 생겼다.

## 원자료와 호출 수

- 새 실험의 실제 모델 호출: **13회**(MCP 10회 + 셸 3회).
- thread: MCP 10개 + 셸 3개. 중복 thread 없음.
- 보존: `experiment/`의 원래 드라이버와 고친 드라이버, fixture, `codex-home/config.toml`, `codex-home/rules/default.rules`, 회차별 JSONL 원문, `thread-summary.json`, `results-summary.json`, 셸 결과 JSONL.
- 인증 원본은 복사·이동·수정하지 않았고, 보존 자료에는 인증 링크와 token을 포함하지 않았다.

가정: “즉시”는 원시 바이트 수신 시각 기준 100ms 미만으로 분류했으며, 5회 모두 같은 요청 도착·승인/거부·실행 결과일 때 조건을 확인으로 판정했다.
