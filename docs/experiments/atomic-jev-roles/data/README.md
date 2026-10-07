# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 기존 사람 검토 입력 40건과 생성 규칙으로 만든 관찰 쌍 120건에 Jev를 두 번 호출 |
| 수집 기간 | 2026-10-07 |
| 개수 | 입력 160건, 응답 320건 |
| 표본 여부 | 사람 검토 전수 40건, 관찰 주제 30개에서 개발 10·확인 20개 |
| 라벨 | 사람 검토의 `constraint`·`not_constraint`·`unsure`; 생성 쌍은 설계의 동일 실행·검사·결과 규칙 |
| 알려진 문제 | 사람 검토는 어려운 사례 위주이고 이전 점수를 일부 열람했다. 생성 문장은 실제 작업 로그의 분포가 아니다 |
| 개인정보 | 사람의 입력과 Jev 원요청·응답은 Git 제외 로컬 폴더에만 보존 |
| 라이선스 | 사람 입력 재배포 금지. 생성 규칙과 원문 없는 집계는 저장소 라이선스 |

## 파일

원자료는 작업 트리의 `.local/experiments/atomic-jev-roles/`에 있다. `SATURN_HUMAN_REVIEW_DIR`는 기존 인간 검토 자료가 있는 폴더로 지정한다. 원본의 두 파일은 복사하지 않고 읽기 전용으로 대조한다. [SHA256SUMS](SHA256SUMS)의 경로는 작업 트리 기준이다.

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `sample.json` | 비공개 사용자 문장과 생성 관찰, 라벨. 수집 전 봉인 | `scripts/run.py prepare` |
| `source.json` | 원본·표본 해시와 설계 커밋 | `scripts/run.py prepare` |
| `reservations.jsonl` | 호출 전 예약. 재전송 방지 | `scripts/run.py collect` |
| `responses.jsonl` | Jev 원요청·원응답·usage·지연·실패 | `scripts/run.py collect` |
| [summary.json](../results/summary.json) | 원문 없는 재현 가능 집계 | `scripts/run.py analyze` |

## 필드

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `id` | string | 없음 | 표본에서 고유 | 문장 또는 관찰 쌍 ID | `o00-0` |
| `role` | string | 없음 | `constraint`, `same_observation` | 판단 역할 | `same_observation` |
| `repeat` | integer | 회 | 1, 2 | 같은 입력 반복 | `1` |
| `at_utc` | datetime | UTC | 응답마다 필수 | 요청 시각 | ISO 8601 |
| `status` | string | 없음 | `ok`, `http_error`, `failed`, `oversize` | 호출·파싱 상태 | `ok` |
| `score` | number | 0~1 | 유효할 때만 | Jev가 반환한 예 확률 | `0.8` |
| `usage` | object | 토큰 | 응답이 제공할 때만 | Jev 보고 사용량 | 입력·출력 |

## 재현

```sh
SATURN_HUMAN_REVIEW_DIR=<기존 인간 검토 폴더> ./run.sh verify
SATURN_HUMAN_REVIEW_DIR=<기존 인간 검토 폴더> ./run.sh analyze
```

검증은 원자료 예약과 응답의 일대일 대응, 원본 두 파일과 표본의 해시, 집계 재생성의 바이트 일치를 확인한다. 공개 집계만으로 비공개 인간 입력의 의미 판정을 재현할 수는 없다. 인증 키는 터미널에서 숨김 입력으로 받아 메모리에서만 사용했고 저장하지 않았다.
