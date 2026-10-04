# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | constraint-deep의 세 라벨이 합의한 짧은 규칙에 생성 입력을 붙이고 독립 재라벨 후 Jev에 질문 |
| 수집 기간 | env.json의 시작 날짜, 상세 시각은 비공개 run.json·calls.jsonl·jev.jsonl |
| 개수 | results/summary.json의 flow·sample·calls |
| 표본 여부 | 합성 유형별 고정 할당, 불일치 제외. 실제 문장은 고정 시드 순서에서 두 모델이 비해제로 합의한 앞 10개 |
| 라벨 | 생성 의도와 독립 라벨 일치. 실제 문장은 두 독립 라벨 일치. 사람 검증 아님 |
| 알려진 문제 | 합의한 쉬운 입력만 남는 선택 편향, 같은 규칙 반복, 실제 입력도 합성 활성 상태에 배치 |
| 개인정보 | 원규칙·실제 턴·상태·원응답은 비공개. 공개 생성문은 비밀값·이메일·절대 경로 가림 후 실제 원문 전체와 일치하는 부분을 추가 가림 |
| 라이선스 | 실제 기록은 비공개 개인 기록으로 재배포하지 않음. 공개 스크립트·집계는 저장소 라이선스 |

## 파일

비공개 경로는 실험 worktree의 `.local/experiments/constraint-cancel/`다. 앞 실험 원본은 메인 저장소에서 읽으며 사본을 만들지 않는다. 아래 비공개 파일 이름은 논리 위치다.

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| plan.json | 원본 해시, 규칙·문장 명세·실제 후보 목록 | 01-prepare.py |
| codex/*/prompt.txt·schema.json·receipt.json·answer.txt | 공식 CLI 요청·응답·종료 상태 | common.py가 재사용하는 앞 실험 labels.py |
| items.json·excluded.json·flow.json | 확정 표본·제외·합의 집계 | 01-prepare.py |
| calls.jsonl | 호출 전 잠금과 fsync로 예약한 장부 | common.py |
| request-hashes.json | 전송 전 고정한 요청별 SHA-256 | 02-collect.py |
| jev.jsonl | 요청 본문·응답·HTTP 상태·지연·반복 | 02-collect.py |
| processed.jsonl | 원문 없는 정규화 관측 행 | 03-process.py |
| [generated.json](generated.json) | 공개 가능한 합성 입력, 실제 규칙은 ID만 | 03-process.py |
| [SHA256SUMS](SHA256SUMS) | 비공개 원자료·응답의 해시와 논리 이름 | 03-process.py |
| [summary.json](../results/summary.json) | 보고서의 모든 측정 수치 | 04-analyze.py |
| [thresholds.csv](../results/tables/thresholds.csv)·[conditions.csv](../results/tables/conditions.csv) | 기준값 곡선·조건별 비율과 구간 | 04-analyze.py |

## 필드

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| run_id | string | UTC | 실행 동안 동일 | 수집 시작 시각 | ISO 8601 |
| trial_id | string | 없음 | 장부에서 고유 | 입력·형식·기록·반복 | s001-pair-1-r1 |
| condition | string | 없음 | pair·choice와 기록 0·1 | 비교 조건 | pair-1 |
| ts_utc | string 또는 null | UTC | 미완료는 null | 응답 시각 | ISO 8601 |
| item_id | string | 없음 | 표본에서 고유 | 입력 ID | s001 |
| cluster | string | 없음 | 가명 | 대상 규칙 또는 실제 대화 | 해시 기반 ID |
| intent | string | 없음 | release·partial·conditional·none | 정답 요청 범위 | partial |
| target | string | 없음 | none 또는 c1..cN | 정답 대상 | c1 |
| candidates | object | 확률 | 각 값 0~1 또는 null | 후보별 점수 | c1: 0.8 |
| request_p | number 또는 null | 확률 | 0~1 | 요청 식별 점수 | 0.8 |
| repeat | integer | 회 | 1·2·3 | 같은 요청 반복 | 1 |
| valid | boolean | 없음 | 유효 확률을 모두 확보 | 응답 형식 유효성 | true |
| request_bytes | integer | 바이트 | 100,000 이하 | UTF-8 요청 본문 크기 | 정수 |
| text_redacted | boolean | 없음 | 공개 생성문만 | 원문 복사·민감값 가림 여부 | false |
| rule_ids | array | 없음 | r- 접두사 가명 | 실제 규칙 ID의 SHA-256 앞 12자리 | r- 접두사 |

공개 생성문에서 가린 부분은 `[원문 가림]`이다. 이는 측정 입력을 바꾸지 않으며 공개 뷰에만 적용한다. 생성 요청의 의도와 독립 라벨은 Jev에 보내지 않는다. 실제 원문이 없는 공개 자료만으로 요청을 완전히 재구성할 수 없으며, 원자료 소유자가 비공개 파일로 재현할 수 있다.

## 재현

```sh
./run.sh process
./run.sh analyze
./run.sh verify
```

분석·검증은 모델을 호출하지 않는다. `analyze`는 비공개 원자료에서 정규화 행을 다시 만들고 집계를 계산한다. 완료 응답을 새로 보내지 않는다. 호출 예산은 실패·모델 검토·미완료 예약을 포함한다. API 키는 프로세스 환경에서만 받고 라벨 프로세스에는 전달하지 않는다.
