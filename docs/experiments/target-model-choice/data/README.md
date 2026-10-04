# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 공식 CLI 모델 확인, 사용자 대화 JSONL의 사용자 메시지와 프로젝트 식별 메타데이터, 공식 router HTTPS 요청 |
| 수집 기간 | env.json의 실행 날짜 |
| 개수 | results/summary.json의 sampling·adjudication·calls |
| 표본 여부 | 시드 338100, 종류·프로젝트·기록 provider 층화, 첫 작업 100개 |
| 라벨 | gpt-6-sol 2회, 불일치 시 3회째, 최선·허용 집합 다수결 |
| 알려진 문제 | 최초 출처 필터에서 자동 실행 세션 혼입, 응답 열람 전 표본 폐기·재추출. 이전 예약도 예산·비용에 포함 |
| 개인정보 | 원문·응답 비공개, 비밀값·이메일·절대 경로 가림, 공개 자료는 집계와 해시 |
| 라이선스 | 원자료 비공개, 실험 스크립트는 저장소 라이선스 |

## 파일

원문·응답은 메인 저장소에서 Git이 제외하는 실험 전용 저장소에만 있다. 위치는 [runtime.py](../scripts/runtime.py)의 `PRIVATE`다. 원자료를 새 위치로 복사하지 않으며, 소유자만 그 위치에서 `run.sh`로 분석을 재현한다. 공개 저장소만으로 원문을 복구하거나 결과를 재계산할 수 없다. 공개한 코드·설계·집계와 SHA-256으로 계산 절차와 소유자 측 데이터 무결성을 확인한다.

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| SHA256SUMS | 비공개 원자료·응답·봉인과 공개 집계의 해시, 절대 경로 제외 | 소유자 측 최종 해시 생성 |
| census.json, samples.json, sampling.json | 출처 목록·가린 작업과 후속 사용자 입력·추출 수 | 01-prepare.py |
| sample-seal.json, design-seal.json, collection-seal.json | 입력과 계약 해시·커밋 | 준비 단계 |
| raw/*.json | 프롬프트 또는 HTTP 요청·응답·상태·usage | runtime.py |
| calls.jsonl | 실행 직전 잠금·fsync한 호출 예약 | runtime.py |
| labels.json, gold-seal.json | 독립 정답·다수결과 해시 | 02-collect.py |
| processed/observations.jsonl | 표본·조건·반복별 파생 관측 | 03-analyze.py |
| results/summary.json, results/conditions.csv | 공개 집계와 조건별 표 | 03-analyze.py |

## 필드

### 정규화 관측

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| run_id | string | 없음 | 필수 | 사전 등록 커밋 식별자 | 7자리 해시 |
| trial_id | string | 없음 | 고유 | 조건·표본·반복 | jev-a1b2c3d4e5f60718-1 |
| condition | string | 없음 | 필수 | 판단 모델 | jev |
| ts_utc | datetime | UTC | 미실행 시 null | 호출 예약 시각 | ISO 8601 |
| sample_id, project_id | string | 없음 | 필수 | 가명 표본·프로젝트 | 해시 |
| selected, effective | string | 없음 | selected 실패 시 null | 원래 답·대체 후 모델 | codex/gpt-6-sol |
| valid, fallback, low_confidence | boolean | 없음 | 필수 | 형식 유효·대체·확신 미달 | false |
| best_match, raw_hit, effective_hit | boolean | 없음 | 정답 제외 시 null | 최선 일치·허용 집합 적중 | true |
| switch, switch_hit | boolean | 없음 | 미전환 switch_hit null | 기본 provider 전환·그 적중 | false |
| latency_s | number | 초 | 미실행 시 null | CLI·HTTP 벽시계 지연 | 1.0 |
| cost_usd | number | USD | 단가·usage 미상 시 null | API 상당 추정 | 0.000001 |

JSON의 결측은 null이다. CSV의 결측은 빈 칸이다. 같은 모델을 가리키는 Claude default·opus는 채점에서 동등하게 처리한다. 응답이 없는 예약을 다시 전송하지 않으며, 검증은 예약·완료·미완료를 구분한다.
