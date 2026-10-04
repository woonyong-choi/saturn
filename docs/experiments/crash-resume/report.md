# 강제 종료 뒤 provider session 재개: 실험 결과

## 요약

Codex는 자식 thread가 실제로 marker 명령을 실행한 3회에서, 자식 정리 없이 메인 `thread/resume`을 해도 재실행 0/3회(0.0%, 95% Wilson [0.0%, 56.1%])였고, 정리 후에도 0/3회였다. Codex raw 재개 가설 H1은 기각하고, 정리 후 재실행 차단 가설 H2는 확인한다. Claude는 첫 수집에서 전용 credentials가 만료되어 확인하지 못했고, 공식 CLI의 기본 로그인으로 다시 수집했다(2026-10-04). `CLAUDE_CODE_RESUME_INTERRUPTED_TURN=1`을 둔 재개는 끊긴 Bash 턴을 3/3회 다시 실행했고(100.0%, [43.9%, 100.0%]), 변수를 뺀 재개는 0/3회(0.0%, [0.0%, 56.1%])였다. H3·H4를 확인한다. `Task` 하위 에이전트를 강제 종료한 탐색 3회에서는 변수가 없는 재개가 세 번 모두 하위 에이전트를 다시 실행하지 않았고, Claude가 그 에이전트를 `stopped`로 알렸다. #66 A안 중 Claude 환경 변수 제거는 실제 재실행을 막는 요소로 확인했다. Codex 자식 정리는 raw 조건에서 이미 재실행이 없어 인과적 필요성을 확인하지 못했다.

## 방법

| 항목 | 값 |
|---|---|
| 설계 | [실험 설계](design.md), 커밋 `0e500f0` |
| 실행 id | `20261003T085117Z-0e500f0`(Codex와 첫 Claude 수집), `20261004T031212Z-00cad83`, `20261004T032243Z-7f3fc43`(Claude 재수집) |
| 환경 | [env.json](env.json) |
| 표본 | 설계 13회, 첫 수집 13회; Codex 6회 확인 분석, Claude 6회와 Task 1회는 인증 실패로 확인 못 함. 재수집으로 Claude 본 조건 6회와 Task 탐색 3회를 분석에 썼다. |
| 수집 원문 | 메인 저장소 `.local/experiments/crash-resume/`에 보관; public raw에는 요약만 남김 |

## 설계와 다른 점

| 변경 | 시점 | 이유 | 결론에 미친 영향 |
|---|---|---|---|
| 첫 수집 시 provider 자식 process 정리와 raw 저장 경로를 수정했다. | 수집 중 | Codex 실행 파일의 자식 process가 stderr pipe를 붙잡았고, `env.json` 경로가 잘못됐다. | 첫 수집의 raw는 버리고 private log와 호출 수만 보조 기록으로 남겼다. 수정된 드라이버의 실행만 결과에 사용했다. |
| Codex 자식 식별을 `thread/started`의 부모 필드뿐 아니라 child `threadId`가 붙은 turn/item event로 보완했다. | 수집 전 재실행 | 실제 app-server가 자식의 `thread/started` 알림 대신 child turn event를 보냈다. | 6회 모두 자식 thread와 marker 실행을 확인할 수 있었다. |
| Claude 인증 실패로 marker를 실행하지 못했다. | 수집 중 | 새 로그인 없이 전용 credentials 심볼릭 링크만 사용했으며, provider가 `OAuth session expired and could not be refreshed`를 반환했다. | H3·H4와 Task 탐색은 판정하지 않고 `확인 못 함`으로 남겼다. |

### Claude 재수집(2026-10-04)

