# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `scripts/01-collect.py`가 전용 `CODEX_HOME`의 공식 Codex app-server를 비차단 바이트 읽기와 줄 큐로 구동하고 JSON-RPC 원문을 기록했다. |
| 수집 기간 | 2026-10-03~2026-10-03 UTC |
| 개수 | formal trial 21행, 수집기 실패 1행, raw JSONL 2개 |
| 표본 여부 | 설계한 7개 경로와 각 3회 전수 |
| 라벨 | driver가 승인 요청 method, 응답, marker·완료 이벤트·MCP fixture 기록을 대조해 붙였다. |
| 알려진 문제 | MCP 조건은 모델 도구 시도가 1/3회였고, 첫 driver 경로 오류 1회는 formal 분석에서 분리했다. |
| 개인정보 | `auth.json`은 원본을 읽지 않고 전용 home에서 심볼릭 링크로만 참조한다. raw/private log에서 토큰·인증값을 가린다. |
| 라이선스 | Saturn 저장소 문서 산출물 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/{run_id}.jsonl` | redaction한 trial 요약과 private 원문 경로 | `scripts/01-collect.py` |
| `processed/trials.csv` | route별 실제 관측과 판정 | `scripts/02-process.py` |
| `../results/summary.json` | 조건별 3회 판정과 호출 수 | `scripts/03-analyze.py` |

큰 원문 JSONL은 메인 저장소 `~/workspace/oss/saturn/.local/experiments/provider-permission-real/`에 저장한다. raw의 `private_log`가 저장소 상대 경로를 가리킨다. 파일이 50MB를 넘으면 이 문서에 SHA-256과 실제 경로를 추가한다.

## 필드

### `raw/{run_id}.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 식별자 | `20261004T000000Z-beb46b1` |
| `trial_id` | string | 없음 | 필수 | 조건과 반복 식별자 | `workdir-edit-1` |
| `condition` | string | 없음 | 필수 | 확인 경로 | `workdir_edit` |
| `ts_utc` | datetime | UTC | 필수 | 행 기록 시각 | `2026-10-04T00:00:00Z` |
| `provider` | string | 없음 | 필수 | provider 이름 | `codex` |
| `model` | string | 없음 | 필수 | 모델 이름 | `gpt-5.6-luna` |
| `process_id` | string | 없음 | 필수 | app-server PID | `12345` |
| `thread_id` | string | 없음 | 필수 | app-server thread ID | `thread_...` |
| `approval_request` | boolean | 없음 | 필수 | 승인 요청 도착 여부 | `true` |
| `approval_methods` | array | 없음 | 필수 | 승인 요청 method | `["item/fileChange/requestApproval"]` |
| `approval_response` | string | 없음 | 필수 | driver 응답 | `decline` |
| `marker_effect` | boolean | 없음 | 필수 | 기대 marker 생성 여부 | `false` |
| `command_effect` | boolean | 없음 | 필수 | 명령 완료 이벤트 여부 | `true` |
| `mcp_effect` | boolean | 없음 | 필수 | fixture 호출 기록 여부 | `false` |
| `turn_status` | string | 없음 | 필수 | 턴 종료 상태 | `completed` |
| `model_call_ordinal` | integer | 회 | 필수 | 전역 호출 순번 | `1` |
| `private_log` | string | 저장소 상대 경로 | 필수 | 큰 원문 위치 | `.local/experiments/provider-permission-real/...jsonl` |
