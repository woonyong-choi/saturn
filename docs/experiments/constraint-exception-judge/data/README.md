# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 공개된 앞 실험 생성문을 원제약에 다시 연결하고 추가 예외 문장과 실제 사용자 턴에 독립 라벨 부여 |
| 수집 기간 | 2026-10-04, 실행 시각은 results/summary.json의 run |
| 개수 | [summary.json](../results/summary.json)의 flow·calls·conditions |
| 표본 여부 | 사전 종류별 생성 할당, 모호함·라벨 불일치 제외. 이어 가기는 같은 원천 재추출·합의 표본 |
| 라벨 | gpt-6-astra·gpt-5.6-luna 독립 판정, 범위 의미는 고정 지침의 gpt-6-astra 판정. 사람 정답 검증 아님 |
| 알려진 문제 | 앞 실험 비공개 worktree 원자료 부재, 쉬운 합의 문장 편향, 실제 제약 요청 양성 부재 |
| 개인정보 | 실제 제약·사용자 턴·응답은 비공개. 생성문 공개 뷰에서 원천 기록과 연속 8자 이상 일치하는 조각 가림 |
| 라이선스 | 실제 기록 재배포 금지. 공개 스크립트·집계는 저장소 라이선스 |

## 파일

원문·응답과 파생 관측은 worktree의 `.local/experiments/constraint-exception/`에만 저장한다. 앞 기록은 읽기 전용 참조이며 원본 파일 전체의 사본은 만들지 않는다. Claude가 자체 저장한 project session 폴더 이름은 보고서에 따로 남긴다.

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| run.json, calls.jsonl | 봉인 커밋·실행 id, 호출 전 잠금·fsync 예약 장부 | runtime.py, 01-prepare.py |
| restored.json, generated-private.json | 원제약에 연결한 기존 입력과 새 생성 입력 | 01-prepare.py |
| constraint-preliminary.json, excluded.json | 독립 라벨 합의와 제외 사유 | 01-prepare.py |
| continuation-candidates.json, continuation-census.json | 같은 추출기로 재수집한 가림 후보와 원파일 메타데이터 | source-continuation.py |
| continuation-labels.json, continuation.json | 독립 합의 라벨과 평가 표본 | 01-prepare.py |
| scope-label-pairs.json, scope-label-grades.json | 정답 범위의 독립 라벨 간 의미 일치 | 03-grade.py |
| items.json, j2-ids.json, input-hashes.json | 고정 평가 표본·탐색 표본 ID·입력 해시 | 01-prepare.py, 02-collect.py |
| codex/, claude/, jev/ | 원요청·원응답·모델·usage·종료 상태·시각·지연 | runtime.py |
| workflows/ | 단계별 호출 연결과 최종 JSON 계약 검증 | 02-collect.py |
| scope-prediction-pairs.json, scope-prediction-grades.json | 조건을 가린 예측 범위 의미 채점 | 03-grade.py |
| processed.jsonl | 원문 없는 정규화 관측, 비공개 | 04-analyze.py |
| exploratory-processed.jsonl | 단일 JSON 코드 블록 추출 후 참고 관측, 비공개 | 04-analyze.py |
| exploratory-rounding.jsonl | Jev 확률 합의 십진수 경계 보정 탐색 관측, 비공개 | 04-analyze.py |
| scope-rounding-pairs.json, scope-rounding-grades.json | 경계 보정으로 추가된 범위 쌍의 의미 채점, 기존 쌍 재사용 | 03-grade.py |
| [generated.json](generated.json) | 공개 가능한 생성문·가명 규칙 ID·종류·대상 | 04-analyze.py |
| [RAW_SHA256SUMS](RAW_SHA256SUMS) | 수집·채점·검토 종료 후 원자료 불변 봉인 | publication.py |
| [SHA256SUMS](SHA256SUMS) | 원자료·재생성 파생 관측의 현재 해시 | 04-analyze.py |
| privacy-sources.json | 공개 가림 원천의 파일 목록·해시, 비공개 | publication.py |
| [summary.json](../results/summary.json), [conditions.csv](../results/conditions.csv) | 공개 집계와 보고서 수치 | 04-analyze.py |

## 필드

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| run_id | string | UTC | 실행 동안 동일 | 수집 시작 시각 | ISO 8601 |
| trial_id | string | 없음 | 예약 장부에서 고유 | 입력·조건·반복·단계 | s001-J2-r1-A |
| item_id | string | 없음 | 표본에 존재 | 입력 식별자 | s001 |
| condition | string | 없음 | J1, J2, L1, B1 | 워크플로우 | J1 |
| repeat | integer | 회 | 1, 2, 3 | 같은 입력 반복 | 1 |
| ts_utc | string 또는 null | UTC | 완료 때 존재 | 관측 시각 | ISO 8601 |
| status | string | 없음 | ok, invalid, failed, incomplete, http_error | 관측 상태 | ok |
| valid | boolean | 없음 | 필수 | 전체 계약 유효성 | true |
| correct | boolean | 없음 | 필수 | 모든 단계와 범위 의미 일치 | false |
| false_permanent | boolean | 없음 | 제약 과제 | 잘못된 종류·대상의 전체 영구 해제 | false |
| latency_ms | number 또는 null | ms | 실제 단계 지연 합 | 큐 대기 제외·CLI 시작 포함 | 1000 |
| cost_usd | number 또는 null | USD | 모든 단계 비용을 알 때만 값 | API 상당액 | 0.001 |
| calls | array | 없음 | 실제 예약된 호출 | 단계별 usage·비용·지연 | 비공개 |

## 재현

```sh
./docs/experiments/constraint-exception-judge/run.sh verify
./docs/experiments/constraint-exception-judge/run.sh analyze
```

`analyze`는 비공개 원자료에서 관측을 다시 처리하고 집계를 생성한다. 외부 모델을 호출하지 않는다. 공개 집계만으로 비공개 원문의 의미 판정을 재현할 수는 없다. 수집 재개는 저장된 영수증을 사용하며 예약된 요청을 다시 보내지 않는다.

단일 JSON 코드 블록 추출은 초기 응답을 확인한 뒤 추가한 탐색 분석이다. 엄격한 JSON 형식의 원래 평가와 별도로 저장하며 확인 가설·권장 조건 선정에는 사용하지 않는다.

중단 시 예약됐지만 응답이 없거나 JSON이 불완전한 회차는 `incomplete`로 보존하고 재전송하지 않는다. 확률 합 경계의 십진수 재검증은 저장된 J1 응답만 사용한 사후 탐색이며 확인 판정은 수집 당시 상태를 유지한다. Jev usage는 검증 실패 응답에도 남아 있어 비용 집계에서 원응답을 읽는다.

공개 내보내기는 가림 원천 폴더가 없거나 [앞 실험의 봉인 목록](../../constraint-deep/data/SHA256SUMS)의 파일 목록·해시와 다르면 중단한다. 전체 턴뿐 아니라 연속 8자 이상의 원문 조각도 가린다. 일반적인 표현까지 가려질 수 있어 공개 생성문만으로 입력 전체를 복원할 수 없다. 원자료 불변 봉인은 분석 전에 대조하며 불일치하면 새 기준으로 덮어쓰지 않고 중단한다.