| 변경 | 시점 | 이유 | 결론에 미친 영향 |
|---|---|---|---|
| 전용 `CLAUDE_CONFIG_DIR`과 credentials 심볼릭 링크를 쓰지 않고 공식 `claude`의 기본 로그인으로 실행했다. | 재수집 전 | 첫 수집에서 링크한 credentials가 만료되어 있었고, 전용 설정 폴더는 로그인이 풀린다. 새 로그인과 복사는 하지 않았다. | H3·H4를 판정할 수 있게 됐다. 세션 기록은 Claude Code가 사용자 `~/.claude/projects/` 아래에 직접 남겼다. 설정·토큰·훅은 읽거나 바꾸지 않았다. |
| 작업 폴더를 worktree 루트 대신 `.runtime/claude-work/<조건-회차>`로 두고, marker 경로를 절대 경로로 바꿨다. `--setting-sources`는 `user` 대신 `project,local`로, `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1`을 더했다. | 재수집 전 | 기본 로그인에서는 사용자 설정과 hook이 섞이지 않게 해야 하고, 세션 재개가 같은 작업 폴더를 요구한다. | 판정 기준(marker `start` 행 수)은 그대로다. |
| 모델을 `haiku`로 지정했다. | 재수집 전 | 지시. | `system/init`의 모델은 `claude-haiku-4-5-20251001`로 첫 수집과 같았다. |
| Claude 호출 상한을 모델 응답 30회에서 `claude` 실행 40회로 바꿨다. 실행 직전에 세어 닿으면 멈추는 구현이다. | 재수집 전 | 지시. | 점검 2회를 포함해 32회를 썼다. |
| `Task` 탐색을 1회에서 3회로 늘리고, 지시문에 `Agent`(`Task`) 도구 이름을 적었다. | 첫 재수집 뒤 | 첫 재수집의 `Task` 3회 중 2회는 모델이 "Task 도구가 없다"며 하위 에이전트를 만들지 않았고, 1회는 `Bash`를 직접 실행했다. 이 버전은 도구 이름을 `Agent`로 보인다. | 둘째 재수집의 3회 모두 하위 에이전트의 `Bash`(`agent_id` 포함)가 marker를 썼다. 첫 재수집의 `Task` 3회는 하위 에이전트가 없어 분석에서 뺐다. |
| 재개 process가 `system/init` 없이 관찰 구간(35초) 끝까지 살아 있고 stderr가 비어 있으면 `observed`로 보는 규칙을 더했다. `Task` 탐색은 하위 에이전트의 `Bash` 호출이 있어야 유효하다. | 첫 재수집 뒤 | 변수가 없는 재개 process는 입력이 올 때까지 `init`을 내지 않아, 첫 재수집의 `env-absent` 3회가 `error_or_timeout`이 됐다. 설계 규칙으로는 "재개했지만 아무 일도 없었다"와 "재개가 실패했다"를 구별하지 못했다. | 결과를 본 뒤 보강한 규칙이다. `env-absent`는 새 규칙으로 다시 수집한 3회만 H4 판정에 썼다. 첫 재수집의 같은 조건 3회도 marker `start` 1개로 같은 방향이었다. |
| 기본 로그인 경로 점검 1회(`env-present` 형식)를 raw에 저장하지 않았다. | 재수집 전 | driver가 기본 로그인에서 동작하는지 보려는 점검이었다. | 분석에 쓰지 않았다. 호출 수에는 포함했다(실행 2회). |

## 결과

### 흐름

| 단계 | 수 |
|---|---:|
| 수집 | 13 |
| 확인 분석: Codex raw·cleaned | 6 |
| 확인 못 함: Claude 환경 변수 있음·없음 | 6 |
| 탐색 분석: Claude Task | 1 |
| Claude 재수집, 분석: `env-present` 3, `env-absent` 3(둘째 재수집), `Task` 3(둘째 재수집) | 9 |
| Claude 재수집, 분석 제외: 첫 재수집의 `env-absent` 3, `Task` 3 | 6 |
| 점검(저장 안 함) | 1 |

### 확인 분석

| 가설 | 지표 | 값 | 95% 신뢰구간 | n | 판정 |
|---|---|---:|---|---:|---|
| H1 | Codex raw 재개 뒤 child marker 재실행 | 0/3 (0.0%) | [0.0%, 56.1%] | 3 | 기각 |
| H2 | Codex 정리 뒤 child marker 재실행 | 0/3 (0.0%) | [0.0%, 56.1%] | 3 | 확인 |

