# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 공식 `claude` 2.1.288을 stream-json 입출력으로 띄워 회차마다 사용자 턴 하나를 보내고, `can_use_tool` 요청과 도구 이벤트, marker 파일을 기록했다. |
| 수집 기간 | 2026-10-04~2026-10-04 |
| 개수 | 회차 51행(본 수집 42, 읽기 지시 재수집 9). driver 점검 호출 4회는 저장하지 않았다. |
| 표본 여부 | 전수. 경로 14개 곱하기 3회 |
| 라벨 | `classification`은 `scripts/01-collect.py`가 요청 도착과 효과로 붙였다. 사람이 고치지 않았다. |
| 알려진 문제 | 모델이 `Edit` 전에 `Read`를 하지 않으면 `Edit`이 요청 전에 도구 오류로 끝나 5회가 `unexpected`로 분류됐다. 이벤트의 글은 앞 300~400자만 남겼다. |
| 개인정보 | 홈 경로는 `~`, worktree 경로는 `<worktree>`로 바꿨다. 인증값, 계정 정보는 기록하지 않았다. |
| 라이선스 | 저장소 라이선스를 따른다. |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/claude-20261004T022621Z-a512100.jsonl` | 본 수집 42회와 meta 행 | `scripts/01-collect.py` |
| `raw/claude-20261004T023711Z-8e21cba.jsonl` | `--read-first` 재수집 9회와 meta 행 | `scripts/01-collect.py --read-first` |
| `processed/trials.csv` | 회차별 요약 한 표 | `scripts/02-process.py` |

## 필드

### `processed/trials.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261004T022621Z-a512100` |
| `variant` | string | 없음 | `main`, `read_first` | 본 수집인지 읽기 지시 재수집인지 | `main` |
| `condition` | string | 없음 | 14개 경로 이름 | 경로 | `deny_shell` |
| `trial_id` | integer | 없음 | 1~3 | 경로 안 회차 | `1` |
| `model_call_ordinal` | integer | 회 | 점검 4회 뒤 5부터 | 전역 호출 순번 | `5` |
| `ts_utc` | datetime | UTC | 필수 | 판정 시각 | `2026-10-04T02:26:35Z` |
| `turn_status` | string | 없음 | `result`, `stream_closed`, `timeout` | 턴 종료 상태 | `result` |
| `classification` | string | 없음 | 설계의 분류 | 판정 | `blocked` |
| `request_count` | integer | 회 | 0 이상 | 도착한 `can_use_tool` 수 | `1` |
| `touch_request_count` | integer | 회 | 0 이상 | 그 중 `touch` 명령 요청 수 | `1` |
| `request_tools` | string | 없음 | `|`로 구분 | 요청의 도구 이름 | `Bash` |
| `saturn_responses` | string | 없음 | `allow`, `deny`를 `|`로 구분 | driver 응답 | `deny` |
| `tools_attempted` | string | 없음 | `|`로 구분 | 모델이 낸 `tool_use` 이름 | `Bash` |
| `marker_effect` | boolean | 없음 | 비어 있을 수 있음 | marker 효과 | `False` |
| `hook_calls` | integer | 회 | 0 이상 | 훅 fixture 호출 | `0` |
| `mcp_fixture_calls` | integer | 회 | 0 이상 | MCP fixture 호출 | `0` |
| `token_seen` | boolean | 없음 | 읽기 경로만 | 도구 결과에 토큰 | `True` |
| `tool_result_errors` | string | 없음 | `|`로 구분 | 도구 결과 `is_error` | `True` |
