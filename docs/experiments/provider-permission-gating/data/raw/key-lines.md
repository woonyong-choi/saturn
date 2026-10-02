# 실행 로그 핵심 줄

Codex 실험 4건의 실행 전체 로그(`codex-194-{home,plug,mcp,prompt-path}.log`, 경로와 SHA-256은 [데이터](../README.md))에서 결과 판단에 쓴 줄만 발췌했다. 줄 번호는 각 로그 파일 안의 번호다. 긴 줄은 `...`로 줄였다. 장치 이름과 설치 id가 든 부분은 줄였고, thread id와 작업 경로는 `<id>`, `<worktree>`로 바꿨다. 이메일 주소와 인증 값이 든 줄은 발췌하지 않았다. 실험 9와 10은 맨 아래 절에 따로 적었다.

## home 실험(실험 5)

`codex-194-home.log` 18855~18884줄. 드라이버가 낸 회차별 요약이다. 열은 라벨, 결과, 승인 요청 수, 명령 상태, 파일 변경 상태, 도구 호출 방법이다.

```text
all-prompt-1	completed	1	completed
all-prompt-2	completed	3	completed,completed,completed
all-prompt-3	completed	1	completed
allow-forbidden-1	completed	0
allow-forbidden-2	completed	0
allow-forbidden-3	completed	0	completed
user-rule-ignored-1	completed	1	completed
user-rule-ignored-2	completed	1	completed
user-rule-ignored-3	completed	1	completed
project-rule-ignored-1	completed	1	completed
project-rule-ignored-2	completed	1	completed
project-rule-ignored-3	completed	1	completed
file-change-1	completed	0		completed
file-change-2	completed	0		completed
file-change-3	completed	0		completed
subagent-1	completed	0
subagent-2	completed	0
subagent-3	completed	1	declined
thread-config-1	completed	1	declined
thread-config-2	completed	1	declined
thread-config-3	completed	1	declined
server-a-1	completed	1	declined
server-a-2	completed	1	declined
server-a-3	completed	1	declined
server-b-1	completed	0
server-b-2	completed	0
server-b-3	completed	0
approval-hold-1	approval-held	1
approval-hold-2	approval-held	1
approval-hold-3	approval-held	1
```

- `allow-forbidden-*`: 명령 상태 칸에 `completed`가 있는 것은 3번 회차뿐이다. [보고서](codex-194-home-report.md) 3번 행은 `echo` allow 3회 모두 실행 완료라고 적었고, 근거로 `remaining-results.json`을 든다. 이 파일은 유실돼, 이 요약만으로는 둘의 일치를 가릴 수 없다
- `file-change-*`: 승인 요청 0, 파일 변경 `completed`. 보고서의 6번 행 파일 편집 결과와 같다
- `approval-hold-*`: 91초 동안 응답하지 않은 회차의 결과가 `approval-held`다

`codex-194-home.log` 443줄, 로그인 전용 폴더에서 프로젝트 규칙이 꺼질 때의 경고다.

```text
경고: "Project-local config, hooks, and exec policies are disabled ... until the project is trusted"
```

`codex-194-home.log` 354줄, 에이전트가 읽은 이슈 댓글(실험 1) 출력에 든 `codex exec` 오류 줄이다. 이 실험에서 새로 낸 오류가 아니다.

```text
`codex exec --ignore-rules -c approval_policy=untrusted` | | | `approval_policy = "untrusted" is no longer supported` 오류. 설정 키로는 `untrusted` 불가, `thread/start` 파라미터로만 가능
```

## plug 실험(실험 6)

`codex-194-plug.log` 30211~30247줄, 최종 실행의 회차별 요청 종류다. `ls/1`은 29755줄에 있고 이 발췌는 `ls/2`부터 시작하며, 중간 줄은 `...`로 줄였다.

```text
safe-command/ls/2 Counter({'item/commandExecution/requestApproval': 1}) created= []
safe-command/ls/3 Counter({'item/commandExecution/requestApproval': 1}) created= []
safe-command/cat README.md/1 Counter({'item/commandExecution/requestApproval': 1}) created= []
...
readonly-touch/1 Counter({'item/commandExecution/requestApproval': 1}) created= []
readonly-touch/2 Counter({'item/commandExecution/requestApproval': 1}) created= []
readonly-touch/3 Counter({'item/commandExecution/requestApproval': 1}) created= []
mcp/1 Counter({'mcpServer/elicitation/request': 1}) created= []
mcp/2 Counter({'mcpServer/elicitation/request': 1}) created= []
mcp/3 Counter() created= []
subagent/1 Counter({'item/commandExecution/requestApproval': 1}) created= []
subagent/2 Counter({'item/commandExecution/requestApproval': 1}) created= []
subagent/3 Counter({'item/commandExecution/requestApproval': 1}) created= []
subagent/4 Counter({'item/commandExecution/requestApproval': 1}) created= []
subagent/5 Counter({'item/commandExecution/requestApproval': 1}) created= []
composite/1 Counter({'item/commandExecution/requestApproval': 2, 'item/fileChange/requestApproval': 1}) created= []
composite/2 Counter({'item/commandExecution/requestApproval': 2, 'item/fileChange/requestApproval': 1, 'mcpServer/elicitation/request': 1}) created= []
composite/3 Counter({'item/commandExecution/requestApproval': 2, 'item/fileChange/requestApproval': 1}) created= []
```

