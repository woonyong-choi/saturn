# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 실제 Saturn plain 입력과 engine 저장소의 읽기 전용 추출이다. |
| 수집 기간 | env.json과 각 trial 시각에 기록한다. |
| 개수 | results/summary.json의 collected와 planned를 따른다. |
| 표본 여부 | 사전 등록한 신규 시드와 실행 순서의 표본이다. |
| 라벨 | 코드 상수의 AST 값과 수집기 메모리의 기대값, 원래 공개 테스트, 티켓 응답을 대조한다. |
| 알려진 문제 | 합성 저장소 한 종류다. 진단용 rescue는 여러 입력으로 나뉘어 사전 설계한 단일 입력 비교에 사용할 수 없다. 기본 압축 비용은 따로 계측되지 않아 결측으로 표시한다. |
| 개인정보 | 합성 입력만 사용하지만 실행 경로와 provider 기록 식별자가 원자료에 들어간다. 원자료는 로컬에 보존하고 공개 Git에는 넣지 않는다. 키 문자열이 나오면 수집을 중단한다. |
| 라이선스 | 생성기와 집계는 저장소 라이선스를 따른다. |

## 파일

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| raw/*.json.gz | 수정하지 않는 실행 원문과 시험 종료 뒤 기록 저장소 스냅샷이다. | collect.py |
| SHA256SUMS | 원문 압축 파일별 해시다. | analyze.py |
| ../results/trials.json | 실행별 채점·토큰·조회·개입·모델 식별자다. | analyze.py |
| ../results/summary.json | 보고서 수치와 대응 비교다. | analyze.py |

## 필드

### 원자료

| 필드 | 타입 | 단위 | 제약 | 뜻 | 예 |
|---|---|---|---|---|---|
| trial | string | 없음 | 고유 | provider·seed·조건 식별자다. | claude-101-provider |
| source_commit | string | 없음 | 필수 | 실행 시작 때 수집기 커밋이다. | Git SHA |
| started_unix | number | 초 | 필수 | 실행 시작 시각이다. | Unix timestamp |
| turns | array | 없음 | 순서 보존 | 입력 식별자·설정·원응답·상태·시각이다. | label=s1 |
| store | object | 없음 | 종료 뒤 추출 | engine의 inputs·runs·usage·judgments·packets·events다. | input_id |
| grades | object | 없음 | 실제 실행 뒤 | F1·F3 코드와 공개 테스트의 결과다. | header_ok |

로컬 원자료가 있을 때 run.sh verify로 해시와 재집계 바이트를 검사한다. 없으면 검증은 실패하며 새 수집을 기존 실행의 재현 성공으로 표시하지 않는다.
