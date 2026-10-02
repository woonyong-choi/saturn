# Saturn 이슈 #194 MCP 승인 요청 원인 조사

## 결론

**확인 못 함.** 정식 비교 표본에서 MCP 도구를 실제로 시도한 31회 모두
`mcpServer/elicitation/request`가 도착했다. 요청이 없었던 정식 회차는 모델이
`mcpToolCall`을 시작하지 않고 idle로 끝난 회차였다. 직전 실험의
“도구 시도 후 승인 요청 없이 timeout” 4회를 이 실험에서 재현하지 못했으므로,
원인을 추정하지 않는다.

## 방법과 호출 수

- Codex CLI 0.158.0, app-server stdio JSON-RPC를 사용했다.
- 전용 `CODEX_HOME`, `~/.codex/auth.json` 심볼릭 링크만 사용했다.
- `thread/start.approvalPolicy="untrusted"`, 첫 턴 전
  `mcpServerStatus/list`, `mcp_optional_startup_grace_ms=12000`을 사용했다.
- fixture는 `write_like_tool` 하나이며 호출 시에만 `tool-calls.jsonl`에 기록했다.
- 모든 stdout JSON-RPC 원문, host→server 메시지, stderr를 회차 로그에 남겼다.
- 실제 `turn/start` 기준 모델 호출은 **36회**다. 사용자가 정한 30회 상한을
  초과했으므로 추가 모델 호출은 하지 않았다. preliminary 4회 중 1회는
  드라이버를 수동 종료한 불완전 로그라 정식 비교 표본에서 제외했다.

호출 수가 늘어난 경위는 원자료에 남겼다. preliminary 4회, baseline 12회,
`all_requests` 20회가 `thread/started`로 확인된다. `all_requests` 실행기는
10개 이름 슬롯마다 서로 다른 thread가 2개씩 남아 요약 출력보다 실제 raw
호출 수가 많았다.

## 회차별 관측

| 묶음 | 회차 | 도구 시도 | elicitation | 호스트 응답 | fixture 실행 | 주요 순서·판정 |
|---|---:|---:|---:|---:|---:|---|
| baseline decline | 1–5 | 5/5 | 5/5 | 5/5 decline | 0/5 | `mcpToolCall started` → `waitingOnApproval` → elicitation → decline. 요청 후 완료까지 간 회차와 timeout 회차가 섞임 |
| baseline accept | 1–5 | 3/5 | 3/3 | 3/3 accept | 실행 기록 있음 | accept 2·5는 도구 시도 없이 idle. 시도한 회차는 모두 요청 도착 |
| baseline 보충 accept | 6–7 | 2/2 | 2/2 | 2/2 accept | 0/2 | 완료 전 150초 경계에 걸려 종료된 회차. 요청 자체는 도착 |
| all_requests 후보 검증 | decline 1–5, accept 1–5 | 19/20 | 19/19 | 19/19 | 일부 completed | 미응답 서버 요청 0건. 10개 슬롯마다 thread가 2개 남아 20회로 기록됨 |

baseline에서 승인 요청이 온 시점은 `mcpToolCall` 시작 직후부터 약 0–145초
사이였다. decline 공식 5회는 약 `141510, 143478, 1, 143635, 1ms`였고,
보충 accept는 약 `123786, 127521ms`였다. 따라서 observed instability는
승인 요청 누락으로 확정되지 않고, 일부 회차의 모델·turn 처리 지연과
완료 통지 timeout으로 관측됐다.

## 요청 종류와 응답 여부

- 실제 도구 시도 회차에서 관측한 승인 요청 이름은
  `mcpServer/elicitation/request` 하나였다.
- `item/commandExecution/requestApproval`와
  `item/fileChange/requestApproval`는 관측되지 않았다.
- 관측된 서버 요청은 모두 id `0`에 대해 host 응답이 있었다.
- `all_requests` 후보 처리에서도 unanswered server request는 0건이었다.
- decline 응답 뒤 fixture의 해당 메시지 실행 기록은 없었다. accept 회차의
  일부는 `item/completed.status=completed`와 fixture 기록이 남았고,
  일부는 승인 뒤 timeout 경계에서 completed가 남지 않았다.

## 원인 분석

확정할 수 있는 범위는 다음과 같다.

1. 이번 raw 로그에서 다른 이름의 승인 요청을 놓친 증거는 없다.
2. 호스트가 먼저 받은 다른 서버 요청에 응답하지 않아 MCP 요청이 막혔다는
   증거도 없다.
3. fixture의 startup은 각 thread에서 `mcpServer/startupStatus/updated`
   `ready`로 관측됐고, preflight `mcpServerStatus/list` 결과에는
   `write_like_tool`이 있었다. 이번 표본에서 startup 미완료는 확인되지 않았다.
4. 실제 시도 후 elicitation이 오지 않은 회차가 없었으므로, 직전 실험의
   4회 timeout이 Codex 내부 어느 지점에서 멈췄는지는 이 원자료만으로
   분리할 수 없다.

Codex Rust source checkout은 이 workspace에 없었고, 사용자가 네트워크와
저장소 사본을 금지했으므로 다음 소스 파일은 열지 못했다.

- `codex-rs/core/src/mcp_tool_call.rs`
- `codex-rs/codex-mcp/`
- app-server 승인·elicitation 구현 source

대신 설치된 0.158.0 바이너리로 생성한 계약만 확인했다.

- `experiment/schema/ServerRequest.json:1905-1945`:
  command/file approval request
- `experiment/schema/ServerRequest.json:1980-2002`:
  `mcpServer/elicitation/request`
- `experiment/schema/McpServerElicitationRequestResponse.json`:
  응답의 필수 `action`과 `accept`, `decline`, `cancel` enum
- `experiment/schema/CommandExecutionRequestApprovalResponse.json`와
  `FileChangeRequestApprovalResponse.json`:
  다른 승인 요청의 응답 계약

## 후보 처리 재확인

baseline 처리에 더해 모든 server request에 즉시 JSON-RPC 응답을 보내는
`all_requests` 드라이버를 실행했다. 미응답 요청은 0건이었고, elicitation은
도구 시도 19/19회에 도착했다. 이 후보가 승인 요청 누락을 고쳤다고 확인할
수 없다. 실행기 중복으로 10개 슬롯이 20개 thread가 되었고, 모델 호출 상한도
초과했기 때문에 추가 5회 거부·5회 승인의 정식 확인 판정은 내리지 않는다.

## 보존 자료

이 보고서와 함께 다음을 보존한다.

- `experiment/`: fixture, 드라이버, 요약 스크립트, 생성 schema
- `codex-home/config.toml`, `codex-home/rules/default.rules`
- `experiment-logs/`: 회차별 원문 JSON-RPC/stderr, thread별 분할 원문,
  fixture 기록, 결과 요약 JSON

인증 파일과 토큰 값은 보존 자료에 포함하지 않았다. 원본
`~/.codex/auth.json`은 수정·복사·이동하지 않았다.

가정: “실제 모델 호출”은 raw 로그에서 `turn/started`가 확인된 thread 수로
계산했다. 실행기 중복으로 같은 회차 이름에 여러 thread가 남은 경우도 별도
호출로 세었다.