Codex raw 3회와 cleaned 3회 모두 강제 종료 전 `start` marker 1개를 기록했고, `thread/resume` 뒤 두 번째 `start`는 기록하지 않았다. cleaned 조건에서는 매회 `thread/archive`와 `thread/unsubscribe`가 성공했다. `turn/interrupt`는 3회 모두 오류였는데, 새 app-server에서 이미 끊긴 child turn을 중단할 수 없었기 때문이다. 메인 `thread/resume` 응답은 6회 모두 성공했다.

### 첫 수집의 Claude: 확인 못 함

첫 수집(`20261003T085117Z-0e500f0`)의 Claude 본 조건 6회는 모두 전용 session 초기화 또는 재개 과정에서 인증 실패를 기록했고, marker `start`는 0개였다. 환경 변수의 효과를 분리할 수 없어 그때는 H3·H4를 확인 못 함으로 두었다. 아래 재수집이 이를 대신한다.

### Claude 재수집: 확인 분석

| 가설 | 지표 | 값 | 95% 신뢰구간 | n | 판정 |
|---|---|---:|---|---:|---|
| H3 | Claude 환경 변수 있음 재개 뒤 Bash marker 재실행 | 3/3 (100.0%) | [43.9%, 100.0%] | 3 | 확인 |
| H4 | Claude 환경 변수 없음 재개 뒤 Bash marker 재실행 | 0/3 (0.0%) | [0.0%, 56.1%] | 3 | 확인 |

`env-present` 3회는 모두 강제 종료 전 `start` 1개, 재개 뒤 `start` 2개였고, 재개 process의 첫 결과 이벤트에 `resume_reason: interrupted_turn`이 실렸다. 재개 직후 모델이 같은 `Bash`를 다시 요청했고(driver가 허용), 그 명령이 두 번째 `start`를 썼다. 사용자 입력은 보내지 않았다. 재실행은 원래 process의 생존이 아니라 새 `Bash` 실행이다. 두 `start`의 pid가 달랐다. 강제 종료 때 provider와 그 자손 process를 함께 끝냈기 때문이다. `complete`와 touch는 관찰 구간(35초) 안에 0개였다. 재실행된 명령의 `sleep 30`이 구간 안에 끝나지 않았다.

`env-absent` 3회는 모두 `start` 1개로 재개 뒤 늘지 않았다. 재개 process는 관찰 구간 내내 살아 있었고 stderr가 비었으며 `system/init`을 포함한 어떤 이벤트도 내지 않았다. 입력이 올 때까지 아무것도 하지 않은 것이다. 이 조건에서도 모델 응답은 재개 쪽에서 0건이었다. 재개 뒤 새 입력을 보냈을 때의 동작은 측정하지 않았다.

### Claude 재수집: 탐색 분석

`Task` 하위 에이전트 탐색 3회(둘째 재수집, 변수 없음)에서는 모두 모델이 `Agent` 도구로 하위 에이전트를 백그라운드로 만들었고, 하위 에이전트가 `Bash`(요청에 `agent_id` 포함)로 marker `start`를 쓴 뒤 provider를 강제 종료했다. 재개 뒤 `start`는 늘지 않았다(0/3). 재개 process는 `system/init`과 함께 `task_notification`을 보냈고, 내용은 세 회 모두 `status: stopped`, "Background agent ... didn't finish before the previous session ended"였다. `parent_tool_use_id`가 붙은 재개 이벤트는 0건이었다. Claude가 끊긴 하위 에이전트를 스스로 `stopped`로 닫고 다시 실행하지 않았다. 표본이 3회이고 변수가 없는 조건 하나만 봤으므로 H1~H4 판정에는 쓰지 않는다. 변수를 두고 같은 하위 에이전트를 재개하는 조건은 측정하지 않았다.

### Codex와 Claude 나란히