`codex-194-plug.log` 36659줄, 셸 명령 승인 요청 메시지의 예다. `ls` 회차이고 응답은 거부였다. 장치 식별 값이 든 부분은 줄였다.

```json
{"case": "safe-command/ls/1", "prompt": "Execute exactly one shell command and no other tool call: `ls`. Then stop.", "sandbox": "workspace-write", "requests": [{"method": "item/commandExecution/requestApproval", "params": {"kind": "command", "itemId": "exec-<id>", "environmentId": "local", "reason": "`/bin/zsh -lc ls` requires approval by policy", "command": "/bin/zsh -lc ls", "cwd": "<worktree>", "commandActions": [{"type": "listFiles", "command": "ls", "path": null}], "availableDecisions": ["accept", "cancel"]}, "response": {"decision": "decline"}}], ...}
```

`codex-194-plug.log` 36681줄, 읽기 전용 샌드박스의 `touch` 요청이다.

```json
{"case": "readonly-touch/1", "prompt": "Execute exactly one shell command and no other tool call: `touch experiment-logs/readonly-touch-1.txt`. Then stop.", "sandbox": "read-only", "requests": [{"method": "item/commandExecution/requestApproval", "params": {"kind": "command", "reason": "`/bin/zsh -lc 'touch experiment-logs/readonly-touch-1.txt'` requires approval by policy", "commandActions": [{"type": "unknown", "command": "touch experiment-logs/readonly-touch-1.txt"}], "availableDecisions": ["accept", "cancel"]}, "response": {"decision": "decline"}}], ...}
```

`codex-194-plug.log` 36686줄, MCP 승인 요청 메시지의 예다. 같은 회차 `mcp/3`(36688줄)은 `requests`가 빈 배열이었다.

```json
{"method": "mcpServer/elicitation/request", "params": {"serverName": "saturn_probe", "mode": "form", "_meta": {"codex_approval_kind": "mcp_tool_call", "tool_description": "A harmless relay probe. The relay rejects execution.", "tool_params": {"label": "mcp-1"}}, "message": "Allow the saturn_probe MCP server to run tool \"probe\"?", "requestedSchema": {"type": "object", "properties": {}}}, "response": {"action": "decline"}}
```

`codex-194-plug.log` 23540줄, 같은 로그에서 `safe-command/ls/1`의 앞선 시도다. 요청이 0건인 채 278초 뒤 종료 코드 130으로 중단됐다. 보고서의 3/3은 이 뒤 다시 돌린 실행(29755줄)이다. 이 중단 시도의 원인과 모델 호출 53회에 포함됐는지는 로그와 보고서에서 확인하지 못했다.

```text
exited 130 in 278899ms:
{"case": "safe-command/ls/1", "requests": [], "events": ["remoteControl/status/changed", "thread/started", ...
```

## mcp 실험(실험 7)

`codex-194-mcp.log` 50753~50771줄, 기준선(10초 지연)과 해결 후보 A(준비 조회, 10초 지연)다. `ready`가 서버가 준비된 수이고 `calls`가 모델이 부른 도구다.

