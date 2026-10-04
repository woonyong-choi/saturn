# Claude provider 동작 묶음: 실험 결과

## 요약

Claude Code 2.1.288(`haiku`)에서 가설 27개를 조건마다 3회씩 쟀고 21개 조건을 모두 실행했다(사용자 턴 72회). 채택 19, 기각 7, 보류 1이다. 기각 가운데 H19 넷과 H26-4는 판정 정의의 세부가 틀렸고 질문의 답은 나왔다. H17-4는 하위 에이전트가 같은 작업을 다시 시작해서, H3-2는 Saturn 훅이 `sh -c "/usr/bin/security find-generic-password ..."`를 3/3 막지 못해서 기각이다. 설계 문서의 "실측으로 확인한다" 문장 네 곳과 훅 문장을 결과로 고쳤고 코드는 고치지 않았다.

## 방법

| 항목 | 값 |
|---|---|
| 설계 | [실험 설계](design.md), 커밋 `9fc9429` |
| 실행 id | `20261004T083629Z-9fc9429` |
| 환경 | [env.json](env.json) |
| 표본 | 설계 조건 21개 곱하기 3회 = 63회차, 사용자 턴 72회. 실제 63회차, 사용자 턴 72회 |

## 설계와 다른 점

| 변경 | 시점 | 이유 | 결론에 미친 영향 |
|---|---|---|---|
| `02-process.py`에 탐색 열(입력 3칸 일치, `modelUsage` 누적 비교, `origin`, `subagent_stats`)을 더하고 `03-analyze.py`에 `exploratory` 절을 더했다. 사전 등록한 판정 정의는 그대로다. | 분석 중 | H19 넷이 출력 토큰 때문에 기각되어 범위를 따로 볼 방법이 필요했다. | 확인 분석 판정은 바뀌지 않았다. `탐색 분석`의 결과는 판정에 쓰지 않았다. |

## 결과

### 흐름

| 단계 | 수 |
|---|---|
| 수집 | 63회차(사용자 턴 72회) |
| 제외 | 0 |
| 분석 | 63회차 |
| 전제가 없었던 회차(`unobserved`) | 2회차. H23-3에서 안쪽 `Bash` 호출이 스트림에 보이지 않았다. |
| 실패한 실행 | 0. 인증 오류, timeout, 호출 상한 도달이 없었다. |

가짜 키체인 항목 `saturn-test-dummy`의 삭제를 확인했고 시험이 띄운 process는 남지 않았다.

### 확인 분석

신뢰구간은 `k/n`의 정확(Clopper-Pearson) 95% 구간이다. n이 3이라 넓다.

