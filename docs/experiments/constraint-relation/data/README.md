# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 앞 실험 고정 대화를 읽고 봉인 후보 규칙별 Jev HTTPS 호출. 추가 짧은 대화는 공식 Codex exec의 독립 라벨 두 통로와 별도 판정 |
| 수집 기간 | 2026-10-04 |
| 개수 | [집계 JSON](../results/summary.json)의 calls·cohorts·extension |
| 표본 여부 | 원래 대화 무작위 400턴, 추가 짧은 대화 무작위 44턴. 시드와 모집단은 설계 참조 |
| 라벨 | 원래 재판정 라벨 불변. 추가 라벨은 gpt-6-astra·gpt-5.6-luna 독립 판정 뒤 별도 astra 재판정 |
| 알려진 문제 | 단일 사용자, 모델 정답, 매우 적은 전이 양성, 원래 실험의 사후 변경 상속 |
| 개인정보 | 원문·응답·프롬프트는 비공개. 공개 파일은 설계·스크립트·집계·체크섬 |
| 라이선스 | 사용자 개인 기록, 원자료 재배포 없음 |

원자료는 원래 저장소의 `.local/experiments/constraint-deep/`를 읽기만 한다. 새 기록은 실험 worktree의 `.local/experiments/constraint-relation/`에 둔다. 원본을 복사하지 않는다. 실행 위치가 다른 경우 `scripts/support.py`의 SOURCE가 가리키는 원래 기록이 필요하다. 아래 파일 경로는 새 비공개 디렉터리 기준이다. 키는 프로세스 환경으로만 전달하며 파일에 저장하지 않는다.

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `plan.json` | 모집단 표본 ID·라벨 묶음·원본 파일 SHA-256 목록 | `01-prepare.py` |
| `run.json` | 실행 id와 수집 전 설계 커밋 | `03-collect.py` |
| `calls.jsonl` | 호출 전 예약 장부, 실패·미완료 포함 | `support.py` |
| `jev.jsonl` | 전송 본문·원응답·후보·바이트·확률·실패 | `03-collect.py` |
| `codex/*/receipt.json` | 공식 CLI 응답, 실제 요청 모델과 종료 코드 | 앞 실험 `labels.py` 재사용 |
| `extension/*.json` | 검증된 추가 라벨, 최초 두 통로와 재판정 구분 | `02-label.py` |
| `processed.jsonl` | 정규화 후보쌍·후보 밖 정답 | `04-process.py` |
| `trial-info.json` | 후보 수·잘림·표본 포함률 | `04-process.py` |
| `coverage.json` | 원래 전수의 제한 전후 후보 포함률 | `04-process.py` |

공개 `SHA256SUMS`는 위 비공개 원자료의 상대 경로와 해시다. 원응답은 수집 뒤 수정하지 않는다. 분석 파생 파일은 `./run.sh analyze`로 재생성한다. C0 원응답은 복사하지 않고 원래 파일 해시를 plan.json에 고정했다. 공개 `results/summary.json`의 source_manifest_sha256은 이 사전 목록의 정렬 JSON 해시다.

## 필드

### processed.jsonl

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| run_id | string | 없음 | 필수 | 실행 식별자 | ISO 8601 시각 |
| trial_id | string | 없음 | 필수 | 표본·조건·프로젝트·턴 식별자 | 조건 포함 문자열 |
| cohort | string | 없음 | original, extension | 원래·추가 표본 | original |
| condition | string | 없음 | C0~C3 | 조건 | C1 |
| conversation_id | string | 없음 | 가명 | 프로젝트 식별자 | 해시 문자열 |
| turn_id | string | 없음 | 필수 | 현재 입력 | u-0002 |
| target | string | 없음 | 필수 | 앞 제약 입력 | u-0001 |
| ts_utc | string 또는 null | UTC | 원응답 시각 또는 결측 | 첫 반복 시각 | ISO 8601 시각 |
| distance | integer | 턴 | 양수 | 입력 사이 거리 | 1 |
| gold | string | 없음 | compatible, partial, replace, release | 정답 관계 | partial |
| probabilities | array | 확률 | 세 반복, 각각 대체·해제 또는 null | 원응답 확률 | 0~1 |
| candidate_included | boolean | 없음 | 필수 | 크기 제한 후 후보에 포함된 대상 | true |
| measured | boolean | 없음 | 필수 | 후보쌍 여부. false는 후보 밖·앞 라벨 제외 정답 | false |

### jev.jsonl

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| run_id, trial_id, condition, ts_utc | string | 없음 | 필수 | 실행·시도·조건·시각 | 위 표 참조 |
| repeat | integer | 회 | 1~3 | 같은 요청 반복 | 1 |
| request | object | 없음 | 키 제외 | model/state/questions | JSON 객체 |
| raw_response | object | 없음 | 성공 시 | 가린 원응답 | JSON 객체 |
| probabilities | object | 확률 | 0~1 또는 null | 질문별 유효 확률 | replaces_0 |
| meta | object | 없음 | 필수 | 후보 ID, 제한 전후 바이트, 제거 수 | JSON 객체 |
| status | string | 없음 | ok, invalid, http_error, failed | 요청 결과 | ok |
| http_status | integer 또는 null | 없음 | 결측 허용 | HTTP 응답 상태 | 200 |

예약은 있지만 원응답이 없는 시도는 미완료다. 재개 시 다시 보내지 않는다. 분석에서는 null이며 행동 없음이다. raw_response가 없는 실패는 error_response 또는 오류 종류만 남긴다. 비율 분모가 없으면 JSON은 null, CSV는 빈 칸이다.
