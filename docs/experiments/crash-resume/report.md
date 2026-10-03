# 강제 종료 뒤 provider session 재개: 실험 결과

## 요약

Codex는 자식 thread가 실제로 marker 명령을 실행한 3회에서, 자식 정리 없이 메인 `thread/resume`을 해도 재실행 0/3회(0.0%, 95% Wilson [0.0%, 56.1%])였고, 정리 후에도 0/3회였다. Codex raw 재개 가설 H1은 기각하고, 정리 후 재실행 차단 가설 H2는 확인한다. Claude는 전용 credentials가 만료되어 6개 본 조건과 Task 탐색 1회 모두 marker 시작을 관찰하지 못해 H3·H4와 Task 동작을 확인 못 함으로 판정한다. #66 A안 중 Codex 자식 정리 요청의 공식 경로는 확인했지만, raw 조건에서 이미 재실행이 없어 정리의 인과적 필요성은 확인하지 못했다.

## 방법

| 항목 | 값 |
|---|---|
| 설계 | [실험 설계](design.md), 커밋 `0e500f0` |
| 실행 id | `20261003T085117Z-0e500f0` |
| 환경 | [env.json](env.json) |
| 표본 | 설계 13회, 실제 13회; Codex 6회 확인 분석, Claude 6회와 Task 1회는 인증 실패로 확인 못 함 |
| 수집 원문 | 메인 저장소 `.local/experiments/crash-resume/`에 보관; public raw에는 요약만 남김 |

## 설계와 다른 점

| 변경 | 시점 | 이유 | 결론에 미친 영향 |
|---|---|---|---|
| 첫 수집 시 provider 자식 process 정리와 raw 저장 경로를 수정했다. | 수집 중 | Codex 실행 파일의 자식 process가 stderr pipe를 붙잡았고, `env.json` 경로가 잘못됐다. | 첫 수집의 raw는 버리고 private log와 호출 수만 보조 기록으로 남겼다. 수정된 드라이버의 실행만 결과에 사용했다. |
| Codex 자식 식별을 `thread/started`의 부모 필드뿐 아니라 child `threadId`가 붙은 turn/item event로 보완했다. | 수집 전 재실행 | 실제 app-server가 자식의 `thread/started` 알림 대신 child turn event를 보냈다. | 6회 모두 자식 thread와 marker 실행을 확인할 수 있었다. |
| Claude 인증 실패로 marker를 실행하지 못했다. | 수집 중 | 새 로그인 없이 전용 credentials 심볼릭 링크만 사용했으며, provider가 `OAuth session expired and could not be refreshed`를 반환했다. | H3·H4와 Task 탐색은 판정하지 않고 `확인 못 함`으로 남겼다. |

## 결과

### 흐름

| 단계 | 수 |
|---|---:|
| 수집 | 13 |
| 확인 분석: Codex raw·cleaned | 6 |
| 확인 못 함: Claude 환경 변수 있음·없음 | 6 |
| 탐색 분석: Claude Task | 1 |

### 확인 분석

| 가설 | 지표 | 값 | 95% 신뢰구간 | n | 판정 |
|---|---|---:|---|---:|---|
| H1 | Codex raw 재개 뒤 child marker 재실행 | 0/3 (0.0%) | [0.0%, 56.1%] | 3 | 기각 |
| H2 | Codex 정리 뒤 child marker 재실행 | 0/3 (0.0%) | [0.0%, 56.1%] | 3 | 확인 |

Codex raw 3회와 cleaned 3회 모두 강제 종료 전 `start` marker 1개를 기록했고, `thread/resume` 뒤 두 번째 `start`는 기록하지 않았다. cleaned 조건에서는 매회 `thread/archive`와 `thread/unsubscribe`가 성공했다. `turn/interrupt`는 3회 모두 오류였는데, 새 app-server에서 이미 끊긴 child turn을 중단할 수 없었기 때문이다. 메인 `thread/resume` 응답은 6회 모두 성공했다.

### 확인 못 함

| 가설 | 지표 | 값 | 95% 신뢰구간 | n | 판정 |
|---|---|---:|---|---:|---|
| H3 | Claude 환경 변수 있음 재개 뒤 Bash marker 재실행 | 산출 불가: marker 0/3 | — | 3 | 확인 못 함 |
| H4 | Claude 환경 변수 없음 재개 뒤 Bash marker 재실행 | 산출 불가: marker 0/3 | — | 3 | 확인 못 함 |

