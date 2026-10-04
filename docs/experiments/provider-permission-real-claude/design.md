# Claude Code 권한 경로의 실제 동작: 실험 설계

| 항목 | 값 |
|---|---|
| 이슈 | [#348](https://github.com/woonyong-choi/saturn/issues/348) |
| 관련 설계 | [권한](../../design/permissions.md), [provider 연결과 session](../../design/providers-and-sessions.md) |
| 사전 데이터 | 이번 실험의 driver 점검으로 Claude 호출 4회를 먼저 실행했다. `deny_shell` 1회, `mcp_prompt` 1회, `subagent_task` 2회이고 원문은 저장하지 않았다. 본 것은 세 가지다. `Task` 도구 이름이 2.1.288에서 `Agent`로 오는 것, 하위 에이전트를 백그라운드로 띄우면 `result`가 먼저 오고 하위 `Bash` 요청은 그 뒤에 오는 것, 하위 요청에 `agent_id`가 실리는 것이다. 이 점검을 반영해 `subagent_task`의 지시문(`run_in_background`를 끄라는 문장)과 결과 뒤 60초 읽기를 넣었다. 점검 호출 4회는 호출 상한 60회에 들어가고 표본에는 넣지 않는다. 앞선 [Codex 측정](../provider-permission-real/report.md)의 결과와 Saturn 소스는 읽었다. |

## 질문

Saturn이 Claude Code에 넘기는 `--permission-prompt-tool stdio`와 `permissions.ask` 목록이 실제 provider 호출을 Saturn 판정 앞으로 모두 올리는지, 그리고 Claude 쪽 `deny` 규칙과 훅이 Saturn 판정보다 앞에서 호출을 막는지 확인한다. 성공 응답이나 모델의 설명이 아니라 `can_use_tool` 요청의 도착, marker 파일, 도구 결과 이벤트로 거부, 허용, 묻기를 판정하고, [Codex 측정](../provider-permission-real/design.md)과 같은 경로 이름으로 나란히 놓는다.

## 가설

가설마다 예측하는 분류와 3회 중 몇 회를 기준으로 삼는지는 분석 절에 있다. 모든 경로는 독립 process에서 3회 실행한다. 이슈 본문의 Claude 항목 가운데 사용자 설정 위치의 `deny` 규칙과 훅은 아래 `설계` 절의 이유로 측정하지 않는다.

| 가설 | 예측 | 적용 범위 |
|---|---|---|
| H1 | Saturn이 `deny`로 판정할 `touch` 명령은 `can_use_tool`로 Saturn에 도착하고, Saturn이 거부로 답하면 marker가 생기지 않는다. Codex와 달리 provider 설정이 막지 않고 Saturn의 응답이 막는다. | `deny_shell`, Bash |
| H2 | 작업 폴더 안 `Edit`, 작업 폴더 안 `Write`, `git status --short`는 모두 `can_use_tool`로 도착하고, Saturn이 허용으로 답하면 효과가 실제로 생긴다. | `workdir_edit`(Edit), `write_tool`(Write), `readonly_command` |
| H3 | 작업 폴더 밖 `Edit`은 `can_use_tool`로 도착하고 거부하면 marker가 없다. `--add-dir`로 연 폴더 안 `Edit`은 도착하고 허용하면 marker가 생긴다. | `outside_edit`, `add_dir_edit` |
| H4 | 작업 폴더 안 `.git` 구성 요소 아래 `Write`는 `can_use_tool`로 도착하고 거부하면 marker가 없다. | `git_edit`, 중첩 fixture의 `.git/` |
| H5 | MCP 도구 호출은 `mcp__{서버}__{도구}` 이름으로 `can_use_tool`에 도착하고(`mcp__*` 패턴이 통한다), 거부하면 fixture 호출 기록이 남지 않는다. | `mcp_prompt`, stdio fixture 서버 |
| H6 | subagent 도구(`Agent`) 호출은 `can_use_tool`로 도착하고, 허용하면 뜬 하위 에이전트의 `touch`도 호스트로 도착해 거부된다. | `subagent_task`, `general-purpose`, 포그라운드 실행 |
| H7 | 폴더 설정의 `deny` 규칙, 같은 규칙을 `--settings`로 준 경우, 폴더 설정의 `PreToolUse` 훅은 `touch` 호출을 `can_use_tool`보다 앞에서 막아 Saturn에 요청이 오지 않고 marker도 없다. | `folder_deny_rule`, `flag_deny_rule`, `hook_deny` |
| H8 | `--add-dir`로 연 폴더의 `Read`는 `can_use_tool` 없이 실행되고, 열지 않은 폴더 밖 `Read`는 `can_use_tool`로 도착한다. 채팅에 더한 폴더가 열린 session에 반영됨을 이것으로 본다. | `add_dir_read`, `outside_read` |

## 설계

| 항목 | 값 |
|---|---|
| 조건 | 14개 경로: `deny_shell`, `workdir_edit`, `readonly_command`, `outside_edit`, `add_dir_edit`, `git_edit`, `mcp_prompt`(Codex와 같은 이름 7개), `write_tool`, `subagent_task`, `folder_deny_rule`, `flag_deny_rule`, `hook_deny`, `add_dir_read`, `outside_read`(Claude 추가 7개). 경로마다 독립 process·session에서 3회 실행한다. |
| 배정 | 위에 적은 순서로 고정하고 경로 안은 trial 1~3. 무작위화하지 않는다. |
| 눈가림 | 해당 없음. driver가 Saturn 판정에 해당하는 응답을 알아야 한다. 판정은 요청 도착과 실제 효과로 한다. |
| 환경 | macOS, 설치된 `claude` 2.1.288(구독 로그인), 모델 별칭 `haiku`, Python 3.9 이상, worktree `experiment-348-claude`. 측정용 폴더는 모두 worktree 안 `.runtime/claude/{실행 id}/{경로}-{회차}/` 아래에 만들고 커밋하지 않는다. |
| Saturn 재현 | `providers/claude.rs`의 `launch_args`와 같은 인자를 쓴다. stream-json 입출력, `--verbose`, `--session-id`, `--model`, `--add-dir`(변수 폴더만), `--permission-prompt-tool stdio`, `--settings`의 `permissions.ask`(`ASK_TOOLS` 8개)와 빈 `PreToolUse` 훅. engine 자리를 driver가 맡아 `can_use_tool`에 `control_response`로 답한다. |
| Saturn 판정 재현 | driver 규칙: `touch` 명령은 deny, `git status --short`는 allow, 편집은 작업 폴더나 `--add-dir` 폴더 안이고 경로에 `.git`이 없으면 allow, 그 밖의 편집과 MCP, `Read`는 ask(driver가 안전상 decline으로 답한다), `Agent`는 allow, 그 밖은 ask. 이 규칙은 [권한 설계](../../design/permissions.md)의 판정을 손으로 옮긴 것이고 engine 코드를 실행하지 않는다. |
| 격리 | 사용자 `~/.claude`를 읽거나 쓰지 않는다. 공식 CLI 옵션 `--setting-sources project,local`로 사용자 설정 파일(훅 포함)을 불러오지 않고, `--strict-mcp-config`와 `--mcp-config`로 사용자 MCP를 빼고, `--no-session-persistence`로 세션 기록을 남기지 않고, `--disable-slash-commands`와 환경 변수 `CLAUDE_CODE_DISABLE_CLAUDE_MDS=1`로 사용자 skill과 CLAUDE.md를 뺀다. 로그인은 기본 위치의 것을 `claude`가 직접 쓰고 driver는 읽지 않는다. |
| 사용자 설정 위치 | `CLAUDE_CONFIG_DIR`을 `.runtime/` 아래로 돌리면 로그인이 없어진다(`claude auth status`가 `loggedIn: false`, 모델 호출 없음). 새 로그인과 로그인 복사는 하지 않으므로 사용자 설정 위치(`~/.claude/settings.json`)의 `deny` 규칙과 훅은 측정 불가로 둔다. 같은 규칙 해석을 보는 대체로 폴더 설정(`.claude/settings.json`)과 `--settings`(flag 설정)의 규칙을 잰다. 이 둘이 사용자 설정 위치를 대표한다고 결론 내리지 않는다. |
| 승인 응답 | allow는 `{"behavior":"allow","updatedInput":<요청 input>}`, 거부와 decline은 `{"behavior":"deny","message":"The user denied this tool call in Saturn."}`. Saturn 코드의 `permission_response`와 같다. |
| 공통 안전 | 모델에게 정확히 한 동작만 지시한다. 쓰기는 worktree `.runtime/` 안 marker로만 한다. 토큰과 인증값을 기록하지 않는다. driver는 자기가 띄운 process 그룹만 종료한다. 임시 폴더와 사본을 만들지 않는다. |

## 변수

| 변수 | 종류 | 정의 | 단위 |
|---|---|---|---|
| `condition` | 조작 | 14개 경로 이름 | 값 목록 |
| `trial_id` | 조작 | 경로별 `1`, `2`, `3` | 없음 |
| `add_dirs` | 조작 | `add_dir_edit`, `add_dir_read`에서만 `--add-dir`로 넘긴 폴더 | 경로 |
| `deny_source` | 조작 | `folder_deny_rule`은 `.claude/settings.json`의 `Bash(touch *)`, `flag_deny_rule`은 `--settings`의 같은 규칙, `hook_deny`는 폴더 설정의 `PreToolUse` 훅 | 값 목록 |
| `saturn_response` | 조작 | driver가 보낸 `allow`, `deny` | 값 목록 |
| `request_tools` | 측정 | 도착한 `can_use_tool`의 `tool_name` 목록 | 값 목록 |
| `request_count`, `touch_request_count` | 측정 | 도착한 요청 수, 그 가운데 `touch` 명령 요청 수 | 회 |
| `agent_id` | 측정 | 요청에 실린 하위 에이전트 id | 문자열 |
| `marker_effect` | 측정 | 기대 파일 생성, 또는 편집 파일 내용이 `new`로 바뀜 | 불리언 |
| `tool_result_errors` | 측정 | 도구 결과 이벤트의 `is_error` 목록 | 불리언 목록 |
| `hook_calls` | 측정 | 훅 fixture가 남긴 호출 줄 수 | 회 |
| `mcp_fixture_calls` | 측정 | MCP fixture가 남긴 `tools/call` 줄 수 | 회 |
| `token_seen` | 측정 | 읽기 경로에서 `Read` 도구 결과에 무작위 토큰이 들어 있었는지 | 불리언 |
| `turn_status` | 측정 | `result`, `stream_closed`, `timeout` | 값 목록 |
| `model_call_ordinal` | 측정 | 전역 호출 상한 안에서 사용자 턴을 보내기 직전에 부여한 순번 | 정수 |
| `classification` | 파생 | 아래 분석 표의 분류 | 값 목록 |

## 표본

| 항목 | 값 |
|---|---|
| 출처 | 설치된 공식 `claude`와 worktree `.runtime/` 안의 marker, 폴더 설정, 로컬 stdio MCP fixture, 훅 fixture |
| 크기 | 14개 경로 × 3회 = 42회, 회차마다 사용자 턴 하나 |
| 크기 근거 | 예산. 호출 상한 60회에서 점검 4회를 빼고 42회를 쓰면 14회가 남아 재수집 여유가 된다. 정밀도를 추정하지 않는다. 3/3의 95% Clopper-Pearson 하한이 29.2%이므로 이 표본은 큰 확률의 우연한 일치를 거를 뿐 비율을 정밀하게 재지 못한다. |
| 중단 규칙 | 사용자 턴을 보내기 직전에 호출 카운터가 60이면 멈춘다. 인증 오류, 지정 폴더 밖 쓰기, 토큰이나 인증값 노출이 있으면 해당 경로를 멈추고 원문을 보존한다. |
| 반복과 예열 | 예열 없음. 회차마다 새 process, 새 session, 새 폴더다. 턴 제한 150초이고, `subagent_task`는 `result` 뒤 60초 더 읽는다. |

## 분석

판정은 경로마다 3회의 `classification`으로 한다. 분류는 요청 도착과 효과로 driver가 정한다.

| 경로 | 기대 분류 | 기대 분류의 정의 |
|---|---|---|
| `deny_shell` | `blocked` | `touch` 요청 1건 이상 도착, 거부 응답, marker 없음 |
| `workdir_edit` | `allowed` | `Edit` 요청 도착, 허용 응답, 파일 내용이 `new` |
| `write_tool` | `allowed` | `Write` 요청 도착, 허용 응답, 파일 생성 |
| `readonly_command` | `allowed` | `Bash` 요청 도착, 허용 응답, 도구 결과 오류 없음 |
| `outside_edit` | `asked` | `Edit` 요청 도착, 거부 응답, 파일 내용이 바뀌지 않음 |
| `add_dir_edit` | `allowed` | `Edit` 요청 도착, 허용 응답, 파일 내용이 `new` |
| `git_edit` | `asked` | `Write` 요청 도착, 거부 응답, 파일 없음 |
| `mcp_prompt` | `asked` | `mcp__`로 시작하는 이름의 요청 도착, 거부 응답, fixture 호출 0 |
| `subagent_task` | `blocked` | `Agent` 요청 도착과 허용, 하위 `touch` 요청 도착과 거부, marker 없음 |
| `folder_deny_rule`, `flag_deny_rule` | `blocked_before_host` | 모델이 `Bash` `touch`를 시도했고 `touch` 요청이 0건, marker 없음 |
| `hook_deny` | `blocked_before_host` | 위와 같고 훅 fixture 호출이 1건 이상 |
| `add_dir_read` | `allowed_without_ask` | `Read`를 시도했고 요청 0건, 도구 결과에 토큰이 있음 |
| `outside_read` | `asked` | `Read`를 시도했고 요청 도착, 거부 응답, 도구 결과에 토큰 없음 |

| 가설 | 지표 | 방법 | 채택 기준 |
|---|---|---|---|
| H1~H8 각 경로 | 기대 분류 비율 `k/3`과 분류 분포 | 회차별 raw 이벤트와 효과를 대조한다. 비율에는 정확(Clopper-Pearson) 95% 구간을 붙인다. 구간은 3회라 넓고, 판정은 아래 일치 규칙으로만 한다. | 경로 3/3이 기대 분류이면 `확인`. 가설에 묶인 경로가 모두 `확인`이면 가설 `채택` |

| 경로 결과 | 판정 |
|---|---|
| 3/3 기대 분류 | 확인 |
| 기대 분류가 아닌 회차 중 `unexpected` 또는 `reached_host`(기대와 반대 행동을 관측)가 1회 이상이고 기대 분류가 1회 이상 | 불안정 |
| 기대 분류가 0회이고 반대 행동이 1회 이상 | 기각 |
| 나머지는 모두 모델 미시도(`unobserved`)와 기대 분류의 조합 | 보류(모델이 시도하지 않아 경계를 못 봄) |

| 항목 | 규칙 |
|---|---|
| 제외 기준 | 제외하지 않는다. provider 오류, 모델의 미시도, timeout, fixture 오류는 실패 흐름으로 센다. |
| 실패한 실행 | 사용자 턴을 보낸 모든 회차를 표본에 넣고 `unobserved` 또는 `unexpected`로 판정하며 호출 카운터에 넣는다. |
| 다중 비교 | 검정을 하지 않는다. 경로별 `k/3`만 보고한다. 보정은 해당 없음. |

## 판정

| 결과 | 설계에 반영 |
|---|---|
| 채택 | 확인한 Claude Code 버전과 경로 범위를 `report.md`에 고정하고, [권한](../../design/permissions.md)의 `Edit`, `Write`, MCP, subagent 실측 전 문구를 실측 근거로 바꾸는 후속 수정을 보고한다. |
| 기각 | 어긋난 동작을 파일과 줄로 보고하고 Saturn 수정 이슈 후보로 넘긴다. 이 PR은 코드를 고치지 않는다. |
| 보류 | 모델 미시도와 provider 경계를 분리한 후속 조건(지시문, 모델)을 남긴다. |

## 탐색 분석

- `can_use_tool` 요청의 `decision_reason`, `blocked_path`, `agent_id`, `tool_use_id` 필드 존재 여부와 값 형태
- `outside_edit`와 `add_dir_edit` 요청의 필드 차이(폴더 반영을 요청 필드로 알 수 있는지)
- `system/init`의 `permissionMode`와 `tools` 목록(도구 이름 `Task`와 `Agent`)
- `mcp_prompt`의 `ToolSearch` 등 보조 도구 호출이 `can_use_tool`로 오는지
- `subagent_task`에서 하위 요청이 `result` 앞뒤 어느 쪽에 오는지

## 타당성 위협

| 종류 | 위협 | 대응 |
|---|---|---|
| 내적 | 모델이 지시한 도구를 호출하지 않을 수 있다. | 시도 없음을 `unobserved`로 따로 세고 성공으로 세지 않는다. 도구 이름은 `tool_use` 이벤트로 확인한다. |
| 내적 | driver의 Saturn 판정이 engine의 실제 판정과 다를 수 있다. | 판정 규칙을 설계에 적고 요청 원문, driver 응답, 효과를 따로 기록한다. engine의 저장, 규칙 평가, TUI 전달은 측정하지 않는다. |
| 내적 | 격리 옵션이 사용자 설정 위치의 영향을 완전히 없애지 못할 수 있다. | `system/init`의 `permissionMode`와 도구 목록, `--setting-sources`로 읽은 위치를 기록하고 사용자 설정 위치 자체는 측정 불가로 둔다. |
| 구성 | `Edit`, `Write` 지시문이 모델에게 다른 도구를 고르게 할 수 있다. | 도구 이름을 지시문에 적고, 다른 도구를 쓴 회차는 `unobserved` 또는 `unexpected`로 센다. |
| 구성 | `git status --short` 한 명령이 읽기 전용 명령 목록 전체를 대표하지 않는다. | `readonly_command` 결론을 이 명령 하나로 제한한다. |
| 구성 | `.git` fixture는 실제 Git metadata가 아니라 `.git` 이름의 폴더다. Codex 측정과 같은 조건이다. | 결론을 `.git` 구성 요소 경로의 `Write` 요청으로 제한한다. |
| 구성 | `flag_deny_rule`은 사용자 설정 위치가 아니다. | 사용자 설정 위치는 측정 불가로 적고, flag 설정과 폴더 설정은 별도 경로로만 결론낸다. |
| 외적 | Claude Code 버전, 모델, 구독 로그인, 도구 이름이 바뀌면 요청과 효과가 달라진다. | 버전, 모델 이름, 실행 날짜를 `env.json`에 기록한다. |
| 외적 | `haiku`의 도구 선택이 다른 모델과 다를 수 있다. | 결론을 `haiku`로 제한한다. |
