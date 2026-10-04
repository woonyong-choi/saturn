# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `./run.sh collect`의 층별 하위 명령. 모든 명령을 스크립트가 직접 실행하고 종료 코드와 시간만 기록 |
| 수집 기간 | 2026-10-04~2026-10-04 |
| 개수 | 행 106개 (파일 8개) |
| 표본 여부 | 전수. 조건마다 정해진 횟수 |
| 라벨 | 없음. 결과는 종료 코드와 `securityd` 로그 사건으로 스크립트가 분류 |
| 알려진 문제 | 확인 창 사건을 시험에 맞추는 규칙이 초 단위 시각에 기대어 근사다. Claude 통제 호출 1회는 Bash 결과에 종료 표식이 없어 무효 |
| 개인정보 | 항목 값은 어디에도 없다. 로그인 이메일을 기록하지 않고 `loggedIn` 불리언만 남긴다. 시험 항목 이름과 `securityd` 사건의 실행 파일 이름만 있다 |
| 라이선스 | 저장소 라이선스 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/codex-sandbox-{실행 id}.jsonl` | Codex 샌드박스 시험과 통제 | `scripts/01-collect.py codex-sandbox` |
| `raw/claude-sandbox-{실행 id}.jsonl` | Claude Code 샌드박스 호출 10회와 로그인 확인 | `scripts/01-collect.py claude-sandbox` |
| `raw/keychain-acl-{실행 id}.jsonl` | 키체인 접근 제어 시험 5쌍 | `scripts/01-collect.py keychain-acl` |
| `raw/*-prompts-{실행 id}.jsonl` | 위 세 실행 시간대의 확인 창 표시와 응답 사건(로그 출처) | `scripts/01-collect.py {acl,codex,claude}-prompt-log` |
| `raw/env-{실행 id}.jsonl` | 환경 격리 단위 테스트 결과 | `scripts/01-collect.py env` |
| `raw/hook-{실행 id}.jsonl` | 훅 수정 브랜치의 앞선 커밋 수 | `scripts/01-collect.py hook` |
| `processed/trials.csv` | 층별 최신 실행을 합친 표 | `scripts/02-process.py` |

## 필드

### `raw/*.jsonl` 공통

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 시각과 커밋 7자리 | `20261004T092123Z-f2334e7` |
| `trial_id` | string | 없음 | 필수, 파일 안 고유 | 층 이름과 순번 | `codex-sandbox-005` |
| `condition` | string | 없음 | 필수 | 방어 설정과 호출 형태 | `:read-only/direct` |
| `ts_utc` | datetime | UTC | 필수 | 행을 쓴 시각 | `2026-10-04T09:21:23Z` |
| `outcome` | string | 없음 | `accessed`, `denied`, `dialog` 또는 없음 | 스크립트 판정 | `denied` |
| `exit_code` | integer | 없음 | 없을 수 있음 | 시험 명령 종료 코드 | `44` |
| `elapsed_s` | number | 초 | 없을 수 있음 | 호출 경과 시간 | `3.32` |
| `bash_exit` | string | 없음 | Claude 호출에만 | Bash 결과의 `exit=` 값 | `0` |
| `event`, `client`, `event_utc` | string | UTC 시각 | `*-prompts` 파일의 `event` 행에만 | `securityd` 사건 종류, 요청한 실행 파일 이름, 시각 | `prompt_displayed`, `security`, `09:21:47.383` |
