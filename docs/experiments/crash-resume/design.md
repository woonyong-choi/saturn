# 강제 종료 뒤 provider session 재개: 실험 설계

| 항목 | 값 |
|---|---|
| 이슈 | [#24](https://github.com/woonyong-choi/saturn/issues/24) |
| 관련 설계 | [엔진 수명과 복구](../../design/engine-lifecycle.md), [provider 연결과 session](../../design/providers-and-sessions.md), [#66 결정](https://github.com/woonyong-choi/saturn/issues/66) |
| 사전 데이터 | 없음. 앞 실험의 비차단 줄 큐와 전용 provider home 구현만 재사용하고, 이번 결과는 새 마커와 새 session에서 수집한다. |

## 질문

Codex app-server와 Claude Code가 provider 프로세스 강제 종료 뒤 끊긴 자식 작업을 스스로 다시 실행하는지 측정한다. #66의 A안에 해당하는 Codex 자식 session 정리와 Claude 재개 환경 변수 제거가 실제 마커 재실행을 막는지도 같은 입력과 같은 worktree에서 비교한다.

## 가설

| 가설 | 예측 | 적용 범위 |
|---|---|---|
| H1 | Codex에서 자식 정리 없이 루트에 `thread/resume`을 보내면 끊긴 자식 명령이 다시 실행된다. | Codex CLI 0.158.0, 전용 `CODEX_HOME`, 조건마다 3회 |
| H2 | Codex에서 루트 재개 전에 자식의 `turn/interrupt`·`thread/archive`·`thread/unsubscribe`를 보내면 자식 명령이 다시 실행되지 않는다. | Codex CLI 0.158.0, 전용 `CODEX_HOME`, 조건마다 3회 |
| H3 | Claude Code에서 `CLAUDE_CODE_RESUME_INTERRUPTED_TURN=1`을 두고 `--resume`하면 끊긴 Bash 턴이 다시 실행된다. | Claude Code 2.1.288, 전용 `CLAUDE_CONFIG_DIR`, 조건마다 3회 |
| H4 | Claude Code에서 위 환경 변수를 제거하고 `--resume`하면 끊긴 Bash 턴이 다시 실행되지 않는다. | Claude Code 2.1.288, 전용 `CLAUDE_CONFIG_DIR`, 조건마다 3회 |

## 설계

| 항목 | 값 |
|---|---|
| 조건 | `codex.raw-resume`, `codex.cleaned-resume`, `claude.resume-env-present`, `claude.resume-env-absent`; 탐색 조건 `claude.task-subagent` |
| 배정 | provider별 조건 순서는 고정한다: Codex raw → Codex cleaned → Claude 환경 변수 있음 → Claude 환경 변수 없음. provider session과 마커가 서로 독립인 전용 home을 매 회 만든다. |
| 눈가림 | 해당 없음. 도구별 공식 요청과 marker/event 판정을 드라이버가 기록한다. 성공 응답만으로 결과를 판정하지 않는다. |
| 환경 | macOS, worktree `/Users/woonyong/workspace/oss/saturn.wt/experiment-24-crash-resume`, Codex 전용 `CODEX_HOME`과 auth 심볼릭 링크, Claude 전용 `CLAUDE_CONFIG_DIR`과 credentials 심볼릭 링크, 저장소 밖 원문 로그는 메인 저장소 `.local/experiments/crash-resume/`에 둔다. 설치·로그인·전역 설정 변경은 하지 않는다. |

Codex의 각 반복은 메인 thread가 자식 agent를 만들고 자식이 `sleep 30; touch`를 포함한 무해한 명령을 실행하는 시점을 marker 시작 행으로 확인한 뒤, app-server의 자기 PID만 `SIGKILL`한다. 새 app-server는 같은 전용 home에서 초기화한다. raw 조건은 바로 메인 `thread/resume`을 보내고, cleaned 조건은 `thread/list`로 부모의 자식을 찾은 뒤 `turn/interrupt`, `thread/archive`, `thread/unsubscribe`를 기록하고 메인만 `thread/resume`한다. 재개 뒤에는 새 입력을 보내지 않고 관찰한다.

Claude의 각 반복은 `claude -p --input-format stream-json --output-format stream-json`으로 고정 UUID session을 만들고, Bash가 marker 시작 행을 쓴 뒤 잠들었을 때 provider 자기 PID만 `SIGKILL`한다. 재개 프로세스에는 조건에 따라 `CLAUDE_CODE_RESUME_INTERRUPTED_TURN=1`을 넣거나 부모 환경에서 제거한다. `--resume <session>` 뒤 새 사용자 입력은 보내지 않고, 재개 요청이 오면 같은 안전 명령에 대해 허용 응답만 하며 marker와 stream event를 관찰한다.

## 변수

| 변수 | 종류 | 정의 | 단위 |
|---|---|---|---|
| `provider` | 조작 | `codex` 또는 `claude` | 명목 |
| `condition` | 조작 | 사전 등록한 재개·정리 조건 | 명목 |
| `resume_interrupted_env` | 조작 | Claude 환경 변수 값 `1` 또는 변수 없음 | 명목 |
| `codex_cleanup_requests` | 조작 | Codex 재개 전 자식에 보낸 공식 요청 목록과 응답 | 요청 목록 |
| `marker_start_count` | 측정 | 명령이 잠들기 전에 marker event에 쓴 `start` 행 수 | 개수 |
| `marker_complete_count` | 측정 | 잠든 뒤 marker event에 쓴 `complete` 행 수 | 개수 |
| `marker_touch_count` | 측정 | 명령 완료 때 marker 파일을 만든 실행 수 | 개수 |
| `resume_child_execution` | 파생 | 재개 시점 뒤 두 번째 `start` 행 또는 provider event가 관찰되면 `true` | boolean |
| `observation_status` | 파생 | `confirmed`, `unstable`, `cannot_distinguish` | 명목 |
| `model_calls` | 측정 | private event log에서 provider 모델 응답 단위로 센 호출 수 | 개수 |

marker event는 `start:<invocation-id>:<pid>`와 `complete:<invocation-id>:<pid>`를 한 줄씩 기록한다. `start`가 두 개 이상이고 두 번째 시작 시각이 provider 강제 종료 뒤이면 재실행으로 판정한다. 원래 자식 process가 살아서 나중에 `complete`만 쓰는 경우에도 `start` 개수는 늘지 않으므로, marker 생성 횟수와 재실행 판정을 구별한다.

## 표본

| 항목 | 값 |
|---|---|
| 출처 | 매 반복 새로 만든 전용 provider session과 worktree 안 `.runtime/markers/` |
| 크기 | 확인 조건마다 3회; Claude Task 하위 에이전트는 가능하면 1회 탐색 |
| 크기 근거 | 전수 아님. 사용자가 지정한 반복 예산이며, 같은 조건 3회의 일치 여부를 확인하는 최소 실측이다. |
| 중단 규칙 | Codex 모델 호출 40회, Claude 모델 호출 30회에 닿기 전에 새 호출을 시작하지 않는다. provider가 marker 시작을 관찰하지 못하거나 session 식별이 불가능하면 해당 반복을 `cannot_distinguish`로 남기고 다음 반복으로 간다. 남은 provider process는 자기 PID만 종료한 뒤 진행한다. |
| 반복과 예열 | 예열 0회, 조건마다 독립 반복 3회. 최초 provider handshake와 모델 응답은 해당 반복의 호출 수에 포함한다. |

## 분석

| 가설 | 지표 | 방법 | 채택 기준 |
|---|---|---|---|
| H1 | `resume_child_execution` 비율 | `k/n`과 95% Wilson 신뢰구간; 세 반복의 범주도 함께 기록 | 세 반복이 모두 `true`면 `확인`, 모두 `false`면 `기각`, 섞이면 `불안정`, 식별 불가가 있으면 `확인 못 함` |
| H2 | `resume_child_execution` 비율 | `k/n`과 95% Wilson 신뢰구간; cleanup 요청 응답과 marker/event를 함께 대조 | 세 반복이 모두 `false`면 `확인`, 모두 `true`면 `기각`, 섞이면 `불안정`, 식별 불가가 있으면 `확인 못 함` |
| H3 | `resume_child_execution` 비율 | `k/n`과 95% Wilson 신뢰구간; `CLAUDE_CODE_RESUME_INTERRUPTED_TURN=1`을 로그에서 확인 | 세 반복이 모두 `true`면 `확인`, 모두 `false`면 `기각`, 섞이면 `불안정`, 식별 불가가 있으면 `확인 못 함` |
| H4 | `resume_child_execution` 비율 | `k/n`과 95% Wilson 신뢰구간; 재개 환경에 해당 이름이 없는지 확인 | 세 반복이 모두 `false`면 `확인`, 모두 `true`면 `기각`, 섞이면 `불안정`, 식별 불가가 있으면 `확인 못 함` |

| 항목 | 규칙 |
|---|---|
| 제외 기준 | 없음. 시작을 못 한 반복, provider 오류, timeout은 실패한 실행으로 흐름 표에 남기고 `cannot_distinguish`로 분석한다. |
| 실패한 실행 | 성공 응답이 아니라 marker/event 판정이 불가능한 실행으로 센다. provider의 오류 응답도 원문을 숨기지 않는 요약과 private log 경로를 남긴다. |
| 다중 비교 | 확인 가설이 네 개이므로 비율의 해석은 가설별 사전 기준으로만 하고, 가설 사이 유의성 검정은 하지 않는다. |

## 판정

| 결과 | 설계에 반영 |
|---|---|
| 채택 | 실제로 재실행을 막은 Codex 정리 요청과 Claude 환경 제거를 크래시 복구 설계에 반영한다. |
| 기각 | 해당 provider의 A안 요소를 자동 차단 근거로 쓰지 않고 보류·수동 재개로 남긴다. |
| 보류 | 세 반복이 일치하지 않거나 provider 버전·session 상태를 구별하지 못한 조건은 추가 반복 또는 버전 고정 후 재측정한다. |

## 탐색 분석

- Claude `Task`가 만든 하위 에이전트의 끊긴 Bash 턴이 `--resume`에서 어떻게 되는지 1회 이상 가능하면 관찰한다. 표본이 3회가 아니므로 H1~H4 판정에는 사용하지 않는다.
- Codex 스키마에서 발견한 정리 요청별 성공 응답과 재개 뒤 `thread/started`, `turn/started`, `item/started` event의 관계를 기록한다.

## 타당성 위협

| 종류 | 위협 | 대응 |
|---|---|---|
| 내적 | provider가 명령 시작 직후 강제 종료되어 재실행 여부와 원래 실행의 완료를 구별하기 어렵다. | 시작·완료·touch를 별도 marker 행과 provider event 시각으로 기록하고 재개 뒤 최소 35초 관찰한다. |
| 내적 | Codex 자식이 실제로 생성되지 않거나 Claude가 Task를 선택하지 않을 수 있다. | 자식 thread의 `parentThreadId`, Claude `parent_tool_use_id`/Task event, marker 시작을 모두 확인하고 불가능하면 `cannot_distinguish`로 남긴다. |
| 구성 | marker 파일이 같은 이름이라 touch가 덮어쓰기를 일으킬 수 있다. | invocation별 `start`/`complete` append log와 별도 `.done` marker를 함께 사용한다. |
| 구성 | 성공 응답만 보고 provider 동작을 오판할 수 있다. | 재개 요청 응답은 보조 증거로만 쓰고 실제 marker와 event를 주 판정으로 사용한다. |
| 외적 | 설치된 provider의 버전·모델·로그인 상태가 바뀌면 결과가 달라질 수 있다. | `env.json`에 실행 버전·모델·실행 날짜를 기록하고, 전용 home과 auth 심볼릭 링크를 사용한다. |
| 외적 | Claude 전역 설정·hook이 전용 session에 섞일 수 있다. | 전용 `CLAUDE_CONFIG_DIR`, `--strict-mcp-config`, 제한된 tool 목록을 사용하고 전역 설정을 수정하지 않는다. |
