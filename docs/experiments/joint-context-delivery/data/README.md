# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 고정 실제 기록의 패킷·질문별 복원을 같은 provider 세션의 두 턴으로 전송 |
| 수집 기간 | 원시 실행 메타데이터의 UTC 시작 시각에 기록 |
| 개수 | `results/summary.json`의 계획·실제 실행 수. 합성 프로토콜 검사는 별도 |
| 표본 여부 | 이전 비교 자료와 실제 저장소 문서. 무작위 모집단 표본이 아님 |
| 라벨 | 기존 정답표를 그대로 복사. 추가 문서는 원문 구절·줄·해시로 고정 |
| 알려진 문제 | 출처와 질문 반복이 겹침. 초기 프로토콜 거부와 재개 사용량 누적 문제는 별도 기록 |
| 개인정보 | 비공개 대화·경로가 포함되어 원문과 개별 응답은 비공개 로컬 보관소에 저장. 공개 집계는 원응답 값 제외 |
| 라이선스 | 저장소 문서는 해당 저장소 라이선스. 비공개 대화 재배포하지 않음 |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| 비공개 고정 입력 | 조건·질문·정답·근거·실행 계획·봉인 해시 | `01-run.py prepare` |
| 비공개 실행 폴더 | 실제 두 입력·시각·명령·원시 JSONL·표준 오류·응답·사용량 | `01-run.py collect` |
| 비공개 세션 사용량 | 해당 실험 세션의 token_count만 추출. 전체 개인 세션은 복사하지 않음 | `03-verify-usage.py` |
| 비공개 `processed/records.jsonl` | 문항별 판정과 응답값을 가진 정규화 관측 | `02-analyze.py` |
| `results/summary.json` | 원응답 값을 뺀 공개 집계와 판정 | `02-analyze.py` |
| `results/verification.json` | 고정 입력·전송 입력·응답 대조 | `01-run.py verify` |
| `results/usage-verification.json` | 원시 턴 사용량과 모델별 누적 사용량 대조 | `03-verify-usage.py` |
| `SHA256SUMS`, `archive.json` | 압축 안 파일별·압축 전체 해시와 보관 경로 | `04-archive.py` |

## 필드

### 정규화 관측

| 필드 | 타입 | 단위 | 제약 | 뜻 |
|---|---|---|---|---|
| `run_id` | string | 없음 | 고유 | provider·조건·반복·방식 식별자 |
| `trial_id` | string | 없음 | 필수 | 대응하는 조건·반복 |
| `condition` | string | 없음 | baseline/candidate | 비교 방식 |
| `ts_utc` | datetime/null | UTC | 미실행이면 null | 원시 요청의 시작 시각 |
| `case` | string | 없음 | 필수 | 고정 입력 식별자 |
| `checks` | object | 정오 | 문항 ID별 boolean | 고정 허용 답과 비교한 결과 |
| `input_tokens`, `output_tokens` | integer/null | 토큰 | 완결 사용량이 없으면 null | 두 턴 합계 |
| `status` | string | 없음 | ok/error/missing | 실행 정상·오류·미실행 |

## 보존과 재현

초기 준비본과 프로토콜 실패를 덮어쓰지 않는다. Codex는 정확한 thread ID로 재개하며 첫·둘째 누적 사용량과 차분을 함께 남긴다. Claude는 마지막 응답의 모델별 누적 사용량과 대조한다. 인증 파일·키·사용자 설정은 보관하지 않는다.

`./run.sh verify`는 봉인 입력, 실제 전송 입력, 원응답, 세션 일치, 원시 사용량의 대응을 확인한다. `./run.sh analyze`는 원응답에서 집계를 다시 생성하며 모델을 호출하지 않는다. 실행 오류와 오답을 계획 분모에 유지하고 비용 결측을 0으로 취급하지 않는다.

`python3 scripts/04-archive.py archive`가 보관본과 파일별 SHA-256을 생성한다. `python3 scripts/04-archive.py verify`는 압축 전체와 내부 파일 해시를 작업 파일과 대조한다. 시제품 성공을 engine 실패 복구·정상 router·TUI 종단의 성공으로 취급하지 않는다.