| 가설 | 지표 | 값 | 사전 지정 신뢰구간 | n | 판정 |
|---|---|---|---|---|---|
| H17-1 | 하위 이벤트의 `parent_tool_use_id` | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H17-2 | `task_started`와 끝 `task_*`의 `tool_use_id` | 6/6 | [54.1, 100.0] | 6 | 채택 |
| H17-3 | 도구 결과와 첫 `result`가 작업 끝 앞 | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H17-4 | 작업 끝 뒤 입력 없는 두 번째 `result` | 2/3 | [9.4, 99.2] | 3 | 기각 |
| H18-1 | 대조: 멈추지 않으면 marker 생성 | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H18-2 | `interrupt` 성공 응답, marker 없음 | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H18-3 | 입력 닫기 10초 안 종료, marker 없음 | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H18-4 | 리더 SIGTERM 10초 안 종료, 자손 0, marker 없음 | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H19-1 | 두 턴의 `usage`가 구간별 메인 합과 같음(4칸) | 0/3 | [0.0, 70.8] | 3 | 기각 |
| H19-2 | 전경 하위 에이전트, `usage`가 메인 합과 같음(4칸) | 0/3 | [0.0, 70.8] | 3 | 기각 |
| H19-3 | 백그라운드 하위 에이전트, 첫 `usage`가 메인 합과 같음(4칸) | 0/3 | [0.0, 70.8] | 3 | 기각 |
| H19-4 | json 결과의 `modelUsage` 합이 `usage`와 같음 | 0/3 | [0.0, 70.8] | 3 | 기각 |
| H23-0 | 대조: 훅 없이 하위 에이전트가 값을 읽음 | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H23-1 | 훅이 전경 하위 에이전트 조회를 막음 | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H23-2 | 훅이 백그라운드 하위 에이전트 조회를 막음 | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H23-3 | 훅이 중첩 하위 에이전트 조회를 막음 | 1/3(보류 회차 2) | [0.8, 90.6] | 3 | 보류 |
| H26-1 | 프로젝트 명령의 `can_use_tool` 왕복과 marker | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H26-2 | `/context`가 요청 없이 글 있는 `result` | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H26-3 | `/compact`가 `compact_boundary`와 두 번째 `result` | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H26-4 | 없는 명령의 `result` 글에 `unknown` | 0/3 | [0.0, 70.8] | 3 | 기각 |
| H4-1a | `--permission-mode acceptEdits`가 `init`에 `acceptEdits` | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H4-1b | 주지 않은 stream 회차 `init`이 `default` | 57/57 | [93.7, 100.0] | 57 | 채택 |
| H4-2 | 샌드박스와 허용 도메인이 `init`이나 제어 응답에 나타남 | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H4-3 | `set_permission_mode` 뒤 두 번째 `init`이 `acceptEdits` | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H3-0 | 대조: 훅 없이 Bash 조회가 값을 돌려줌 | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H3-1 | 훅이 직접 `security find-generic-password`를 막음 | 3/3 | [29.2, 100.0] | 3 | 채택 |
| H3-2 | 훅이 `sh -c "/usr/bin/security ..."`를 막음 | 0/3 | [0.0, 70.8] | 3 | 기각 |

### 이슈별 답

#### #17 Claude 하위 에이전트 이벤트

- `Agent` 도구를 호출하면 `system` 이벤트가 온다. `task_started`(`task_id`, `tool_use_id`, `subagent_type`, `is_backgrounded`, `spawn_depth`, `task_type`, `prompt`), `task_progress`(`last_tool_name`, `usage`), `task_updated`(`patch`), `task_notification`(`status`, `summary`, `usage`, `output_file`)이다. `tool_use_id`는 `Agent` 도구 호출 id와 같았다(H17-2, 6/6). 실행 중인 작업 전체는 `background_tasks_changed.tasks`가 싣는다.
- 하위 에이전트의 `assistant`, `user` 이벤트는 `parent_tool_use_id`가 `Agent` 도구 호출 id였다. 전경에서 하위 이벤트는 회차마다 3건이었고 모두 그 id를 가리켰으며 도구 결과는 마지막 하위 이벤트 뒤에 왔다(H17-1, 3/3).
- 백그라운드에서는 `Agent` 도구 결과가 시작 직후(`async_launched`)에 왔고 메인의 첫 `result`도 작업이 끝나기 전에 왔다(H17-3, 3/3). 도구 결과를 끝으로 읽으면 틀린다.
- 끝난 뒤 사용자 입력 없이 `origin.kind`가 `task-notification`인 `result`가 다시 왔다(`sub_bg`, `bg_none`의 두 번째 `result` 6/6). 그러나 이 `result`가 마지막 작업이 끝난 뒤라는 보장은 없다. 하위 에이전트가 `sleep` 명령을 자기 백그라운드 셸 작업으로 돌리면 하위 에이전트 작업이 먼저 `completed`가 되어 `result`가 오고, 셸이 끝나면 같은 `task_id`가 다시 `task_started`로 시작해 또 `completed`가 된다. H17-4가 2/3인 이유이고, `bg_none`은 `result`가 3개였다.
- 답: 시작과 끝은 이벤트로 추적할 수 있다. 끝 판정은 도구 결과가 아니라 `task_*`와 `background_tasks_changed`로 해야 하고, 한 번의 `completed`를 최종으로 보면 안 된다.

#### #18 Claude 백그라운드 하위 에이전트 정지

