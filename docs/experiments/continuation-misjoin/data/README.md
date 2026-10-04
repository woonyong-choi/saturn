# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 앞 실험 B0와 라벨 재사용, 봉인 질문 B1~B3 HTTPS 수집 |
| 수집 기간 | 2026-10-04~2026-10-04 |
| 개수 | 기존 608턴·추가 1턴, 새 Jev 응답 3,654건·추가 라벨 호출 2회 |
| 표본 여부 | 기존 적격 전수와 같은 규칙의 추가 적격 전수 |
| 라벨 | 기존 두 Codex 독립 라벨과 재판정 고정, 추가 1턴은 같은 두 모델 최초 일치 |
| 알려진 문제 | 새 작업 72건으로 검정력 부족, 같은 표본을 보고 질문 개발, 현재 state 복원 오차 |
| 개인정보 | 원문·모델 응답·출처 경로는 Git 제외 비공개 경로에만 보관 |
| 라이선스 | 개인 기록은 재배포하지 않음, 공개 자료는 저장소 라이선스 적용 |

## 파일

기존 원본은 main 저장소의 `.local/experiments/continuation-ko/`에서 읽기만 한다. 새 자료는 지정 worktree의 `.local/experiments/continuation-misjoin/`에 있다. 아래 경로는 각 실험의 비공개 루트 기준이다. 기존 파일의 사본을 만들지 않았다.

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `continuation-ko/sample.json`, `final-labels.json`, `jev.jsonl` | 기존 표본·정답·B0의 첫 두 응답 | 앞 실험의 원본 재사용 |
| `continuation-misjoin/run.json` | 봉인 커밋·실행 id·기존 파일 해시 | `scripts/01-collect.py prepare` |
| `continuation-misjoin/sample.json`, `extension-census.json` | 추가 표본·원본 파일 해시·추출 흐름 | `scripts/01-collect.py prepare` |
| `continuation-misjoin/extraction-audit.json` | 당시 파일 해시와 바이트 접두를 확인한 중복 제외 수 복원 | `scripts/audit-extraction.py` |
| `continuation-misjoin/calls.jsonl` | 호출 전 예약 장부 | `scripts/support.py` |
| `continuation-misjoin/jev.jsonl` | 새 요청·원형 응답·확률·실패·바이트·시각 | `scripts/01-collect.py judge` |
| `continuation-misjoin/labels-*.json`, `labels/`, `codex/` | 추가 독립 라벨과 공식 CLI 호출 영수증 | `scripts/01-collect.py label` |
| `../results/summary.json`, `../results/tables/thresholds.csv` | 공개 집계, 원문·개별 세션 식별자 없음 | `scripts/02-analyze.py` |
| `SHA256SUMS` | 기존 입력과 새 비공개 산출물 해시 | `scripts/02-analyze.py` |

## 필드

### `jev.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| run_id | string | 없음 | 필수 | 봉인 커밋과 수집 시각의 식별자 | `시각-커밋` |
| trial_id | string | 없음 | 고유 | 표본·조건·반복의 결합 | `해시-B1-r1` |
| id | string | 없음 | 표본에 존재 | 입력 식별자 | `가림 해시` |
| condition | string | 없음 | B1, B2, B3 | 실험 조건 | `B1` |
| repeat | integer | 회 | 1, 2 | 주 분석과 안정성 반복 | `1` |
| ts_utc | datetime | UTC | 필수 | 요청 준비 시각 | ISO 8601 |
| request | object | 없음 | incomplete 제외 필수 | 전송 본문, 비공개 | 공개하지 않음 |
| request_bytes | integer | 바이트 | 전송분 <=100000 | 직렬화 본문 크기 | `53256` |
| raw_response | string | 없음 | 응답 수신 때 | 가린 원형 응답, 비공개 | 공개하지 않음 |
| answers | object | 확률 | 유효분 0~1 | 질문별 확률 | `keep_current` |
| model | string 또는 null | 없음 | 응답값 보존 | 실제 모델명 | `jev-1.13.0` |
| status | string | 없음 | 필수 | ok, oversize, incomplete, failed, http_error, model_mismatch | `ok` |
| http_status | integer 또는 null | 없음 | 상태가 없으면 null | HTTP 결과 | `200` |
| latency_ms | number | ms | 전송분 | 요청 소요 시간 | `250.0` |

## 재현

`run.sh analyze`는 원본 라벨과 응답에서 조건별 판정·분모를 다시 계산해 표와 JSON을 생성한다. `run.sh process`도 같은 과정을 실행한다. 원문을 정규화 사본으로 저장하지 않는다. 추가 추출 감사는 수집 당시 파일의 해시를 확인한 바이트 범위만 읽는다. 파일을 복원할 수 없으면 `unreconstructable_files`를 기록한다.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 docs/experiments/continuation-misjoin/scripts/audit-extraction.py
./docs/experiments/continuation-misjoin/run.sh analyze
PYTHONDONTWRITEBYTECODE=1 python3 docs/experiments/continuation-misjoin/scripts/report.py
./docs/experiments/continuation-misjoin/run.sh verify
```