```text
baseline-10-1 status= completed fixture= {'processes': 1, 'ready': 0, 'tools_list': 0, 'calls': []} direct= None elic= 0 readiness= None
baseline-10-2 status= completed fixture= {'processes': 1, 'ready': 1, 'tools_list': 1, 'calls': []} direct= None elic= 1 readiness= None
baseline-10-3 status= completed fixture= {'processes': 1, 'ready': 1, 'tools_list': 1, 'calls': []} direct= None elic= 0 readiness= None
baseline-10-4 status= completed fixture= {'processes': 1, 'ready': 0, 'tools_list': 0, 'calls': []} direct= None elic= 0 readiness= None
baseline-10-5 status= completed fixture= {'processes': 1, 'ready': 0, 'tools_list': 0, 'calls': []} direct= None elic= 0 readiness= None
ready-10-1 status= completed fixture= {'processes': 2, 'ready': 2, 'tools_list': 2, 'calls': ['echo_tool']} direct= None elic= 0 readiness= status_query
ready-10-2 status= completed fixture= {'processes': 2, 'ready': 1, 'tools_list': 1, 'calls': []} direct= None elic= 0 readiness= status_query
ready-10-3 status= completed fixture= {'processes': 2, 'ready': 1, 'tools_list': 1, 'calls': []} direct= None elic= 0 readiness= status_query
ready-10-4 status= completed fixture= {'processes': 2, 'ready': 2, 'tools_list': 2, 'calls': ['echo_tool']} direct= None elic= 0 readiness= status_query
ready-10-5 status= completed fixture= {'processes': 2, 'ready': 1, 'tools_list': 1, 'calls': []} direct= None elic= 0 readiness= status_query
```

- 기준선 10초 지연은 `ready`가 2/5(회차 2, 3)이고 나머지 3/5(회차 1, 4, 5)는 0이다
- `ready-10-*`의 `processes: 2`는 준비 조회용 프로세스와 thread용 프로세스다. 보고서의 설명과 같다

`codex-194-mcp.log` 50781~50804줄, 번역표 실측이다. 허용과 거부 행만 줄여 적는다.

```text
translate-allow-1 status= completed fixture= {..., 'calls': ['echo_tool', 'echo_tool']} direct= {'status': 'completed', 'response_keys': ['content', 'isError']} elic= 0 readiness= status_query
translate-allow-4 status= completed fixture= {..., 'calls': ['echo_tool']} direct= {'status': 'completed', 'response_keys': ['content', 'isError']} elic= 0 readiness= status_query
translate-prompt-1 status= completed fixture= {..., 'calls': ['write_like_tool']} direct= {'status': 'completed', 'response_keys': ['content', 'isError']} elic= 0 readiness= status_query
translate-prompt-model-4 status= completed fixture= {..., 'calls': []} direct= None elic= 1 readiness= status_query
translate-deny-5 status= completed fixture= {..., 'calls': []} direct= {'status': 'error', 'error_type': 'RuntimeError', 'error': "mcpServer/tool/call: tool 'write_like_tool' is disabled for MCP server 'test_fixture'"} elic= 0 readiness= status_query
```

- `translate-allow-1~3`은 `echo_tool` 호출이 둘(모델 선택 하나와 호스트 직접 호출 하나), `translate-allow-4`, `translate-allow-5`는 하나(호스트 직접 호출)다. 모델이 선택한 회차는 3/5다
- `translate-prompt-1~5`의 `direct`는 호스트가 `mcpServer/tool/call`로 직접 부른 결과이고 5/5 실행됐다. 승인 경계를 우회한 관측이다
- `translate-prompt-model-1~5`는 모델 전용 5회이고 `elic`(`mcpServer/elicitation/request`)이 1/5(회차 4)였다. 도구 호출 `calls`는 5회 모두 비었다

## prompt-path 실험(실험 8)

`codex-194-prompt-path.log` 9069줄과 9847줄, 승인 요청이 온 회차의 드라이버 기록이다. 각각 decline 회차 1과 3이다.

```json
{"approval_request_count": 1, "approval_request_methods": ["mcpServer/elicitation/request"], "attempt": true, "command_attempt": false, "decision": "decline", "fixture_log_lines": [], "message_count": 80, "number": 1, "phase": "decline", "turn_status": "completed"}
{"approval_request_count": 1, "approval_request_methods": ["mcpServer/elicitation/request"], "attempt": true, "command_attempt": false, "decision": "decline", "fixture_log_lines": [], "message_count": 84, "number": 3, "phase": "decline", "turn_status": "completed"}
```

`codex-194-prompt-path.log` 19124줄, accept 회차 1에서 요청이 오지 않은 때의 에이전트 메모다.

```text
accept 1도 도구 시도 항목은 생겼지만 승인 요청 없이 180초 대기 후 종료됐습니다. 이 자체가 "매번 승인 요청" 조건과 맞지 않는 관측입니다.
```

`codex-194-prompt-path.log` 21151줄, accept 회차 1과 2의 메모다.

```text
accept 1·2 모두 도구 시도 항목 후 승인 요청 없이 timeout 됐고 fixture 로그도 없었습니다. 이는 이미 accept 조건의 불안정 신호입니다.
```

## rootcause 실험(실험 9)

