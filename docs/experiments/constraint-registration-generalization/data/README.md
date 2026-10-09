# 데이터

## 출처

사용자 대화에서 추출한 입력과 앞 맥락을 사용한다. 원문은 공개하지 않는다. 표본 규모와 출처별 분포는 API 수집 전 생성한 census.json에 기록한다. 기존 실험과 같은 세션, 중복 상태, 자동 메시지를 제외한다.

## 파일

| 파일 | 내용 | 생성 단계 |
|---|---|---|
| 비공개 census.json | 출처별 파일·입력 수와 제외 사유 | 표본 준비 |
| 비공개 samples.json | 가린 입력·앞 맥락·출처 식별자 | 표본 준비 |
| 비공개 design-seal.json | 설계·질문·코드·표본의 해시와 시각 | 호출 전 봉인 |
| 비공개 raw/*.json | 요청, 응답, 상태, 지연, 사용량; 인증 헤더 제외 | 호출 |
| 비공개 calls.jsonl | 호출 전 예약과 요청 식별자 | 호출 |
| results/summary.json | 공개 집계와 미평가 이유 | 분석 |
| results/retrospective-1000-summary.json | 사후 1,000건의 공개 집계·품질 한계·해시 | 사후 평가 |

사후 정답지 후보와 원응답은 버전 관리에서 제외된 `.local/experiments/constraint-registration-generalization/retrospective/`에 둔다. `quality1000/selection.json`은 점수층과 선택 순서를, `quality1000/cases.json`은 입력 ID에 연결된 원본 세션·메시지 위치와 가린 판정 입력을 보존한다. `quality1000/primary`, `secondary`, `third`의 원응답에는 실패와 재시도를 함께 보존한다. `quality1000/goldset.json`에는 입력 ID, Jev 확률, 최종 라벨, 모든 판정 이유와 원문을 복사하지 않은 근거 위치, 이후 대화 생략량을 기록한다. `quality1000/summary.json`은 비공개 집계이며 `quality1000/manifest.json`은 원자료 1,286개 파일의 SHA-256 목록이다. 원본 연결 복구 63건과 복구 방법은 `retrospective/recovered/`에 있다. 공개 집계의 `goldset_sha256`은 키 정렬 JSON 내용의 해시이며 `goldset_file_sha256`은 저장된 파일 바이트의 해시다.

## 필드

| 필드 | 타입 | 뜻 |
|---|---|---|
| sample_id | string | 가린 입력 상태의 SHA-256 |
| source_id | string | 출처 세션 식별자의 SHA-256 |
| project_id | string | 프로젝트 식별자의 SHA-256 |
| state | object | previous_context와 latest_user_input |
| gold | string 또는 null | 독립 정답; 없으면 null |
| status | string | ok, HTTP 실패, 형식 실패, 전송 여부 불명 등 |
| usage | object 또는 null | API가 반환한 토큰 사용량 |

원문과 응답의 재배포는 하지 않는다. user 역할과 사람의 직접 작성 여부는 동일하지 않다. 정답 없는 관측의 비율을 정확도로 표시하지 않는다.
