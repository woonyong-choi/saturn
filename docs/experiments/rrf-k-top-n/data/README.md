# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `scripts/01-collect.py`가 Saturn 저장소 개발 중 Claude Code 메인 session 기록에서 입력 지점을 뽑고, 직전 도구 호출을 후보로 채널 값을 계산한 뒤 judge `jev-1.13.0`에 후보마다 `call_<id>_keep`, `result_<id>_keep`을 물었다. |
| 수집 기간 | 2026-10-01~2026-10-01 |
| 개수 | 후보 세트 50개(session 10개), 후보 5,943개, 재판단 행 포함 6,521행 |
| 표본 여부 | 시드 116으로 섞은 입력 지점에서 session당 10개까지, 설계의 중단 규칙대로 뽑은 표본이다. |
| 라벨 | 사람 라벨 없음. `kept`는 judge 전체 판단에서 `max(p_call, p_result) ≥ 0.5`로 스크립트가 정했다. |
| 알려진 문제 | 같은 session의 세트는 후보가 겹친다. `kept`가 일부 세트에 몰렸다. |
| 개인정보 | 저장소 파일에는 입력 원문, 도구 인자와 결과, judge 질문이 없다. session과 세트 식별자는 파일 이름의 SHA-256 앞 12자다. 원문은 저장소 밖 비공개 위치에만 두고 SHA-256만 아래 표에 적는다. |
| 라이선스 | Saturn 저장소와 같은 조건 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/claude-20261001T111152Z-8adcb79.jsonl` | 후보별 judge 확률과 채널 값. 한 줄에 후보 하나와 판단 회차 하나 | `scripts/01-collect.py` |
| `SHA256SUMS` | raw 파일의 SHA-256 | `run.sh collect` |
| `processed/channel-ranks.csv` | 1회차 후보별 `kept`, 채널 순위, 남김 확률 | `scripts/02-process.py` |
| `processed/replicas.csv` | 두 번 판단한 5세트의 후보별 1회차와 2회차 `kept` | `scripts/02-process.py` |
| `processed/flow.csv` | 단계별 세트 수 | `scripts/02-process.py` |

| 비공개 파일 | SHA-256 |
|---|---|
| judge 요청의 state, 질문, 응답 원문 `claude-20261001T111152Z-8adcb79.jsonl` | `dda4d29b527921149a2b00c1198badd90845eec4067aa6549912ba41188fc9c2` |

## 필드

### `raw/claude-20261001T111152Z-8adcb79.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T111152Z-8adcb79` |
| `trial_id` | string | 없음 | 필수 | 후보 세트 id. session 파일 이름과 입력 번호의 SHA-256 앞 12자 | `79fda4b6f423` |
| `condition` | string | 없음 | 필수, `full-judge` | 조건. judge가 후보 전체를 판단한 결과 | `full-judge` |
| `repeat` | integer | 회 | 필수, 1~2 | 판단 회차. 2는 처음 5세트의 재판단 | `1` |
| `ts_utc` | datetime | 없음 | 필수 | 세트 판단 시작 시각(UTC) | `2026-10-01T11:11:53Z` |
| `candidate_id` | string | 없음 | 필수, 회차 안에서 고유 | 후보 id. `{trial_id}-{record_no}` | `79fda4b6f423-001` |
| `session_id` | string | 없음 | 필수 | session 파일 이름의 SHA-256 앞 12자 | `0afd8613df00` |
| `status` | string | 없음 | 필수, `ok`, `invalid`, `failed` | 세트의 judge 응답 상태 | `ok` |
| `p_call` | number | 확률 | 0~1, 응답 없으면 `null` | `call_<id>_keep`의 P(yes) | `0.43` |
| `p_result` | number | 확률 | 0~1, 응답 없으면 `null` | `result_<id>_keep`의 P(yes) | `0.53` |
| `record_no` | integer | 번 | 필수, 1부터 | 세트 안 기록 순서. 클수록 최근 | `1` |
| `has_paths` | boolean | 없음 | 필수 | 도구 인자에서 경로를 찾았는지 | `true` |
| `file_overlap` | integer | 개 | 필수, 0 이상 | 기준 파일과 겹치는 경로 수 | `1` |
| `bm25` | number | 없음 | 필수, 0 이상 | 마지막 입력과의 BM25 점수 | `0.762084` |

### `processed/channel-ranks.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T111152Z-8adcb79` |
| `trial_id` | string | 없음 | 필수 | 후보 세트 id | `012e72416ea7` |
| `session_id` | string | 없음 | 필수 | session id | `ed4a90b00da8` |
| `candidate_id` | string | 없음 | 필수, 고유 | 후보 id | `012e72416ea7-001` |
| `record_no` | integer | 번 | 필수 | 세트 안 기록 순서 | `1` |
| `kept` | integer | 없음 | 필수, 0 또는 1 | `max(p_call, p_result) ≥ 0.5` | `0` |
| `has_paths` | integer | 없음 | 필수, 0 또는 1 | 도구 인자에서 경로를 찾았는지 | `1` |
| `rank_file` | integer | 순위 | 1 이상, 겹침 0이면 빈 칸 | 파일 겹침 순위 | `23` |
| `rank_word` | integer | 순위 | 1 이상, 점수 0이면 빈 칸 | 단어 겹침 순위 | `28` |
| `rank_recent` | integer | 순위 | 필수, 1 이상 | 최근성 순위 | `85` |
| `p_keep` | number | 확률 | 필수, 0~1 | `max(p_call, p_result)` | `0.29` |

### `processed/replicas.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `trial_id` | string | 없음 | 필수 | 후보 세트 id | `5319b2112b47` |
| `candidate_id` | string | 없음 | 필수, 고유 | 후보 id | `5319b2112b47-001` |
| `kept_first` | integer | 없음 | 필수, 0 또는 1 | 1회차 `kept` | `0` |
| `kept_second` | integer | 없음 | 필수, 0 또는 1 | 2회차 `kept` | `0` |

### `processed/flow.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `stage` | string | 없음 | 필수, `collected`, `analyzed`, `excluded_{상태}` | 단계 | `analyzed` |
| `sets` | integer | 세트 | 필수, 0 이상 | 세트 수 | `50` |
