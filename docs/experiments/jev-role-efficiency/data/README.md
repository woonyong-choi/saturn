# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 공식 Claude·Codex CLI와 TypeSafe HTTPS 호출의 원응답을 비공개 원자료로 보존 |
| 수집 기간 | 2026-10-06 |
| 개수 | 실제 수집 개수는 집계 JSON의 collection |
| 표본 여부 | 새 합성 과제 22개를 조건별로 반복 |
| 라벨 | fixture의 숨긴 기대 설정·정확 답·표현식 검사 계약 |
| 알려진 문제 | 실제 저장소 구현과 사용자 의도를 대표하지 않는 합성 초소형 과제 |
| 개인정보 | 사용자 대화 원문과 API 키 제외 |
| 라이선스 | 저장소 라이선스 적용 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| 비공개 원자료 | 요청·응답·실패·usage·시각, 원문 fixture, 선택 ID, 호출 예약, 수집 전 해시 | scripts/01-collect.py |
| [SHA256SUMS](SHA256SUMS) | 원응답 파일의 SHA-256과 비공개 데이터 루트 기준 이름 | scripts/02-analyze.py |
| [집계](../results/summary.json) | 역할별 성과와 수집 개수 | scripts/02-analyze.py |

원자료는 저장소의 비공개 실험 공간에서 실험 폴더 이름으로 보관한다. 원자료 접근 권한이 있는 환경에서 run.sh analyze와 run.sh verify로 재현한다. 공개 fixture 생성기는 같지만 모델 재호출의 응답·시간까지 재현되지는 않는다.

## 필드

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| trial_id | string | 없음 | 고유 | 요청과 후속 성과 연결 | route-run-r0-sonnet |
| ts_utc | string | UTC | 원시 요청 필수 | 호출 시각 | ISO 8601 |
| kind | string | 없음 | claude, codex, jev | 호출 경계 | jev |
| status | string | 없음 | 필수 | 전송·프로세스 결과 | ok |
| latency_s | number | 초 | 음수 불가 | 외부 호출 벽시계 시간 | 0.3 |
| request | object | 없음 | Jev 요청 | 정답을 제외한 state와 질문 | 해당 없음 |
| prompt | string | 없음 | CLI 요청 | 실행 과제 | 해당 없음 |
| success | boolean | 없음 | 후처리 | 프로그램 채점 결과 | true |
| selected_ids | array | 없음 | 조건별 | 원문 또는 후보 식별자 | b0 |
