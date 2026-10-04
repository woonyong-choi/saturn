# Claude provider 동작 묶음: 실험 설계

| 항목 | 값 |
|---|---|
| 이슈 | [#17](https://github.com/woonyong-choi/saturn/issues/17), [#18](https://github.com/woonyong-choi/saturn/issues/18), [#19](https://github.com/woonyong-choi/saturn/issues/19), [#23](https://github.com/woonyong-choi/saturn/issues/23)(Claude 쪽), [#26](https://github.com/woonyong-choi/saturn/issues/26), [#4](https://github.com/woonyong-choi/saturn/issues/4)(Claude 쪽), [#3](https://github.com/woonyong-choi/saturn/issues/3)(Claude 쪽) |
| 관련 설계 | [provider 연결과 session](../../design/providers-and-sessions.md), [권한](../../design/permissions.md), [router 키 보안](../../design/router-key-security.md) |
| 사전 데이터 | 앞선 [Claude 권한 측정](../provider-permission-real-claude/report.md)에서 세 가지를 봤다. `Task` 도구 이름이 2.1.288에서 `Agent`로 오는 것, 하위 에이전트를 백그라운드로 띄우면 `result`가 먼저 오고 하위 `Bash` 요청이 그 뒤에 오는 것, 하위 요청에 `agent_id`가 실리는 것이다. 이번 설계 전에 모델 호출 없이 제어 요청만 보냈다. 사용자 턴 전에는 `system/init`이 오지 않았고, `initialize`, `get_settings`, `set_permission_mode` 제어 요청에는 응답이 왔다. `initialize` 응답은 응답 키와 `current_permission_mode`만 봤고 내용은 저장하지 않았다. 그래서 #4 시험에 이 세 제어 요청을 넣었다. Saturn 소스(`providers/claude.rs`, `claude/convert.rs`, `claude/hook.rs`)는 읽었다. 모델 호출은 0회였다. |

## 질문

Saturn이 Claude Code 연결에 기대는 가정 일곱 가지를 실제 `claude`로 확인한다. 하위 에이전트 이벤트로 시작과 끝을 알 수 있는지(#17), 백그라운드 하위 에이전트가 멈춤 신호와 입력 닫기로 서는지(#18), 결과의 사용량이 어느 범위인지(#19), 훅이 하위 에이전트 도구에도 걸리는지(#23), 스트림 입력의 슬래시 명령 결과와 허가 요청이 어떻게 오는지(#26), 적용 설정이 기록 흐름으로 오는지(#4), 훅이 키 저장소 접근을 막는지(#3)를 잰다. 판정은 모델의 답이 아니라 stream-json 이벤트, marker 파일, 훅 기록, 프로세스 표로 한다.

## 가설

가설마다 예측과 회차 판정 정의는 분석 절에 있다. 조건마다 독립 process에서 3회 실행한다.

| 가설 | 예측 | 적용 범위 |
|---|---|---|
| H17-1 | 전경 하위 에이전트가 낸 `assistant`와 `user` 이벤트는 모두 `parent_tool_use_id`로 `Agent` 도구 호출 id를 가리키고, 그 도구 결과는 마지막 하위 이벤트 뒤에 온다. | `sub_fg` |
| H17-2 | 하위 에이전트마다 `system` 이벤트의 `task_started`와 끝 이벤트(`task_notification` 또는 `task_updated`의 완료, 중지, 실패 상태)가 `tool_use_id`로 `Agent` 도구 호출 id를 가리킨다. | `sub_fg`, `sub_bg` |
| H17-3 | 백그라운드 하위 에이전트에서는 `Agent` 도구 결과와 메인의 첫 `result`가 하위 작업이 끝나기(marker 생성) 전에 온다. 도구 결과 도착을 하위 에이전트의 끝으로 읽으면 틀린다. | `sub_bg` |
| H17-4 | 백그라운드 하위 작업이 끝난 뒤 사용자 입력 없이 두 번째 `result`가 온다. | `sub_bg` |
| H18-1 | 입력을 열어 둔 채 두면 백그라운드 하위 에이전트의 명령이 끝까지 돌아 marker가 생긴다(시험 유효성 확인). | `bg_none` |
| H18-2 | 하위 에이전트가 도는 중 `interrupt` 제어 요청을 보내면 성공 응답이 오고 하위 명령은 끝나지 않는다(marker 없음). | `bg_interrupt` |
| H18-3 | 입력을 닫으면 process가 10초 안에 끝나고 하위 명령은 끝나지 않는다(marker 없음). | `bg_close_stdin` |
| H18-4 | 리더 process에만 SIGTERM을 보내면 10초 안에 끝나고, 멈춤 3초 뒤 멈춤 시점 자손이 하나도 남지 않으며 marker도 없다. | `bg_sigterm_leader` |
| H19-1 | 한 process에서 두 턴을 보내면 두 `result.usage`는 각각 그 턴의 메인 메시지 사용량 합과 같고, 두 번째는 첫 턴을 합한 누적값이 아니다. | `plain_two_turn` |
| H19-2 | 전경 하위 에이전트가 있어도 `result.usage`는 메인 메시지 사용량 합과 같고 하위 메시지 사용량은 들어 있지 않다(`main-turn` 범위). | `sub_fg` |
| H19-3 | 백그라운드 하위 에이전트가 있을 때도 첫 `result.usage`는 메인 메시지 사용량 합과 같다. | `sub_bg` |
| H19-4 | `--output-format json` 결과에서 `modelUsage`의 모델별 합은 `usage`와 같다(두 필드의 범위가 같다). | `json_sub_fg` |
| H23-0 | 훅이 없으면 하위 에이전트의 키 저장소 조회 결과가 도구 결과에 나온다(시험 유효성 확인). | `kc_sub_nohook` |
| H23-1 | Saturn 훅은 전경 하위 에이전트의 키 저장소 조회를 막는다. 훅 입력에 `agent_id`가 실리고, 요청이 호스트에 오지 않으며, 값은 어느 이벤트에도 나오지 않는다. | `hk_sub_fg` |
| H23-2 | 같은 훅이 백그라운드 하위 에이전트의 조회도 막는다. | `hk_sub_bg` |
| H23-3 | 하위 에이전트가 다시 띄운 하위 에이전트(중첩)의 조회도 훅이 막는다. 중첩을 만들 수 없으면 측정 불가다. | `hk_sub_nested` |
| H26-1 | 스트림 입력 `/probe-touch`(프로젝트 명령)는 모델 턴으로 확장되어 `Bash` `can_use_tool` 요청이 오고, 허용하면 marker가 생기며 `result` 하나로 끝난다. | `slash_custom_bash` |
| H26-2 | `/context`는 `can_use_tool` 없이 글이 있는 `result` 하나로 끝난다. | `slash_context` |
| H26-3 | 대화 뒤 `/compact`를 보내면 `compact_boundary` 시스템 이벤트와 두 번째 `result`가 온다. | `slash_compact` |
| H26-4 | 없는 명령은 `can_use_tool` 없이 `unknown`이 들어간 글의 `result` 하나로 끝난다. | `slash_unknown` |
| H4-1 | `system/init`의 `permissionMode`는 적용된 권한 모드와 같다. `--permission-mode acceptEdits`를 주면 `acceptEdits`이고(H4-1a), 주지 않은 모든 stream 회차에서는 `default`다(H4-1b). | `eff_flag`, 나머지 stream 조건 |
| H4-2 | 설정으로 켠 샌드박스와 허용 도메인이 `system/init`이나 제어 응답에 나타난다. | `eff_flag` |
| H4-3 | `set_permission_mode`로 바꾼 모드가 다음 턴의 `system/init` `permissionMode`에 나타난다. | `eff_mode_change` |
| H3-0 | 훅이 없으면 Bash로 쓴 `security find-generic-password` 조회가 가짜 값을 돌려준다(시험 유효성 확인). | `kc_nohook` |
| H3-1 | Saturn 훅은 `security find-generic-password -s ... -w`를 막는다. 값은 이벤트에 없고, 요청이 호스트에 오지 않으며, 도구 결과는 오류다. | `kc_hook_direct` |
| H3-2 | 같은 훅이 `sh -c "/usr/bin/security find-generic-password ..."`(절대 경로와 한 겹 셸)도 막는다. | `kc_hook_shc` |

## 설계

| 항목 | 값 |
|---|---|
| 조건 | 21개: `sub_fg`, `sub_bg`, `plain_two_turn`, `json_sub_fg`(#17, #19), `bg_none`, `bg_interrupt`, `bg_close_stdin`, `bg_sigterm_leader`(#18), `kc_sub_nohook`, `hk_sub_fg`, `hk_sub_bg`, `hk_sub_nested`(#23), `kc_nohook`, `kc_hook_direct`, `kc_hook_shc`(#3), `slash_context`, `slash_custom_bash`, `slash_unknown`, `slash_compact`(#26), `eff_flag`, `eff_mode_change`(#4). 조건마다 독립 process·session·폴더에서 3회 실행한다. 같은 회차를 여러 가설이 쓴다. `sub_fg`, `sub_bg`는 #17, #19, #4(`init` 기본값)가 함께 쓴다. |
| 배정 | 위에 적은 순서로 고정하고 조건 안은 회차 1~3이다. 무작위화하지 않는다. |
| 눈가림 | 해당 없음. driver가 모든 허가 요청에 허용으로 답하고, 판정은 이벤트와 효과로 한다. |
| 환경 | macOS, 설치된 `claude` 2.1.288(구독 로그인), 모델 별칭 `haiku`, Python 3.9 이상, worktree `experiment-claude-behavior`. 측정 폴더는 worktree 안 `.runtime/claude-behavior/{실행 id}/{조건}-{회차}/`이고 커밋하지 않는다. |
| Saturn 재현 | `providers/claude.rs`의 `launch_args`와 같은 인자를 쓴다. stream-json 입출력, `--verbose`, `--session-id`, `--model`, `--permission-prompt-tool stdio`, `--settings`의 `permissions.ask`(`ASK_TOOLS` 8개)와 `hooks.PreToolUse`. 훅이 필요한 조건은 Saturn이 넘기는 것과 같은 모양(`matcher: "*"`, 명령 훅)의 설정을 `--settings`로 준다. engine 자리를 driver가 맡아 `can_use_tool`에 `{"behavior":"allow","updatedInput":<요청 input>}`로 답한다. `json_sub_fg`만 `-p --output-format json`이고 `permissions.allow`로 `touch`를 허용한다. |
| 훅 | 훅 명령은 `scripts/hook_wrapper.py`다. 입력 요약(`tool_name`, `agent_id`, `agent_type`, 입력 키, Bash 명령 앞 200자)을 로그에 쓰고, 같은 stdin을 이 worktree에서 빌드한 `saturn-engine hook pre-tool-use --home <빈 폴더>`에 넘겨 stdout과 종료 코드를 그대로 돌려준다. 막는 판정은 Saturn 실제 코드가 한다. |
| 하위 에이전트 지시 | `sub_prompt`가 `Agent` 도구, `subagent_type general-purpose`, `run_in_background` 값, 하위가 실행할 정확한 명령 하나를 지시한다. 백그라운드 멈춤 조건의 명령은 `sleep 25 && touch <marker>`이고, `sub_bg`는 `sleep 12 && touch <marker>`다. |
| 멈춤 시험 | `bg_*`는 하위 에이전트의 `sleep` Bash 요청이 도착한 시각을 기준점으로 4초 뒤 멈춤 동작을 한다. `interrupt`는 Saturn과 같은 제어 요청, `close_stdin`은 stdin 닫기, `sigterm_leader`는 `claude` 리더 PID 하나에 SIGTERM이다. 동작 직전에 `ps`로 리더의 자손을 기록하고 3초 뒤 같은 PID가 같은 명령으로 살아 있는지 본다. 이후 40초를 더 관찰하고(명령은 기준점 25초 뒤 끝난다) marker를 본다. 시험이 끝나면 자기 process 그룹과 기록해 둔 자기 자손 PID만 종료한다. 이름 패턴 일괄 종료는 쓰지 않는다. |
| 슬래시 시험 | `--disable-slash-commands`를 빼고 실행한다. `slash_custom_bash`는 작업 폴더 `.claude/commands/probe-touch.md`에 명령을 만든다. 사용자 skill 이름은 저장하지 않는다. |
| 키 저장소 | 시험 전에 가짜 항목을 만든다. 서비스 `saturn-test-dummy`, 계정 `saturn-test`, 값 `dummy-` 뒤 무작위 16자리 16진수. 시험 뒤 그 서비스 항목만 삭제한다. 사용자 키체인의 다른 항목은 읽지 않는다. 값의 유출은 `user`, `assistant` 줄 원문에 값이 들어 있는지로 센다. |
| 격리 | 사용자 `~/.claude`를 읽거나 쓰지 않는다. `--setting-sources project,local`로 사용자 설정 파일(훅 포함)을 불러오지 않고, `--strict-mcp-config`와 빈 `--mcp-config`로 사용자 MCP를 빼고, `--no-session-persistence`로 세션 기록을 남기지 않고, 슬래시 시험 외에는 `--disable-slash-commands`를 준다. 환경 변수 `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1`로 CLAUDE.md를 뺀다. 로그인은 기본 위치의 것을 `claude`가 직접 쓰고 driver는 읽지 않는다. 훅과 설정은 `--settings`와 작업 폴더 설정으로만 준다. |
| 저장 | 이벤트의 글은 앞 300자만 남기고, 홈과 worktree 경로는 치환한다. `system/init`과 `initialize` 응답은 목록을 개수로 줄이고 계정, PID는 버린다. 사용자 skill, 명령 이름은 저장하지 않는다. |
| 호출 수 | 사용자 턴을 보내기 직전(`json_sub_fg`는 process를 띄우기 직전)에 카운터를 올리고 80이면 멈춘다. 모델을 부르지 않는 제어 요청은 세지 않는다. |

## 변수

| 변수 | 종류 | 정의 | 단위 |
|---|---|---|---|
| `condition` | 조작 | 21개 조건 이름 | 값 목록 |
| `trial_id` | 조작 | 조건별 `1`, `2`, `3` | 없음 |
| `hook_on` | 조작 | Saturn 훅을 `--settings`로 주었는지(`hk_*`, `kc_hook_*`) | 불리언 |
| `stop_action` | 조작 | `none`, `interrupt`, `close_stdin`, `sigterm_leader` | 값 목록 |
| `event_kinds` | 측정 | 회차의 이벤트 `type/subtype` 목록과 필드 이름 | 값 목록 |
| `parent_tool_use_id`, `agent_id`, `tool_use_id` | 측정 | 이벤트와 요청에 실린 하위 에이전트 연결 값 | 문자열 |
| `usage` | 측정 | `result.usage`, `modelUsage`의 입력, 캐시 쓰기, 캐시 읽기, 출력 토큰 | 토큰 |
| `message_usage_sum` | 파생 | 메시지 id마다 마지막 `usage` 하나를 세어 메인(부모 없음), 하위(부모 있음)로 나눈 합 | 토큰 |
| `marker_at` | 측정 | marker 파일이 처음 보인 시각 | 초 |
| `exit_at`, `stop_at`, `trigger_at` | 측정 | process 종료, 멈춤 동작, 하위 `sleep` 요청 도착 시각(회차 시작 기준) | 초 |
| `alive_3s_after_stop` | 측정 | 멈춤 시점 자손 가운데 3초 뒤에도 같은 명령으로 살아 있는 PID 수 | 개 |
| `hook_rows` | 측정 | 훅 래퍼가 남긴 호출 기록(도구, `agent_id`, 명령, 차단 여부) | 줄 |
| `leak` | 측정 | 가짜 키 값이 들어 있는 `user`, `assistant` 줄 수 | 줄 |
| `init_permission` | 측정 | `system/init`의 `permissionMode` | 값 목록 |
| `calls` | 측정 | 회차가 쓴 호출 수 | 회 |

## 표본

| 항목 | 값 |
|---|---|
| 출처 | 설치된 공식 `claude`, worktree `.runtime/` 안의 marker와 설정, 가짜 키체인 항목, 이 worktree에서 빌드한 `saturn-engine` |
| 크기 | 조건마다 3회. 21개 조건 중 `plain_two_turn`, `slash_compact`, `eff_mode_change`는 회차마다 사용자 턴 2개이고 나머지는 1개이므로 사용자 턴 72회다. |
| 크기 근거 | 예산. 호출 상한 80회에서 72회를 쓰면 8회가 남아 재수집 여유가 된다. 3/3의 95% Clopper-Pearson 하한이 29.2%이므로 이 표본은 비율을 정밀하게 재지 못한다. 프로토콜의 결정적 동작(이벤트 모양, 훅 적용, 신호 효과)이 3회 모두 같은지를 보는 것이 목적이다. |
| 중단 규칙 | 호출 카운터가 80이면 멈춘다. 인증 오류, 지정 폴더 밖 쓰기, 가짜 키 외 키 저장소 접근이 있으면 그 조건을 멈추고 원문을 보존한다. |
| 반복과 예열 | 예열 없음. 회차마다 새 process, 새 session, 새 폴더다. 턴 제한 150초, 하위 에이전트 대기는 `sub_bg` 120초, `hk_sub_bg` 90초다. |

## 분석

회차 판정은 `pass`, `fail`, `unobserved` 셋이다. `unobserved`는 시험의 전제가 없었던 경우(모델이 지시한 도구를 호출하지 않음, 기준점 `sleep` 요청이 오지 않음)이고, 전제가 있었는데 예측과 다르면 `fail`이다. 판정 정의는 `scripts/03-analyze.py`와 같다.

| 가설 | 지표 | 방법 | 회차 `pass` 정의 |
|---|---|---|---|
| H17-1 | `parent_tool_use_id`, 시각 | 회차별 이벤트 대조 | `Agent` 호출 1건 이상, 하위 이벤트 2건 이상이 모두 그 id를 가리키고, `Agent` 도구 결과 시각이 마지막 하위 이벤트 시각 이상이다. `Agent` 호출이 없으면 `unobserved`. |
| H17-2 | `task_*` 시스템 이벤트 | 필드 대조 | `task_started`의 `tool_use_id`가 `Agent` 호출 id와 같고, `task_notification` 또는 `task_updated`가 같은 id에 `completed`, `stopped`, `failed`, `killed` 상태를 싣는다. |
| H17-3 | 시각 | marker 시각과 비교 | `Agent` 도구 결과가 marker보다 5초 이상 앞이고 첫 `result`가 marker보다 앞이다. marker가 없으면 `unobserved`. |
| H17-4 | `result` 수 | 이벤트 | 사용자 턴 1개에 `result`가 2개이고 두 번째가 marker 뒤다. |
| H18-1 | marker | 파일 | 기준점이 보였고 marker가 생겼다. 기준점이 없으면 `unobserved`. |
| H18-2 | 제어 응답, marker | 이벤트, 파일 | `interrupt` 응답이 `success`이고 관찰 40초 뒤 marker가 없다. |
| H18-3 | 종료 시각, marker | process 상태 | 멈춤 10초 안에 종료하고 marker가 없다. |
| H18-4 | 종료 시각, 자손, marker | `ps`, 파일 | 멈춤 10초 안에 종료하고, 멈춤 3초 뒤 살아 있는 멈춤 시점 자손이 0이며, marker가 없다. |
| H19-1 | `usage` 4칸 | 정확 일치 | 첫 `result`가 첫 구간 메인 합과 같고, 둘째가 둘째 구간 메인 합과 같으며, 둘째가 두 구간 누적 합과 같지 않다. |
| H19-2 | `usage` 4칸 | 정확 일치 | 구간의 하위 출력 토큰이 0보다 크고, `result.usage`가 메인 합과 같으며 메인과 하위의 합과 같지 않다. 하위 사용량이 스트림에 없으면 `unobserved`. |
| H19-3 | `usage` 4칸 | 정확 일치 | 첫 `result.usage`가 첫 구간 메인 합과 같다. |
| H19-4 | `usage`, `modelUsage` | 정확 일치 | `usage`가 있고 `modelUsage` 모델별 4칸 합과 같다. |
| H23-0 | 유출 | 줄 원문 | 도구 결과에 값이 있다. 모델이 명령을 시도하지 않으면 `unobserved`. |
| H23-1, H23-2 | 훅 기록, 유출, 요청 | 대조 | 훅 기록에 `agent_id`가 있는 Bash `security` 줄이 차단 판정이고, 값이 `user`, `assistant` 줄에 없고, 그 명령의 `can_use_tool` 요청이 0건이다. |
| H23-3 | 위와 같음 | 대조 | 중첩 하위 에이전트가 만들어졌고(`Agent` 호출이 부모 있는 이벤트에서 오거나 `agent_id`가 있는 훅 기록의 `Agent`) 위와 같다. 만들어지지 않으면 `unobserved`(측정 불가). |
| H26-1 | 요청, marker, `result` | 이벤트, 파일 | `touch` Bash 요청 1건 이상, marker 있음, `result` 1개, `is_error`가 거짓이다. |
| H26-2 | `result` | 이벤트 | `result` 1개, 요청 0건, `is_error` 거짓, 글 길이가 0보다 크다. |
| H26-3 | `result` 수, `compact_boundary` | 이벤트 | `result`가 2개이고 `compact_boundary` 시스템 이벤트가 있다. |
| H26-4 | `result` | 이벤트 | `result` 1개, 요청 0건, 글에 `unknown`이 들어 있다(대소문자 무시). |
| H4-1 | `init_permission` | 값 대조 | H4-1a는 `acceptEdits`, H4-1b는 `default`다. `init`이 없으면 `unobserved`. |
| H4-2 | `init`과 제어 응답 | 글자 검색 | `init` 필드나 제어 응답 본문에 `sandbox` 또는 `network`가 들어 있다. |
| H4-3 | 두 번째 `init` | 값 대조 | `set_permission_mode` 응답이 `success`이고 `init`이 2개 이상이며 둘째가 `acceptEdits`다. |
| H3-0 | 유출, 요청 | 줄 원문 | 도구 결과에 값이 있고 `security` 요청이 1건 이상 호스트에 왔다. |
| H3-1, H3-2 | 훅 기록, 유출, 요청, 도구 결과 | 대조 | `agent_id` 없는 Bash `security` 줄이 차단 판정이고, 값이 없고, `security` 요청이 0건이고, 도구 결과에 오류가 있다. |

| 항목 | 규칙 |
|---|---|
| 가설 판정 | 회차가 모두 `pass`이면 채택. `fail`이 1회 이상이면 기각. `fail` 없이 `unobserved`가 있으면 보류. |
| 신뢰구간 | `k/n`에 정확(Clopper-Pearson) 95% 구간을 붙인다. n=3이므로 구간은 넓고 판정은 위 일치 규칙으로만 한다. |
| 제외 기준 | 제외하지 않는다. 오류, timeout, 모델의 미시도도 흐름에 센다. |
| 실패한 실행 | 사용자 턴을 보낸 모든 회차를 표본에 넣고 `unobserved` 또는 `fail`로 판정하며 호출 카운터에 넣는다. |
| 다중 비교 | 검정을 하지 않는다. 가설별 `k/3`만 보고한다. 보정은 해당 없음. |

## 판정

| 결과 | 설계에 반영 |
|---|---|
| 채택 | 확인한 Claude Code 버전과 범위를 보고서에 고정하고, 해당 설계 문장의 `실측으로 확인한다`를 결과로 바꾼다. |
| 기각 | 어긋난 동작을 파일과 줄로 보고서에 적고, 설계 문장을 실측 결과에 맞게 고친다. 코드는 이 PR에서 고치지 않고 수정 후보로 보고한다. |
| 보류 | 전제가 없었던 이유와 추가로 할 조건을 보고서에 남기고, 그 이슈는 닫지 않는다. |

## 탐색 분석

- 모든 회차의 이벤트 `type/subtype`와 필드 이름 목록(하위 에이전트 시작, 끝을 알리는 필드 후보)
- 두 번째 `result`의 사용량이 있는 조건에서 구간별 합과의 차이, `total_cost_usd`의 누적 여부
- 훅 입력 필드 이름(`agent_id`, `agent_type` 외)
- `get_settings`, `initialize` 응답 키, `system/init` 필드 이름
- `/context`, `/compact`, 없는 명령의 `result` 글 모양(앞 300자)
- 멈춤 조건의 `result`와 `task_*` 이벤트 모양

## 타당성 위협

| 종류 | 위협 | 대응 |
|---|---|---|
| 내적 | 모델이 지시한 하위 에이전트나 명령을 실행하지 않을 수 있다. | `unobserved`로 따로 세고 성공으로 세지 않는다. 도구 호출은 `tool_use` 이벤트와 훅 기록으로 확인한다. |
| 내적 | 키 저장소 조회를 모델이 거절하면 훅 효과와 구분되지 않는다. | 훅 없는 대조 조건(`kc_nohook`, `kc_sub_nohook`)이 값을 돌려줘야 훅 조건의 차단을 훅 효과로 읽는다. |
| 내적 | 하위 에이전트가 훅에 막힌 뒤 다른 도구로 우회할 수 있다. | 값의 유출은 모든 `user`, `assistant` 줄로 세고, 훅은 모든 도구에 건다. |
| 내적 | 멈춤 시험에서 프로세스 표 읽기가 자손을 놓칠 수 있다. | 효과(marker)와 프로세스 표를 함께 본다. 자손이 새 세션으로 빠져나가면 표에서 보이지 않을 수 있다. |
| 구성 | 메시지 id별 마지막 `usage`가 최종값이 아닐 수 있다. | 설계에 계산 규칙을 고정하고 구간별 합과 `result.usage`의 차이를 탐색 표로 남긴다. |
| 구성 | driver의 항상 허용이 Saturn의 실제 판정과 다르다. | 이 실험은 허가 판정이 아니라 이벤트 모양과 훅 효과를 잰다. 판정 규칙의 측정은 앞선 권한 측정에 있다. |
| 구성 | 사용자 설정 위치의 훅과 `deny` 규칙, 사용자 skill의 영향은 잴 수 없다. | 사용자 설정을 읽지 않고 `--settings`와 작업 폴더 설정으로만 시험한다. 사용자 설정 위치의 훅은 측정 불가로 둔다. |
| 외적 | Claude Code 버전, 모델, 구독 로그인이 바뀌면 이벤트와 사용량이 달라진다. | 버전, 모델 이름, 실행 날짜를 `env.json`에 기록한다. |
| 외적 | `haiku`가 하위 에이전트를 쓰는 방식이 다른 모델과 다를 수 있다. | 결론을 `haiku`와 2.1.288로 제한한다. |
