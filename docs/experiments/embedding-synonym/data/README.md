# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `scripts/01-collect.py collect`가 커밋 `1258e17`의 Saturn 저장소 파일을 `git show`로 읽어 설명 짝, 식별자 후보, 순위 문서와 질의를 만들고 두 모델로 짝 제안과 순위를 계산했다. `cost`가 모델마다 새 프로세스에서 비용을 쟀다. |
| 수집 기간 | 2026-10-01~2026-10-01 |
| 개수 | 설명 질의 200개, 식별자 후보 2,415개, 순위 문서 1,194개, 순위 질의 600개, 짝 제안 400행, 순위 1,200행, 라벨 583행, 비용 1,204행 |
| 표본 여부 | 설명 질의는 같은 설명을 하나로 줄인 설명 짝 후보에서 시드 155로 섞어 앞 200개를 뽑았다. 단어 겹침 질의는 시드 155로 섞은 후보 위치에서 언어마다 200개를 뽑았다. 식별자 후보와 순위 문서는 전수다. |
| 라벨 | 실험자 한 명이 설계의 판정 기준표로 붙였다. 시트에는 행 번호, 한국어 설명, 식별자만 보였다. |
| 알려진 문제 | 설명에서 영문과 코드 표시를 지워 조각난 문장이 많다. `cost-input` 파일은 비용 측정에 넣은 문장 목록이라 필수 필드가 없다. |
| 개인정보 | 없음. 공개 저장소의 문서와 코드만 썼다. |
| 라이선스 | Saturn 저장소와 같은 라이선스 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/identifiers-20261001T112417Z-e61c3d2.jsonl` | 식별자 후보와 임베딩에 넣은 단어 | `scripts/01-collect.py` |
| `raw/pairs-20261001T112417Z-e61c3d2.jsonl` | 설명 질의와 바로 아래 정의(정답 짝) | `scripts/01-collect.py` |
| `raw/corpus-20261001T112417Z-e61c3d2.jsonl` | 순위 문서(Markdown 절, 식별자만 남긴 코드 묶음) | `scripts/01-collect.py` |
| `raw/rank-queries-20261001T112417Z-e61c3d2.jsonl` | 같은 뜻 질의와 단어 겹침 질의, 정답 문서 | `scripts/01-collect.py` |
| `raw/proposals-20261001T112417Z-e61c3d2.jsonl` | 모델마다 설명 질의의 상위 5개 식별자와 정답 순위 | `scripts/01-collect.py` |
| `raw/rankings-20261001T112417Z-e61c3d2.jsonl` | 모델마다 순위 질의의 세 순위 조건 정답 순위와 상위 10개 | `scripts/01-collect.py` |
| `raw/label-sheet-20261001T112417Z-e61c3d2.jsonl` | 라벨 시트(정답 짝과 두 모델 1위 제안을 섞은 행) | `scripts/01-collect.py` |
| `raw/labels-20261001T112417Z-e61c3d2.csv` | 라벨 시트 행마다 붙인 라벨 | 사람이 작성, `scripts/01-collect.py seal`이 검사 |
| `raw/cost-input-20261001T112417Z-e61c3d2.jsonl` | 지연 측정에 넣은 설명 문장 | `scripts/01-collect.py cost` |
| `raw/cost-20261001T112417Z-e61c3d2.jsonl` | 설치 크기, 상주 메모리 증가, 문장별 지연 | `scripts/01-collect.py cost` |
| `processed/proposals.csv` | 짝 제안과 라벨을 합친 표 | `scripts/02-process.py` |
| `processed/rankings.csv` | 순위 질의별 정답 순위와 정답 짝 라벨 | `scripts/02-process.py` |
| `processed/cost.csv` | 비용 측정 표 | `scripts/02-process.py` |

## 필드

모든 JSON Lines 행(`cost-input` 제외)은 `run_id`(string, 실행 id), `trial_id`(string 또는 null, 행 식별자), `condition`(string 또는 null, 모델 `e5`·`minilm`), `ts_utc`(datetime, 기록 시각)를 가진다.

### `pairs-20261001T112417Z-e61c3d2.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `query_id` | string | 없음 | 고유 | 설명 질의 id | `pair-001` |
| `fold` | integer | 없음 | 0, 1 | 기준값 교차 적용 접힘 | `0` |
| `def_id` | string | 없음 | 필수 | 정의 위치 | `saturn-terminal/engine/src/settings/mod.rs#L220` |
| `path` | string | 없음 | 필수 | 파일 경로 | `saturn-terminal/engine/src/settings/mod.rs` |
| `line` | integer | 줄 | 1 이상 | 정의 줄 | `220` |
| `comment_line` | integer | 줄 | 1 이상 | 문서 주석 첫 줄 | `219` |
| `kind` | string | 없음 | `fn`, `struct`, `enum`, `trait`, `type`, `const`, `static`, `mod`, `field`, `variant` | 정의 종류 | `fn` |
| `identifier` | string | 없음 | 필수 | 정답 식별자 | `lookup` |
| `description` | string | 없음 | 한글 6음절 이상 | 영문과 코드 표시를 지운 주석 첫 문장 | `옛 스냅샷에 없던 키도 기본값 층 값으로 읽는다` |

