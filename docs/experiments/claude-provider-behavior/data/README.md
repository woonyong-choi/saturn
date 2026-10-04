# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 공식 `claude` 2.1.288을 stream-json 입출력으로 띄워 회차마다 사용자 턴을 보내고, 이벤트, `can_use_tool` 요청, 제어 응답, marker 파일, 훅 래퍼 기록, 프로세스 표를 기록했다. `json_sub_fg`만 `--output-format json`으로 한 번 실행했다. |
| 수집 기간 | 2026-10-04~2026-10-04 |
| 개수 | 회차 63행(조건 21개 곱하기 3회), 사용자 턴 72회 |
| 표본 여부 | 전수. 설계의 조건 곱하기 3회 |
| 라벨 | 판정은 `scripts/02-process.py`, `scripts/03-analyze.py`가 raw 이벤트와 관측으로 계산했다. 사람이 고치지 않았다. |
| 알려진 문제 | 이벤트의 글은 앞 300자만 남겼다. `bg_*` 회차에서 하위 에이전트가 지시한 `sleep` 명령을 백그라운드 셸 작업으로 바꿔 돌렸다. 메시지 줄의 출력 토큰은 스트리밍 도중 값이다. |
| 개인정보 | 홈 경로는 `~`, worktree 경로는 `<worktree>`로 바꿨다. `initialize` 응답의 계정과 명령 목록, 사용자 skill 이름은 저장하지 않았다. 키 저장소 시험 값은 가짜 항목(`saturn-test-dummy`)의 무작위 값이고 시험 뒤 항목을 삭제했다. |
| 라이선스 | 저장소 라이선스를 따른다. |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/claude-20261004T083629Z-9fc9429.jsonl` | meta 행 1개와 회차 행 63개 | `scripts/01-collect.py` |
| `processed/metrics.csv` | 회차별 지표 한 표(이슈별 열이 한 표에 있고 해당 없는 칸은 빈 칸) | `scripts/02-process.py` |

## 필드

### `raw/claude-20261004T083629Z-9fc9429.jsonl` 회차 행

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `kind` | string | 없음 | `meta`, `trial` | 행 종류 | `trial` |
| `run_id` | string | 없음 | 필수 | 실행 id | `20261004T083629Z-9fc9429` |
| `condition` | string | 없음 | 21개 조건 이름 | 조건 | `sub_bg` |
| `trial_id` | integer | 없음 | 1~3 | 조건 안 회차 | `1` |
| `ts_utc` | datetime | UTC | 필수 | 회차 끝 시각 | `2026-10-04T08:36:55Z` |
| `calls` | array | 없음 | 호출 순번 | 이 회차가 쓴 호출 순번 | `[1]` |
| `obs` | object | 없음 | 필수 | 결과 수, marker 시각, 종료 시각, 훅 기록, 프로세스 표, 제어 응답 시각 | |
| `requests` | array | 없음 | 필수 | 도착한 `can_use_tool` 요청과 driver 응답 | |
| `events` | array | 없음 | 필수 | 줄여 저장한 stream 이벤트. `at`은 회차 시작 기준 초 | |

### `processed/metrics.csv`

회차마다 한 행이다. 열 이름은 `scripts/02-process.py`의 `metrics()`가 정한다. 불리언은 `True`, `False`, 결측은 빈 칸이다. 주요 열은 `condition`, `trial_id`, `calls`, `results`, `marker_at`(초), `trigger_at`, `stop_at`, `exit_at`(초), `r1_usage_in`/`cc`/`cr`/`out`(`result.usage` 4칸), `r1_main_*`/`r1_sub_*`(메시지 줄 합), `hook_bash_agent_denied`, `leak_tool_result`, `init1_permission`이다.
