# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | Claude 프로젝트 JSONL 읽기 전용 추출, 같은 파일의 직전 사용자 입력과 assistant text 발췌 |
| 수집 기간 | 2026-10-04 |
| 개수 | [집계 JSON](../results/summary.json)의 `selection`, `labels`, `receipts` |
| 표본 여부 | 프로젝트 층별 비례 표본을 계획하고 적격 모집단 부족 시 전수 |
| 라벨 | 독립 Codex 두 모델, 불일치 별도 재판정, 사람 검토 미실시 |
| 알려진 문제 | Saturn 상태 복원, 한 사용자의 프로젝트 편중, 발췌 잘림 |
| 개인정보 | 원문·라벨·응답 비공개, 비밀값·이메일·절대 경로 가림 |
| 라이선스 | 원자료 재배포하지 않음, 공개 코드·집계는 저장소 라이선스 |

## 파일

원자료는 메인 저장소의 Git 제외 실험 저장소 한 곳에만 둔다. 위치는 `scripts/storage.py`의 `PRIVATE`다. 공개 체크섬은 비공개 원자료 파일명과 SHA-256만 담는다. 원본 로그 전체를 복사하지 않는다.

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| census.json | 원본 경로·해시·크기 | 01-collect.py |
| population.json, sample.json | 가린 입력·직전 작업, 층화 표본 | 01-collect.py |
| selection.json | 층별 모집단·선택 수, 제외 수 | 01-collect.py |
| calls.jsonl | 호출 직전 예약, 종류·번호·시각 | storage.py |
| codex/ | 프롬프트·스키마·stdout·stderr 영수증 | 02-label.py |
| labels-*.json | 최초 독립 라벨과 재판정 | 02-label.py |
| jev.jsonl | 조건·반복별 요청·응답·실패·크기 | 03-judge.py |
| final-labels.json, human-review.json | 최종 라벨과 사람 확인 30개 | 04-analyze.py |
| results/summary.json | 공개 집계의 정본 | 04-analyze.py |
| results/tables/thresholds.csv | 기준값 곡선 | 04-analyze.py |
| data/SHA256SUMS | 비공개 입력 체크섬 | 04-analyze.py |

## 필드

### 정규화 Jev 관측 행

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| run_id | string | 없음 | 필수 | 봉인 커밋 포함 실행 식별자 | 형식 `YYYYMMDDTHHMMSSZ-hash` |
| trial_id | string | 없음 | 고유 | 입력·조건·반복 식별자 | `id-A-r1` |
| id | string | 없음 | 필수 | 입력 가명 식별자 | 해시 |
| condition | string | 없음 | A 또는 B | state 정보 조건 | A |
| repeat | integer | 회 | 1~3 | 반복 번호 | 1 |
| ts_utc | string | UTC | ISO 8601 | 요청 시각 | 실행 시각 |
| request_bytes | integer | 바이트 | 0 이상 | compact UTF-8 JSON 크기 | 측정값 |
| request | object | 없음 | 필수 | model·state·questions | 실제 engine 형식 |
| status | string | 없음 | ok, oversize, http_error, failed, incomplete | 성공·실패 구분 | ok |
| http_status | integer 또는 null | 없음 | 응답 없으면 null | HTTP 상태 | 200 |
| raw_response | string | 없음 | 응답 있으면 저장 | 키 문자열을 가린 응답 | JSON 문자열 |
| answers | object | 확률 | 유효 응답만 | P(yes), choice 분포 | 0~1 |
| latency_ms | number | ms | 외부 호출만 | 요청 소요 시간 | 측정값 |

JSON 결측은 null, CSV 결측은 빈 칸이다. 원자료 없이는 정확도 분석을 다시 실행할 수 없다. 공개 `verify`의 계약 검사와 공개 집계 검토는 원자료 없이 가능하다. 키는 키체인에서 프로세스 환경으로만 전달하고 라벨러 환경에서는 제거한다.
