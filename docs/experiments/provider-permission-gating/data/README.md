# 데이터

> [!WARNING]
> 회차별 원문 메시지와 실행 스크립트는 대부분 없다. 이 폴더는 남은 근거(지시서, 보고서, 결과 요약, 로그 발췌)만 보관한다. 수치의 1차 근거는 [이슈 #194](https://github.com/woonyong-choi/saturn/issues/194)의 결과 댓글 9개다.

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 실험 에이전트가 실험 worktree에서 Claude Code와 Codex CLI를 실행하고 관측을 이슈 댓글에 올렸다. 실험 5~9는 Codex 에이전트의 보고서와 실행 전체 로그가 남았다. 실험 9는 드라이버, 전용 `CODEX_HOME` 설정, 회차별 원문 JSON-RPC 로그, 결과 요약도 남았다. 실험 1~4는 이슈 댓글만 남았다. |
| 수집 기간 | 2026-10-02~2026-10-02. 결과 댓글 9개의 작성 시각은 2026-10-01T15:36:26Z~2026-10-01T23:12:12Z다. |
| 개수 | 결과 댓글 9개. 지시서 5개, 보고서 5개, 실행 로그 5개. 실험 9의 회차별 원문 로그는 26개 파일 |
| 표본 여부 | 전수. 표본 추출 없이 실험마다 지시서에 정한 반복 횟수를 모두 실행했다. |
| 라벨 | 없음. 회차별 판정은 실험 에이전트가 붙였다. 사람이 다시 확인한 기록은 없다. |
| 알려진 문제 | 회차별 원자료가 유실됐다. 실험 3의 인용 일부가 틀렸고 실험 4는 항목 일부를 잘못 건너뛰었다. 실험 5와 실험 9가 모델 호출 상한을 넘겼다. 실험 9는 `all_requests` 실행기 중복으로 슬롯 10개가 thread 20개가 됐고, 정식 시도 수가 보고서 본문(31회)과 원문 로그에서 센 값(29회)이 다르다. 상세는 [보고서](../report.md) 한계 절이다. |
| 개인정보 | 지시서, 보고서, 결과 요약 JSON에 사용자 홈 경로(`/Users/...`)가 있다. 계정 플랜 종류(`planType=pro`)가 있다. 토큰, 키, 인증 값은 없다(복사 전에 `token`, `key`, `secret`, `bearer`, `sk-`, `@` 패턴을 검색해 확인. 실험 9 결과 요약 JSON에서 걸린 것은 메서드 이름 `thread/tokenUsage/updated`뿐이다). 실행 로그에는 계정 이메일과 의존 패키지 저자 이메일이 있어 저장소에 넣지 않았다. |
| 라이선스 | 저장소 라이선스를 따른다. |

## 원자료 상태

| 대상 | 상태 | 비고 |
|---|---|---|
| 실험 1~4의 회차별 원문 메시지와 스크립트 | 유실 | 실험 worktree와 함께 지워졌다. 이슈 댓글 1~4만 남았다. |
| 실험 5~8의 드라이버, 회차별 관측 파일(`matrix-results.json`, `remaining-results.json`, `account-results.json`, `mcp-results.json`, `observations.jsonl`, `results.jsonl`, `tool-calls.jsonl` 등), 전용 `CODEX_HOME` | 유실 | 실험 worktree와 함께 지워졌다. 보고서가 인용한 파일 줄 번호는 확인할 수 없다. |
| 실험 5~8의 지시서 | 보관 | `raw/codex-194-{home,plug,mcp,prompt-path}-prompt.md` |
| 실험 5~8의 보고서 | 보관 | `raw/codex-194-{home,plug,mcp,prompt-path}-report.md`. 이슈 댓글 5~8과 같은 내용 |
| 실험 9의 원자료 | 보존됨(비공개) | 드라이버, 전용 `CODEX_HOME` 설정과 규칙, 회차별 원문 JSON-RPC 로그, 결과 요약 JSON, 에이전트 실행 로그. 저장소 밖 비공개 폴더에 있다. 아래 절 |
| 실험 9의 지시서, 보고서, 결과 요약 JSON | 보관 | `raw/codex-194-rootcause-{prompt,report}.md`, `raw/codex-194-rootcause-{results.jsonl,thread-summary.json}`. 보고서는 이슈 댓글 9와 같은 내용 |
| 실험 5~9의 Codex 실행 전체 로그 | 저장소 밖 보관 | 아래 표. 파일 크기 규칙(한 파일 50MB 이하)과 별개로 개인 정보가 있어 저장소에 넣지 않았다. |
| 실행 로그의 핵심 줄 | 보관 | `raw/key-lines.md` |
| 이슈 댓글 8개 | GitHub 보관 | 저장소에 사본을 두지 않았다. |

## 저장소 밖 실행 로그

비공개 경로다. 저장소 루트 기준 `.local/experiments/provider-permission-gating/`(git 제외 비공개 폴더) 안에만 있고 저장소와 GitHub에 올라가 있지 않다. 사용자 로컬 장치의 작업본에만 있어 다른 장치에서는 읽을 수 없다.

| 실험 | 파일 | 크기 | SHA-256 |
|---|---|---|---|
| 5. 전용 `CODEX_HOME` | `.local/experiments/provider-permission-gating/codex-194-home.log` | 3,070,349바이트 | `3cd4606bd74735c2fc13b8e26ecc1ef9009b1fb7d7036711c11a654ec1d37551` |
| 6. 구멍 막기 | `.local/experiments/provider-permission-gating/codex-194-plug.log` | 2,963,403바이트 | `f4999c9ac27e862c94b2a76e1884d1c2d48f379a5fb585fe25811f7fe3067eec` |
| 7. MCP 준비 시점 | `.local/experiments/provider-permission-gating/codex-194-mcp.log` | 3,393,037바이트 | `ddcfa97f84e1f53cbf68cddd9faadd4886b12dd9064b991eb8044592958a146b` |
| 8. MCP 묻기 경로 | `.local/experiments/provider-permission-gating/codex-194-prompt-path.log` | 3,402,504바이트 | `a4fe82296bb2c8fe0e54abd4650b907a6c6aa6848ce18e53c0677f5963f71d18` |
| 9. MCP 승인 요청 원인 찾기 | `.local/experiments/provider-permission-gating/codex-194-rootcause.log` | 8,957,152바이트 | `0c1d62712dee0d541ba62ef119eb52aa9b22c328f9c09a4aef646f04e5066d13` |

로그는 Codex 에이전트(`gpt-5.6-luna`)의 실행 전체 기록이다. 에이전트가 읽은 파일, 실행한 명령, 만든 드라이버 코드, 드라이버 출력이 들어 있다. 로그에서 확인한 시험 thread의 모델 이름은 [env.json](../env.json)에 적었다.

실험 9는 에이전트 로그 외에 폴더 하나가 더 있다. 저장소 루트 기준 `.local/experiments/provider-permission-gating/rootcause/`(같은 비공개 폴더)이고 다음이 들어 있다.

| 경로(`rootcause/` 기준) | 내용 | SHA-256 |
|---|---|---|
| `report.md` | 에이전트 보고서 원본. `raw/codex-194-rootcause-report.md`와 같다. | `e5eb57d37bf65d366c554a041d6144d8ae72c41943dba625e88ea69b898a280e` |
| `experiment/run_trial.py` | 회차 드라이버 | `2c7ceb4aa87a901c6b00fe3c2e763cfe1c58667ccb7565821059430cbb5a0818` |
| `experiment/mcp_prompt_server.py` | 테스트 MCP 서버(`write_like_tool`) | `59ed9864ac40de34f7aa47f7e13ffe5661bfeed75a96666cc75c9fe421697354` |
| `experiment/run_all_requests.py`, `run_matrix.py`, `split_and_summarize.py` | 후보 처리 실행기, 회차 실행기, 로그 분할과 요약 | 해시 목록 파일 참조 |
| `experiment/schema/` | 설치된 0.158.0 바이너리가 생성한 app-server schema(파일 440개) | 해시 목록에 넣지 않았다. 생성물이라 같은 버전에서 다시 만들 수 있다. |
| `codex-home/config.toml`, `codex-home/rules/default.rules` | 전용 `CODEX_HOME` 설정과 규칙. 인증 파일과 링크는 없다. | 해시 목록 파일 참조 |
| `experiment-logs/baseline/`, `experiment-logs/all_requests/` | 회차별 원문 JSON-RPC와 stderr(파일 22개) | 해시 목록 파일 참조 |
| `experiment-logs/preliminary-*.jsonl`, `preflight-failure-1.jsonl` | 준비 실행과 수동 종료한 불완전 로그 | 해시 목록 파일 참조 |
| `experiment-logs/results.jsonl`, `thread-summary.json`, `tool-calls.jsonl` | 결과 요약 JSON과 fixture 호출 기록. 앞 둘은 `raw/`에 사본이 있다. | 해시 목록 파일 참조 |
| `experiment-logs/split/` | 원문 로그를 thread별로 나눈 것. 로그에서 다시 만들 수 있어 해시 목록에 넣지 않았다. | 해당 없음 |

회차별 원문 로그에는 계정과 의존 패키지 정보가 있을 수 있어 저장소에 넣지 않았다. 파일별 SHA-256은 [`raw/codex-194-rootcause-private-sha256.txt`](raw/codex-194-rootcause-private-sha256.txt)에 있다. 비공개 폴더가 있는 장치에서 `cd .local/experiments/provider-permission-gating && shasum -a 256 -c <해시 목록 파일의 절대 경로>`로 확인한다.

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/codex-194-home-prompt.md` | 실험 5 지시서 | 해당 없음(에이전트에게 준 지시서 사본) |
| `raw/codex-194-home-report.md` | 실험 5 보고서 | 해당 없음(에이전트 작성) |
| `raw/codex-194-plug-prompt.md` | 실험 6 지시서 | 해당 없음 |
| `raw/codex-194-plug-report.md` | 실험 6 보고서 | 해당 없음 |
| `raw/codex-194-mcp-prompt.md` | 실험 7 지시서 | 해당 없음 |
| `raw/codex-194-mcp-report.md` | 실험 7 보고서 | 해당 없음 |
| `raw/codex-194-prompt-path-prompt.md` | 실험 8 지시서 | 해당 없음 |
| `raw/codex-194-prompt-path-report.md` | 실험 8 보고서 | 해당 없음 |
| `raw/codex-194-rootcause-prompt.md` | 실험 9 지시서 | 해당 없음 |
| `raw/codex-194-rootcause-report.md` | 실험 9 보고서 | 해당 없음(에이전트 작성) |
| `raw/codex-194-rootcause-results.jsonl` | 실험 9 회차별 결과 요약(30행: baseline 10, `all_requests` 20) | `experiment/run_trial.py`(비공개 폴더) |
| `raw/codex-194-rootcause-thread-summary.json` | 실험 9 thread별 요약(72개 묶음, 모델 호출 36회) | `experiment/split_and_summarize.py`(비공개 폴더) |
| `raw/codex-194-rootcause-private-sha256.txt` | 실험 9 비공개 폴더 파일의 SHA-256 목록 | 해당 없음(`shasum -a 256`) |
| `raw/key-lines.md` | 실행 로그 5개의 핵심 줄 발췌(줄 번호 표기) | 해당 없음(손으로 발췌) |
| `SHA256SUMS` | `raw/` 파일의 SHA-256 | 해당 없음(`shasum -a 256`) |

복사한 지시서, 보고서, 결과 요약 JSON은 원본과 바이트가 같다. `raw/key-lines.md`는 로그에서 줄을 옮겨 적은 것이고 긴 줄은 `...`로 줄였다. `scripts/`, `run.sh`, `results/`, `processed/`는 원본이 없어 만들지 않았다. 재현 절차는 [설계](../design.md)의 실험별 방법 표다.

## 필드

실험 1~8은 JSON Lines나 CSV 원자료가 없어 정의할 필드가 없다. 보고서 표의 회차별 관측 칸이 이슈 댓글과 에이전트 보고서에서 옮긴 값이다. 실험 9의 결과 요약은 다음 필드를 가진다.

`raw/codex-194-rootcause-results.jsonl`은 한 줄이 한 회차다.

| 필드 | 의미 |
|---|---|
| `mode` | 처리 방식. `baseline`은 승인 요청 3종(`mcpServer/elicitation/request`, 명령, 파일)에만 응답, `all_requests`는 모든 서버 요청에 응답 |
| `phase`, `number` | 호스트의 응답(`decline`, `accept`)과 회차 번호 |
| `decision` | 호스트가 elicitation에 보낸 `action` |
| `tool_attempt` | `mcpToolCall` 시작 또는 elicitation 도착이 있었는지 |
| `approval_request_count`, `approval_request_methods` | 승인 요청 수와 이름 |
| `server_requests`, `response_count`, `unanswered_server_requests` | 서버 요청별 응답 여부, 응답 수, 미응답 요청 |
| `fixture_log_lines` | fixture 서버의 `tool-calls.jsonl`에 남은 호출 기록 |
| `turn_status`, `turn_completion_signal` | turn 상태와 완료 신호 |
| `error` | 드라이버 오류. 150초 마감이면 `timed out waiting for app-server` |
| `message_count`, `model_calls`, `raw_log` | 회차 메시지 수, 모델 호출 수, 원문 로그 경로(삭제된 실험 worktree 경로) |

`raw/codex-194-rootcause-thread-summary.json`은 최상위에 `thread_group_count`(72)와 `model_call_count`(36), 그리고 로그를 thread 단위로 나눈 `summaries` 배열을 가진다. 항목마다 `source`(원문 로그 파일), `segment`, `thread_id`, `tool_attempt`, `approval_request_count`, `approval_request_methods`, `approval_actions`, `unanswered_server_requests`, `turn_completed`, `capture_duration_ms`, `mcp_item_to_elicitation_ms`(`mcpToolCall` 시작부터 elicitation 도착까지의 ms), `methods`(메서드별 메시지 수)를 담는다.
