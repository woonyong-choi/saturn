# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `scripts/01-collect.py collect`가 커밋 `1279d40`의 파일을 `git show`로 읽어 문서를 만들고, 질의와 오타를 만든 뒤 조건마다 BM25 순위를 계산했다. |
| 수집 기간 | 2026-10-01~2026-10-01 |
| 개수 | 문서 1,174개(Markdown 절 336개, 코드 조각 838개), 질의 1,600개(언어마다 800개), 순위 행 22,400개 |
| 표본 여부 | 질의는 섞은 후보 위치에서 설계의 중단 규칙대로 뽑은 표본이다. 문서는 전수다. |
| 라벨 | 사람 라벨 없음. 정답 집합은 오타 없는 질의 문자열을 본문에 포함한 문서로 스크립트가 정했다. |
| 알려진 문제 | 같은 구절이 여러 문서에 있으면 정답 문서가 2~5개다. 코드 조각은 40줄 경계에서 함수가 잘린다. |
| 개인정보 | 없음. 공개 저장소의 문서, 코드 주석, 식별자만 담는다. |
| 라이선스 | Saturn 저장소와 같은 조건 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/corpus-20261001T105556Z-6235a29.jsonl` | 문서 집합. 한 줄에 문서 하나 | `scripts/01-collect.py` |
| `raw/index-20261001T105556Z-6235a29.jsonl` | 조건별 색인 크기. 한 줄에 조건 하나 | `scripts/01-collect.py` |
| `raw/rankings-20261001T105556Z-6235a29.jsonl` | 질의, 오타, 정답 집합, 조건별 순위. 한 줄에 질의 하나와 조건 하나 | `scripts/01-collect.py` |
| `SHA256SUMS` | raw 파일의 SHA-256 | `scripts/01-collect.py` |
| `processed/outcomes.csv` | 질의와 조건별 정답 순위, 상위 10개 적중, 1위 적중 | `scripts/02-process.py` |
| `processed/index.csv` | 조건별 색인 크기 | `scripts/02-process.py` |

## 필드

### `raw/corpus-20261001T105556Z-6235a29.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T105556Z-6235a29` |
| `ts_utc` | datetime | 없음 | 필수 | 기록 시각(UTC) | `2026-10-01T10:56:09Z` |
| `trial_id` | string | 없음 | 항상 `null` | 문서 행에는 시행이 없다. | `null` |
| `condition` | string | 없음 | 항상 `null` | 문서 행에는 조건이 없다. | `null` |
| `doc_id` | string | 없음 | 필수, 고유 | 문서 식별자. Markdown은 `{경로}#s{절 번호}`, 코드는 `{경로}#L{시작}-{끝}` | `docs/design/judge.md#s001` |
| `path` | string | 없음 | 필수 | 저장소 안 파일 경로 | `docs/design/judge.md` |
| `kind` | string | 없음 | `markdown`, `code` | 문서 종류 | `markdown` |
| `start_line` | integer | 줄 | 1 이상 | 원본 파일의 시작 줄 | `8` |
| `end_line` | integer | 줄 | `start_line` 이상 | 원본 파일의 끝 줄 | `11` |
| `text` | string | 없음 | 필수 | 색인한 본문. 코드는 주석 글과 식별자만 | `## 요약` |

### `raw/index-20261001T105556Z-6235a29.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T105556Z-6235a29` |
| `ts_utc` | datetime | 없음 | 필수 | 기록 시각(UTC) | `2026-10-01T10:56:09Z` |
| `trial_id` | string | 없음 | 항상 `null` | 색인 행에는 시행이 없다. | `null` |
| `condition` | string | 없음 | 필수, 고유 | 단어 조각 조건 | `base` |
| `vocab_size` | integer | 개 | 0 이상 | 서로 다른 조각 수 | `6527` |
| `postings` | integer | 개 | 0 이상 | 조각과 문서 쌍의 수 | `83494` |
| `total_tokens` | integer | 개 | 0 이상 | 모든 문서의 조각 수 합 | `150044` |

