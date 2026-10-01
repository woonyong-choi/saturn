# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `scripts/01-collect.py`가 앞 입력 서로 다른 문장 209개의 등록 판정을 먼저 기준 judge `jev-1.13.0`에 묻고, 입력 350건을 조건 5개로, 쌍 320건을 `replaces_1`로 한 번씩 물어 답을 그대로 저장했다. |
| 수집 기간 | 2026-10-01~2026-10-01 |
| 개수 | 2279건(등록 판정 209건, 입력 1750건, 쌍 320건) |
| 표본 여부 | 평가 세트 전수 |
| 라벨 | 평가 세트를 쓴 사람이 사전 등록 라벨 기준표로 붙였다. 라벨은 `eval/`에만 있고 수집 파일에는 없다. |
| 알려진 문제 | 입력 조건 `oracle_flag`의 두 건(`in-341` 시간 초과, `in-079` HTTP 520)은 `no_answer`다. 429, 529로 다시 보낸 횟수는 기록하지 않았다. |
| 개인정보 | 없음. 평가 세트는 합성 한국어 문장이다. |
| 라이선스 | 저장소 라이선스를 따른다. |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/jev-20261001T141638Z-056c173.jsonl` | 시행마다 judge 답 한 줄 | `scripts/01-collect.py` |
| `SHA256SUMS` | 수집 파일과 평가 세트의 SHA-256 | `scripts/01-collect.py` |
| `processed/registrations.csv` | 앞 입력 문장별 등록 판정 답과 정답 | `scripts/02-process.py` |
| `processed/inputs.csv` | 입력 항목과 조건별 라벨, 답, 0.7 기준 판정 | `scripts/02-process.py` |
| `processed/pairs.csv` | 쌍 항목별 라벨, 답, 구간 | `scripts/02-process.py` |

## 필드

### `raw/jev-20261001T141638Z-056c173.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T141638Z-056c173` |
| `trial_id` | string | 없음 | 필수, 고유 | 묻은 순서의 시행 번호 | `t-0001` |
| `condition` | string | 없음 | `registration`, `baseline`, `flag`, `flag_text`, `oracle_flag`, `flag_hint`, `replaces` | 조건 | `flag` |
| `ts_utc` | datetime | 없음 | 필수 | 답을 받은 UTC 시각 | `2026-10-01T14:16:39Z` |
| `item_id` | string | 없음 | 등록 판정이면 앞 입력 원문, 입력은 `in-001`..`in-350`, 쌍은 `pr-001`..`pr-320` | 평가 세트 항목 | `pr-001` |
| `question_set` | string | 없음 | `route@1.1`, `route@1.2-draft`, `constraint@1.0` | 질문 세트와 버전 | `route@1.1` |
| `question_id` | string | 없음 | `is_constraint`, `is_constraint_hint`, `replaces_1` | 질문 식별자 | `replaces_1` |
| `status` | string | 없음 | `ok`, `invalid`, `no_answer` | 답 상태 | `ok` |
| `http_status` | integer | 없음 | 응답이 없으면 `null` | HTTP 상태 코드 | `200` |
| `answer` | number | 확률 | 0~1, 결측이면 `null` | 답의 `noul` 확률 | `0.93` |
| `model` | string | 없음 | 응답이 없으면 `null` | 응답이 알린 모델 | `jev-1.13.0` |
| `latency_ms` | integer | ms | 0 이상 | 요청을 보낸 뒤 답을 받기까지 걸린 시간 | `812` |
| `error` | string | 없음 | 정상이면 `null` | 실패 원인 요약 | `timeout` |

### `processed/registrations.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `previous_text` | string | 없음 | 고유 | 앞 입력 원문 | `에러 메시지는 영어로 통일해` |
| `trial_id` | string | 없음 | 시행이 없으면 빈 칸 | 수집 파일의 시행 | `t-0042` |
| `status` | string | 없음 | `ok`, `invalid`, `no_answer`, `missing` | 답 상태 | `ok` |
| `answer` | number | 확률 | 0~1, 결측이면 빈 칸 | 등록 판정 질문 답 | `0.91` |
| `registered` | integer | 없음 | 0, 1 | 답이 0.7 이상이면 1 | `1` |
| `label` | integer | 없음 | 0, 1 | 앞 입력이 제약인지 정답 | `1` |

### `processed/inputs.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `item_id` | string | 없음 | 조건과 함께 고유 | 입력 항목 | `in-001` |
| `trial_id` | string | 없음 | 시행이 없으면 빈 칸 | 수집 파일의 시행 | `t-0042` |
| `condition` | string | 없음 | `baseline`, `flag`, `flag_text`, `oracle_flag`, `flag_hint` | 조건 | `baseline` |
| `category` | string | 없음 | `constraint`, `mixed`, `request`, `indirect-constraint`, `indirect-request` | 평가 세트 구간 | `constraint` |
| `indirect` | integer | 없음 | 0, 1 | 간접 지시 여부 | `0` |
| `origin` | string | 없음 | `121`, `new` | #121 평가 세트 항목인지 새 항목인지 | `new` |
| `label` | integer | 없음 | 0, 1 | 제약 라벨 | `1` |
| `status` | string | 없음 | `ok`, `invalid`, `no_answer`, `missing` | 답 상태 | `ok` |
| `answer` | number | 확률 | 0~1, 결측이면 빈 칸 | `is_constraint` 답 | `0.91` |
| `predicted` | integer | 없음 | 0, 1 | 답이 0.7 이상이면 1 | `1` |
| `latency_ms` | integer | ms | 시행이 없으면 빈 칸 | 응답 시간 | `241` |

### `processed/pairs.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `item_id` | string | 없음 | 고유 | 쌍 항목 | `pr-001` |
| `trial_id` | string | 없음 | 시행이 없으면 빈 칸 | 수집 파일의 시행 | `t-0007` |
| `category` | string | 없음 | `direct-replace`, `indirect-replace`, `partial`, `compatible`, `indirect-compatible` | 평가 세트 구간 | `direct-replace` |
| `indirect` | integer | 없음 | 0, 1 | 간접 지시 여부 | `0` |
| `origin` | string | 없음 | `121`, `new` | #121 평가 세트 항목인지 새 항목인지 | `new` |
| `label` | string | 없음 | `replaces`, `partial`, `compatible` | 대체 라벨 | `replaces` |
| `status` | string | 없음 | `ok`, `invalid`, `no_answer`, `missing` | 답 상태 | `ok` |
| `answer` | number | 확률 | 0~1, 결측이면 빈 칸 | `replaces_1` 답 | `0.93` |
| `band` | string | 없음 | `replace`, `possible`, `none` | 0.8, 0.5 기준 구간 | `replace` |
| `latency_ms` | integer | ms | 시행이 없으면 빈 칸 | 응답 시간 | `241` |