| 조건 | Codex 0.158.0 | Claude Code 2.1.288(`haiku`) |
|---|---|---|
| 정리 없이 재개(H1·H3) | 재실행 0/3 [0.0%, 56.1%], 기각 | 변수 있음: 재실행 3/3 [43.9%, 100.0%], 확인 |
| 정리 후 재개(H2·H4) | 자식 archive·unsubscribe 후 0/3 [0.0%, 56.1%], 확인 | 변수 없음: 0/3 [0.0%, 56.1%], 확인 |
| 하위 에이전트(탐색) | 자식 thread 3회가 raw 재개에서 재실행되지 않음 | `Task` 3회, 변수 없는 재개에서 재실행 0/3, `stopped` 알림 3/3 |

## 논의

### 해석

이 설치 환경과 Codex CLI 0.158.0에서는 부모 thread를 강제 종료 뒤 raw `thread/resume`해도 끊긴 child 명령이 자동으로 다시 실행되지 않았다. 따라서 #66 A안의 Codex 자식 정리는 안전한 공식 경로로 구현할 수 있고 실제 정리 요청도 성공했지만, 이 실험만으로 raw 재개보다 정리가 재실행을 막았다고 말할 수는 없다.

Claude Code 2.1.288에서는 반대로 변수 하나가 결과를 갈랐다. `CLAUDE_CODE_RESUME_INTERRUPTED_TURN=1`이 있으면 `--resume`만으로 끊긴 턴이 다시 실행됐고, 같은 입력과 같은 절차에서 변수를 빼면 재개 process가 아무것도 하지 않았다. 변수 제거가 실제로 재실행을 막는 요소라는 근거가 Claude에서는 직접 확보됐다. 다만 이 비교는 재개 뒤 새 입력이 없는 경우이고, 사용자가 `/continue`로 새 입력을 보내는 경우의 동작은 아니다. 하위 에이전트는 변수 없이 재개해도 Claude가 `stopped`로 닫았으므로 이 탐색에서는 되살아나지 않았다.

### 타당성 위협

| 종류 | 위협 | 이 실험에서 |
|---|---|---|
| 내적 | provider 강제 종료가 child command도 함께 끝내면 provider session 재개와 process 생존 효과가 섞일 수 있다. | marker `start`를 강제 종료 전에 확인했고, 재개 뒤 추가 `start`를 독립적으로 셌다. Codex 양 조건의 비교는 같은 process 정리 방식이었다. |
| 내적 | Codex raw 조건에서 재실행이 없어 cleanup의 차단 효과를 직접 비교할 수 없다. | H2를 “차단 경로 확인”으로만 채택하고, cleanup의 인과적 필요성은 보류했다. |
| 구성 | touch 하나만 보면 원래 실행 완료와 재실행을 구별하기 어렵다. | invocation별 `start`·`complete` 행과 `.done` 파일을 분리했다. |
| 구성 | 성공 응답이 실제 실행을 뜻하지 않을 수 있다. | marker와 child thread/event를 주 판정으로 삼고 응답은 보조 증거로만 사용했다. |
| 외적 | provider 버전과 인증 상태가 결과를 바꿀 수 있다. | `env.json`에 버전·모델을 기록했으며 첫 수집의 Claude 인증 실패는 결론에서 제외하고 재수집으로 대체했다. |
| 내적 | 재개 뒤 재실행이 provider의 자동 재개인지 모델의 새 판단인지 구별하기 어렵다. | `resume_reason: interrupted_turn` 이벤트와 새 입력 없음으로 재개 경로를 확인했다. 모델 응답 내용 자체의 재현성은 따지지 않았다. |
| 내적 | 판정 규칙을 첫 재수집 결과를 본 뒤 보강했다(재개 process 생존 확인, `Task` 하위 에이전트 `Bash` 확인). | 설계와 다른 점에 적었다. `env-absent`와 `Task`는 보강한 규칙으로 다시 수집한 자료만 썼다. |
| 외적 | 사용자 계정의 기본 로그인과 `~/.claude` 상태가 재개 동작에 섞일 수 있다. | 사용자 설정은 `--setting-sources project,local`로 읽지 않았다. 세션 저장소와 인증은 기본 그대로라 완전한 격리는 아니다. |

