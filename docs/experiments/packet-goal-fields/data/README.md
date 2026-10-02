# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `scripts/01-collect.py`가 시나리오를 만들고 packet 예제와 provider를 실행한다. |
| 수집 기간 | 수집 뒤 기록한다. |
| 개수 | 수집 뒤 기록한다. |
| 표본 여부 | 시드 249의 합성 시나리오 24개 전수 |
| 라벨 | `scripts/02-process.py`가 design.md의 정답 목록으로 자동 채점한다. |
| 알려진 문제 | provider 출력 형식이 다르므로 처리 스크립트가 provider별 답 추출기를 쓴다. |
| 개인정보 | 포함하지 않는다. 시나리오는 합성 기록이고 raw에는 계정 정보와 토큰을 저장하지 않는다. |
| 라이선스 | 저장소 라이선스를 따른다. |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/scenarios-{실행 id}.jsonl` | 시나리오, 기록, 질문, 정답 | `scripts/01-collect.py` |
| `raw/packets-{실행 id}.jsonl` | 세 조건 패킷과 summary model 출력·토큰 | `scripts/01-collect.py` |
| `raw/sessions-{실행 id}.jsonl` | provider 입력 결과와 호출 정보 | `scripts/01-collect.py` |
| `processed/*.csv` | 질문 채점, session 상태, evidence, 비용 | `scripts/02-process.py` |
| `results/summary.json` | 보고서 수치의 정본 | `scripts/03-analyze.py` |

## 필드

### `raw/sessions-{실행 id}.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실험 실행 식별자 | `20261002T120000Z-abc1234` |
| `trial_id` | string | 없음 | 필수·고유 | scenario, provider, condition 조합 | `s01-claude-rule` |
| `condition` | string | 없음 | 필수·값 목록 | `last-input`, `rule`, `summary` | `rule` |
| `ts_utc` | string | UTC | 필수 | 기록 시각 | `2026-10-02T12:00:00+00:00` |
| `scenario_id` | string | 없음 | 필수 | 시나리오 식별자 | `s01` |
| `provider` | string | 없음 | 필수·값 목록 | provider 이름 | `claude` |
| `exit_code` | integer 또는 null | 없음 | 필수 | subprocess 종료 코드 | `0` |
| `prompt_tokens` | integer | 토큰 | 필수 | prompt 추정 토큰 수 | `1234` |

### `raw/packets-{실행 id}.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실험 실행 식별자 | `20261002T120000Z-abc1234` |
| `trial_id` | string | 없음 | 필수·고유 | packet 생성 단위 | `s01-packets` |
| `condition` | string | 없음 | 필수 | `packets` | `packets` |
| `ts_utc` | string | UTC | 필수 | 기록 시각 | `2026-10-02T12:00:00+00:00` |
| `scenario_id` | string | 없음 | 필수 | 시나리오 식별자 | `s01` |
| `summary` | object | 없음 | 필수 | summary model 고정 구역과 토큰 | `{"goal":[]}` |
| `packets` | object | 없음 | 필수 | 세 조건 packet 출력 | `{"rule":{}}` |
