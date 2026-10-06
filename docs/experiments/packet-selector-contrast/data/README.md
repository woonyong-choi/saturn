# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 실제 Saturn engine에서 Codex로 파일 세 개를 읽고 같은 기록을 복제해 Claude에서 RRF·Jev 전환을 각각 실행 |
| 수집 기간 | 2026-10-06~2026-10-06 |
| 개수 | 예정 12계열, 유효 source 7개, 실행된 target 14개, 로컬 원자료 JSON 26개 |
| 표본 여부 | seed 54026006으로 사전 생성한 합성 표본 전체 |
| 라벨 | 파일 첫 줄의 `RELEASE=current` 숫자로 결정. target 입력과 작업 폴더에는 숫자를 주지 않음 |
| 알려진 문제 | source 5개에서 파일 본문이 도구 결과에 남지 않아 target 미실행. 처음 봉인한 검증기는 JSON 목록과 Python 튜플의 직접 비교로 거짓 실패 |
| 개인정보 | 합성 로그에 개인정보 없음. 로컬 원자료의 실행 메타데이터는 공개 저장소에 넣지 않음 |
| 라이선스 | Saturn 저장소와 동일 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `.runtime/sc/manifest.json` | 수집 전 봉인한 시드·가격·모델·코드·바이너리 해시 | `scripts/01-collect.py` |
| `.runtime/sc/raw/s{번호}.json` | 출발 Codex 알림·기록 저장소 행과 재생 적합성 | `scripts/01-collect.py` |
| `.runtime/sc/raw/t{번호}{조건}.json` | 후속 Claude 알림·기록 저장소 행·전송 직전 패킷 캡처 | `scripts/01-collect.py` |
| `data/SHA256SUMS` | 봉인 파일과 수집 원자료 26개의 SHA-256 및 로컬 상대 경로 | 수집 뒤 해시 계산 |
| `results/summary.json` | 원자료에서 재생성한 대응 쌍·가설 집계 | `scripts/02-process.py` |

로컬 원자료는 `.runtime/sc`에 보관한다. 공개 `data/SHA256SUMS`는 무결성을 확인할 수 있지만 원자료를 대신하지 않는다. `.runtime`은 Git 제외 경로다. `./verify-posthoc.sh`는 봉인된 검사기의 표현 형식 결함 한 곳만 정규화해 같은 검사를 다시 실행한다.

## 필드

### `raw/s{번호}.json`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `chat` | integer | 없음 | 유효 source에서 필수 | 원본 채팅 ID | `1` |
| `seen` | object | 없음 | 파일 세 개 모두 | `ToolResult`에서 각 파일 표지를 찾았는지 | `{"build.log": true}` |
| `valid` | boolean | 없음 | 필수 | 세 파일의 도구 결과와 완료 상태가 모두 있는지 | `true` |
| `db` | object | 없음 | 필수 | engine 기록 저장소에서 읽은 이벤트·사용량·판단 행 | `{}` |

### `raw/t{번호}{조건}.json`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `answer` | string | 없음 | 빈 문자열 가능 | target이 실제로 낸 답 | `"35"` |
| `duration_s` | number | 초 | 0 이상 | 후속 실행 벽시계 시간 | `7.91` |
| `started_at_ms` | integer | Unix 밀리초 | 필수 | source 기록과 target 사용량을 나누는 시각 | `1791290000000` |
| `captures` | array | 없음 | 빈 목록 가능 | engine이 provider 호출 직전에 캡처한 패킷 본문 | `[]` |
| `db` | object | 없음 | 필수 | 패킷·항목·사용량·판단·이벤트 원행 | `{}` |