- 대조(`bg_none`): 멈추지 않으면 marker가 36.5~38.7초에 생겼다(3/3).
- `interrupt` 제어 요청: 성공 응답이 왔고(3/3), 하위 에이전트와 그 셸 작업에 `task_notification`의 `stopped`가 왔으며, process는 끝나지 않았다. 40초 뒤에도 marker가 없었다(3/3). 하위 에이전트가 살아 있던 회차에서 `result`는 `terminal_reason`이 `aborted_streaming`, `subagent_stats.killed.system`이 1이었다.
- 입력 닫기: process가 7.8~9.6초 뒤 종료 코드 0으로 끝났고 그 사이 셸 작업에 `stopped`가 왔으며 marker는 없었다(3/3). 닫은 뒤 3초에는 멈춤 시점 자손 2개가 아직 살아 있었다.
- 리더 SIGTERM: 0.8~1.0초 만에 종료 코드 143으로 끝났고 3초 뒤 자손이 없었고 marker도 없었다(3/3).
- 답: 세 방법 모두 백그라운드 하위 에이전트의 작업을 멈춘다. `interrupt`는 process를 남기고, 입력 닫기는 약 9초가 걸린다.

#### #19 Claude 사용량 보고 범위

- 사전 등록한 H19-1~H19-4는 기각이다. 판정 정의가 출력 토큰을 포함한 4칸 정확 일치였는데, 메시지 줄의 출력 토큰은 스트리밍 도중 값이라 `result.usage`의 출력 토큰과 맞지 않았다.
- 입력, 캐시 쓰기, 캐시 읽기 3칸으로 본 탐색은 일관했다. `result.usage`는 메인 에이전트 메시지 합과 같았다(`plain_two_turn` 6/6, `sub_bg` 6/6, `sub_fg` 3/3). 두 번째 `result`는 두 턴의 누적이 아니었다(누적 합과 같은 회차 `plain_two_turn`, `sub_bg` 모두 0/3). 전경 하위 에이전트의 사용량은 `result.usage`에 없었다(메인과 하위의 합과 같은 회차 0/3).
- `modelUsage`와 `total_cost_usd`는 범위가 다르다. `plain_two_turn`에서 `modelUsage`는 `result.usage`의 누적 합과 같았다(6/6). 하위 에이전트가 있으면 누적 `usage`를 넘었다(`sub_fg` 3/3, `json_sub_fg` 3/3). 하위 에이전트 사용량은 `task_progress`, `task_notification`의 `usage`(`total_tokens`, `tool_uses`, `duration_ms`)에도 있다.
- 답: `result.usage`는 그 턴의 메인 에이전트 사용량(`main-turn`)이다. `modelUsage`와 `total_cost_usd`는 session 누적이면서 하위 에이전트를 포함한다. 출력 토큰은 `result` 값만 쓴다.

#### #23 하위 에이전트 훅 적용 범위(Claude 쪽)

- 훅이 없으면 하위 에이전트가 가짜 키 값을 읽었다(3/3).
- Saturn 훅은 전경과 백그라운드 하위 에이전트의 `security find-generic-password` 조회를 요청이 호스트에 오기 전에 막았다(각 3/3). 훅 입력에 `agent_id`, `agent_type`이 실렸다. 입력 키는 `agent_id`, `agent_type`, `cwd`, `hook_event_name`, `permission_mode`, `prompt_id`, `session_id`, `tool_input`, `tool_name`, `tool_use_id`, `transcript_path`다.
- 중첩: 하위 에이전트가 다시 하위 에이전트를 띄웠고(3/3), 안쪽 `Bash` 조회는 훅 기록에서 3/3 막혔고 값은 새지 않았다. 그러나 안쪽 `Bash` 호출이 스트림 이벤트에 보인 것은 1/3이다. 사전 등록한 판정 정의가 스트림의 호출을 전제로 해서 2회차가 `unobserved`가 되었고 H23-3은 보류다.
- 답(Claude): 훅은 전경과 백그라운드 하위 에이전트의 도구에 걸린다. 중첩은 훅 기록으로는 막혔으나 사전 등록한 기준으로는 보류다. Codex 쪽은 재지 않았다.

#### #26 Claude 스트림 입력 슬래시 명령

