# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 기존 한국어 대화 608턴의 가린 입력과 직전 목표·진행을 Jev에 두 질문으로 한 번씩 보냄 |
| 수집 기간 | 2026-10-07 |
| 개수 | 608턴, 신규 요청 608회 |
| 표본 여부 | 기존 개발 실험의 전수 재사용. 독립 확인 표본이 아님 |
| 라벨 | 기존 Astra·Luna의 독립 판정과 불일치 재판정. 사람 확정 정답이 아님 |
| 알려진 문제 | Claude 한국어 대화에 한정, 새 질문 개발에 기존 결과를 열람, HTTP 520 한 건 |
| 개인정보 | 원문·요청·응답은 Git 제외 로컬 실험 폴더에만 보존 |
| 라이선스 | 개인 대화 원자료라 공개 재배포 대상 아님 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| 비공개 `manifest.json` | 실행 ID와 기존 표본 SHA-256 | `scripts/01-collect.py` |
| 비공개 `{id}.reservation.json` | Jev 호출 전 예약 | `scripts/01-collect.py` |
| 비공개 `{id}.json` | 가린 요청, 원응답, 상태, 사용량, 지연 | `scripts/01-collect.py` |
| 비공개 `processed.csv` | 라벨과 기존 B·새 조건의 대응 행동 | `scripts/02-process.py` |
| [SHA256SUMS](SHA256SUMS) | 원자료 해시 목록 | `scripts/02-process.py` |

## 필드

### `{id}.json`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `trial_id` | string | 없음 | 고유 | 가린 입력 ID | `해시 문자열` |
| `request` | object | 없음 | 필수 | 두 Jev 질문과 state | `{"model":"jev-1.13.0"}` |
| `response` | object 또는 null | 없음 | 실패 시 null | Jev 원응답 | `{"answers":{...}}` |
| `status` | string | 없음 | `ok`, `http_error`, `failed` | 호출 상태 | `ok` |
| `elapsed_ms` | number | 밀리초 | 0 이상 | 요청 벽시계 시간 | `300` |

로컬 실행 ID는 `20261006T211341Z-ac733b3`이다. `SATURN_CONTINUATION_SAMPLE`, `SATURN_CONTINUATION_LABELS`, `SATURN_CONTINUATION_OLD_JEV`에 원자료 경로를 지정한 뒤 `./run.sh verify 20261006T211341Z-ac733b3`로 재검증한다. 공개 저장소에는 개인 대화 원문을 넣지 않는다.
