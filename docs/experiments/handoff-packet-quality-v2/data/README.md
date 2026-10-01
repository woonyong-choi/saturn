# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `01-collect`가 시드 218로 합성 시나리오 48개를 만들고, 시나리오마다 Jev(`jev-1.13.0`)에 후보 전체를 묻고 `saturn-core`의 `packet` 예제로 두 패킷을 만든 뒤, Claude Code Opus 5.5(`claude-opus-5-5`)와 Codex GPT-6 Sol(`gpt-6-sol`)의 새 session 세션을 조건 3개로 실행해 답을 받았다. 시험 시나리오 5개를 먼저 수집하고 같은 실행 id로 이어 받았다. 시나리오 45번째의 judge 요청 중 연결이 끊겨 수집이 멈췄고, 연결 오류를 잡도록 `01-collect`를 고친 뒤 같은 실행 id로 이어 받았다. |
| 수집 기간 | 2026-10-02~2026-10-02 |
| 개수 | 시나리오 48, 패킷 행 48, 세션 288(시나리오 48 × provider 2 × 조건 3), 질문 시행 2,880 |
| 표본 여부 | 전수. 시나리오는 생성 규칙으로 만든 합성 데이터다. |
| 라벨 | 정답과 근거 항목 번호는 시나리오 생성 규칙이 붙인다. 채점은 `02-process`가 정답 목록과 문자열 포함으로 한다. |
| 알려진 문제 | 근거 항목의 포함 여부는 패킷 포함 항목 목록으로 판단해 축약본으로 든 항목과 원문으로 든 항목을 가르지 못한다. 사용률 값은 정수 퍼센트이고 같은 계정의 다른 작업이 섞인다. 멈춘 수집의 45번째 시나리오는 judge 판단을 한 번 버리고 다시 받았다. |
| 개인정보 | 없음. 시나리오는 합성 데이터이고 Jev 키는 환경 변수로만 전달해 어떤 파일에도 남기지 않았다. |
| 라이선스 | 저장소 라이선스를 따른다. |

사용량 기록은 `env.json`의 `usage`에 있다. 수집 시작, 시험 5개 뒤, 시나리오 6개마다, 수집 끝의 Claude 5시간 창과 주간 사용률, Codex 주간 사용률(창 10080분)이다. Claude 주간 사용률은 27%에서 28%로, Codex 주간 사용률은 10%에서 11%로 올랐다. 시험 5개 뒤 값은 Claude 27%, Codex 10%였다. 시험 5개의 Claude 비용 합은 1.379달러였고 이전 실험의 비용(8.063달러, 사용률 +2%p)으로 환산한 전체 증가 추정은 3.3%p였다. Codex 토큰 합은 376,059개였고 이전 실험(3,177,567개, +1%p)으로 환산한 추정은 1.1%p였다. Claude 5시간 창은 0%에서 5%까지 올라 85%를 넘지 않았다.

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/scenarios-20261001T185207Z-aa1e13a.jsonl` | 시나리오 48개의 기록(session과 시각 포함), 질문, 정답, 근거 항목 번호 | `scripts/01-collect.py` |
| `raw/packets-20261001T185207Z-aa1e13a.jsonl` | 시나리오마다 judge 판단 확률, judge 호출 수와 실패 수, `judge-all`과 `rrf-fallback` 패킷 | `scripts/01-collect.py` |
| `raw/sessions-20261001T185207Z-aa1e13a.jsonl` | 세션마다 provider 출력 원문과 종료 코드, 소요 시간 | `scripts/01-collect.py` |
| `SHA256SUMS` | `raw/` 파일의 SHA-256 | `run.sh collect` |

`processed/`의 `trials.csv`, `sessions.csv`, `evidence.csv`, `packets.csv`는 `scripts/02-process.py`가 만든다.

## 필드

### `scenarios-20261001T185207Z-aa1e13a.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T185207Z-aa1e13a` |
| `scenario_id` | string | 없음 | 필수, 고유 | 시나리오 번호 | `s10` |
| `module` | string | 없음 | 필수 | 시나리오가 다루는 모듈 | `search` |
| `record` | array | 없음 | 필수 | 기록 항목. `seq`, `session`, `kind`, `text`나 `tool`, `args`, `result`, `ts` | `{"seq": 1, "kind": "user"}` |
| `questions` | array | 없음 | 필수, 10개 | 질문. `qid`, `qtype`, `text`, `gold`, `stale`, `evidence_seq`, `evidence_relation` | `{"qid": "q1", "qtype": "extract"}` |

### `packets-20261001T185207Z-aa1e13a.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T185207Z-aa1e13a` |
| `trial_id` | string | 없음 | 필수, 고유 | 시나리오 번호와 `packets` | `s10-packets` |
| `condition` | string | 없음 | `packets` | 이 행은 두 패킷 조건을 함께 담는다. | `packets` |
| `ts_utc` | datetime | UTC | 필수 | 패킷을 만든 시각 | `2026-10-01T18:52:20+00:00` |
| `scenario_id` | string | 없음 | 필수 | 시나리오 번호 | `s10` |
| `judge_calls` | integer | 회 | 필수 | 나눠 보낸 judge 요청 수 | `6` |
| `judge_failures` | integer | 회 | 필수 | 응답을 받지 못한 요청 수 | `0` |
| `judge_questions` | integer | 개 | 필수 | 물은 질문 수 | `256` |
| `judged` | integer | 개 | 필수 | 판단을 받은 후보 수 | `128` |
| `judgments` | array | 확률 | 필수 | 후보 `seq`와 남길 확률 `probability`(0~1) | `{"seq": 3, "probability": 0.28}` |
| `packets` | object | 없음 | 필수 | `judge-all`, `rrf-fallback`마다 예제의 JSON 출력(`packet`, `included`, `rrf_order`, `tokens`) | `{"judge-all": {}}` |

### `sessions-20261001T185207Z-aa1e13a.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T185207Z-aa1e13a` |
| `trial_id` | string | 없음 | 필수, 고유 | 시나리오, provider, 조건 | `s10-codex-no-packet` |
| `condition` | string | 없음 | `judge-all`, `rrf-fallback`, `no-packet` | 조건 | `no-packet` |
| `ts_utc` | datetime | UTC | 필수 | 세션을 끝낸 시각 | `2026-10-01T18:52:29+00:00` |
| `scenario_id` | string | 없음 | 필수 | 시나리오 번호 | `s10` |
| `provider` | string | 없음 | `claude`, `codex` | 맥락을 받은 provider | `codex` |
| `model` | string | 없음 | 필수 | 요청한 모델 이름 | `gpt-6-sol` |
| `attempt` | integer | 회 | 1 또는 2 | 성공까지의 실행 횟수 | `1` |
| `packet_tokens` | integer | 토큰 | 필수 | 맥락 글자 수를 4로 나눈 값 | `5` |
| `prompt_sha256` | string | 없음 | 필수 | 프롬프트의 SHA-256 | `ab12...` |
| `exit_code` | integer | 없음 | 필수, 시간 초과면 `null` | provider 종료 코드 | `0` |
| `stdout` | string | 없음 | 필수 | provider 출력 원문 | `{"type":"thread.started"}` |
| `stderr` | string | 없음 | 필수 | 오류 출력의 끝 2,000자 | 빈 문자열 |
| `elapsed_s` | number | 초 | 필수 | 소요 시간 | `6.364` |