Claude 본 조건 6회는 모두 전용 session 초기화 또는 재개 과정에서 인증 실패를 기록했다. 따라서 환경 변수의 유무가 interrupted turn 재개에 미친 영향을 분리할 수 없다.

### 탐색 분석

Claude `Task` 하위 에이전트 탐색 1회도 같은 인증 실패로 marker와 child event를 만들지 못했다. 하위 에이전트 동작에 대한 판정에는 사용하지 않는다.

## 논의

### 해석

이 설치 환경과 Codex CLI 0.158.0에서는 부모 thread를 강제 종료 뒤 raw `thread/resume`해도 끊긴 child 명령이 자동으로 다시 실행되지 않았다. 따라서 #66 A안의 Codex 자식 정리는 안전한 공식 경로로 구현할 수 있고 실제 정리 요청도 성공했지만, 이 실험만으로 raw 재개보다 정리가 재실행을 막았다고 말할 수는 없다. Claude는 인증 상태 때문에 `CLAUDE_CODE_RESUME_INTERRUPTED_TURN`의 효과를 측정하지 못했으므로 A안 전체는 부분 근거만 확보했다.

### 타당성 위협

| 종류 | 위협 | 이 실험에서 |
|---|---|---|
| 내적 | provider 강제 종료가 child command도 함께 끝내면 provider session 재개와 process 생존 효과가 섞일 수 있다. | marker `start`를 강제 종료 전에 확인했고, 재개 뒤 추가 `start`를 독립적으로 셌다. Codex 양 조건의 비교는 같은 process 정리 방식이었다. |
| 내적 | Codex raw 조건에서 재실행이 없어 cleanup의 차단 효과를 직접 비교할 수 없다. | H2를 “차단 경로 확인”으로만 채택하고, cleanup의 인과적 필요성은 보류했다. |
| 구성 | touch 하나만 보면 원래 실행 완료와 재실행을 구별하기 어렵다. | invocation별 `start`·`complete` 행과 `.done` 파일을 분리했다. |
| 구성 | 성공 응답이 실제 실행을 뜻하지 않을 수 있다. | marker와 child thread/event를 주 판정으로 삼고 응답은 보조 증거로만 사용했다. |
| 외적 | provider 버전과 인증 상태가 결과를 바꿀 수 있다. | `env.json`에 버전·모델을 기록했으며 Claude 인증 실패는 결론에서 제외했다. |

### 한계

- Claude 인증을 새로 만들지 않는 안전 규칙 때문에 H3·H4를 실제 marker로 확인하지 못했다.
- Codex 표본은 조건마다 3회뿐이고 상한 신뢰구간이 넓다. 결과는 설치 버전의 동작 확인이지 모든 버전의 보장이 아니다.
- 이번 Codex cleaned 경로는 `turn/interrupt`·`thread/archive`·`thread/unsubscribe`를 함께 사용했으므로 개별 요청 하나의 차단 효과는 분리하지 않았다.

## 재현

```sh
./run.sh verify
./run.sh process
./run.sh analyze
```

provider 인증이 유효하지 않으면 `./run.sh collect`를 재실행하지 않고 `env.json`의 인증 blocker를 먼저 해결해야 한다. 실험 당시 실제 provider model call은 수정된 실행에서 Codex 12회, Claude 0회였고, 저장 경로 오류가 난 예비 Codex 실행의 12회를 합치면 Codex 총 24회였다. Claude는 인증 실패 응답만 있어 실제 model call은 0회였다.

| 파일 | SHA-256 |
|---|---|
| `data/raw/crash-resume-20261003T085117Z-0e500f0.jsonl` | `f8911650f7faf7ad3bd244d56541b12955f2b210ce00d5a17933da6108b67acf` |

## 결론

| 가설 | 판정 | 반영한 문서 |
|---|---|---|
| H1 | 기각: Codex raw 재개 child 재실행 0/3 (0.0%) | [엔진 수명과 복구](../../design/engine-lifecycle.md) |
| H2 | 확인: Codex child 정리 후 재실행 0/3 (0.0%), archive·unsubscribe 성공 3/3 | [엔진 수명과 복구](../../design/engine-lifecycle.md), [provider 연결과 session](../../design/providers-and-sessions.md) |
| H3 | 확인 못 함: Claude 인증 실패, marker 0/3 | 없음 |
| H4 | 확인 못 함: Claude 인증 실패, marker 0/3 | 없음 |
