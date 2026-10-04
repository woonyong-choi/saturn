# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 로컬 대화 파일 전수 목록, 앞 실험과 같은 사용자 입력 추출·가림, 프로젝트 연결, 독립 라벨과 Jev 반복 호출 |
| 수집 기간 | 정확한 시작은 `results/summary.json`의 `run_started`, 종료는 비공개 호출 영수증의 `ts_utc` |
| 개수 | `results/summary.json`의 `census`, `selection`, `calls` |
| 표본 여부 | 길이·프로젝트 층을 만든 뒤 적격 프로젝트 전수. 질문 대안은 길이별 무작위 일부 |
| 라벨 | 공식 Codex exec의 독립 모델 두 개와 별도 지시의 재판정. 사람 확인 대기 |
| 알려진 문제 | 목록 이후 해시 변경 파일 제외, 같은 프로젝트의 병렬 세션을 첫 시각 순으로 연결, 라벨 미해소·끝 집합 오류·부분 응답과 수집 중 출력 스키마 변경. 영향받은 뒤 상태까지 확인 검정에서 제외 |
| 개인정보 | 원문·라벨·응답은 Git 제외 실행 디렉터리에만 보관. 앞 실험 가림 함수를 재사용하고 수집 중 발견한 누락은 정확한 환경 키 가림으로 보완. 공개는 집계와 가명 프로젝트 ID |
| 라이선스 | 원자료는 비공개 개인 기록이며 재배포하지 않는다. 공개 스크립트는 저장소 라이선스를 따른다. |

## 파일

비공개 파일의 기준 위치는 메인 저장소의 `.local/experiments/constraint-deep/`다. worktree에 사본을 만들지 않는다. 아래 파일명은 그 디렉터리에 대한 논리 이름이며 공개 링크가 아니다. `SHA256SUMS`에는 내용 해시와 논리 이름만 둔다.

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `census.json` | 파일별 사용자 턴 수, 프로젝트, 기간, 원본 SHA-256과 경로 | `01-census.py` |
| `conversations/*.json` | 가린 사용자 입력 전체와 세션 경계, 프로젝트별 순서 | `02-prepare.py` |
| `selection.json` | 표본과 제외·중복 집계 | `02-prepare.py` |
| `calls.jsonl` | 호출 전 fsync한 예산 예약. 실패·미완료도 포함 | `storage.py` |
| `codex/*/prompt.txt`, `answer.txt`, `receipt.json` | 라벨 요청·응답·공식 CLI 종료 영수증 | `labels.py` |
| `labels/*/*.jsonl` | 묶음별 라벨과 검증 결과. 사후 보존한 부분 응답·잘못된 끝 집합은 별도 표시 | `labels.py` |
| `jev.jsonl` | 요청과 응답, 확률, 첫 반복·반복 순서, HTTP 상태, 지연 | `judge.py` |
| `redaction-repair.json` | 가림 보완의 파일별 횟수, 실제 문자열 없음 | 수집 중 보완 |
| `human-review.jsonl` | 불일치·경계 사례와 최초 라벨·재판정, 사람 확인 대기 표시 | `05-analyze.py` |

## 필드

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `trial_id` | string | 없음 | 호출 장부에서 고유 | 조건·대화·턴·반복을 식별 | `base-example-u-0001-r1` |
| `conversation_id` | string | 없음 | 프로젝트 가명 | 연결 대화 ID | `example` |
| `turn_id` | string | 없음 | 대화 안 고유 | 사용자 입력 순서 | `u-0001` |
| `ts_utc` | string | UTC | ISO 8601 | 예약·응답·라벨 저장 시각 | `2026-10-04T00:00:00+00:00` |
| `kind` | string | 없음 | 지정 범주 | 장부는 codex·jev, 판단은 base·alternative·relation | `base` |
| `repeat` | integer | 회 | 1~3 | 같은 판단 요청의 반복 | `1` |
| `probabilities` | object | 확률 | 유효 값은 0~1, 실패는 null 또는 미제공 | 질문별 noul 값 | `{"is_constraint":0.8}` |
| `status` | string | 없음 | ok·invalid·http_error·failed·incomplete | 응답 상태 | `ok` |
| `is_constraint` | boolean | 없음 | 모든 유효 라벨 턴 필수 | 지속 제약 여부 | `false` |
| `operation` | string | 없음 | register·replace·partial·release·none | 상태 변화 | `release` |
| `targets` | array | 없음 | 앞서 유효한 턴 ID만 | 해제·대체·부분 변경 대상 | `["u-0001"]` |
| `ambiguous` | boolean | 없음 | 모든 유효 라벨 턴 필수 | 판정 미해소 여부 | `true` |
| `final_active_turn_ids` | array | 없음 | 원응답 그대로, 불일치는 별도 표시 | 묶음 끝 선언 집합 | `[]` |

`final_set_valid=false`는 선언 집합 검증 실패, `partial_output=true`는 검증한 접두부만 보존, `structured_output=true`는 출력 스키마 적용이다. `protocol_deviation=true`인 묶음과 그 뒤 의존 상태는 확인 검정에서 제외한다. 턴별 판정은 탐색 분석에 보존한다.

## 재현

분석·검증은 외부 모델을 호출하지 않는다. `run.sh analyze`는 고정 원자료에서 공개 집계를 다시 만든다. `run.sh collect`는 기존 표본과 완료 응답을 재사용한다. 장부에 시작만 남은 요청은 재전송하지 않는다. 모델 종료 실패는 영수증을 먼저 확인해야 하며 임의로 장부를 지우지 않는다.

새 수집에는 먼저 전수 목록을 만들고 설계를 봉인해야 한다. Jev 키는 호출 프로세스의 `SATURN_JUDGE_KEY` 환경으로만 넘긴다. 라벨 호출 환경에서는 이 변수를 제거한다. 공식 CLI의 임시 경로·캐시도 비공개 실행 디렉터리 아래로 지정한다.
