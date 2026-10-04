# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 재수집은 `scripts/01-collect.py --claude-default-login`. `scripts/01-collect.py`가 전용 Codex app-server와 Claude stream-json process를 시작하고, marker 시작 후 자기 provider PID만 `SIGKILL`한 뒤 재개 event와 marker를 관찰했다. |
| 수집 기간 | 실행 시각에 `env.json`으로 기록 |
| 개수 | 확인 조건 4개 × 3회와 가능하면 Claude Task 1회 |
| 표본 여부 | 사용자가 정한 반복 수의 목적 표본 |
| 라벨 | `scripts/03-analyze.py`가 marker와 event의 규칙으로 파생 |
| 알려진 문제 | 원래 command process가 provider PID 종료 뒤 살아 있을 수 있어 `complete`와 `start`를 분리해 기록한다. |
| 개인정보 | provider 응답 원문과 환경은 메인 저장소 `.local/experiments/crash-resume/`에만 두며, public raw에는 요약과 private log 경로만 둔다. token·auth 내용은 기록하지 않는다. |
| 라이선스 | 저장소 실험 자료; provider 출력 원문은 private `.local`에 보관한다. |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/{provider}-{실행 id}.jsonl` | 반복별 요약 관측값과 판정 입력 | `scripts/01-collect.py` |
| `processed/observations.csv` | raw를 행·열로 펼친 재생성 표 | `scripts/02-process.py` |
| `../env.json` | 실행 환경·버전·모델·호출 수 | `scripts/01-collect.py` |
| `SHA256SUMS` | raw 파일의 SHA-256 | `run.sh collect` |

## 필드

### `raw/*.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261003T000000Z-abcdef0` |
| `trial_id` | string | 없음 | 필수·고유 | 조건과 반복 식별자 | `codex-raw-1` |
| `condition` | string | 없음 | 필수·값 목록 | 사전 등록 조건 | `codex.raw-resume` |
| `ts_utc` | datetime | UTC | 필수 | 행 기록 시각 | `2026-10-03T00:00:00Z` |
| `provider` | string | 없음 | 필수·`codex` 또는 `claude` | provider | `codex` |
| `model` | string | 없음 | 필수 | 모델 이름 | `gpt-5.6-luna` |
| `request_result` | string | 없음 | 필수 | 주요 provider 요청 상태 | `observed` |
| `marker_start_count` | integer | 개수 | 0 이상 | marker start 행 수 | `2` |
| `marker_complete_count` | integer | 개수 | 0 이상 | marker complete 행 수 | `1` |
| `marker_touch_count` | integer | 개수 | 0 이상 | 완료 marker의 관측 수 | `1` |
| `resume_child_execution` | boolean 또는 null | 없음 | 식별 불가면 null | 재개 뒤 자식/끊긴 턴 재실행 여부 | `true` |
| `observation_status` | string | 없음 | `confirmed`, `unstable`, `cannot_distinguish` | 반복 내 또는 개별 행 판정 상태 | `confirmed` |
| `recollect_of` | string | 없음 | 재수집 행만 | 다시 수집한 앞 실행 id | `20261003T085117Z-0e500f0` |
| `default_login` | boolean | 없음 | Claude 재수집 행만 | 전용 설정 폴더 없이 기본 로그인으로 실행했는지 | `true` |
| `resume_process_alive` | boolean | 없음 | Claude 재수집 행만 | 관찰 구간 끝에 재개 process가 살아 있는지 | `true` |
| `resume_init_event` | boolean | 없음 | Claude 재수집 행만 | 재개 process가 `system/init`을 냈는지 | `false` |
| `initial_subagent_bash` | integer | 개수 | Claude 재수집 행만 | 강제 종료 전 하위 에이전트가 낸 `Bash` 호출 수 | `1` |
| `resume_parent_tool_events` | integer | 개수 | Claude 재수집 행만 | 재개 뒤 `parent_tool_use_id`가 붙은 이벤트 수 | `0` |
| `resume_reason` | string 또는 null | 없음 | Claude 재수집 행만 | 재개 결과 이벤트의 `resume_reason` | `interrupted_turn` |
| `private_log` | string | 저장소 상대 경로 | 필수 | 큰 원문 event log 경로 | `.local/experiments/crash-resume/...` |