### 한계

- 첫 수집에서는 Claude 인증이 만료되어 H3·H4를 확인하지 못했다. 재수집은 기본 로그인으로 했으므로 전용 설정 폴더와 같은 격리는 아니다.
- Claude 표본도 조건마다 3회이고 모델은 `haiku` 하나다. 모델이 `Bash`를 다시 요청한 결과이므로 다른 모델에서는 달라질 수 있다.
- 변수가 없는 재개 뒤 새 입력을 보낸 경우와, 변수가 있는 재개에서 `Task` 하위 에이전트가 어떻게 되는지는 측정하지 않았다.
- 재수집 세션 기록 10개 폴더가 사용자 `~/.claude/projects/` 아래 Claude Code가 만든 그대로 남아 있다(`claude-work` 이름 포함).
- Codex 표본은 조건마다 3회뿐이고 상한 신뢰구간이 넓다. 결과는 설치 버전의 동작 확인이지 모든 버전의 보장이 아니다.
- 이번 Codex cleaned 경로는 `turn/interrupt`·`thread/archive`·`thread/unsubscribe`를 함께 사용했으므로 개별 요청 하나의 차단 효과는 분리하지 않았다.

## 재현

```sh
./run.sh verify
./run.sh process
./run.sh analyze
```

provider 인증이 유효하지 않으면 `./run.sh collect`를 재실행하지 않고 `env.json`의 인증 blocker를 먼저 해결해야 한다. Claude 재수집은 `claude auth status`가 `loggedIn: true`일 때 `python3 scripts/01-collect.py --claude-default-login [--only 조건,...]`로 한다. 실험 당시 실제 provider model call은 수정된 첫 실행에서 Codex 12회, Claude 0회였고, 저장 경로 오류가 난 예비 Codex 실행의 12회를 합치면 Codex 총 24회였다. Claude 재수집은 `claude` 실행 32회(점검 2회 포함, 상한 40회)였고 모델 응답 단위로는 64건(점검 6, 첫 재수집 32, 둘째 재수집 26)이었다. 첫 수집의 Claude는 인증 실패 응답뿐이어서 model call이 0회였다.

| 파일 | SHA-256 |
|---|---|
| `data/raw/crash-resume-20261003T085117Z-0e500f0.jsonl` | `f8911650f7faf7ad3bd244d56541b12955f2b210ce00d5a17933da6108b67acf` |
| `data/raw/crash-resume-20261004T031212Z-00cad83.jsonl` | `3b17bba9ed1e3d76712d80e16c82f4cdb011e224e85bc796bd9818de46f72d92` |
| `data/raw/crash-resume-20261004T032243Z-7f3fc43.jsonl` | `7d5436e92597789784cf0fd216508eeb5699c1cdb1b1bc9c81ba48ed51b6c076` |

## 결론

| 가설 | 판정 | 반영한 문서 |
|---|---|---|
| H1 | 기각: Codex raw 재개 child 재실행 0/3 (0.0%) | [엔진 수명과 복구](../../design/engine-lifecycle.md) |
| H2 | 확인: Codex child 정리 후 재실행 0/3 (0.0%), archive·unsubscribe 성공 3/3 | [엔진 수명과 복구](../../design/engine-lifecycle.md), [provider 연결과 session](../../design/providers-and-sessions.md) |
| H3 | 확인(재수집): 변수 있음 재개 뒤 Bash 재실행 3/3 (100.0%), 첫 수집은 인증 실패로 확인 못 함 | [provider 연결과 session](../../design/providers-and-sessions.md) |
| H4 | 확인(재수집): 변수 없음 재개 뒤 재실행 0/3 (0.0%), 재개 process는 입력 전까지 무동작 | [provider 연결과 session](../../design/providers-and-sessions.md) |