### `raw/rankings-20261001T105556Z-6235a29.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T105556Z-6235a29` |
| `ts_utc` | datetime | 없음 | 필수 | 기록 시각(UTC) | `2026-10-01T10:56:09Z` |
| `trial_id` | string | 없음 | 필수, 조건 안에서 고유 | `{query_id}-{variant}` | `ko-0001-typo` |
| `condition` | string | 없음 | `base`, `ko-jamo3`, `en-char4`, `ko-jamo2`, `ko-jamo4`, `ko-union`, `en-union` | 단어 조각 조건 | `base` |
| `query_id` | string | 없음 | 필수 | 입력 식별자. 언어와 번호 | `ko-0001` |
| `lang` | string | 없음 | `ko`, `en` | 질의 언어 | `ko` |
| `source_doc_id` | string | 없음 | 필수 | 질의를 자른 문서 | `docs/design/judge.md#s001` |
| `clean_text` | string | 없음 | 필수 | 오타 없는 질의 | `판단 방식 설정으로` |
| `gold_doc_ids` | array of string | 없음 | 1~5개 | 정답 집합 | `["docs/design/judge.md#s001"]` |
| `variant` | string | 없음 | `clean`, `typo` | 오타 여부 | `typo` |
| `typo_type` | string | 없음 | 오타 종류 값, `clean`이면 `null` | 넣은 오타 종류 | `jamo-del` |
| `typo_detail` | object | 없음 | `clean`이면 `null` | 오타를 넣은 위치와 자리 | `{"position": 0, "slot": "jong"}` |
| `query_text` | string | 없음 | 필수 | 순위에 쓴 질의 | `파단 방식 설정으로` |
| `n_query_tokens` | integer | 개 | 0 이상 | 질의의 서로 다른 조각 수 | `5` |
| `n_retrieved` | integer | 개 | 0 이상 | 점수가 0보다 큰 문서 수 | `387` |
| `gold_rank` | integer | 순위 | 1 이상, 순위에 없으면 `null` | 정답 집합 문서 중 가장 높은 순위 | `8` |
| `top_doc_ids` | array of string | 없음 | 0~10개 | 상위 10개 문서 | `["docs/decisions/2026-10-01-context-mode-setting.md#s003"]` |
| `top_scores` | array of number | 없음 | `top_doc_ids`와 같은 길이 | 상위 10개 BM25 점수 | `[14.604748]` |

### `processed/outcomes.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T105556Z-6235a29` |
| `query_id` | string | 없음 | 필수 | 입력 식별자 | `ko-0001` |
| `lang` | string | 없음 | `ko`, `en` | 질의 언어 | `ko` |
| `variant` | string | 없음 | `clean`, `typo` | 오타 여부 | `typo` |
| `typo_type` | string | 없음 | `clean`이면 빈 칸 | 오타 종류 | `jamo-del` |
| `condition` | string | 없음 | 조건 이름 | 단어 조각 조건 | `base` |
| `n_gold` | integer | 개 | 1~5 | 정답 집합 크기 | `1` |
| `gold_rank` | integer | 순위 | 순위에 없으면 빈 칸 | 정답 순위 | `8` |
| `hit_at_10` | integer | 없음 | 0, 1 | 정답이 10위 안이면 1 | `1` |
| `hit_at_1` | integer | 없음 | 0, 1 | 정답이 1위면 1 | `0` |
| `n_query_tokens` | integer | 개 | 0 이상 | 질의 조각 수 | `5` |
| `n_retrieved` | integer | 개 | 0 이상 | 점수가 있는 문서 수 | `387` |

### `processed/index.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T105556Z-6235a29` |
| `condition` | string | 없음 | 필수, 고유 | 단어 조각 조건 | `base` |
| `vocab_size` | integer | 개 | 0 이상 | 서로 다른 조각 수 | `6527` |
| `postings` | integer | 개 | 0 이상 | 조각과 문서 쌍의 수 | `83494` |
| `total_tokens` | integer | 개 | 0 이상 | 조각 수 합 | `150044` |
