# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 실제 Saturn engine에서 Codex 출발 로그 전문을 세 번 읽고, 같은 저장소를 복제해 Claude 전환을 RRF·Jev로 각각 실행 |
| 수집 기간 | 2026-10-06~2026-10-06 |
| 개수 | source 12개, target 24개, 원자료 JSON 36개 |
| 표본 여부 | seed 54026008의 독립 합성 숫자 12계열 전수 |
| 라벨 | 첫 파일의 `RELEASE=current` 숫자로 결정. target에는 정답을 주지 않음 |
| 알려진 문제 | 숫자 찾기와 인위적 예산 압박만 측정했다. 같은 작업의 모델 출력을 반복하지 않았다 |
| 개인정보 | 합성 로그에 개인정보 없음. 실행 원자료는 로컬에서만 보관 |
| 라이선스 | Saturn 저장소와 동일 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `.runtime/so/manifest.json` | 실행 전 시드·가격·모델·코드·바이너리 봉인 | `scripts/run.py seal` |
| `.runtime/so/raw/s{번호}.json` | 출발 도구 결과와 기록 행 | `scripts/run.py collect` |
| `.runtime/so/raw/t{번호}{조건}.json` | target 알림, 기록 행, 패킷 전송 직전 캡처 | `scripts/run.py collect` |
| `data/SHA256SUMS` | 봉인 파일과 원자료의 SHA-256 | 수집 뒤 해시 계산 |
| `results/summary.json` | 예정 12쌍의 집계와 대응 원행 | `scripts/run.py analyze` |

로컬 `.runtime/so`는 Git 제외 경로다. 공개 해시만으로는 원자료를 재생할 수 없다. [사후 독립 검증기](../verify-posthoc.py)는 봉인된 수집·분석 코드를 변경하지 않고 원문 도구 결과, 복제 쌍, 보호 항목, 패킷 캡처, target 도구 미사용과 답을 확인한다.

## 필드

### `raw/s{번호}.json`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `chat` | integer | 없음 | 필수 | 출발 채팅 ID | `1` |
| `seen` | object | 없음 | 파일 세 개 모두 | 각 파일 표지가 실제 도구 결과에 있는지 | `{"build.log": true}` |
| `valid` | boolean | 없음 | 필수 | 세 도구 결과와 턴 완료 상태가 유효한지 | `true` |
| `db` | object | 없음 | 필수 | 기록 저장소에서 읽은 사건·사용량·판단 행 | `{}` |

### `raw/t{번호}{조건}.json`

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| `answer` | string | 없음 | 필수 | 후속 Claude의 실제 답 | `"33"` |
| `duration_s` | number | 초 | 0 이상 | 후속 입력 벽시계 시간 | `8.5` |
| `started_at_ms` | integer | Unix 밀리초 | 필수 | 후속 호출 사용량 분리 기준 | `1791290000000` |
| `captures` | array | 없음 | 최소 1개 | engine 전송 직전 패킷 본문 | `[]` |
| `db` | object | 없음 | 필수 | 패킷·항목·사용량·판단·이벤트 원행 | `{}` |
