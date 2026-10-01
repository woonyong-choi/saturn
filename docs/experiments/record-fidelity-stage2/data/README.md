# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | [record-fidelity](../../record-fidelity/data/README.md)의 원시 줄(공개 합성 작업 24개를 Codex `gpt-6-sol`과 Claude Code `claude-opus-5-5`가 진행한 세션 48개)을 수정된 변환(#210, #214, #215)으로 `record-fidelity` 예제로 재생해 Saturn 이벤트를 만들었다. 이벤트로 `saturn-core`의 `packet` 예제가 패킷을 만들고, 두 provider가 모두 거친 근거 사실 질문과 함께 Opus 5.5와 GPT-6 Sol의 새 session(도구 끔)에 넘겨 답을 받았다. provider 작업은 다시 실행하지 않았다. 작업 5개를 먼저 받는 쪽에 넘기고 같은 실행 id로 이어 받았다. |
| 수집 기간 | 2026-10-02~2026-10-02 |
| 개수 | 작업 24, 재생한 이벤트 줄 수는 `raw/events-*.jsonl` 파일마다 세션 길이에 따라 다르다, 패킷 48(작업 24 × 기록 2), 받는 쪽 세션 96(작업 24 × 기록 2 × 받는 쪽 2), 근거 사실 시행 1,248(사실 312 × 기록 2 × 받는 쪽 2) |
| 표본 여부 | 전수. 작업은 규칙으로 만든 합성 데이터다. |
| 라벨 | 정답 근거 사실과 항목은 `scripts/tasks.py`가 작업을 만들 때 함께 만든다. 사람이 붙인 라벨은 없다. |
| 알려진 문제 | 작업 `t03`과 `t12`는 실패 테스트 둘의 이름이 같은 낱말(`juniper`)을 포함해 두 질문(`t1-0`, `t1-1`)의 문구가 같아 받는 쪽이 어느 이름이 어느 질문의 정답인지 가를 수 없다. 이 질문 4개는 모든 조건에서 틀렸다. 원시 줄은 1단계 수집 때 개인 설정과 계정 줄을 지운 뒤 저장한 줄이다. |
| 개인정보 | 없음. 작업은 합성이고 원시 줄은 1단계에서 작업 폴더, 홈 경로, 호스트 이름, 사용자 이름을 바꿨다. |
| 라이선스 | 저장소 라이선스를 따른다. |