- 프로젝트 명령(`/probe-touch`)은 모델 턴으로 확장되어 `Bash` 요청이 `can_use_tool`로 왔고, 허용하자 marker가 생겼으며 `result` 하나로 끝났다(3/3).
- `/context`는 요청 없이 `assistant` 이벤트 하나(글이 `local_command_source`에 실림)와 글이 있는 `result` 하나로 끝났다(3/3).
- `/compact`는 `system/status`(`compacting`), `compact_boundary`(`trigger`, `pre_tokens`, `post_tokens`), 합성 `user` 이벤트, `status`(`compact_result`)를 거쳐 글이 빈 `result`로 끝났다(3/3).
- 없는 명령은 오류 없이 모델에 글로 전달되었고 모델이 쓸 수 없는 명령이라고 답하는 `result`로 끝났다. `is_error`는 거짓이었다. 글에 `unknown`이 없어 H26-4는 기각이지만 동작은 확인됐다.
- 답: 슬래시 명령은 일반 사용자 입력 줄로 보내면 되고, 허가 요청은 명령이 도구를 쓸 때만 `can_use_tool`로 온다.

#### #4 공급자 적용 설정 보고 범위(Claude 쪽)

- `system/init`의 `permissionMode`는 적용된 모드와 같았다(`--permission-mode acceptEdits` 3/3, 주지 않은 stream 회차 57/57 `default`). `init`은 턴마다 왔다.
- `set_permission_mode`로 바꾸자 곧바로 `system/status`가 `permissionMode`를 싣고 왔고(3/3), 다음 턴의 `init`도 새 모드였다(3/3).
- 샌드박스와 허용 도메인은 `init`에 없었다(0/3). `get_settings` 제어 응답의 `effective`와 `sources`에는 있었다(3/3). 이것은 `--settings`로 준 값의 되돌림이고 샌드박스가 실제로 걸렸는지는 보여 주지 않는다. `initialize`, `get_settings`는 모델 호출 없이 읽을 수 있고, 사용자 턴 전에는 `init`이 오지 않는다.
- 답(Claude): 권한 모드는 기록 흐름으로 알려 준다. 샌드박스와 네트워크 설정은 흐름 이벤트로는 오지 않고, 제어 요청으로 설정값을 읽을 수 있을 뿐 적용 여부는 알 수 없다. Codex 쪽은 재지 않았다.

#### #3 훅의 키 저장소 접근 차단(Claude 쪽)

- 훅이 없으면 Bash 조회가 가짜 값을 돌려줬다(3/3).
- Saturn 훅은 직접 `security find-generic-password -s ... -w`를 막았다. 요청이 호스트에 오지 않았고, 값은 이벤트에 없었고, 도구 결과는 오류였다(3/3).
- `sh -c "/usr/bin/security find-generic-password ..."`는 3/3 막지 못했다. 요청이 호스트에 왔고 값이 도구 결과에 나왔다. 절대 경로 때문인지 `sh -c` 때문인지는 이 시험으로 가르지 못했다.
- 답(Claude): 직접 명령은 막고 셸을 거친 명령은 막지 못한다. Codex 쪽은 재지 않았다.

### 탐색 분석

- 이벤트 종류(`type/subtype`)와 필드 이름은 `results/summary.json`의 `event_census`에 있다. 하위 에이전트 관련은 `task_started`, `task_progress`, `task_updated`, `task_notification`, `background_tasks_changed`이고, `result`에는 `subagent_stats`, `origin`, `terminal_reason`, `modelUsage`, `total_cost_usd`가 있다.
- `sub_bg`에서 `Agent` 도구 결과는 3.7~4.4초, 첫 `result`는 5.2~5.6초, marker는 21.1~26.3초에 왔다.
- `system/status`가 `permissionMode`를 싣고 온 회차는 `eff_mode_change` 3회차였다.

## 논의

### 해석

