# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | `run.sh inventory`와 `run.sh collect`의 source 가용성 사전 검사 |
| 수집 기간 | 2026-10-07~2026-10-07 |
| 개수 | 독립 실과제 source 스냅샷 0개, 대응쌍 0개, provider 호출 0회 |
| 표본 여부 | 확인 평가 80쌍은 미등록. 기능 진단 후보도 미등록 |
| 라벨 | 없음 |
| 알려진 문제 | 실제 source 대화·작업 폴더·숨긴 검사가 갖춰진 후보가 없다 |
| 개인정보 | 원문과 응답을 수집하지 않았다. 향후 원자료는 Git 제외 경로에만 둔다 |
| 라이선스 | 새 원자료 없음 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `results/summary.json` | 가용성 검사 결과와 실측 미실행 상태 | `run.sh inventory`, 수동으로 출력 대조 |
| `data/SHA256SUMS` | 원자료가 없어 빈 해시 목록 | 원자료 수집 뒤 생성 |
| `.runtime/jev-real-task-effect/manifest.json` | 향후 source와 과제의 비공개 등록 파일 | 수집 전 작성 |
| `.runtime/jev-real-task-effect/raw/{과제}-{조건}.json` | 향후 실제 engine 알림과 기록 행 원본 | `scripts/01-collect.py` |

## 필드

### `results/summary.json`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `real_source_snapshots` | integer | 개 | 0 이상 | 등록 가능한 실제 source 수 | `0` |
| `runnable_pairs` | integer | 쌍 | 0 이상 | 봉인된 source에서 실행 가능한 대응쌍 수 | `0` |
| `provider_calls` | integer | 회 | 0 이상 | 사전 검사 중 provider 호출 수 | `0` |
| `effect_status` | string | 없음 | `not_measured` | 효과 가설의 측정 상태 | `not_measured` |

기능 진단과 80쌍 확인 평가의 원자료가 생기면 출처, 제외 사유, 개인정보 처리, 전체 필드 계약을 별도 봉인 자료와 보고서에 기록한다. 지금 파일은 효과 실험의 원자료가 아니다.
