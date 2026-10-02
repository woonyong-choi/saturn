# Saturn 이슈 #194 실험 10: MCP 승인 요청이 약 140초 늦게 오는 원인 찾기와 줄이기

저장소: `~/workspace/oss/saturn`. 먼저 `gh issue view 194 --comments`의 마지막 세 댓글을 읽고, 직전 실험(실험 9) 보고서와 원자료 `<비공개 경로>/rootcause/`(report.md, experiment/의 드라이버·fixture, codex-home/config.toml, experiment-logs/)를 읽어라. 실험 9에서 MCP 도구(`approval_mode="prompt"`) 호출 시작(`mcpToolCall started`, `waitingOnApproval`) 뒤 `mcpServer/elicitation/request`가 오기까지 바로(약 1ms) 오거나 약 141~144초 걸렸다. 원인은 확정하지 못했고 Codex 소스는 읽지 못했다.

## 새로 발견된 단서 (가장 먼저 검증하라)
실험 9 원문 로그를 다시 세어 보니, 늦게 온 승인 요청 16회는 모두 드라이버가 150초 마감 때 보낸 `turn/interrupt` **1~4ms 뒤**에 도착했다. 나머지 13회는 도구 시작 0~2ms 뒤(1회 약 7.9초)에 도착했다. 실험 9 드라이버는 `selectors`로 읽을 거리를 기다린 뒤 텍스트 모드 `readline()`으로 읽는 구조다. 이 구조는 이미 도착한 줄이 파이썬 버퍼에 남아 있어도 `select`가 깨어나지 않아, 다음 데이터(예: interrupt 응답)가 올 때까지 줄을 못 보는 고전적인 버그를 만든다. 즉 **"140초 지연"은 Codex가 아니라 드라이버의 읽기 버그일 수 있다.**
- 1순위 검증: 드라이버를 버퍼 문제 없는 방식(바이트 단위 비차단 읽기, 또는 전용 읽기 스레드가 줄마다 즉시 큐에 넣는 방식)으로 고친 뒤 기준선 5회를 다시 측정한다. 요청이 도구 시작 직후 오면 원인은 드라이버로 확정하고, 원문 메시지 수신 시각(원시 바이트 수신 시각)으로 근거를 남긴다
- 드라이버를 고쳐도 지연이 남을 때만 아래 Codex 소스 가설(H-a~H-c)로 넘어간다

## 목표
약 140초 지연의 원인을 Codex 소스와 원문 메시지로 찾고, 설정이나 호출 방식으로 지연을 없애거나 줄인 뒤 반복 측정으로 확인한다.

## 1. 소스 읽기 (이번에는 허용한다)
openai/codex 소스를 `gh api`, raw.githubusercontent.com, WebFetch로 읽어라(clone 금지). 설치 버전 0.158.0에 맞는 태그나 커밋을 먼저 찾는다. 볼 곳: `codex-rs/core/src/mcp_tool_call.rs`, `codex-rs/codex-mcp/`, app-server의 elicitation·승인 처리, 승인 검토자(`approvals_reviewer`, 자동 검토 모델, guardian 등) 관련 코드. 근거는 실제로 열어 본 경로·줄만.
가설(소스로 확인하거나 기각한다. 추정으로 채우지 마라):
- H-a: MCP 승인 요청이 사용자에게 오기 전에 자동 검토자(예: 자동 리뷰 모델)에게 먼저 가고, 그 검토가 끝나거나 시간 초과(약 2분)된 뒤에 호스트로 넘어온다
- H-b: elicitation 전에 다른 대기(MCP 서버 응답, `tool_timeout_sec`, 연결 상태 확인 등)가 있다
- H-c: 모델의 턴 처리나 스트리밍 순서 때문에 요청 전송이 늦어진다
- 그 밖에 소스에서 찾은 원인

## 2. 실측 (조건마다 5회, 5회 모두 같아야 "확인")
- 실험 9의 fixture를 재사용하되 드라이버 읽기 방식은 위 단서대로 고친다. 고친 드라이버와 원래 드라이버를 같은 조건으로 비교할 수 있게 둘 다 남긴다(사본은 아래 worktree 안에). 드라이버의 승인 대기 시간 제한은 300초 이상으로 늘려 늦은 요청도 끝까지 받는다
- 회차마다 `mcpToolCall started` → `mcpServer/elicitation/request` 도착까지 걸린 시간을 기록하고, 그 사이에 오간 모든 메시지를 원문으로 남긴다
- 기준선 5회(지연이 다시 나오는지)
- 원인에 맞는 해결 후보(예: 승인 검토자 끄기·사용자 직접 승인 설정, 관련 시간 제한 설정 등, 소스에서 찾은 키만)를 적용해 5회. 거부 시 미실행, 승인 시 실행도 함께 확인
- 지연이 사라지면 해결 후보를 셸 명령 승인(`item/commandExecution/requestApproval`)에도 같은 지연이 있는지 3회 확인한다(있으면 같은 해결이 통하는지)

## 3. 원자료 보존
끝내기 전에 드라이버, fixture, 전용 home의 `config.toml`과 rules(인증 링크·토큰 제외), 회차별 원문 로그, 결과 요약 JSON, 보고서를 `<비공개 경로>/delay/`로 옮긴다.

## 장소와 안전 규칙
- `git -C ~/workspace/oss/saturn worktree add ../saturn.wt/experiment-194-mcp-delay -b experiment/194-mcp-delay origin/main`. 실험 파일은 이 안. 커밋하지 않는다
- 로그인 원본(`~/.codex/auth.json`) 복사·이동·수정 금지, 전용 home 안 심볼릭 링크만, 토큰 출력 금지. 사용자 전역 설정은 읽기만. 사용자 MCP 서버는 넣지 않는다
- 소스 읽기용 GitHub·웹 조회는 허용. 그 밖에 Codex에 실행시키는 명령은 worktree 안 무해한 것만. `/tmp`·`$TMPDIR`·`mktemp`·저장소 사본·`--dangerously-*`·설치 명령 금지. `mcpServer/tool/call` 직접 호출 금지
- 실행기를 한 회차에 thread 하나만 만들도록 확인하고 시작하라(실험 9에서 중복 thread가 생겨 호출이 늘었다)
- Codex 모델 호출 상한 30회. 상한에 닿기 직전에 멈추고 거기까지로 보고한다. 넘기지 마라
- 실험이 띄운 프로세스는 끝날 때 모두 종료. 다른 세션 프로세스는 건드리지 마라
- 커밋, PR, 이슈 닫기 금지. AI 흔적 금지

## 보고
이슈 #194에 댓글(한국어, 표): 소스 근거, 가설별 판정, 회차별 지연 시간, 해결 후보와 결과, 결론(원인, 해결 방법, 조건). 같은 내용을 `.../delay/report.md`에. 실제 모델 호출 횟수를 적는다. 끝나면 worktree 정리: `git -C ~/workspace/oss/saturn worktree remove --force ../saturn.wt/experiment-194-mcp-delay`, `git -C ~/workspace/oss/saturn branch -D experiment/194-mcp-delay`.