사용량 기록은 `env.json`의 `usage`에 있다. 시작 때 Claude 주간 28%, 5시간 창 5%, Codex 주간 11%였고, 작업 5개 뒤와 배치마다 읽은 값은 모두 같았으며, 끝났을 때 Claude 주간 28%, 5시간 창 6%, Codex 주간 11%였다. 두 실험 합산 기준(첫 실험 시작 Claude 27%, Codex 10%)으로 각각 +1%p였다. 시험 5개의 Claude 비용 합은 0.261달러(세션 10개)였고 전체 24개로 환산하면 약 1.26달러다. 같은 계정을 쓰는 다른 작업의 사용이 섞여 있다.

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/events-codex-20261001T193037Z-bd1a5ab.jsonl` | Codex 기록을 재생한 Saturn 이벤트 | `scripts/01-collect.py` |
| `raw/events-claude-20261001T193037Z-bd1a5ab.jsonl` | Claude Code 기록을 재생한 Saturn 이벤트 | `scripts/01-collect.py` |
| `raw/packets-20261001T193037Z-bd1a5ab.jsonl` | 작업과 기록마다 패킷 글, 넣은 항목, 근거 기록 번호, 물은 근거 사실 | `scripts/01-collect.py` |
| `raw/receiver-claude-20261001T193037Z-bd1a5ab.jsonl` | 받는 쪽 Claude Code의 프롬프트와 출력 원문 | `scripts/01-collect.py` |
| `raw/receiver-codex-20261001T193037Z-bd1a5ab.jsonl` | 받는 쪽 Codex의 프롬프트와 출력 원문 | `scripts/01-collect.py` |
| `processed/trials.csv` | 근거 사실 시행별 정답과 패킷 포함 | `scripts/02-process.py` |
| `processed/flow.csv` | 받는 쪽 세션별 상태, 실행 횟수, 소요 시간 | `scripts/02-process.py` |
| `processed/items.csv` | 두 provider가 모두 거친 항목별 도구 종류, 경로, 메모 | `scripts/02-process.py` |
| `processed/packets.csv` | 패킷 토큰 수와 넣은 항목 수 | `scripts/02-process.py` |
| `processed/activity.csv` | 기록의 도구 종류별 수와 추론 항목, 감싸기 수 | `scripts/02-process.py` |
| `SHA256SUMS` | `raw/` 파일의 SHA-256 | `run.sh collect` |

## 필드

### `raw/*.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T193037Z-bd1a5ab` |
| `trial_id` | string | 없음 | 필수 | 이벤트와 패킷은 `기록 provider-작업`, 받는 쪽은 `기록 provider-받는 쪽-작업` | `codex-claude-t06` |
| `condition` | string | 없음 | 필수 | 기록을 만든 provider나 `기록 provider->받는 쪽` | `codex->claude` |
| `ts_utc` | datetime | UTC | 필수 | 기록한 시각 | `2026-10-01T19:30:53+00:00` |
| `task_id` | string | 없음 | 필수 | 합성 작업 | `t06` |
| `dir` | string | 없음 | `event`, `packet`, `in`, `out` | 재생한 이벤트, 패킷, 받는 쪽 프롬프트, 받는 쪽 출력 | `out` |
| `line` | string | 없음 | 필수 | 저장한 내용(JSON 문자열이거나 프롬프트 글). `out`은 종료 코드, 출력 원문, 실행 횟수, 소요 시간 | `{"exit_code": 0}` |

### `processed/trials.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `task_id` | string | 없음 | 필수 | 작업 | `t01` |
| `fact` | string | 없음 | 필수 | 근거 사실 id | `r1-0` |
| `source` | string | 없음 | `codex`, `claude` | 기록을 만든 provider | `codex` |
| `receiver` | string | 없음 | `claude`, `codex` | 받는 쪽 | `claude` |
| `status` | string | 없음 | `ok`, `failed`, `tool_used`, `unparsed` | 세션 상태 | `ok` |
| `correct` | integer | 없음 | 0, 1 | 답에 정답 값이 든 여부 | `1` |
| `in_packet` | integer | 없음 | 0, 1 | 근거 사실의 값이 패킷 글에 든 여부 | `1` |

### `processed/flow.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `task_id` | string | 없음 | 필수 | 작업 | `t01` |
| `source` | string | 없음 | `codex`, `claude` | 기록을 만든 provider | `codex` |
| `receiver` | string | 없음 | `claude`, `codex` | 받는 쪽 | `claude` |
| `status` | string | 없음 | `ok`, `failed`, `tool_used`, `unparsed` | 세션 상태 | `ok` |
| `attempt` | integer | 회 | 1 또는 2 | 성공까지의 실행 횟수 | `1` |
| `elapsed_s` | number | 초 | 필수 | 소요 시간 | `8.2` |

### `processed/items.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `task_id` | string | 없음 | 필수 | 작업 | `t01` |
| `provider` | string | 없음 | `codex`, `claude` | 기록을 만든 provider | `claude` |
| `item` | string | 없음 | 필수 | 항목 id | `c2` |
| `truth_kind` | string | 없음 | `FileRead`, `Shell`, `FileEdit`, `TestRun` | 정답 도구 종류 | `Shell` |
| `captured` | integer | 없음 | 0, 1 | 대응하는 기록의 존재 여부 | `1` |
| `kind_ok` | integer | 없음 | 0, 1 | 도구 종류와 정답의 일치 여부 | `1` |
| `path_ok` | integer | 없음 | 0, 1, 빈 칸 | 경로 추출. 경로가 없는 항목은 빈 칸 | `1` |
| `memo_ok` | integer | 없음 | 0, 1 | 메모 필드를 모두 정답으로 얻은 여부 | `1` |

`packets.csv`와 `activity.csv`는 `task_id`, `provider`와 위 파일 표의 내용을 담는 정수 열로 이뤄진다.
