# provider 권한 정본: 실험 설계

> [!NOTE]
> 사후 기록이다. 이 실험 시리즈는 사전 등록 없이 이슈 [#194](https://github.com/woonyong-choi/saturn/issues/194)에서 진행했고, 실험 1~8이 끝난 뒤 이슈 댓글과 보고서를 근거로 이 문서를 썼다. 가설, 방법, 판정 기준은 실험 당시 지시서와 댓글에 있던 것만 옮겼다. 사전 기록이 없는 칸은 `기록 없음`으로 적었다.

| 항목 | 값 |
|---|---|
| 이슈 | [#194](https://github.com/woonyong-choi/saturn/issues/194) |
| 관련 설계 | [provider 연결과 session](../../design/providers-and-sessions.md), [결정 기록](../../decisions/2026-09-29-minimal-provider-control.md) |
| 사전 데이터 | 없음. 앞 실험의 결과를 읽고 다음 실험을 설계하는 순서로 진행했다. 각 실험의 설계 입력은 아래 실험별 방법 표에 적었다. |

## 질문

Saturn이 Codex와 Claude Code CLI를 감싸면서 명령, 파일 편집, MCP 도구 실행의 허용, 묻기, 거부를 Saturn 설정 하나로 정할 수 있는지 잰다. provider가 자기 허용 목록으로 묻지 않고 실행하는 경로가 남아 있으면 Saturn 규칙이 무력해지므로, 그런 경로를 실제 CLI 실행으로 찾는다.

## 배경

OpenCode는 자체 `shell` 도구가 명령을 실행하고 매번 `ctx.ask`로 권한을 확인한다. Codex CLI와 app-server를 띄우지 않고 모델 API를 직접 호출한다(실험 2에서 소스로 확인). 실행자가 OpenCode 자신이므로 권한 정본이 OpenCode 설정 하나다.

Saturn은 Codex와 Claude Code CLI를 감싼다. 실행자는 provider이고 승인 흐름도 provider에 있다. provider 허용 규칙에 걸린 명령은 Saturn을 거치지 않고 실행될 수 있다.

[provider 설정과 subagent 사용은 사용자 설정을 따르고 Saturn은 추적만 한다](../../decisions/2026-09-29-minimal-provider-control.md)(2026-09-29 채택)는 Saturn이 provider의 권한 설정을 바꾸지 않기로 정했다. 이 실험은 그 반대 방향, 곧 Saturn 설정을 정본으로 두는 일이 기술적으로 가능한지만 잰다. 결정 기록은 이 실험으로 바꾸지 않았다.

## 가설

실험 시작 때 적어 둔 가설은 없다. 각 실험의 확인할 것을 가설 형태로 다시 적었다. 예측 칸은 실험이 확인하려던 명제이고, 실험 전에 정한 기대 방향이 아니다.

| 가설 | 예측 | 적용 범위 |
|---|---|---|
| H1 | 실행 인자만으로 provider가 항상 묻게 강제할 수 있다. | Claude Code 2.1.285, Codex 0.158.0, Bash와 셸 명령 |
| H2 | 허용 규칙에 걸린 Codex 명령까지 Saturn이 판단하는 방법이 있다. | Codex 0.158.0, 소스 `b527ce4`(2026-10-01 main) |
| H3 | 훅 방식이 통과 조건 5개(대기 초과, 오류, 적용 확인, subagent, 사용자 규칙)를 모두 만족한다. | Codex 0.158.0 PreToolUse 훅 |
| H4 | 훅의 `ask` 응답이 Codex 기본 승인 경로로 넘어간다(A안). | Codex 0.158.0 PreToolUse 훅 |
| H5 | 전용 `CODEX_HOME`과 Saturn 규칙을 번역한 execpolicy만으로 Codex의 모든 실행을 Saturn 규칙대로 처리할 수 있다. | Codex 0.158.0 app-server, 셸 명령, `workspace-write` |
| H6 | 설정만으로 H5의 구멍(안전 목록, 파일 편집, MCP, subagent)을 막아 네 경로 모두 승인 요청으로 받을 수 있다. | Codex 0.158.0 app-server, `untrusted`, 읽기 전용 샌드박스 |
| H7 | MCP 도구가 첫 턴에 준비된 상태로 쓰이게 할 수 있고, Saturn 규칙(허용, 묻기, 거부)을 MCP 설정으로 번역할 수 있다. | Codex 0.158.0 app-server, 선택적 MCP 서버 |
| H8 | `approval_mode="prompt"`인 MCP 도구는 시도한 모든 회차에서 승인 요청이 오고, 거부하면 실행되지 않고, 승인하면 실행된다. | Codex 0.158.0 app-server, `write_like_tool` 하나 |

## 설계

| 항목 | 값 |
|---|---|
| 조건 | 실험마다 다르다. 실험별 방법 표에 적었다. 기준 조건은 같은 환경에서 규칙이나 훅 없이 실행한 대조(실험 2, 실험 7 기준선)다. |
| 배정 | 해당 없음. 회차 순서를 섞지 않았고 시드도 없다. |
| 눈가림 | 해당 없음. 실험 에이전트가 직접 실행하고 직접 판정했다. |
| 환경 | macOS 25.5.0, Codex CLI 0.158.0, Claude Code 2.1.285. 실험 에이전트는 실험 1 Sonnet, 실험 2 기록 없음, 실험 3과 4 Haiku, 실험 5~8 `gpt-5.6-luna`다. 모델 응답은 시드로 고정할 수 없다. [env.json](env.json)에 정리했다. |

## 실험별 방법

| 실험 | 이슈 댓글 | 에이전트 | 설계 입력 | 방법 | 반복 | 모델 호출 상한과 실제 |
|---|---|---|---|---|---|---|
| 1. 실행 인자로 항상 묻기 | 댓글 1 | Sonnet | 이슈 제목과 질문 | 실제 CLI 실행. Claude는 `--permission-mode`, `--permission-prompt-tool stdio`, `--settings`를 조합한다. Codex는 app-server `thread/start`의 `approvalPolicy`와 사용자 또는 폴더 execpolicy 허용 규칙을 조합한다. 실행한 명령은 `echo`, `node -e`, `touch`, `sort`뿐이다. | 시도마다 1회 | 상한 기록 없음. 실제 Claude 8회, Codex 8회 |
| 2. Codex 우회 방법 조사와 실측 | 댓글 2 | 기록 없음 | 실험 1: Codex는 허용 규칙을 못 이긴다 | 후보 6개(규칙 끄기, 승인 정책 변경, `CODEX_HOME` 분리, 내장 셸 끄기와 `dynamicTools`, 세션 훅, OpenCode 방식)를 소스 `b527ce4`로 조사한다. 후보 3, 4, 5는 실측한다. | 호출마다 1회 | 상한 기록 없음. 실제 6회 |
| 3. 훅 신뢰성 | 댓글 3 | Haiku | 실험 2: 훅이 가장 유력 | `-c 'hooks.PreToolUse=[...]'`와 `-c 'hooks.state={...}'`로 주입한 훅에 대기 초과, 종료 코드 1, 잘못된 JSON, 출력 없음, 훅 적용 확인, `spawn_agent`, 사용자 규칙 `sort` allow를 시험한다. | 조건마다 3회 | 기록 없음 |
| 4. A안: 훅 ask에서 Codex 승인 경로로 | 댓글 4 | Haiku | 실험 3: 훅은 fail-open, deny는 허용 규칙을 이김 | 훅이 Saturn 규칙으로 즉시 답하고 `묻기`면 `permissionDecision: "ask"`를 돌려 Codex 승인 요청으로 넘기는 경로를 확인한다. 확인할 것은 ask 응답, 허용 규칙 명령의 ask 경로와 승인 대기, 즉시 허용과 거부와 오류 감싸기와 파일 편집 훅, subagent 안 명령의 훅이다. | 기록 없음 | 기록 없음 |
| 5. 전용 `CODEX_HOME`과 execpolicy 번역 | 댓글 5 | `gpt-5.6-luna` | 실험 2: `CODEX_HOME` 분리는 인증 공유가 필요해 보류 | 전용 폴더 두 개(`codex-home-a`, `codex-home-b`)에 `auth.json` 심볼릭 링크, 권한 키를 뺀 `config.toml`, `rules/default.rules`를 두고 app-server를 띄운다. 확인할 것 9개: 로그인 공유, 모두 묻기, `allow`와 `forbidden`, 사용자와 프로젝트 규칙 무시, 승인 대기 90초 이상, 파일 편집과 MCP, subagent, 사용자 설정 이관, 채팅마다 다른 규칙(app-server 둘, thread별 변경) | 조건마다 3회 | 상한 40회, 실제 44회 |
| 6. 구멍 막기 | 댓글 6 | `gpt-5.6-luna` | 실험 5: 구멍 4개(모두 묻기, 파일 편집, MCP, subagent) | 실험 5의 전용 `CODEX_HOME`에 `approvalPolicy=untrusted`와 읽기 전용 샌드박스, MCP 서버별 도구 `approval_mode`, 로컬 MCP 중계 서버를 더해 확인한다. 확인할 것 5개: 안전 목록과 무해 명령, 파일 편집과 읽기 전용 샌드박스, MCP 묻기, subagent, 종합 시나리오 | 조건마다 3회, subagent 5회 | 상한 60회, 실제 53회(부모 48, subagent 자식 5) |
| 7. MCP 준비 시점 | 댓글 7 | `gpt-5.6-luna` | 실험 6: MCP 도구가 3회 중 1회 접근 불가 | 시작 지연 3초와 10초를 줄 수 있는 테스트 MCP 서버(`echo_tool`, `write_like_tool`)로 기준선, 준비 조회(`mcpServerStatus/list`, 후보 A), 유예 늘리기(`mcp_optional_startup_grace_ms=12000`, 후보 B), 번역표(허용, 묻기, 거부)를 비교한다. 사용자 MCP 서버는 기동만 하고 도구를 부르지 않는다. | 조건마다 5회 | 상한 70회, 실제 60회 |
| 8. MCP 묻기 경로 | 댓글 8 | `gpt-5.6-luna` | 실험 7: 묻기가 1/5 | `write_like_tool` 하나뿐인 서버에 `approval_mode="prompt"`를 두고 첫 턴 전 준비 조회, 유예 12초, `startup_timeout_sec=30`으로 거부 5회와 승인 5회를 시험한다. 호스트의 `mcpServer/tool/call` 직접 호출은 쓰지 않는다. | 거부 5회와 승인 5회 | 상한 20회, 실제 11회 |

실험 1의 Claude 시도는 6가지 인자 조합, Codex 시도는 7가지 조합이다. 조합마다 모델 호출 수는 댓글에 적혀 있지 않다.

## 변수

| 변수 | 종류 | 정의 | 단위 |
|---|---|---|---|
| `approval_policy` | 조작 | Codex `thread/start`의 `approvalPolicy`(`on-request`, `untrusted`) | 해당 없음 |
| `permission_mode` | 조작 | Claude Code `--permission-mode`와 `--settings`의 `ask` 목록 | 해당 없음 |
| `rule_source` | 조작 | 허용 규칙의 위치(사용자, 폴더, 전용 `CODEX_HOME`) | 해당 없음 |
| `sandbox` | 조작 | Codex 샌드박스(`workspace-write`, `read-only`) | 해당 없음 |
| `mcp_approval_mode` | 조작 | MCP 도구별 `approval_mode`(`approve`, `prompt`)와 `disabled_tools` | 해당 없음 |
| `startup_delay` | 조작 | 테스트 MCP 서버의 시작 지연(3초, 10초)과 유예 `mcp_optional_startup_grace_ms` | 초 |
| `asked` | 측정 | 호스트에 승인 요청(`can_use_tool`, `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `mcpServer/elicitation/request`)이 도착한 회차 수 | 회 |
| `executed` | 측정 | 명령 완료, 파일 생성, 테스트 서버 호출 기록 중 하나로 확인한 실행 수 | 회 |
| `tool_ready` | 측정 | 첫 턴 전후에 `mcpServerStatus/list`나 서버 로그로 도구 목록이 확인된 회차 수 | 회 |
| `model_calls` | 측정 | 실험 중 모델을 실제로 부른 횟수 | 회 |

## 표본

| 항목 | 값 |
|---|---|
| 출처 | 실험 에이전트가 실험 worktree에서 실행한 Claude Code와 Codex CLI. 무해한 명령(`echo`, `touch`, `ls`, `cat`, `head`, `wc`, `sort`, `node -e`)과 로컬 테스트 MCP 서버만 쓴다. |
| 크기 | 실험 1, 2, 4는 시도마다 1회, 실험 3과 5는 조건마다 3회, 실험 6은 조건마다 3회와 subagent 5회, 실험 7은 조건마다 5회, 실험 8은 거부 5회와 승인 5회다. 실험 4의 반복 수는 기록 없음이다. |
| 크기 근거 | 예산. 모델 호출 상한을 지시서에 적었다(실험 5 40회, 실험 6 60회, 실험 7 70회, 실험 8 20회). 통계적 정밀도 계산은 없다. |
| 중단 규칙 | 상한에 닿으면 멈추고 거기까지의 결과로 보고한다. 지시서가 실험 6, 7, 8에 이 규칙을 적었다. 실험 5의 지시서는 상한만 적었다. |
| 반복과 예열 | 반복은 위 크기와 같다. 예열은 없다. 드라이버 수정 전에 모델을 부른 탐색 호출은 모델 호출 수에 넣는다(실험 5). 모델 호출 전에 끝난 준비 실행은 넣지 않는다(실험 8). |

## 분석

| 가설 | 지표 | 방법 | 채택 기준 |
|---|---|---|---|
| H1 | `asked`, `executed` | 시도마다 호스트 요청 도착과 실행 여부를 기록한다. | 사전 기준 없음. 요청이 오고 거부하면 실행되지 않는 조합을 가능으로 읽는다. |
| H2 | `asked`, `executed` | 후보마다 소스 근거와 실측 여부, 허용 규칙 명령의 호스트 판단 여부를 표로 기록한다. | 사전 기준 없음. 규칙 allow 명령이 호스트에 도착하고 거부하면 막히는 후보를 가능으로 읽는다. |
| H3 | `executed` | 통과 조건 5개마다 3회를 기록한다. | 3회 모두 같을 때만 확인. 5개 모두 통과해야 채택. |
| H4 | `asked` | ask 지원 여부를 소스와 실행으로 기록한다. | 사전 기준 없음. |
| H5 | `asked`, `executed` | 확인할 것 9개마다 3회를 기록한다. | 3회 모두 같으면 확인, 다르면 불안정, 하지 못하면 확인 못 함. 추정으로 채우지 않는다. |
| H6 | `asked`, `executed` | 확인할 것 5개마다 3회(subagent 5회)를 기록한다. | H5와 같은 규칙. subagent는 5회 모두 같을 때 확인. |
| H7 | `tool_ready`, `asked`, `executed` | 조건마다 5회를 기록한다. | 5회 모두 같으면 `확인`, 다르면 `불안정`, 하지 못하면 `확인 못 함`이다. |
| H8 | `asked`, `executed` | 시도한 회차를 기준으로 회차마다 시도 여부, 승인 요청, 답, 실행 기록을 표로 기록한다. | 시도한 회차가 거부와 승인 각 5회 이상이고 모두 승인 요청, 거부 시 미실행, 승인 시 실행이면 확인. 하나라도 다르면 불안정. 시도 없음이 2회 이상이면 지시를 고쳐 상한 안에서 다시 돌린다. |

| 항목 | 규칙 |
|---|---|
| 제외 기준 | 없음. 드라이버 수정 전 탐색 호출도 모델 호출 수에는 넣는다. |
| 실패한 실행 | 모델이 도구를 시도하지 않은 회차는 `시도 없음`으로 따로 센다(실험 6, 8). 시간 초과로 끝난 회차는 승인 경계 관측 불가로 센다(실험 8). |
| 다중 비교 | 해당 없음. 통계 검정과 신뢰구간을 쓰지 않는다. 표본이 3~10회라 `k/n`만 적는다. |

## 판정

| 결과 | 설계에 반영 |
|---|---|
| 채택 | 해당 경로를 Saturn 권한 정본의 후보로 남긴다. |
| 기각 | 그 경로를 후보에서 빼고 다음 실험의 설계 입력으로 남긴다. |
| 보류 | 불안정이나 확인 못 함이 남은 항목을 후속 실험으로 넘긴다. 실험 9(원인 찾기)가 그 후속이다. |

이 사후 기록에서는 설계 문서를 바꾸지 않는다. 시리즈가 끝나지 않았으므로 설계 반영은 시리즈 결론 뒤에 한다.

## 타당성 위협

| 종류 | 위협 | 대응 |
|---|---|---|
| 내적 | 모델 응답에 의존한다. 같은 지시에도 모델이 도구를 고르지 않거나 명령을 묶어 실행하는 회차가 생긴다. | 회차마다 시도 여부와 요청 수를 따로 적었다. 반복 수를 늘리지는 못했다. |
| 내적 | 실험 에이전트의 실수다. 실험 3은 인용 일부가 틀렸고, 실험 4는 항목 일부를 잘못 건너뛰었다. | 보고서 한계 절에 적었다. 틀린 인용은 판정 근거로 쓰지 않는다. |
| 내적 | 실험 5가 모델 호출 상한 40회를 넘겼다(44회). | 보고서에 호출 수를 그대로 적었다. |
| 구성 | `asked`는 호스트에 도착한 요청 수이고, 실제 보안 경계가 막는지와 다르다. | 요청이 온 회차에서 거부 뒤 실행 여부를 따로 확인했다(파일 생성, 호출 기록). |
| 구성 | 호스트가 `mcpServer/tool/call`로 직접 부르는 경로는 승인 경계를 우회한다. | 실험 7에서 별도 표시했고 실험 8은 이 경로를 쓰지 않았다. |
| 외적 | 사용자 한 명의 환경이다. 사용자 규칙은 `sort` allow만 직접 확인했다. 관리형(managed) 설정, 키체인 로그인은 시험하지 않았다. | 한계와 남은 것에 적었다. |
| 외적 | Codex 0.158.0, Claude Code 2.1.285, 소스 `b527ce4` 기준이다. 버전이 바뀌면 결과가 달라질 수 있다. | 확인한 날짜와 버전을 함께 적었다(2026-10-02 확인). |
