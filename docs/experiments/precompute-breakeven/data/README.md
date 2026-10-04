# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `scripts/01-collect.py`가 이 컴퓨터의 Claude Code 기록과 Codex 기록을 한 번 읽어 session별 숫자만 뽑았다. |
| 수집 기간 | 2026-10-01~2026-10-01. 원 기록의 날짜는 수집하지 않았다. |
| 개수 | 1154 session, 11330 턴 |
| 표본 여부 | 전수. subagent 기록, 턴 없는 session, 맥락 토큰 없는 session, 중복 session은 뺐다. |
| 라벨 | 없음 |
| 알려진 문제 | 맥락 토큰은 모델 사용량 보고로 근사했다. 다른 id로 이어 쓴 session은 중복으로 남을 수 있다. |
| 개인정보 | 원문, 경로, 프로젝트 이름, 파일 이름, 시각을 넣지 않았다. session id는 실행마다 새로 만든 무작위 소금과 원 id를 SHA-256으로 해시한 앞 16자이고, 소금은 저장하지 않아 원 기록과 다시 이을 수 없다. 줄 순서는 해시 순서다. |
| 라이선스 | 저장소 라이선스를 따른다. |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/agent-logs-20261001T105302Z-0069654.jsonl` | session마다 한 줄. 턴별 도구 호출 수, 도구 입출력 글자 수, 맥락 토큰, provider 압축 여부 | `scripts/01-collect.py` |
| `raw/agent-logs-20261001T105302Z-0069654.meta.json` | provider별 읽은 파일 수, 제외 수, 실패 수 | `scripts/01-collect.py` |
| `processed/turns.csv` | 턴마다 한 행으로 편 표 | `scripts/02-process.py` |
| `SHA256SUMS` | `raw/` 파일의 SHA-256 | `run.sh collect` |

## 필드

### `raw/agent-logs-20261001T105302Z-0069654.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T105302Z-0069654` |
| `trial_id` | string | 없음 | 필수, 고유 | `session_id`와 같은 값 | `0a1b2c3d4e5f6a7b` |
| `condition` | string | 없음 | `observed` | 관측 자료라는 표시 | `observed` |
| `ts_utc` | datetime | 없음 | 필수 | 수집 시각. 모든 행이 같다. | `2026-10-01T10:53:02Z` |
| `session_id` | string | 없음 | 필수, 고유, 16자 16진수 | 익명 session id | `0a1b2c3d4e5f6a7b` |
| `provider` | string | 없음 | `claude`, `codex` | 기록을 만든 provider | `codex` |
| `turns` | array | 없음 | 1개 이상 | 턴 순서대로 놓은 턴 객체 | `[{...}]` |
| `turns[].tool_calls` | integer | 개 | 0 이상 | 메인 에이전트의 도구 호출 수 | `4` |
| `turns[].tool_io_chars` | integer | 글자 | 0 이상 | 도구 호출 인자와 결과의 글자 수 합 | `12034` |
| `turns[].context_tokens` | integer | 토큰 | 0 이상, 결측 `null` | 턴의 마지막 모델 응답이 보고한 입력과 출력 토큰 합 | `84752` |
| `turns[].compacted` | boolean | 없음 | 필수 | 턴 안에서 provider 압축이 일어났는지 | `false` |

### `processed/turns.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T105302Z-0069654` |
| `session_id` | string | 없음 | 필수 | 익명 session id | `0a1b2c3d4e5f6a7b` |
| `provider` | string | 없음 | `claude`, `codex` | provider | `claude` |
| `turn` | integer | 없음 | 1 이상 | session 안 턴 번호 | `3` |
| `tool_calls` | integer | 개 | 0 이상 | `turns[].tool_calls` | `4` |
| `tool_io_chars` | integer | 글자 | 0 이상 | `turns[].tool_io_chars` | `12034` |
| `context_tokens` | integer | 토큰 | 결측은 빈 칸 | `turns[].context_tokens` | `84752` |
| `compacted` | integer | 없음 | 0, 1 | `turns[].compacted` | `0` |
