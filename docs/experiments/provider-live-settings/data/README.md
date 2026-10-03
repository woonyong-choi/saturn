# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 설치된 Claude·Codex CLI의 정적 inventory와 전용 process driver의 stream-json·app-server 원문 수집 |
| 수집 기간 | 수집 뒤 기록 |
| 개수 | 수집 뒤 기록 |
| 표본 여부 | route마다 3회 반복. 모델 호출 상한에 닿으면 그 시점까지 |
| 라벨 | driver가 request result와 실제 fixture 효과를 대조해 붙인다 |
| 알려진 문제 | provider 출력은 모델 변동과 CLI 버전에 영향을 받는다 |
| 개인정보 | 원문 로그는 메인 저장소 `.local/experiments/provider-live-settings/`에 두고, 공개 raw에는 토큰·인증값·개인 절대 경로를 넣지 않는다 |
| 라이선스 | 실험 기록, 별도 재배포 없음 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/{run_id}.jsonl` | 수집 당시 요약과 route별 원문 참조 | `scripts/01-collect.py` |
| `processed/trials.csv` | 공개 raw를 정규화한 route·trial 결과 | `scripts/02-process.py` |
| `results/summary.json` | 보고서 수치와 반복 판정 | `scripts/03-analyze.py` |
| `SHA256SUMS` | 공개 raw의 SHA-256 | `run.sh collect` |

## 필드

### `raw/{run_id}.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 전체 실행 식별자 | `20261003T010203Z-86914ad` |
| `trial_id` | string | 없음 | 필수 | route와 반복 식별자 | `claude-permission-mode-1` |
| `condition` | string | 없음 | 필수 | 측정 route | `claude.set_permission_mode` |
| `ts_utc` | datetime | UTC | 필수 | 공개 요약 기록 시각 | `2026-10-03T01:02:03Z` |
| `provider` | string | 없음 | 필수 | provider 이름 | `claude` |
| `request_result` | string | 없음 | 필수 | 성공·오류·미지원 | `success` |
| `process_id` | string | 없음 | 필수 | 비밀값이 아닌 process 식별자 | `same` |
| `tool_decision` | string | 없음 | 필수 | 실제 도구 판단 | `deny` |
| `fixture_effect` | boolean | 없음 | 필수 | marker 또는 호출 기록이 바뀌었는지 | `false` |
| `next_turn_effect` | boolean | 없음 | 필수 | 다음 턴 행동이 달라졌는지 | `true` |
| `restart_effect` | boolean | 없음 | 필수 | 새 process 뒤 행동이 달라졌는지 | `false` |
| `private_log` | string | 없음 | 필수 | 큰 원문 로그의 메인 저장소 상대 경로 | `.local/experiments/provider-live-settings/...jsonl` |
