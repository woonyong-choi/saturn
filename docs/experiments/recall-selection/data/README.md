# 복원 근거 선택 데이터

## 출처

기존 대화는 [질문별 원문 복원](../../context-recall/data/README.md)의 마스킹된 실제 기록을 재사용했다. 추가 자료는 저장소의 `permissions`, `router-key-security`, `settings`, `records` 설계 문서 원문이다. 원문을 대화처럼 감싼 문서 질의 입력은 합성 데이터다. 문서의 정책을 회수하는지 검사하며 정책 구현 여부를 검증하지 않는다.

| 항목 | 값 |
|---|---|
| 수집 방법 | 실제 provider CLI에 고정한 원문 전체·이전 복원·새 복원 입력 전송 |
| 표본 | 기존 대화의 겹치는 조건과 같은 프로젝트의 설계 문서 |
| 라벨 | 기존 정답표 유지. 추가 정답은 원문에서 정의하고 `gold-provenance.json`에 근거 줄 보존 |
| 개인정보 | 기존 대화의 마스킹 유지. 공개 결과와 비공개 원응답 분리 |
| 알려진 문제 | 개발에 사용한 질문, 겹치는 대화 구간, 단일 질문의 재등장. 초기 단계의 출력 키 계약이 불명확해 명시적 계약으로 재수집 |
| 라이선스 | 개인 대화는 비공개. 저장소 문서는 저장소 라이선스 적용 |

## 파일

비공개 원자료의 보관 위치와 해시는 [보관 영수증](archive.json)에 있다. 저장소 루트에 아카이브를 풀면 수집 때의 상대 경로가 복원된다. 파일 목록은 [SHA256SUMS](SHA256SUMS)이며 바이너리 기준선은 수집 환경의 macOS arm64용이다. 원자료를 새로 받지 않고 기존 분석과 해시 검증을 다시 실행할 수 있다.

| 파일 | 내용 | 만드는 경로 |
|---|---|---|
| `cases.json`, `sources.json`, `sources/` | 마스킹된 실제 기록, 문서 원문과 출처 해시 | `01-run.py prepare` |
| `questions.json`, `queries.json` | 정답표와 실제 입력. 검색 함수는 정답표를 받지 않는다. | `01-run.py prepare` |
| `baseline/`, `candidate/`, `packets/` | engine의 원문 복원·패킷 재생 결과 | Rust 비공개 재생 테스트 |
| `calls-plan.json`, `seal.json` | 호출 순서·입력과 수집 전 파일 해시 | 준비 단계 |
| `reuse.json`, `reuse-seal.json` | 바이트가 같은 입력의 원응답 참조와 해시 | 재사용 단계 |
| `raw/*/request.json` | provider 명령, 실행 식별자, UTC 시작 시각 | 수집기 |
| `raw/*/context.txt`, `question.txt` | 실제 provider로 보낸 입력 | 수집기 |
| `raw/*/*.jsonl`, `stderr.txt` | 완결·부분 원응답과 오류 출력 | 수집기 |
| `raw/*/result.json` | 원응답에서 추출한 본문·사용량·종료 상태 | 수집기 |
| `code/`, `baseline/engine-tests`, `final-source/` | 단계별 코드, 기준 바이너리, 최종 코드와 변경분 | 준비·보관 단계 |
| `results/summary.json` | 보고서 수치와 단계별 실제 호출·재사용 수 | `04-summarize.py` |

## 필드

분석 행은 `provider`, `case`, `trial`, `arm`으로 식별한다. `source_phase`는 실제 응답을 만든 단계를 가리킨다. 재사용된 행은 새 호출로 세지 않는다. `checks`는 사전 정의한 정규화를 적용한 문항별 판정이고 `answers`는 파싱한 실제 응답이다. 원응답과 사용량은 `02-verify.py`가 다시 대조한다. 성공하지 못한 실행은 정답 분모에 남는다.

새 수집은 새 단계 경로와 사전 설계가 필요하다. 기존 경로는 덮어쓰지 않는다. `prepare`는 현재 runtime을 사용하므로 옛 단계 재현에는 그 단계의 코드나 기준 바이너리를 사용해야 한다. `analyze`는 보존된 입력과 원응답을 사용하며 모델을 다시 부르지 않는다.

## 재현

```sh
bash docs/experiments/recall-selection/run.sh analyze
bash docs/experiments/recall-selection/run.sh verify
```

각 `request.json`에는 실행 명령과 시작 시각이 있다. `env.json`에는 도구 버전과 원본 단계의 환경이 있고 반복 단계는 별도 시드와 원본 환경 경로를 남긴다. 도구 실행을 막은 질의 실험이므로 실제 파일 수정이나 정상 router 종단 검증으로 해석하지 않는다.
