# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | Claude 프로젝트 JSONL과 Codex 세션 JSONL을 읽고 한국어 사용자 입력 쌍을 추출했다. 무작위 대조와 결정적 후보를 비복원 추출했다 |
| 수집 기간 | [env.json](../env.json)의 collection_started와 호출 장부의 ts_utc |
| 개수 | [summary.json](../results/summary.json)의 selection·labels·calls |
| 표본 여부 | 출처 비례 무작위 대조 최대 800턴, 미선택 후보 보강 최대 1,200턴 |
| 라벨 | 공식 codex exec의 독립 두 모델과 불일치 재판정. 모델·지침은 [설계](../design.md)에 고정했다 |
| 알려진 문제 | 후보 보강의 분포 변화, Codex만 남은 새 표본, 모델 합의와 사용자 의도 차이, 과거 B1·B2 비공개 응답 부재 |
| 개인정보 | 입력·직전 입력·진행 발췌·개별 라벨·응답·출처 경로는 지정 worktree의 Git 제외 비공개 경로에만 보관한다. 원기록은 읽기만 했다 |
| 라이선스 | 개인 기록은 재배포하지 않는다. 공개 집계와 스크립트는 저장소 라이선스를 따른다 |

## 파일

새 비공개 파일은 지정 worktree의 `.local/experiments/continuation-newtask/` 기준이다. 원기록의 사본은 만들지 않았다. `sample.json`은 선택한 가림 입력 쌍이며 원본 전체의 복사본이 아니다. 과거 `continuation-ko`는 main 저장소의 기존 비공개 파일을 읽기만 한다.

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `run.json` | 봉인 커밋·실행 id·기존 입력 및 과거 공개 집계 해시 | `01-collect.py` |
| `census.json` | 읽은 원본 경로·바이트 수·해시와 제외 흐름 | `01-collect.py` |
| `sample.json`, `selection.json` | 가림 표본과 출처/추출 집합/거르기 여부 집계 | `01-collect.py` |
| `calls.jsonl` | 호출 직전 잠금·fsync 예약. 실패·수리·미완료도 포함 | 재사용 예산 함수 |
| `codex/`, `labels/`, `labels-*.json` | 라벨 프롬프트·schema·공식 CLI 영수증·최초 답·재판정 | `02-run.py`, 앞 실험 라벨러 |
| `jev.jsonl` | B1·B2의 요청·가린 원형 응답·확률·상태·시각·바이트 수 | `02-run.py`, 앞 실험 수집기 |
| `extraction-audit.json` | 사후 내부 이벤트 제외 기준·사례 ID·집합별 제외 수. 원래 sample은 보존 | `audit.py` |
| `final-labels.json` | 최초 합의와 재판정을 적용한 가림 사례, 최초 라벨 누락 표시 | `03-analyze.py` |
| `runtime/` | 실행 로그·일시 파일·합성 검증 fixture, 비공개 | 실행 래퍼와 검증기 |
| `../results/summary.json` | 보고서 수치, 새 표본·합산·대조·출처·민감도 집계 | `03-analyze.py` |
| `../results/tables/thresholds.csv` | 동일 집계의 분자·분모·비율·Wilson 구간 | `03-analyze.py` |
| `SHA256SUMS` | 비공개 원자료·응답·라벨 산출물 해시. runtime과 잠금 파일 제외 | `03-analyze.py` |

## 필드

### `sample.json`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| id | string | 없음 | 고유 | 원본 경로·줄 번호의 해시 | 비공개 |
| source | string | 없음 | Claude, Codex | 원기록 출처 | Codex |
| arm | string | 없음 | random, enriched | 거르지 않은 대조 또는 후보 보강 | random |
| prefilter | boolean | 없음 | 필수 | 봉인한 1차 거르기 통과 | true |
| session, project | string | 없음 | 해시 | 군집 및 출처 정보 | 비공개 |
| input, previous_input | string | 문자 | 가림 | 현재와 직전 사용자 입력 | 비공개 |
| goal_excerpt, progress_excerpt | string | 문자 | 가림·길이 상한 | 작업 목표와 진행 text 발췌 | 비공개 |
| running | boolean | 없음 | 필수 | 완료 상태의 규칙 기반 복원 | false |

### `jev.jsonl`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| run_id | string | 없음 | 필수 | 봉인 커밋과 수집 시각 | 시각-커밋 |
| trial_id | string | 없음 | 고유 | 입력·조건·반복 | 해시-B1-r1 |
| id | string | 없음 | 표본에 존재 | 입력 식별자 | 비공개 |
| condition | string | 없음 | B1, B2 | 실험 조건 | B1 |
| repeat | integer | 회 | 1 | 첫 응답만 수집 | 1 |
| ts_utc | datetime | UTC | 필수 | 호출 준비 시각 | ISO 8601 |
| request | object | 없음 | incomplete 제외 필수 | 전송 본문, 비공개 | 비공개 |
| request_bytes | integer 또는 null | 바이트 | 전송분 100000 이하 | 직렬화 크기 | null |
| raw_response | string | 없음 | 수신 시 | 가린 원형 응답, 비공개 | 비공개 |
| answers | object | 확률 | 유효분 유한한 0~1 | 질문별 답 | keep_current |
| status | string | 없음 | 필수 | ok·failed·http_error·oversize·incomplete·model_mismatch | ok |
| model | string 또는 null | 없음 | 응답값 | 실제 모델명 | jev-1.13.0 |
| http_status | integer 또는 null | 없음 | 응답 시 | HTTP 상태 | 200 |
| latency_ms | number | ms | 전송 시 | 요청 시간 | 250.0 |

정규화 기록은 UTF-8 JSONL 한 줄 한 객체이고, CSV는 머리 행을 가진 RFC 4180 형식이다. 결측은 JSON null, CSV 빈 칸으로 둔다. 라벨 영수증의 trial_id·시각과 상위 run.json으로 라벨 실행을 추적한다. 과거 합산은 커밋된 summary의 분자·분모를 사용한다.

## 재현

```sh
./docs/experiments/continuation-newtask/run.sh process
./docs/experiments/continuation-newtask/run.sh analyze
./docs/experiments/continuation-newtask/run.sh verify
```

분석은 저장한 표본·라벨·응답을 다시 읽으며 외부 호출을 하지 않는다. 수집 재개 시 예약된 trial을 재호출하지 않는다. 비공개 자료가 없는 환경에서는 공개 집계만으로 원문 의미나 합산 군집 구간을 복원할 수 없다.