실험 9의 회차별 원문 로그(비공개 원자료의 `rootcause/experiment-logs/`, 파일 이름과 SHA-256은 [데이터](../README.md))에서 승인 요청 도착 시각을 보인 줄이다. 줄 번호는 각 회차 로그 파일 안의 번호이고 `capturedAtMs`는 드라이버가 줄을 읽어 기록한 시각이다. 긴 줄은 `...`로 줄였고 thread id와 turn id는 `<id>`로 바꿨다. 이 절의 해석은 보고서가 아니라 원문 줄을 읽은 결과이고, 원인 판정은 아니다.

`baseline/decline-1.jsonl` 44~49줄. 도구 호출 시작 뒤 약 141.5초 동안 아무 줄도 없다가, 드라이버가 `turn/interrupt`를 보낸 1ms 뒤에 `mcpServer/elicitation/request`가 기록됐다. `turn/interrupt` 요청 자체는 `turnId` 누락 오류로 거부됐다.

```text
44 1790893948593 server->host item/started        mcpToolCall write_like_tool inProgress
45 1790893948594 server->host thread/status/changed active waitingOnApproval
46 1790894090102 host->server turn/interrupt       {"threadId":"<id>"}
47 1790894090103 server->host mcpServer/elicitation/request id=0 turnId=<id> serverName=saturn_prompt_fixture ...
48 1790894090104 host->server {"id":0,"result":{"action":"decline"}}
49 1790894090104 server->host {"error":{"code":-32600,"message":"Invalid request: missing field `turnId`"},"id":5}
```

`baseline/decline-3.jsonl` 44~47줄. 같은 방법의 즉시 도착 회차다. 도구 호출 시작 1ms 뒤에 요청이 기록됐고 `turn/interrupt`는 없었다.

```text
44 1790894248197 server->host item/started        mcpToolCall write_like_tool inProgress
45 1790894248198 server->host thread/status/changed active waitingOnApproval
46 1790894248198 server->host mcpServer/elicitation/request id=0 turnId=<id> serverName=saturn_prompt_fixture ...
47 1790894248198 host->server {"id":0,"result":{"action":"decline"}}
```

`experiment/run_trial.py` 중 읽기와 중단 부분(드라이버, 원문 그대로 발췌). 읽기는 `selectors`로 stdout 파일 기술자를 기다린 뒤 텍스트 모드 `readline()`으로 한 줄씩 읽고, 시간 초과 때 `turn/interrupt`에 `threadId`만 보낸다.

```text
36  self.selector.register(process.stdout, selectors.EVENT_READ, "stdout")
...
79  events = self.selector.select(max(0.05, deadline - time.monotonic()))
...
83  line = key.fileobj.readline()
...
224 deadline = time.monotonic() + 150
...
273 interrupt_id = rpc.request("turn/interrupt", {"threadId": thread_value})
```

## delay 실험(실험 10)

실험 10의 드라이버 두 개(원래 `experiment/run_trial_original.py`, 고친 `experiment/run_trial.py`, 비공개 원자료의 `delay/`, 파일 이름과 SHA-256은 [데이터](../README.md))에서 읽기 부분만 발췌했다. 줄 번호는 각 파일 안의 번호다. 앞 절의 `run_trial.py`는 실험 9 드라이버이고 `run_trial_original.py`와 같은 파일이다(SHA-256 `2c7ceb4a...`).

버그가 있던 원래 드라이버. 자식 프로세스의 stdout을 텍스트 모드(`text=True`, `bufsize=1`)로 열고, `select()`가 fd를 읽을 수 있다고 알릴 때마다 `readline()`을 한 번만 부른다. `readline()`은 한 번의 OS read로 여러 줄을 파이썬 쪽 버퍼에 미리 들여올 수 있다. 그러면 다음 줄이 이미 버퍼에 있어도 fd에는 읽을 것이 없어 `select()`가 깨어나지 않고, 줄은 다음 입력이 올 때까지 처리되지 않는다.

```text
216 text=True,
218 bufsize=1,
79  events = self.selector.select(max(0.05, deadline - time.monotonic()))
83  line = key.fileobj.readline()
```

고친 드라이버. fd를 비차단으로 바꾸고 `os.read()`로 읽을 수 있는 바이트를 모두 비운 뒤, 완성된 줄을 큐(`pending`)에 넣는다. 줄마다 첫 `os.read()`가 반환한 시각을 `rawByteReceivedAtNs`로 기록한다. 회차 마감은 150초에서 300초로 늘렸다.

```text
44  os.set_blocking(stream.fileno(), False)
100 chunk = os.read(fd, 65536)
113 line, _, remainder = buffer.partition(b"\n")
126 self.pending.append(message)
90  if self.pending:
91      return self.pending.popleft()
257 deadline = time.monotonic() + 300
249 text=False,
251 bufsize=0,
```
