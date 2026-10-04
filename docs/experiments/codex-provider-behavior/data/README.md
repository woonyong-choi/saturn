# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 설치된 공식 `codex app-server`(codex-cli 0.158.0, 모델 `gpt-5.6-luna`)를 `scripts/01-collect.py`가 시험마다 새 process로 구동해 JSON-RPC 원문, 승인 요청과 응답, 파일·프로세스·훅 로그 효과를 기록했다. 시험마다 전용 `CODEX_HOME`, 작업 폴더, thread를 새로 만들었다. |
| 수집 기간 | 2026-10-04~2026-10-04 |
| 개수 | 시험 행 117(조건 36개, 조건마다 3회 기준), 키체인 정리 행 2. 모델 호출 116회(사전 호출 2회 포함, 한도 120회). 수집 중 driver 실패 행 0 |
| 표본 여부 | 전수. 조건마다 3회 |
| 라벨 | 없음. 판정은 `scripts/02-process.py`와 `scripts/03-analyze.py`가 이벤트와 효과로 하며 사람이 라벨을 붙이지 않았다 |
| 알려진 문제 | 결과를 본 뒤 재수집한 조건(`steer_review_turn`, `sandbox_cargo`, `sandbox_write`)은 첫 시도 행도 남겼고 `report.md` 설계와 다른 점에 이유를 적었다. `hook_none` 읽기 전용 조건의 가짜 값은 샌드박스가 먼저 막아 출력되지 않았다. 이벤트는 델타·토큰 사용량·속도 제한 알림을 뺐고 긴 문자열은 240자로 잘랐다 |
| 개인정보 | 계정 정보를 저장하지 않았다(`account/read`는 로그인 여부만 기록, 요청은 이메일 키를 가림). `scripts/redact-raw.py`로 계정 연동 서비스의 도구 이름 목록(`mcpServerStatus/list` 응답)을 개수로 바꾸고 OS 사용자 이름과 홈 경로를 지웠다. 인증 파일은 읽거나 복사하지 않았다. 가짜 키체인 항목(`saturn-test-dummy`)은 끝에 삭제했다 |
| 라이선스 | 저장소 라이선스를 따른다 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/{실행 id}-{주제}.jsonl` | 시험 하나가 한 줄. 이벤트, 승인 기록, 시험이 모은 관측(`facts`) | `scripts/01-collect.py`, 개인 정보 처리 `scripts/redact-raw.py` |
| `raw/{실행 id}-calls.jsonl` | 그 실행에서 센 모델 호출 순번 | `scripts/01-collect.py` |
| `raw/pilot-calls.jsonl` | 설계 전 사전 호출 2회의 순번 | `scripts/common.py`의 호출 기록 |
| `processed/trials.csv`, `processed/trials.jsonl` | 시험마다 판정용 관측값 | `scripts/02-process.py` |
| `SHA256SUMS` | `raw/` 파일의 SHA-256 | `cd data && shasum -a 256 raw/* > SHA256SUMS` |

## 필드

### `raw/{실행 id}-{주제}.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | `YYYYMMDDTHHMMSSZ-{커밋 7자리}` | `20261004T092939Z-0d046b0` |
| `trial_id` | string | 없음 | 필수, 실행 안에서 고유 | `{조건}-{번호}` | `steer_accept-1` |
| `condition` | string | 없음 | 필수, `design.md`의 시험 이름 또는 추가 시험 이름 | 조건 | `child_signals` |
| `topic` | string | 없음 | 필수 | 시험 묶음 | `child` |
| `ts_utc` | datetime | UTC | 필수 | 시험 끝 시각 | `2026-10-04T09:29:39Z` |
| `provider`, `model` | string | 없음 | 필수 | 고정값 `codex`, `gpt-5.6-luna` | `codex` |
| `status` | string | 없음 | 필수, `completed`, `no_call`, `driver_failure` 등 | 턴 상태 | `completed` |
| `call_ordinals` | integer 목록 | 없음 | 필수 | 이 시험이 쓴 모델 호출의 전역 순번 | `[24, 25]` |
| `events` | 객체 목록 | ms | 필수 | `ms`는 첫 이벤트 뒤 경과, `dir`은 `in`·`out`, `msg`는 JSON-RPC 원문(축약) | `{"ms":1156,"dir":"out","msg":{...}}` |
| `approvals` | 객체 목록 | 없음 | 필수 | driver가 받은 승인 요청의 method, 요청 번호, 요청 값, 보낸 응답 | `{"method":"item/commandExecution/requestApproval",...}` |
| `facts` | 객체 | 없음 | 필수 | 시험이 직접 모은 효과: 파일 존재, 프로세스 수, 명령 결과, 훅 로그, steer 응답 등 | `{"marker_effect":true}` |
| `stderr_head` | string | 없음 | 선택 | app-server 표준 오류 앞 600자 | 빈 문자열 |

### `processed/trials.csv`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id`, `trial_id`, `condition`, `topic`, `ts_utc`, `status` | string | 없음 | 필수 | raw의 같은 이름 | `steer_accept-1` |
| `model_calls` | integer | 회 | 필수, 0 이상 | 시험이 쓴 모델 호출 수 | `1` |
| `request_methods` | string | 없음 | 필수, 빈 값 가능 | 받은 승인 요청 method 목록 | `item/fileChange/requestApproval` |
| `declined_commands` | integer | 개 | 필수 | 규칙에 없어 `decline`한 명령 승인 요청 수 | `1` |
| `child_*`, `parent_*`, `spawn_attempted`, `wait_called`, `sleep_alive_*` | boolean, integer | ms, 개 | 시험별 | 자식 세션과 멈춤 관측(`child_thread_id`, `child_started_notification`, `parent_after_child` 등) | `true` |
| `*_ok`, `*_error`, `*_turn_id`, `*_pattern_match`, `marker_effect`, `steer_*`, `turn_start_*` | boolean, string | 없음 | 시험별 | 끼워 넣기 응답과 효과. `*_pattern_match`는 Saturn `is_no_active_turn` 패턴에 걸리는지 | `no active turn to steer` |
| `mcp_attempts`, `elicitation_requests`, `request_meta_keys`, `tool_name_in_meta`, `tool_name_in_message`, `fixture_calls` | integer, boolean, string | 개 | 시험별 | MCP 승인 관측 | `2` |
| `command_requests`, `executions`, `available_decisions_first`, `file_requests`, `file_changes` | integer, string | 개 | 시험별 | `항상 허용` 관측 | `1` |
| `command_approvals`, `last_exit`, `denial_in_output`, `artifact_exists`, `outside_exists`, `sub_exists`, `fixture_error` | integer, boolean | 없음 | 시험별 | 샌드박스 관측 | `101` |
| `start_*`, `resume_*`, `curl_*`, `settings_updated_*`, `config_read_empty`, `turn_started_keys` | boolean, integer, string | 없음 | 시험별 | 적용 설정 보고와 네트워크 관측 | `200` |
| `account_present`, `account_type`, `auth_is_symlink` | boolean, string | 없음 | 시험별 | 로그인 공유 관측 | `true` |
| `hook_*`, `command_executed`, `dummy_in_output`, `fake_key_in_output`, `edit_applied`, `approval_requests` | boolean, integer, string | 없음 | 시험별 | 훅 관측 | `blocked` |
