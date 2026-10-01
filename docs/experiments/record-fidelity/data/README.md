# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 공개 합성 작업 24개(시드 199)를 Codex(`gpt-6-sol`, `codex app-server`)와 Claude Code(`claude-opus-5-5`, stream-json)가 각각 실제로 진행했다. engine의 `record-fidelity` 예제가 provider 연결로 지시문을 한 번 보내고, `scripts/shim.py`가 오간 줄을 저장했다. |
| 수집 기간 | 2026-10-02~2026-10-02 |
| 개수 | 세션 48개(작업 24 × provider 2), 줄 수는 `raw/` 파일마다 세션 길이에 따라 다르다. |
| 표본 여부 | 전수. 제외한 세션은 없다. |
| 라벨 | 정답 근거 사실과 항목은 `scripts/tasks.py`가 작업을 만들 때 함께 만든다. 사람이 붙인 라벨은 없다. |
| 알려진 문제 | Claude Code는 같은 작업에서 설정 파일을 수정하기 전에 먼저 읽어 읽기 호출이 118개(Codex 96개)다. 저장 전 `clean`이 개인 설정과 계정 줄을 지우므로 `raw/`는 provider가 낸 줄 전체가 아니다. |
| 개인정보 | 작업 폴더, 홈 경로, 호스트 이름, 사용자 이름은 `/work`, `/home/user`, `host`, `user`로 바꿨다. skills 목록, MCP 서버, 훅, 계정과 한도 알림, 설치 식별자는 저장하지 않았다. 작업은 모두 합성이라 비공개 내용이 없다. |
| 라이선스 | 저장소 라이선스를 따른다. |

수집 중 사용률(`claude -p /usage`의 주간 전체 모델, Codex 최근 session의 `rate_limits.primary`)은 다음과 같다. 사전 시험 전 Claude 주간 24%, 5시간 창 40%, Codex 주간 9%였고, 1단계 수집 시작 때 Claude 주간 25%, 5시간 창 44%, Codex 주간 9%였으며, 끝났을 때 Claude 주간 26%, 5시간 창 50%, Codex 주간 10%였다. 같은 계정을 쓰는 다른 실험의 사용이 섞여 있다.

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/claude-20261001T174816Z-025352f.jsonl` | Claude Code 세션 24개의 원시 줄 | `scripts/01-collect.py` |
| `raw/codex-20261001T174816Z-025352f.jsonl` | Codex 세션 24개의 원시 줄 | `scripts/01-collect.py` |
| `processed/flow.csv` | 작업과 provider별 세션 성공 여부 | `scripts/02-process.py` |
| `processed/facts.csv` | 두 provider가 모두 거친 근거 사실별 포착과 패킷 포함 | `scripts/02-process.py` |
| `processed/items.csv` | 두 provider가 모두 거친 항목별 도구 종류, 경로, 메모 | `scripts/02-process.py` |
| `processed/packets.csv` | 패킷 조건별 토큰 수와 넣은 기록 수 | `scripts/02-process.py` |
| `processed/activity.csv` | 기록의 도구 종류별 수와 명령 감싸기 수 | `scripts/02-process.py` |
| `SHA256SUMS` | `raw/` 파일의 SHA-256 | `run.sh` |

## 필드

### `raw/*.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T174816Z-025352f` |
| `trial_id` | string | 없음 | 필수 | provider와 작업 | `codex-t01` |
| `condition` | string | 없음 | `codex`, `claude` | 기록을 만든 provider | `codex` |
| `ts_utc` | datetime | UTC | 필수 | 세션 시작 시각 | `2026-10-01T17:48:17+00:00` |
| `task_id` | string | 없음 | 필수 | 합성 작업 | `t01` |
| `dir` | string | 없음 | `in`, `out`, `meta` | engine이 provider에 보낸 줄, provider가 낸 줄, 세션 종료 코드 줄 | `out` |
| `line` | string | 없음 | 필수 | 저장한 줄 원문 | `{"id":1,"result":{}}` |

### `processed/facts.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `task_id` | string | 없음 | 필수 | 작업 | `t01` |
| `provider` | string | 없음 | `codex`, `claude` | 기록을 만든 provider | `codex` |
| `fact` | string | 없음 | 필수 | 근거 사실 id | `r1-0` |
| `item` | string | 없음 | 필수 | 사실이 든 항목 | `r1` |
| `captured` | integer | 없음 | 0, 1 | 값이 Saturn 기록의 도구 결과에 든 여부 | `1` |
| `in_packet_rrf_only` | integer | 없음 | 0, 1 | `rrf-only` 패킷에 값이 든 여부 | `0` |
| `in_packet_judge_only` | integer | 없음 | 0, 1 | `oracle` 패킷에 값이 든 여부 | `1` |

### `processed/items.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `task_id` | string | 없음 | 필수 | 작업 | `t01` |
| `provider` | string | 없음 | `codex`, `claude` | 기록을 만든 provider | `claude` |
| `item` | string | 없음 | 필수 | 항목 id | `c2` |
| `truth_kind` | string | 없음 | `FileRead`, `Shell`, `FileEdit`, `TestRun` | 정답 도구 종류 | `Shell` |
| `captured` | integer | 없음 | 0, 1 | 대응하는 기록의 존재 여부 | `1` |
| `kind_ok` | integer | 없음 | 0, 1 | 도구 종류와 정답의 일치 여부 | `1` |
| `path_ok` | integer | 없음 | 0, 1, 빈 칸 | 경로 추출. 경로가 없는 항목은 빈 칸 | `0` |
| `memo_ok` | integer | 없음 | 0, 1 | 메모 필드를 모두 정답으로 얻은 여부 | `1` |
