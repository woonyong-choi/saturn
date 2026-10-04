# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | constraint-deep의 사용자 턴 추출·가림 규칙, 같은 세션의 앞뒤 assistant 텍스트 추가 |
| 수집 기간 | 2026-10-04 |
| 개수 | 표본·판단 불가·호출 수는 [집계](../results/summary.json)의 sampling·gold·calls |
| 표본 | 프로젝트·프로젝트 길이·입력 형태 층별 고정 난수 표본 |
| 라벨 | sol 독립 사후 판정 두 번, 불일치만 세 번째 다수결 |
| 알려진 문제 | 한 사용자 기록, 제한된 뒤 맥락, 입력 작성자는 형태로 추정 |
| 개인정보 | 원문·응답·정답은 메인 저장소의 Git 제외 실험 저장소에만 보관. 공개 파일은 집계·해시 |
| 라이선스 | 비공개 개인 대화 기록, 원문 재배포 제외 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| 비공개 census.json | 원천 파일 경로·해시·턴 수 | 01-prepare.py |
| 비공개 samples.json | 표본 ID·층·앞 state·뒤 실제 대화 | 01-prepare.py |
| 비공개 raw/*.json | 요청·CLI 또는 HTTP 원응답·시각·지연 | runtime.py |
| 비공개 gold.json | sol 각 회차와 최종 다수결 | 02-collect.py |
| 비공개 calls.jsonl | 호출 전 잠금 예약 장부 | runtime.py |
| 비공개 processed.jsonl | 검증된 확률·정답·실패·사용량·비용 | 03-process.py |
| [SHA256SUMS](SHA256SUMS) | 원자료와 단계별 봉인의 불변 해시 | 수집 완료 뒤 봉인 |
| [집계](../results/summary.json) | 보고서 모든 결과 수치 | 04-analyze.py |
| [모델 표](../results/tables/models.csv) | 모델별 점추정·비용·지연 | 04-analyze.py |
| [기준값 표](../results/tables/thresholds.csv) | Jev 기준값 곡선 | 04-analyze.py |
| [차트 입력](../results/chart.json) | 공개 집계에서 추출한 Jev 곡선·구간 | figures.py |

분석 재현에는 비공개 실험 저장소가 필요하다. 원천 대화의 사본은 만들지 않았고 가린 필요한 텍스트만 표본과 요청에 보존했다. JSON은 UTF-8이며 원응답은 수정하지 않는다. 정규화 관측은 JSON Lines이고 표는 머리 행을 포함한 CSV다. 결측값은 JSON null·CSV 빈 칸이다.

## 필드

### 정규화 관측

| 필드 | 타입 | 단위 | 제약 | 뜻 |
|---|---|---|---|---|
| run_id | string | 없음 | 필수 | 봉인 설계 커밋 |
| trial_id | string | 없음 | 고유 | 모델·표본·반복 식별자 |
| condition | string | 없음 | 필수 | 평가 모델 통로 |
| ts_utc | string | UTC | ISO 8601 | 호출 예약 시각 |
| sample_id | string | 없음 | 필수 | 가린 표본 ID |
| repeat | integer | 회차 | 양수 | 동일 요청 반복 |
| project_id | string | 없음 | 필수 | 군집 재표집 단위 |
| input_kind | string | 없음 | 두 추정 범주 | 입력 형태 |
| length_band | string | 턴 수 구간 | 네 구간 | 프로젝트 전체 사용자 입력 길이 층 |
| gold | string | 없음 | constraint·not_constraint·uncertain | 사후 정답 |
| valid | boolean | 없음 | 필수 | 형식·모델·도구 사용 검사 |
| probability | number 또는 null | 확률 | 유효 시 닫힌 단위 구간 | P(제약) |
| claimed_prediction | boolean 또는 null | 없음 | 필드 존재 시 보존 | 탐색 분석용 원응답 boolean |
| format_reason | string 또는 null | 없음 | 실패 시 코드 | JSON·출력 계약·모델 검증 실패 구분 |
| wrapped | boolean | 없음 | 필수 | 단일 코드 블록 추출 |
| multiline | boolean | 없음 | 필수 | 여러 줄 JSON 계약 이탈 |
| status | string | 없음 | 필수 | 호출 성공·HTTP·프로세스·시간 초과·미완료 |
| latency_s | number 또는 null | 초 | 음수 불가 | CLI·HTTP 실행 시간 |
| cost_usd | number 또는 null | USD | 음수 불가 | 공식 단가 기준 API 상당 비용 |
| cli_cost_usd | number 또는 null | USD | 응답 보고 값 | CLI가 자체 계산한 비용 |
| usage | object | 토큰 | 응답 보고 값 | 사용량 |
| actual_models | array | 모델 ID | 보고된 값만 | CLI 또는 HTTP 응답 모델 |
| tool_events | integer | 개 | 음수 불가 | Codex 응답의 도구 이벤트 수 |

샘플링·정답·원자료 봉인을 먼저 검사한 뒤 분석한다. 분석 코드는 봉인을 갱신하지 않으며 원응답을 다시 읽어 파생 관측과 집계를 재생성한다.
