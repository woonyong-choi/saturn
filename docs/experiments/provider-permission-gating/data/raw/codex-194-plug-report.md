# Saturn 이슈 #194 후속 실험: 전용 CODEX_HOME 방식의 구멍 막기

## 실험 범위

- 기준: `woonyong-choi/saturn` 이슈 #194의 앞선 댓글 전체와 마지막 전용 `CODEX_HOME` 실험 방법.
- Codex CLI: `0.158.0`.
- worktree: `../saturn.wt/experiment-194-codex-plug`, `origin/main` 기준.
- 전용 home: `codex-home/`. `auth.json`은 `~/.codex/auth.json`을 가리키는 심볼릭 링크만 만들었다. 원본 값은 읽거나 출력하지 않았다.
- 설정: `approvalPolicy=untrusted`를 `thread/start`에 전달했다. 전용 `config.toml`에는 `approval_policy="on-request"`, `sandbox_mode="workspace-write"`, MCP 서버·도구별 `prompt` 설정을 두었다. `rules/default.rules`에는 실험 명령을 모두 `prompt`로 적었다.
- 사용한 app-server 드라이버와 관측 원자료: 실험 worktree의 `run-experiment.py`, `experiment-logs/observations.jsonl`.
- 모델 호출: 전용 home의 rollout 기록 53회(부모 thread 48회, subagent child thread 5회). 제한 60회 안에서 종료했다. 초기 timeout/중단으로 끝난 호출도 시도 횟수에 포함했다.

## 결과 요약

