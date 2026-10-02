# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `scripts/01-collect.py`가 합성 시나리오와 fixture를 만들고 Claude Code, Codex CLI app-server의 원문 이벤트를 JSON Lines로 기록한다. |
| 수집 기간 | 2026-10-02 |
| 개수 | exp1 64 시행, exp2 20 시행, exp3 10 시행, exp4 12 시행 |
| 표본 여부 | 이슈에 정한 반복 수의 전수 |
| 라벨 | 실험 1은 기존 자동 채점기가 붙이고, 실험 2~4는 원문 이벤트와 fixture 로그가 붙인다. |
| 알려진 문제 | exp4는 Claude가 일부 `sleep` 요청을 거부하거나 백그라운드화했고, Codex TUI는 인증 오류로 명령을 실행하지 못했다. 자세한 내용은 report.md에 적었다. |
| 개인정보 | 토큰·계정 값은 저장하지 않는다. 개인 절대 경로는 `~`로 바꾼다. |
| 라이선스 | 저장소 라이선스를 따른다. |

## 원자료 상태

수집을 마쳤다. 최신 본시험 원자료와 TUI 화면은 `raw/`에 있고, provider 원문은 저장소 `.local/experiments/provider-decision-checks/`에 있다. `SHA256SUMS`로 raw 무결성을 확인한다.

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/exp1-*.jsonl` | 패킷, provider 답, 시나리오 요약 | `scripts/01-collect.py` |
| `raw/exp2-*.jsonl` | 재개 문구별 실행 명령과 상태 확인 | `scripts/01-collect.py` |
| `raw/exp3-*.jsonl` | Codex·Claude 입력 요청 원문과 응답 | `scripts/01-collect.py` |
| `raw/exp4-*.jsonl` | 중단·강제 종료 원문 이벤트와 문구 | `scripts/01-collect.py` |
| `raw/exp4-*.txt` | tmux TUI 화면 원문 | `scripts/01-collect.py` |
| `processed/*.csv` | 원자료에서 계산한 시행 단위 표 | `scripts/02-process.py` |
| `results/summary.json` | 보고서 수치의 유일한 원천 | `scripts/03-analyze.py` |

## 필드

### 공통 JSON Lines 필드

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 식별자 | `20261002T120000Z-590e5cc` |
| `trial_id` | string | 없음 | 필수, 파일 안 고유 | 시행 식별자 | `exp2-claude-no-warning-1` |
| `condition` | string | 없음 | 필수 | 시행 조건 | `tail-preserve` |
| `ts_utc` | string | ISO 8601 | 필수 | 기록 시각 | `2026-10-02T12:00:00Z` |
| `provider` | string | 없음 | 해당 실험에서 필수 | provider 이름 | `codex` |
| `model` | string | 없음 | 해당 실험에서 필수 | 모델 이름 | `gpt-5.6-luna` |

### 실험별 필드

| 파일 | 추가 필드 |
|---|---|
| `raw/exp1-*.jsonl` | `scenario_id`, `strategy`, `packet_tokens`, `question_results`, `stdout_sha256`, `status` |
| `raw/exp2-*.jsonl` | `state_checked_first`, `duplicate_touch`, `commands`, `approval_events`, `status` |
| `raw/exp3-*.jsonl` | `request_method`, `request_params`, `response`, `server_observation`, `round_trip`, `status` |
| `raw/exp4-*.jsonl` | `stop_kind`, `display_text`, `screen_path`, `raw_screen_path`, `status` |

결측값은 JSON `null`, CSV 빈 칸으로 쓴다. 원문 이벤트의 비밀 값과 개인 절대 경로는 수집기에서 제거한다.
