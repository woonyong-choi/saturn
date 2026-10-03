# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `scripts/01-collect.py`가 전용 `CODEX_HOME`의 Codex app-server JSON-RPC를 비차단 바이트 읽기로 수집 |
| 수집 기간 | 수집 뒤 기록 |
| 개수 | 수집 뒤 기록 |
| 표본 여부 | 조건별 지정 3회 전수 |
| 라벨 | driver가 이벤트·marker·승인 요청을 규칙에 따라 기록 |
| 알려진 문제 | 모델 출력 변동과 read-only sandbox 승인 정책은 기록하되 제외하지 않음 |
| 개인정보 | 인증 원본은 읽기만 하는 심볼릭 링크이며 raw/private log에 토큰·인증값을 저장하지 않음 |
| 라이선스 | Saturn 저장소 문서 산출물 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/{run_id}.jsonl` | redaction한 trial 요약과 이벤트 참조 | `scripts/01-collect.py` |
| `processed/trials.csv` | raw의 route별 판정 필드 | `scripts/02-process.py` |
| `../results/summary.json` | route별 3회 일치 결과 | `scripts/03-analyze.py` |

큰 원문 JSONL은 메인 저장소 `~/workspace/oss/saturn/.local/experiments/codex-live-reload/`에 저장한다. raw의 `private_log`가 그 상대 경로를 가리킨다. 파일이 50MB를 넘으면 `data/README.md`에 SHA-256과 실제 경로를 추가한다.

## 필드

### `raw/{run_id}.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 식별자 | `20261003T000000Z-027a16f` |
| `trial_id` | string | 없음 | 필수 | trial 식별자 | `questions-1` |
| `condition` | string | 없음 | 필수 | 확인하는 route | `questions.toggle` |
| `ts_utc` | datetime | UTC | 필수 | 행 기록 시각 | `2026-10-03T00:00:00Z` |
| `provider` | string | 없음 | 필수 | provider 이름 | `codex` |
| `process_id` | string | 없음 | 필수 | app-server PID | `12345` |
| `thread_id` | string | 없음 | 필수 | app-server thread id | `thread` |
| `request_result` | string | 없음 | 필수 | RPC·턴 결과 | `success` |
| `user_input_request` | boolean | 없음 | 필수 | 질문 요청 관측 여부 | `true` |
| `approval_request` | boolean | 없음 | 필수 | 승인 요청 관측 여부 | `false` |
| `marker_effect` | boolean | 없음 | 필수 | marker 생성 여부 | `false` |
| `private_log` | string | 저장소 상대 경로 | 필수 | 큰 원문 위치 | `.local/experiments/codex-live-reload/run.jsonl` |
