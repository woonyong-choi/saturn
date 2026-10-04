# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 앞 실험 표본과 로컬 실제 대화 기록의 프로젝트·입력 형태 층화 표본 |
| 수집 기간 | 2026-10-04 |
| 개수 | 입력 400건, 앞 실험 정답 점검 90건 |
| 표본 여부 | 앞 실험 100건과 신규 300건. 정의 점검에 사용한 사전 열람 표본 포함 |
| 라벨 | sol·Astra 독립 합의, 불일치는 다른 지시의 Astra로 재판정. 사람 정답 아님 |
| 알려진 문제 | 한 사용자·프로젝트 편중, 희소한 붙여넣기 추정군, 뒤 맥락 창 제한, 자동 요약의 작성자 구분 불가, CLI 프로젝트 지침 격리 미확인 |
| 개인정보 | 원문·응답은 Git 제외 저장소에만 보존. 공개 자료는 집계·해시·스크립트 |
| 라이선스 | 대화 원문 재배포 안 함. 공개 코드·집계는 저장소 라이선스 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| 비공개 samples.json | 원자료 식별·앞뒤 맥락·추출 층·사전 열람 여부 | scripts/02-prepare.py |
| 비공개 raw/*.json | 예약한 호출의 요청·응답·usage·실패·지연 | scripts/01-audit.py, scripts/03-collect.py |
| 비공개 gold.json | 각 라벨러 판정과 합의 | scripts/03-collect.py |
| 비공개 calls.jsonl | 호출 전 잠금 예약 장부 | scripts/runtime.py가 재사용하는 실행 경계 |
| 비공개 review-followup.json | 실행 격리·거절 중단의 검토 결과 | 저장 실행 명령·상위 지침 파일 대조 |
| 비공개 processed.jsonl | 원응답에서 만든 관측 행 | scripts/04-process.py |
| [SHA256SUMS](SHA256SUMS) | 원자료 파일 해시 | scripts/05-analyze.py |
| [집계](../results/summary.json) | 보고서의 측정 수치·구간·판정 | scripts/05-analyze.py |
| [기준값 표](../results/tables/thresholds.csv) | 조건별 기준값 곡선 | scripts/05-analyze.py |
| [정책 표](../results/tables/policies.csv) | 자동 기준·묻기 하한 조합 | scripts/05-analyze.py |

## 필드

### 정규화 관측

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| run_id | string | 없음 | 필수 | 설계 봉인 커밋 식별자 | 해시 접두사 |
| trial_id | string 또는 null | 없음 | 호출 예약 시 고유 | 호출과 관측 연결 | 조건·표본·반복 식별자 |
| ts_utc | datetime 또는 null | UTC | 저장 응답에서 필수 | 요청 시작 시각 | ISO 8601 |
| sample_id | string | 없음 | 조건·반복 안에서 고유 | 입력 식별자 | 해시 접두사 |
| condition | string | 없음 | 네 조건 중 하나 | current, scope, scope_gate, astra | scope |
| repeat | integer | 없음 | Jev 1~3, Astra 1 | 같은 요청의 회차 | 1 |
| gold | string | 없음 | 세 범주 | constraint, not_constraint, uncertain | uncertain |
| probability | number 또는 null | 없음 | 0~1 | 조건별 최종 점수 | 0.9 |
| task_only | number 또는 null | 없음 | 0~1 | 작업 한정 보조 확률 | 0.5 |
| valid | boolean | 없음 | 필수 | 응답 계약·모델 유효성 | true |
| latency_s | number 또는 null | 초 | 0 이상 | 호출 벽시계 시간 | 0.3 |
| cost_usd | number 또는 null | USD | 0 이상 | 실제 usage의 입력당 API 상당 비용 | 0.0001 |
| previously_seen | boolean | 없음 | 필수 | 앞 실험에 있던 표본 | false |

### 공개 비율

비율 객체의 k는 분자, n은 분모, value는 k/n, ci는 지정한 신뢰구간이다. 빈 분모는 JSON null, CSV 빈 칸이다. 반복 일치의 분모는 입력 수이며 반복 수로 곱하지 않는다. 원자료·파일 경로·모델 응답의 이유 문장은 공개 집계에 넣지 않는다.

재현에는 이 실험과 model-compare의 비공개 원자료가 모두 필요하다. 앞 실험 파일의 해시는 summary.json의 audit.source_hashes에 기록한다. 원자료를 공개 저장소에 복제하지 않는다.