Saturn 연결의 가정 가운데 도구 결과로 하위 에이전트 끝을 판정하는 것(#17)과 훅의 셸 감싸기 처리(#3)는 실측과 어긋났다. 나머지(`result.usage`의 `main-turn` 기록, `interrupt`로 트리 멈춤, 모드 보고, 훅의 하위 에이전트 적용, 슬래시 명령 경로)는 가정이 맞았다. H19의 기각은 판정 정의가 출력 토큰에 걸려서이므로 범위 결론은 탐색 분석(입력 3칸 일치, `modelUsage` 누적 비교)에 기댄다.

### Saturn 설계와 코드에 미치는 영향

| 위치 | 영향 | 처리 |
|---|---|---|
| `saturn-terminal/engine/src/providers/claude/convert.rs` 268~271행, 308~309행 | `Agent` 도구 호출에서 `SubagentStarted`를, 도구 결과에서 `SubagentEnded`를 낸다. 백그라운드 subagent는 도구 결과가 시작 직후에 오므로 끝을 일찍 선언한다. `task_*` 이벤트는 읽지 않는다. | 코드 수정 후보로 보고. 설계 문장은 고쳤다. |
| 같은 파일 107행 | `result`에서 `running.clear()`를 한다. 백그라운드 subagent가 남아 있어도 트리가 빈 것으로 본다. | 수정 후보(`background_tasks_changed` 사용) |
| 같은 파일 75~98행 | `result.usage`를 `main-turn`으로 기록한다. 범위가 맞다. 하위 에이전트 사용량(`modelUsage` 누적의 차이, `task_notification.usage`)은 읽지 않는다. | 설계 문장 수정, 코드 유지 |
| 같은 파일 188~230행 | `init`의 `permissionMode` 변화를 `SettingsApplied`로 알린다. 턴마다 `init`이 오므로 동작한다. `system/status`의 모드 알림은 읽지 않는다. | 코드 유지 |
| 같은 파일 203~210행 | `init`의 `slash_commands`로 명령 목록을 만든다. 없는 명령은 모델에 글로 가서 한 턴을 쓴다. | 설계 문장 수정 |
| `saturn-terminal/engine/src/providers/claude.rs` 503~545행 | `interrupt`는 트리 전체에 닿고 process는 남는다. `Subagent` 대상 멈춤을 보내지 않는 508행은 맞다. | 코드 유지 |
| 같은 파일 550~560행 | `compact`를 `/compact` 글로 보낸다. `compact_boundary` 뒤 글이 빈 `result`가 온다. | 코드 유지 |
| `saturn-terminal/engine/src/secrets/hook.rs` 19행, 133~146행 | `sh -c "..."` 같은 셸 감싸기를 막지 못한다. 첫 낱말이 `sh`이고 `COMMAND_WRAPPERS`에 없다(코드 읽기로 추정, 실행으로 따로 확인하지 않음). | 결함 이슈 후보. 설계 문장에 현재 상태를 적었다. |
| `docs/design/router-key-security.md` | 훅 절의 "실험으로 확인한다"를 결과와 결함으로 바꿨다. | 반영함 |
| `docs/design/providers-and-sessions.md` 185, 301, 316, 332행과 요구사항 표 | "실측으로 확인한다" 네 문장과 표 행 네 개를 결과로 바꿨다. | 반영함 |

### 타당성 위협

| 종류 | 위협 | 이 실험에서 |
|---|---|---|
| 내적 | 모델이 지시한 하위 에이전트나 명령을 실행하지 않을 수 있다. | `unobserved`는 H23-3의 2회차뿐이었다. 나머지는 도구를 시도했다. |
| 내적 | 키 저장소 조회를 모델이 거절하면 훅 효과와 구분되지 않는다. | 훅 없는 대조 두 조건이 3/3으로 값을 읽었다. 거절은 없었다. |
| 내적 | 하위 에이전트가 훅에 막힌 뒤 우회할 수 있다. | 값은 `user`, `assistant` 줄 어디에도 없었다. |
| 내적 | 멈춤 시험에서 프로세스 표가 자손을 놓칠 수 있다. | marker와 표가 같은 방향이었다. 새 세션으로 빠져나간 자손은 표로 볼 수 없다. |
| 구성 | 메시지 id별 마지막 `usage`가 최종값이 아닐 수 있다. | 출력 토큰이 최종값과 달랐고 H19 넷이 기각된 원인이다. 입력 3칸은 일치했다. |
| 구성 | driver의 항상 허용이 Saturn 판정과 다르다. | 결론은 이벤트 모양과 훅 효과에 한정된다. |
| 구성 | 사용자 설정 위치의 훅과 사용자 skill의 영향을 잴 수 없다. | 사용자 설정을 읽지 않았다. 사용자 설정 위치의 훅은 측정 불가다. |
| 구성 | `bg_*`에서 하위 에이전트가 `sleep` 명령을 자기 백그라운드 셸 작업으로 바꿔 돌렸다. | 하위 에이전트 자체와 그 셸 작업의 정지를 따로 가르지 못했다. 둘 다 `stopped`로 보고됐다. |
| 외적 | 버전, 모델, 구독 로그인이 바뀌면 달라진다. | 결론은 2.1.288과 `haiku`로 제한한다. |

### 한계

- 조건마다 3회이므로 3/3의 95% 신뢰구간 하한은 29.2%다. 프로토콜의 결정적 동작을 보는 용도이고 비율을 정밀하게 재지 못한다.
- Codex 쪽 #3, #4, #23은 재지 않았다.
- 사용자 설정 위치(`~/.claude/settings.json`)의 훅과 `deny` 규칙은 측정 불가로 두었다. 로그인을 바꾸지 않기 위해 `CLAUDE_CONFIG_DIR`을 쓰지 않았다.
- 세션 기록: `--no-session-persistence`를 줬는데도 하위 에이전트를 띄운 33회차가 `~/.claude/projects/` 아래에 폴더를 남겼다. 폴더 이름은 `-Users-woonyong-workspace-oss-saturn-wt-experiment-claude-behavior--runtime-claude-behavior-20261004T083629Z-9fc9429-{조건}-{회차}-work`이다. 조건은 `sub-fg`, `sub-bg`, `json-sub-fg`, `bg-none`, `bg-interrupt`, `bg-close-stdin`, `bg-sigterm-leader`, `kc-sub-nohook`, `hk-sub-fg`, `hk-sub-bg`, `hk-sub-nested`이고 회차는 1~3이다. 이 실험이 지우지 않았다.
- 백그라운드 작업의 출력 파일은 Claude Code가 시스템 임시 위치에 만든다(`task_notification.output_file`). 이 실험이 만든 것이 아니고 지우지 않았다.
- 훅 시험은 이 worktree의 `target/`에 빌드한 `saturn-engine`(main `e73d46e`)을 썼다.

## 재현

```sh
./run.sh verify
./run.sh analyze
```

| 파일 | SHA-256 |
|---|---|
| `data/SHA256SUMS` | `74f3dc3d8ad1ec41803b20bf300f6e7a2eb107f0ac2440287e98c9c2c1671af0` |

## 결론

| 가설 | 판정 | 반영한 문서 |
|---|---|---|
| H17-1, H17-2, H17-3 | 채택 | [provider 연결과 session](../../design/providers-and-sessions.md) |
| H17-4 | 기각 | [provider 연결과 session](../../design/providers-and-sessions.md) |
| H18-1~H18-4 | 채택 | [provider 연결과 session](../../design/providers-and-sessions.md) |
| H19-1~H19-4 | 기각(판정 정의가 출력 토큰에 걸림, 범위는 탐색 분석) | [provider 연결과 session](../../design/providers-and-sessions.md) |
| H23-0, H23-1, H23-2 | 채택 | [router 키 보안](../../design/router-key-security.md) |
| H23-3 | 보류 | [router 키 보안](../../design/router-key-security.md) |
| H26-1, H26-2, H26-3 | 채택 | [provider 연결과 session](../../design/providers-and-sessions.md) |
| H26-4 | 기각 | [provider 연결과 session](../../design/providers-and-sessions.md) |
| H4-1a, H4-1b, H4-2, H4-3 | 채택 | 없음. 설계에 "실측으로 확인" 문장이 없고 코드 영향 표에 적었다. |
| H3-0, H3-1 | 채택 | [router 키 보안](../../design/router-key-security.md) |
| H3-2 | 기각 | [router 키 보안](../../design/router-key-security.md) |
