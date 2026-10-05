# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `scripts/01-collect.py`가 요청 목록을 섞은 순서로 기준 judge `jev-1.13.0`에 하나씩 보내 요청과 응답 원문을 시도마다 한 줄로 저장했다. |
| 수집 기간 | 2026-10-05~2026-10-05(UTC) |
| 개수 | 개발 300줄, 확인 2,331줄(시행 2,330건과 재시도 1건) |
| 표본 여부 | 개발은 기존 평가 세트의 `origin` 121 항목에서 범주 비례로 뽑은 입력 20건과 쌍 20건, 확인은 `origin` new 항목 전수(입력 150건, 쌍 160건)와 그 안에서 뽑은 변형·선택지 순서 항목. 표본 추출 규칙은 `design.md` |
| 라벨 | 평가 세트의 라벨은 `indirect-constraint-accuracy/eval`에 있고 수집 파일에는 없다. 변형 정답은 `eval/variants.json`에 쓴 사람이 사전에 정했다. |
| 알려진 문제 | 확인에서 한 시행이 HTTP 529를 받아 다시 보냈다. 응답 확률은 소수 둘째 자리로 반올림되어 있다. |
| 개인정보 | 없음. 평가 세트는 합성 한국어 문장이다. 키 값은 저장하지 않았고 일치 문자열은 0건이다. |
| 라이선스 | 저장소 라이선스를 따른다. |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/dev.jsonl` | 개발 시도마다 요청 원문과 응답 원문 | `scripts/01-collect.py` |
| `raw/confirm.jsonl` | 확인 시도마다 요청 원문과 응답 원문 | `scripts/01-collect.py` |
| `processed/trials.csv` | 시행마다 마지막 시도의 답, 행동 구간, 확신도, usage | `scripts/02-process.py` |
| `processed/legacy_152.csv` | 2026-10-01 #152 수집의 확인 항목 기준 조건 답 | `scripts/02-process.py` |
| `SHA256SUMS` | 수집 파일, 평가 세트, 질문, 변형, #152 원자료의 SHA-256. `shasum -a 256`으로 만든다. | 직접 |

## 필드

### `raw/dev.jsonl`, `raw/confirm.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `trial_id` | string | 없음 | 필수, 시도가 여럿이면 같은 값 | 단계, 묶음, 역할, 항목, 반복으로 만든 시행 식별자 | `confirm/identical/replaces_1/pr-290/r3` |
| `phase` | string | 없음 | `dev`, `confirm` | 단계 | `confirm` |
| `group` | string | 없음 | `identical`, `neutral_insert`, `injection_insert`, `paraphrase`, `flip`, `order` | 묶음 | `identical` |
| `role` | string | 없음 | `is_constraint`, `replaces_1`, `relation_choice` | 질문 역할 | `replaces_1` |
| `item_id` | string | 없음 | 필수 | 평가 세트 항목 | `pr-290` |
| `variant` | string | 없음 | 필수 | 원문 `original` 또는 변형 이름 | `original` |
| `rep` | integer | 회 | 1 이상 | 같은 항목·묶음 안의 반복 번호 | `3` |
| `option_order` | array | 없음 | `relation_choice`만, 아니면 `null` | 선택지 나열 순서 | `["compatible","partly_limited","replaced"]` |
| `attempt` | integer | 회 | 1 이상 | 이 시행의 시도 번호 | `2` |
| `final` | boolean | 없음 | 필수 | 이 줄이 시행의 마지막 시도인지 | `true` |
| `request` | object | 없음 | 필수 | 보낸 요청 본문 | `{"model":"jev-1.13.0",...}` |
| `request_sha256` | string | 없음 | 필수 | 직렬화한 요청 바이트의 SHA-256 | `cdbcb01a...` |
| `ts_utc` | datetime | 없음 | 필수 | 답을 받은 UTC 시각 | `2026-10-05T18:45:13.385278Z` |
| `http_status` | integer | 없음 | 응답이 없으면 `null` | HTTP 상태 코드 | `200` |
| `latency_ms` | integer | ms | 필수 | 보낸 뒤 응답까지 걸린 시간 | `391` |
| `error` | string | 없음 | 성공이면 `null` | 오류 종류 | `http 529` |
| `response` | object | 없음 | 응답이 JSON이 아니면 `null` | 응답 원문 | `{"model":"jev-1.13.0","answers":{...},"usage":{...}}` |
| `response_text` | string | 없음 | `response`가 있으면 `null` | JSON이 아닌 응답 본문 | `null` |

### `processed/trials.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `trial_id`, `phase`, `group`, `role`, `item_id`, `variant`, `rep` | string, integer | 없음 | 필수 | 원자료와 같다. | `pr-290` |
| `attempts` | integer | 회 | 1 이상 | 이 시행에 쓴 시도 수 | `2` |
| `http_status` | integer | 없음 | 필수 | 마지막 시도의 상태 코드 | `200` |
| `status` | string | 없음 | `ok`, `invalid`, `failed` | 답 상태 | `ok` |
| `answer` | number | 확률 | 0~1, `noul` 역할만, 아니면 빈 칸 | 답의 `noul` 확률 | `0.05` |
| `band` | string | 없음 | `auto`, `ask`, `replace`, `possible`, `none`, 결측이면 빈 칸 | 행동 구간 | `none` |
| `top_option` | string | 없음 | `choice`만 | 최고 확률 선택지 | `compatible` |
| `confidence` | number | 없음 | 0~1, `choice`만 | 계산 확신도 | `0.97` |
| `api_confidence` | number | 없음 | 0~1, `choice`만 | 응답이 준 확신도 | `0.96` |
| `option_order` | string | 없음 | `choice`만, `\|`로 구분 | 선택지 순서 | `compatible\|replaced\|partly_limited` |
| `model` | string | 없음 | 응답이 없으면 빈 칸 | 응답 모델 | `jev-1.13.0` |
| `input_tokens`, `output_tokens` | integer | 토큰 | 응답에 usage가 없으면 빈 칸 | 응답 usage | `346` |
| `latency_ms` | integer | ms | 필수 | 마지막 시도의 지연 | `391` |
| `request_sha256` | string | 없음 | 필수 | 요청 해시 | `cdbcb01a...` |

### `processed/legacy_152.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `item_id` | string | 없음 | 확인 항목 | 평가 세트 항목 | `in-201` |
| `role` | string | 없음 | `is_constraint`, `replaces_1` | 질문 역할 | `is_constraint` |
| `status` | string | 없음 | `ok`, `invalid`, `no_answer` | 답 상태 | `ok` |
| `answer` | number | 확률 | 0~1, 결측이면 빈 칸 | 2026-10-01 답 | `0.79` |
| `model` | string | 없음 | 결측이면 빈 칸 | 응답 모델 | `jev-1.13.0` |
| `ts_utc` | datetime | 없음 | 필수 | 그 답을 받은 시각 | `2026-10-01T14:19:35Z` |