| 확인할 것 | 회차별 관측 | 결과 | 근거 |
|---|---|---|---|
| 1. 안전 목록과 무해 명령 | 소스에 고정된 known-safe/read-only 명령 목록이 없었다. 전용 규칙에 적은 `ls`, `cat`, `head`, `wc`, `sort`는 각각 3/3회 `item/commandExecution/requestApproval`이 왔다. `node -e "console.log(1)"`, worktree 안 `touch`도 각각 3/3회 요청이 왔고 모두 거부했다. | **확인 못 함**: 소스상 전부 열거할 안전 목록이 없기 때문이다. 다만 규칙으로 명시한 대표 명령은 **확인**(각 3/3) | `codex-home/rules/default.rules:1-8`; `experiment-logs/observations.jsonl:5-25`; exec policy 소스의 미매칭 명령 판정 [exec_policy.rs](https://github.com/openai/codex/blob/b527ce4/codex-rs/core/src/exec_policy.rs#L757-L855) |
| 2. 파일 편집과 read-only sandbox | `read-only`에서 `apply_patch`는 3/3회 `item/fileChange/requestApproval`, 거부 뒤 파일 미생성. 같은 sandbox의 `touch`도 3/3회 명령 승인 요청, 거부 뒤 파일 미생성. 일반 읽기·테스트의 통과 여부는 이 실험에서 독립 표본으로 측정하지 않았다. | 편집·셸 쓰기 차단은 **확인**. 일반 작업 불편 정도는 **확인 못 함** | `experiment-logs/observations.jsonl:26-31`; `run-experiment.py:240-250`; patch 안전성 [safety.rs](https://github.com/openai/codex/blob/b527ce4/codex-rs/core/src/safety.rs#L67-L124) |
| 3. MCP | Codex 소스에 서버 기본 승인 모드와 도구별 `approval_mode`가 있었다. `mcp/1`, `mcp/2`는 `mcpServer/elicitation/request`가 오고 거부됐다. `mcp/3`은 도구 접근 불가 메시지로 끝났다. 서버 ready를 기다린 보충 `mcp-controlled/1-3`도 모두 도구 접근 불가였고, 중계 서버의 `tools/call`은 한 번도 도달하지 않았다. | MCP 승인 관측은 **불안정**(2/3 요청, 1/3 도구 없음). 중계 거부까지의 실행은 **확인 못 함** | `codex-home/config.toml:12-19`; `mcp-relay.py:45-72`; `experiment-logs/observations.jsonl:32-34,43-45`; MCP 설정 [mcp_types.rs](https://github.com/openai/codex/blob/b527ce4/codex-rs/config/src/mcp_types.rs#L26-L34), [mcp_types.rs](https://github.com/openai/codex/blob/b527ce4/codex-rs/config/src/mcp_types.rs#L77-L88), [mcp_types.rs](https://github.com/openai/codex/blob/b527ce4/codex-rs/config/src/mcp_types.rs#L278-L304); 호출 승인 흐름 [mcp_tool_call.rs](https://github.com/openai/codex/blob/b527ce4/codex-rs/core/src/mcp_tool_call.rs#L214-L285) |
| 4. subagent | 5회 모두 자식 thread가 생성됐다. 5/5회 자식이 실행한 `touch`에 `item/commandExecution/requestApproval`이 왔고 모두 거부됐다. `subagent-*.txt`는 생성되지 않았다. 실행 없음 회차는 0회. | **확인**(5/5) | `experiment-logs/observations.jsonl:35-39`; 부모 설정 상속 [child_config.rs](https://github.com/openai/codex/blob/b527ce4/codex-rs/core/src/agent/child_config.rs#L102-L194), [spawn.rs](https://github.com/openai/codex/blob/b527ce4/codex-rs/core/src/agent/control/spawn.rs#L471-L565) |
| 5. 종합 시나리오 | `composite/1`: 셸 읽기·셸 쓰기·파일 편집 요청, MCP 요청 없음. `composite/2`: 네 종류 모두 요청. `composite/3`: 셸 읽기·셸 쓰기·파일 편집 요청, MCP 요청 없음. 요청된 항목은 모두 거부됐고 파일은 생성되지 않았다. | 네 종류가 매 회 모두 승인 요청으로 온다는 조건은 **불안정**. 요청된 셸·편집·MCP는 실행 거부로 막혔다. | `experiment-logs/observations.jsonl:40-42` |

## 소스·설정 확인

1. `exec_policy.rs`의 현재 코드에는 앞선 실험에서 말한 고정 read-only 명령 목록을 열거한 함수가 없었다. `UnlessTrusted`/`untrusted`는 미매칭 명령을 승인 요청으로 만들고, `OnRequest`는 sandbox·위험도·경로 조건에 따라 허용할 수 있다. 따라서 “안전 목록 전부를 rules에 옮겼다”는 전제 자체를 현재 소스에서는 확인할 수 없다.
2. 파일 변경은 별도 safety 경로를 거친다. `UnlessTrusted`에서는 patch가 즉시 `AskUser`가 된다. 이번 `read-only` 결과는 그 경로와 일치했다.
3. MCP에는 `AppToolApproval::{Auto, Prompt, Writes, Approve}`와 서버 기본 모드, 도구별 `approval_mode`가 있다. app-server 프로토콜에서 실제로 관측한 MCP 승인 요청 이름은 `mcpServer/elicitation/request`였다. 중계 서버를 전용 설정에만 넣었고 사용자 전역 MCP 설정은 넣지 않았다.
4. 자식 설정 생성 코드는 부모의 유효 approval policy, cwd, permission profile을 반영한다. spawn 경로도 부모 exec policy와 MCP 확장을 넘긴다.
5. multi-agent 기능 플래그는 소스 스키마에 `[multi_agent].enabled`와 `features.multi_agent`/`features.multi_agent_v2`로 존재한다. 이번 실험에서는 기능을 끄지 않고 기록만 했다. [config.schema.json](https://github.com/openai/codex/blob/b527ce4/codex-rs/core/config.schema.json#L51-L53), [config.schema.json](https://github.com/openai/codex/blob/b527ce4/codex-rs/core/config.schema.json#L995-L1002)
6. app-server 프로토콜에는 명령 승인과 파일 변경 승인 요청이 각각 정의돼 있다. [common.rs](https://github.com/openai/codex/blob/b527ce4/codex-rs/app-server-protocol/src/protocol/common.rs#L1774-L1810)

## 결론

이 설정 조합만으로 Saturn이 Codex의 셸, 파일 편집, MCP, subagent 실행을 모두 매번 판단할 수 있다고 **확인할 수 없다**.

- 셸 명령: 전용 규칙에 명시한 명령은 3/3 승인 요청으로 확인됐다. 다만 현재 소스에 고정 안전 목록이 없어 “목록 전부” 주장은 성립하지 않는다.
- 파일 편집: `read-only`에서 3/3 승인 요청과 거부 차단을 확인했다. 반대로 일반 읽기·테스트가 얼마나 통과하는지는 이번 표본으로 판단하지 않는다.
- MCP: 도구별 승인 설정은 존재하지만 관측이 2/3 요청, 1/3 접근 불가로 불안정했다. 중계 서버 거부 경로는 도구 호출 자체가 중계에 도달하지 않아 확인하지 못했다.
- subagent: 5/5 자식 명령이 부모와 같은 전용 규칙의 승인 요청으로 들어왔고 거부됐다.
- 종합: 셸·파일 편집·MCP가 모두 한 회차에서 요청되는 것은 1/3뿐이다. 요청된 작업은 차단됐지만, 네 종류 모두를 항상 판단한다고 결론낼 수 없다.

남는 구멍은 MCP 도구 목록·startup timing에 따른 도구 접근 불안정, 소스에 고정 안전 목록이 없다는 전제 불일치, 그리고 `read-only`에서 일반 작업 통과성을 별도로 측정하지 않았다는 점이다. Saturn이 네 경로를 확실히 통제하려면 app-server의 각 승인 요청을 직접 받고, MCP는 도구 노출·startup 완료·거부 결과를 별도 계약으로 검증해야 한다.

가정: “실제 모델 호출 수”는 전용 home에 남은 48개 부모 rollout과 5개 자식 rollout을 각각 한 번의 모델 호출로 세었다.
