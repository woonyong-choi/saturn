# 데이터

## 출처

| 항목 | 값 |
|---|---|
| 수집 방법 | 시드로 만든 합성 주문 파일을 Claude Code의 실제 Read 도구로 읽고, 같은 대화에서 기본 축약과 fast-jev 축약을 분기했다. 같은 파일 자료로 Saturn 패킷도 만들었다. |
| 수집 기간 | 2026-10-05 UTC |
| 표본 여부 | 시드 20261005의 합성 시나리오 48개, 각 기록 위치를 6번씩 질문 대상으로 골랐다. |
| 라벨 | 생성기가 미리 정한 독립 12자리 코드의 정확한 포함 여부로 채점했다. 사람이나 다른 모델의 주관적 채점은 쓰지 않았다. |
| 알려진 문제 | 원래 패킷 조건은 패킷과 질문을 한 턴에 합쳤다. 실제 인수 순서의 추가 검증은 별도 원자료와 집계로 보존했다. |
| 개인정보 | 실제 사용자 대화는 넣지 않았다. CLI 로그에는 로컬 작업 폴더와 실행 식별자가 있어 공개 저장소에 원문을 넣지 않는다. 비밀 키는 파일에 보존하지 않았다. |
| 라이선스 | 합성 fixture와 수집·분석 스크립트는 저장소 라이선스를 따른다. 외부 도구의 소스는 이 폴더에 복사하지 않는다. |

## 파일

원자료는 저장소의 `.local/experiments/same-session-compaction/formal/`, `handoff/`, `persistent/`에 보존한다. 이 경로는 Git에서 제외한다. 공개 해시 목록의 파일명은 각 원자료 폴더 기준 상대 경로다. 원자료가 없는 환경에서는 공개 집계를 읽을 수 있지만 검증을 재실행할 수는 없다.

| 파일 | 내용 | 만드는 스크립트 |
|---|---|---|
| `formal/results.jsonl` | 시나리오별 원답, 자동 점수, session ID, 누적 보고 비용 | `scripts/01-collect.py` |
| `formal/scenario-*/case-*.txt` | 독립 코드와 무관한 메모를 담은 합성 원자료 | `scripts/01-collect.py` |
| `formal/scenario-*/*.jsonl` | 실제 Claude CLI 원응답, 축약 경계와 훅 로그 | `scripts/01-collect.py` |
| `formal/scenario-*/session-*.jsonl` | 완료된 실험의 session ID로 찾아 복사한 Claude 원본 기록 | 원본 파일을 그대로 복사, 이후 수정 금지 |
| `formal/scenario-*/packet.json` | Saturn 예제의 패킷 원출력 | `scripts/01-collect.py` |
| `handoff/results.jsonl`, `handoff/scenario-*/*.jsonl` | 별도 턴으로 패킷을 인수한 뒤 질문한 원응답 | `scripts/03-handoff.py` |
| `persistent/results.jsonl`, `persistent/scenario-*/*.jsonl` | 프로세스를 유지한 축약·질문 대응 실행 | `scripts/05-persistent.py` |
| [SHA256SUMS](SHA256SUMS) | 본 실험 원자료의 파일별 SHA-256 | `scripts/02-analyze.py` |
| [HANDOFF-SHA256SUMS](HANDOFF-SHA256SUMS) | 전달 순서 추가 검증의 파일별 SHA-256 | `scripts/04-handoff-analysis.py` |
| [PERSISTENT-SHA256SUMS](PERSISTENT-SHA256SUMS) | 프로세스 유지 실행의 파일별 SHA-256 | `scripts/06-persistent-analysis.py` |
| [persistent-summary.json](../results/persistent-summary.json) | 프로세스 유지 실행 집계 | `scripts/06-persistent-analysis.py` |
| [summary.json](../results/summary.json) | 본 실험 검증과 집계 | `scripts/02-analyze.py` |
| [handoff-summary.json](../results/handoff-summary.json) | 추가 검증 집계 | `scripts/04-handoff-analysis.py` |

## 필드

| 필드 | 타입 | 단위 | 제약 | 뜻 |
|---|---|---|---|---|
| `index` | integer | 시나리오 | 고유, 1~48 | 같은 입력을 묶는 대응 번호 |
| `target` | integer | 기록 위치 | 1~8 | 후속 질문이 가리키는 기록 |
| `answer` | string | 없음 | 12자리 | 생성기가 보관한 정답 |
| `arms` | object | 없음 | full, native, fast, packet | 조건별 원답과 session ID |
| `exact` | boolean | 없음 | 원답으로 재검증 | 정답의 정확한 포함 여부 |
| `costs` | object | USD | 누적 보고값 | 조건 비용은 공통 사전 대화 누적을 빼서 계산 |
| `ready` | boolean | 없음 | 추가 검증만 | 첫 인수 응답이 Ready인지 |
| `error` | string | 없음 | 실패 때만 | 수집 실패 원인 |

본 실험의 수집기에는 훅 표시의 `text` 대신 `message`를 읽은 오류가 있었다. 원본 CLI 로그는 정상 보존됐다. 분석기는 `fast_compact.jsonl`의 `text`에서 적용·제거 수를 다시 읽고, 잘못 저장된 `arms.fast.hook_applied`를 판정에 쓰지 않는다. 원자료는 수정하지 않는다. 프로세스 유지 실행도 메시지 재개 지점과 비용 누적의 기준이 달라, 수집 행의 비용 대신 원본 result 누적에서 source session의 마지막 누적을 빼서 재계산한다.

## 재현

실제 호출에는 사전 등록한 Claude 버전과 모델, fast-jev 커밋의 작업 사본, Saturn packet 예제가 필요하다. router 키는 `TYPESAFE_API_KEY` 환경으로만 받는다. 새 수집은 기존 결과가 있는 폴더를 덮어쓰지 않으므로 별도 보존 사본에서 수행한다.

```sh
./run.sh collect 48
./run.sh handoff
./run.sh persistent
./run.sh analyze
./run.sh verify
```

이미 수집된 자료로는 마지막 두 명령만 실행한다. `verify`는 원본 응답에서 채점을 다시 계산하고, Read 범위·분기 조상·축약 경계·도구 재호출·오류 응답·파일 해시와 저장 집계를 대조한다.
