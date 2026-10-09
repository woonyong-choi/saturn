# 전송 원문 중복 데이터

실제 대화와 문서의 출처는 앞선 recall-selection의 고정 입력이며, 추가 두 문서는 수집 시점의 저장소 원문이다. 문서를 대화로 감싼 입력과 중복 여부를 조작한 대조 입력은 합성 데이터다. 원래 대화의 마스킹은 유지했다.

| 자료 | 내용 |
|---|---|
| `formal/cases.json`, `questions.json`, `queries.json` | 20조건의 원자료와 고정 질문·정답 |
| `formal/sources/`, `gold-provenance.json` | 추가 문서 원문, 해시·정답 근거 줄 |
| `formal/baseline/`, `candidate/` | 기준·후보 실행 파일이 만든 패킷·복원 입력 |
| `formal/payload-checks.json` | 실제 자료 80개 입력의 길이·동일성·원문 복원 대조 |
| `formal/calls-plan.json`, `seal.json` | 실행하지 않은 모델 호출 계획과 고정 파일 해시 |
| `controls/` | 중복을 조작한 합성 대조 입력과 두 실행 파일의 결과 |
| `cross-message-overlap.json` | 패킷과 복원 사이의 동일 구간 위치·길이 |
| `baseline/`, `candidate-source/` | 작업 시작 상태, 후보 코드와 변경분·실행 파일 |
| `rollback.json` | 이번 변경을 되돌린 파일의 시작 시점 해시 일치 |

실제 provider 호출은 없다. 계획한 실행 240개는 `missing`으로 남기며 실패 응답이나 정상 응답을 만들어 채우지 않는다. `summary.json`의 모델 토큰 값 `null`은 결측이며 0이 아니다. 문자 수 0% 변화만 직접 관측했다.

`run.sh analyze`는 원시 재생 파일과 대조 자료에서 집계를 다시 만든다. `run.sh verify`는 고정 파일, 원문 전개와 아카이브를 검사한다. `run.sh collect`는 보존된 계획을 실제로 실행하는 별도 명령이므로 재현 검증에 사용하지 않는다. 새 호출을 하려면 별도 실행 경로와 사전 설계를 사용한다. 기준·후보 실행 파일은 수집 환경의 macOS arm64용이며 다른 운영체제의 실행 파일이 아니다.

아카이브를 저장소 루트에 풀면 보존 당시 상대 경로가 복원된다. 개인 대화와 원시 입력은 비공개로 보관하고 공개 보고서에는 집계와 제한만 적었다. 저장 위치·전체 해시는 [보관 영수증](archive.json), 파일별 해시는 [SHA256SUMS](SHA256SUMS)에 있다.
