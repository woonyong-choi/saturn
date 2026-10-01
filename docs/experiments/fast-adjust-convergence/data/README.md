# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `scripts/01-collect.py` 시뮬레이션. 실제 사용자 기록과 모델 호출 없음 |
| 수집 기간 | 2026-10-01~2026-10-01 |
| 개수 | 묶음 기록 70,000줄(조건 7개 × 복제 100개 × 100건 묶음 100개), 분포 기록 7줄 |
| 표본 여부 | 전수. 설계의 모든 조건과 복제를 수집했다 |
| 라벨 | 없음. 정답은 합성 분포에서 난수로 정했다 |
| 알려진 문제 | 없음 |
| 개인정보 | 포함하지 않는다 |
| 라이선스 | 저장소 라이선스를 따른다 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `raw/sim-20261001T105423Z-4f2af18.jsonl` | 복제별 100건 묶음의 행동, 신호, 기준값 | `scripts/01-collect.py` |
| `raw/population-20261001T105423Z-4f2af18.jsonl` | 조건별 사용자 차이 δ와 합성 분포에서 계산한 기준값 | `scripts/01-collect.py` |
| `SHA256SUMS` | `raw/` 파일의 SHA-256 | `run.sh collect` |
| `processed/blocks.csv` | 묶음 기록을 조건, 복제, 묶음 순으로 정렬한 표 | `scripts/02-process.py` |
| `processed/replicates.csv` | 복제별 후반 5,000건 지표 | `scripts/02-process.py` |
| `processed/population.csv` | 분포 기록 표 | `scripts/02-process.py` |

## 필드

### `sim-20261001T105423Z-4f2af18.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T105423Z-4f2af18` |
| `trial_id` | string | 없음 | 필수, 고유 | `{condition}-r{복제}-b{묶음}` | `behavior-3-r000-b001` |
| `condition` | string | 없음 | 필수, 설계의 조건 7개 | 조건 | `behavior-3` |
| `ts_utc` | datetime | 없음 | 필수 | 수집 시작 시각 | `2026-10-01T10:54:23Z` |
| `signal_model` | string | 없음 | `behavior`, `asked` | 신호 출처 | `behavior` |
| `replicate` | integer | 없음 | 0~99 | 복제 번호, 난수 시드 `[120, replicate]`의 입력 식별자 | `0` |
| `block` | integer | 없음 | 1~100 | 100건 묶음 번호 | `1` |
| `acted` | integer | 건 | 0~100 | 행동한 판단 수 | `58` |
| `acted_wrong` | integer | 건 | 0~`acted` | 행동했는데 정답이 아닌 판단 수 | `1` |
| `skipped_correct` | integer | 건 | 0~100 | 행동하지 않았는데 정답인 판단 수 | `29` |
| `wrong_signals` | integer | 건 | 0~`acted_wrong` | 틀림 신호 수 | `1` |
| `missed_signals` | integer | 건 | 0~`skipped_correct` | 놓침 신호 수 | `29` |
| `asks` | integer | 건 | 0~100 | 사용자에게 물은 판단 수 | `2` |
| `restarts` | integer | 회 | 0 이상 | 급변으로 다시 시작한 수 | `0` |
| `frozen_end` | boolean | 없음 | 필수 | 묶음 끝에서 멈춤 상태인지 | `false` |
| `threshold_mean` | number | 확률 | 0.75~0.85 | 판단 당시 기준값의 평균 | `0.753743` |
| `threshold_min` | number | 확률 | 0.75~0.85 | 판단 당시 기준값의 최소 | `0.75` |
| `threshold_max` | number | 확률 | 0.75~0.85 | 판단 당시 기준값의 최대 | `0.8` |
| `threshold_end` | number | 확률 | 0.75~0.85 | 묶음 마지막 신호 반영 뒤 기준값 | `0.75` |

### `population-20261001T105423Z-4f2af18.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `run_id` | string | 없음 | 필수 | 실행 id | `20261001T105423Z-4f2af18` |
| `trial_id` | string | 없음 | 필수, 고유 | `{condition}-population` | `behavior-3-population` |
| `condition` | string | 없음 | 필수, 고유 | 조건이자 입력 식별자 | `behavior-3` |
| `ts_utc` | datetime | 없음 | 필수 | 수집 시작 시각 | `2026-10-01T10:54:23Z` |
| `signal_model` | string | 없음 | `behavior`, `asked` | 신호 출처 | `behavior` |
| `wrong_rate_before` | number | 비율 | 0~1 | 급변 전 기준값 0.8의 행동 중 틀림 비율 | `0.03` |
| `wrong_rate_after` | number | 비율 | 0~1 | 급변 뒤 기준값 0.8의 행동 중 틀림 비율 | `0.03` |
| `delta_before` | number | logit | 필수 | 급변 전 사용자 차이 δ | `-1.251982` |
| `delta_after` | number | logit | 필수 | 급변 뒤 사용자 차이 δ | `-1.251982` |
| `oracle_threshold_after` | number | 확률 | 0.01~0.99 | 급변 뒤 분포에서 행동 중 틀림 비율이 5%가 되는 가장 낮은 기준값 | `0.64299` |
| `balance_threshold_after` | number | 확률 | 0.01~0.99 | 급변 뒤 분포에서 틀림 신호 가중합 × 0.95와 놓침 신호 가중합 × 0.05가 같아지는 기준값 | `0.6943` |
| `wrong_rate_at_low_after` | number | 비율 | 0~1 | 급변 뒤 분포에서 기준값 0.75의 행동 중 틀림 비율 | `0.036825` |
| `wrong_rate_at_high_after` | number | 비율 | 0~1 | 급변 뒤 분포에서 기준값 0.85의 행동 중 틀림 비율 | `0.022831` |