### `identifiers-20261001T112417Z-e61c3d2.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `identifier` | string | 없음 | 고유 | 식별자 후보 | `ABSOLUTE_PATH_MARK` |
| `words` | string | 없음 | 필수 | 임베딩에 넣은 소문자 단어 | `absolute path mark` |

### `corpus-20261001T112417Z-e61c3d2.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `doc_id` | string | 없음 | 고유 | 문서 id | `AGENTS.md#s000` |
| `path` | string | 없음 | 필수 | 파일 경로 | `AGENTS.md` |
| `kind` | string | 없음 | `markdown`, `code` | 문서 종류 | `markdown` |
| `start_line`, `end_line` | integer | 줄 | 1 이상 | 문서 범위 | `1` |
| `text` | string | 없음 | 20자 이상 | BM25에 넣은 본문 | `# Saturn ...` |
| `embed_text` | string | 없음 | 필수 | 임베딩에 넣은 본문. 코드는 소문자 단어 | `# Saturn ...` |

### `rank-queries-20261001T112417Z-e61c3d2.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `rank_query_id` | string | 없음 | 고유 | 순위 질의 id | `syn-pair-001` |
| `set` | string | 없음 | `synonym`, `lexical-ko`, `lexical-en` | 질의 묶음 이름 | `synonym` |
| `pair_query_id` | string 또는 null | 없음 | `synonym`만 값 | 원래 설명 질의 | `pair-001` |
| `query_text` | string | 없음 | 필수 | 질의 글 | `옛 스냅샷에 없던 키도 기본값 층 값으로 읽는다` |
| `gold_doc_ids` | string 배열 | 없음 | 1~5개 | 정답 문서 | `["saturn-terminal/engine/src/settings/mod.rs#L201-240"]` |

### `proposals-20261001T112417Z-e61c3d2.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `query_id` | string | 없음 | 필수 | 설명 질의 id | `pair-001` |
| `top_identifiers` | string 배열 | 없음 | 5개 | 유사도 상위 5개 식별자 | `["previous", ...]` |
| `top_scores` | number 배열 | 코사인 | −1~1 | 상위 5개 유사도 | `[0.83022, ...]` |
| `gold_identifier` | string | 없음 | 필수 | 정답 식별자 | `lookup` |
| `gold_score` | number | 코사인 | −1~1 | 정답 식별자 유사도 | `0.794063` |
| `gold_rank` | integer | 순위 | 1 이상 | 같은 단어의 식별자 중 가장 높은 순위 | `157` |

### `rankings-20261001T112417Z-e61c3d2.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `rank_query_id` | string | 없음 | 필수 | 순위 질의 id | `syn-pair-001` |
| `set` | string | 없음 | 위 표의 값 | 질의 묶음 이름 | `synonym` |
| `base_gold_rank`, `embedding_gold_rank`, `rrf_gold_rank` | integer 또는 null | 순위 | 1 이상 | 조건별 정답 문서의 가장 높은 순위. 순위에 없으면 null | `383` |
| `base_top_doc_ids`, `embedding_top_doc_ids`, `rrf_top_doc_ids` | string 배열 | 없음 | 10개 이하 | 조건별 상위 10개 문서 | `["docs/design/settings.md#s011", ...]` |

### `label-sheet-20261001T112417Z-e61c3d2.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `row_id` | string | 없음 | 고유 | 시트 행 | `r0001` |
| `query_id` | string | 없음 | 필수 | 설명 질의 id | `pair-020` |
| `korean` | string | 없음 | 필수 | 한국어 설명 | `없으면 빈 기록` |
| `identifier` | string | 없음 | 필수 | 판정할 식별자 | `load` |

### `labels-20261001T112417Z-e61c3d2.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T112417Z-e61c3d2` |
| `trial_id` | string | 없음 | 고유 | 시트 행 | `r0001` |
| `condition` | string | 없음 | 빈 칸 | 해당 없음 | 빈 칸 |
| `ts_utc` | datetime | 없음 | 필수 | 라벨 기록 시각 | `2026-10-01T11:31:27Z` |
| `row_id` | string | 없음 | 고유 | 시트 행 | `r0001` |
| `label` | string | 없음 | `same`, `different`, `unclear` | 기준표 라벨 | `different` |

### `cost-20261001T112417Z-e61c3d2.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `measure` | string | 없음 | `install_bytes`, `rss_increase_bytes`, `latency_ns` | 측정 종류 | `install_bytes` |
| `value` | integer | 바이트 또는 ns | 0 이상 | 측정값 | `577665304` |
| `detail` | object | 없음 | 필수 | 설치 크기 구성, RSS 전후와 최대값, 모델 올리는 시간, 지연의 반복 번호와 문장 번호 | `{"model_snapshot": 487352505, ...}` |

### `cost-input-20261001T112417Z-e61c3d2.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `text` | string | 없음 | 200행 | 지연 측정에 넣은 설명 질의 | `옛 스냅샷에 없던 키도 기본값 층 값으로 읽는다` |
