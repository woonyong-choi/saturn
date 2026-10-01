# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `./run.sh collect`로 Saturn 저장소 커밋 `31e5311`의 문서와 코드에서 괄호 표기와 주석 짝 후보를 캤다. 같은 명령으로 이 컴퓨터의 Claude Code 기록에서 턴마다 한글 단어와 도구 인자의 영문 조각을 뽑았다. |
| 수집 기간 | 2026-10-01~2026-10-01. 함께 나옴 기록은 2026-10-01 00:00 UTC 이전 턴만 썼다. |
| 개수 | 괄호 표기 353건(짝 295개), 주석 5,437건(규칙대로 캔 짝 375개, 넓힌 짝 4,937개), 함께 나옴 기록 파일 667개의 턴 1,234개(후보 짝 25,297개), 라벨 802행 |
| 표본 여부 | 괄호 표기와 주석은 전수를 캤다. 라벨은 출처마다 시드 127로 무작위 200개를 뽑고, 괄호 표기의 증거 3번 이상 짝 9개는 전수를 더했다. |
| 라벨 | 실험자 한 명이 [실험 설계](../design.md)의 판정 기준표로 붙였다. 출처, 증거 수, Dice를 가린 섞인 시트에서 두 말만 보고 판정했다. |
| 알려진 문제 | 괄호 표기 말뭉치에는 설계 문서의 예시 문장(`로그인(login)`)이 들어 있다. 같은 두 말이 여러 출처에 나오면 라벨은 하나만 붙였다. |
| 개인정보 | 괄호 표기와 주석 자료는 공개 저장소 글이다. 함께 나옴 자료는 개인 대화 기록이라 원문, 짝 목록, 라벨을 저장소 밖에만 두고, 여기에는 경로, SHA-256, 집계 수만 적는다. 저장소 밖 자료도 프로젝트 이름과 session 이름은 SHA-256 앞 12자리로 바꿔 저장했다. |
| 라이선스 | 저장소 자료는 Saturn 저장소와 같다. 저장소 밖 자료는 공개하지 않는다. |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/paren-20261001T105613Z-31e5311.jsonl` | 괄호 표기 짝 후보가 나온 곳마다 한 행 | `scripts/01-collect.py` |
| `raw/comment-20261001T105613Z-31e5311.jsonl` | 주석 아래 정의마다 짝 후보 한 행. 규칙대로 캔 `strict`와 탐색용 `extended` | `scripts/01-collect.py` |
| `raw/readme-20261001T105613Z-31e5311.jsonl` | 토큰 추정에 쓴 프로젝트 설명 문단 | `scripts/01-collect.py` |
| `raw/labels-public-20261001T105613Z-31e5311.jsonl` | 공개 자료에 나오는 두 말의 라벨 603행 | `scripts/01-collect.py labels` |
| `processed/pairs-paren.csv`, `processed/pairs-comment.csv`, `processed/pairs-comment-extended.csv` | 출처별 짝과 증거 수, 예문 두 개 | `scripts/02-process.py` |
| `processed/samples.csv` | 공개 출처의 라벨 표본 목록 | `scripts/02-process.py` |
| `SHA256SUMS` | 저장소 `raw/` 파일의 SHA-256 | `run.sh collect` |
| `private-SHA256SUMS` | 저장소 밖 원자료의 SHA-256 | `run.sh collect` |

저장소 밖 자료는 `TERM_CATALOG_PRIVATE_DIR`(기본 `~/workspace/woon/.local/orchestration/saturn-experiments/term-catalog-precision/raw/`)에 있다. `./run.sh verify`가 아래 해시를 확인한다.

| 저장소 밖 파일 | 내용 | SHA-256 |
|---|---|---|
| `cooc-20261001T105613Z-31e5311.jsonl` | 턴 1,234개의 한글 단어와 영문 조각 | `72dbff77c6dca66ec88c14b2a5660f8a3dbfd695eea9aa9e4e072135d1cf6c85` |
| `collect-log-20261001T105613Z-31e5311.json` | 읽은 파일 667개, 읽지 못한 파일 0개, 깨진 줄 0개 | `838e3c6c063acb3d06e93cfad5b554c81abcd2877c89b2d86d34e3f21d15129a` |
| `labels-cooc-20261001T105613Z-31e5311.jsonl` | 함께 나옴에만 나오는 두 말의 라벨 199행 | `7a85d023a9dfd22c20564d400f62d0cdd7b95c49a8c926b8f7a9156a6a28510f` |

- 저장소 밖 폴더에는 `scripts/02-process.py`가 만드는 함께 나옴 짝, 후보, 표본, 라벨 시트도 있다. 이 파일들은 위 원자료로 다시 만든다.

## 필드

### `paren-*.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T105613Z-31e5311` |
| `trial_id` | string | 없음 | 필수, 고유 | 행 번호 | `paren-00001` |
| `condition` | string | 없음 | `paren` | 출처 | `paren` |
| `ts_utc` | datetime | 없음 | 필수 | 수집 시각 | `2026-10-01T10:56:13Z` |
| `input_id` | string | 없음 | 필수 | 파일과 줄 번호 | `AGENTS.md:15` |
| `pattern` | string | 없음 | `ko_paren_en`, `en_paren_ko`, `code_adjacent` | 찾은 모양 | `code_adjacent` |
| `ko` | string | 없음 | 2~10글자 | 조사를 뗀 한글 말 | `실행` |
| `en` | string | 없음 | 소문자, 2~40자 | 영문 식별자 | `saturn` |
| `sentence` | string | 없음 | 300자까지 | 찾은 줄 | `saturn` 실행 파일, 명령줄 처리와 engine 시작 |

### `comment-*.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id`, `trial_id`, `ts_utc` | string, string, datetime | 없음 | 필수 | `paren-*.jsonl`의 같은 필드 | `comment-00001` |
| `condition` | string | 없음 | `comment` | 출처 | `comment` |
| `input_id` | string | 없음 | 필수 | 정의 줄의 파일과 줄 번호 | `saturn-protocol/src/codegen.rs:35` |
| `variant` | string | 없음 | `strict`, `extended` | 설계 규칙대로 캔 짝, 탐색용으로 넓힌 짝 | `strict` |
| `keyword` | string | 없음 | 정의 키워드 | 정의 줄의 키워드 | `fn` |
| `ko` | string | 없음 | 2~10글자 | 주석의 한글 말 | `정수` |
| `en` | string | 없음 | 소문자 | 정의 이름 | `typescript` |
| `comment` | string | 없음 | 300자까지 | 주석 글 | `큰 정수는 number로 쓴다` |
| `definition` | string | 없음 | 300자까지 | 정의 줄 | `pub fn typescript() -> String {` |

### `readme-*.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id`, `trial_id`, `ts_utc` | string, string, datetime | 없음 | 필수 | `paren-*.jsonl`의 같은 필드 | `readme-1` |
| `condition` | string | 없음 | `project` | 자료 종류 | `project` |
| `input_id` | string | 없음 | 필수 | 파일과 줄 번호 | `README.md:19` |
| `text` | string | 없음 | 80자 이상 | 프로젝트 설명 문단 | `Developers who use Codex and Claude Code together` |

### `labels-public-*.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id`, `ts_utc` | string, datetime | 없음 | 필수 | 실행 id, 라벨을 들여온 시각 | `2026-10-01T10:57:39Z` |
| `trial_id`, `input_id` | string | 없음 | 필수, 고유 | 라벨 시트 행 id | `r9ee9c4d2` |
| `condition` | string | 없음 | `label` | 자료 종류 | `label` |
| `ko`, `en` | string | 없음 | 필수 | 라벨을 붙인 두 말 | `범위`, `main-turn` |
| `label` | string | 없음 | `same`, `different`, `unclear`, `null` | 판정 기준표의 라벨 | `different` |
