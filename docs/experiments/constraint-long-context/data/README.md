# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `~/.claude/projects/*/*.jsonl`을 읽어 사람이 쓴 `user` 턴만 추출하고, state 가림을 적용한다. |
| 수집 기간 | 실행 시각에 기록한다. |
| 개수 | 측정 전. 최대 6개 대화, 최대 120개 Jev 표본 턴이다. |
| 표본 여부 | 사용자 턴 수 구간별 최대 2개 대화와 대화별 최대 20개 턴을 시드 316으로 표본 추출한다. |
| 라벨 | Codex `gpt-6-astra`와 Claude `claude -p --model sonnet --tools ''`가 대화 전체를 독립적으로 읽는다. 일치한 항목만 확인 분석에 쓴다. |
| 알려진 문제 | 실제 기록에는 정답 라벨이 없고, 자동 삽입 제외는 JSON 구조와 표식에 의존한다. |
| 개인정보 | 포함될 수 있는 사용자 입력은 가림 뒤 Git 제외 private 실행 디렉터리에만 저장한다. 공개 파일에는 원문을 저장하지 않는다. |
| 라이선스 | 사용자 private 자료. 공개 배포 금지. |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `data/SHA256SUMS` | 공개 설계 파일과 공개 결과의 SHA-256 | `scripts/04-process.py` |
| private `selected.jsonl` | 가림 처리한 선택 대화와 표본 | `scripts/01-collect.py` |
| private `labels/*.jsonl` | 두 라벨러의 대화별 원문 응답 | `scripts/02-label.py` |
| private `jev/*.jsonl` | 조건·반복별 Jev 응답 | `scripts/03-jev.py` |
| `results/summary.json` | 집계 수치와 신뢰구간 | `scripts/05-analyze.py` |

## 필드

### private `selected.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 식별자 | `20261003T120000Z-abcdef1` |
| `conversation_id` | string | 없음 | 고유 | 원본 파일 해시 식별자 | `0123456789abcdef` |
| `length_band` | string | 없음 | 값 목록 | 대화 길이 구간 | `120-399` |
| `turn_id` | string | 없음 | 필수 | 사용자 턴 식별자 | `u-0012` |
| `ts_utc` | string | UTC | 필수 | 추출 시각 | `2026-10-03T12:00:00Z` |
| `text` | string | 없음 | 필수 | 가림된 사용자 입력 | `[redacted]` |
| `files` | array | 없음 | 필수 | 턴에서 찾은 가림된 절대 경로의 끝 이름 | `["src/lib.rs"]` |
| `sampled` | boolean | 없음 | 필수 | Jev 표본 여부 | `true` |

### `results/summary.json`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 결과 실행 식별자 | `20261003T120000Z-abcdef1` |
| `calls` | integer | 회 | 0 이상 | Jev 호출 수와 라벨 호출 수 | `738` |
| `label_agreement` | number | 비율 | 0~1 | 라벨러 일치율 | `0.82` |
| `metrics` | object | 없음 | 필수 | 길이·조건·확신·거리별 집계 | `{}` |
