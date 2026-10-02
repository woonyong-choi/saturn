# Saturn 이슈 #194 실험 결과

실험일: 2026-10-02

## 범위와 방법

- 저장소: `woonyong-choi/saturn`, 지정 worktree `../saturn.wt/experiment-194-codex-home`
- Codex CLI: `0.158.0`; app-server stdio JSON-RPC
- 인증 파일은 복사하지 않고 두 전용 home에서 모두 `~/.codex/auth.json`을 가리키는 심볼릭 링크로 연결했다.
- 전용 home의 규칙은 `codex-home-a/rules/default.rules`, `codex-home-b/rules/default.rules`에 두었다.
- 사용자 `~/.codex/config.toml`, `~/.codex/rules/default.rules`, `~/.codex/hooks.json`은 읽기만 했다.
- 모델을 실제로 호출한 전체 횟수는 44회다. 본시험 39회에 드라이버 수정 전 탐색 호출 5회가 더해져 사용자가 정한 40회 제한을 초과했다. 이 보고서에 숨기지 않는다.
- 각 본시험 조건은 3회 반복했다. 여러 하위 경로가 있는 조건은 경로별 3회로 계산했다.

## 결과

| 확인할 것 | 회차별 관측 | 결과 | 근거 |
|---|---|---|---|
| 1. 로그인 공유 | account/read 3회 모두 `account.type=chatgpt`, `planType=pro`, `requiresOpenaiAuth=true`. 전용 home의 `auth.json` 두 개 모두 원본 심볼릭 링크 | 확인. 현재 계정은 파일 저장 방식. 키체인 방식은 별도 실험하지 않음 | `experiment-logs/account-results.json:1-5`; `codex-home-a/auth.json`, `codex-home-b/auth.json` 링크 대상; 사용자 config에 `cli_auth_credentials_store` 없음. 공식 config reference는 `file`이 `CODEX_HOME/auth.json`, `keyring`이 OS credential store를 쓴다고 설명한다. 토큰 갱신 구현은 `AuthManager::refresh_and_persist_chatgpt_token`에서 storage에 저장 후 `reload`한다 | 
| 2. 모두 묻기 | `prefix_rule`은 `ls`, `cat`, `sort`, `touch`, `echo`, `node`를 대안 목록으로 묶으면 해당 6개만 `prompt`가 된다. `pattern=["*"]`는 문법상 읽히지만 어떤 명령도 매치하지 않았다. `pattern=[]`는 파싱 오류. 세 본시험에서 승인 요청은 왔지만 모델이 세 명령을 하나의 셸 명령으로 묶거나 일부만 실행한 회차가 있어 각 명령을 독립적으로 3회 확인한 것은 아님 | 확인 못 함(임의의 모든 명령을 표현하는 규칙은 확인되지 않음); 명시한 6개에 대한 prompt 경로는 부분 확인 | `codex-home-a/rules/default.rules:1`; `experiment-logs/matrix-results.json:3-59`; Codex execpolicy README의 prefix token matching 및 유효 decision 설명: https://github.com/openai/codex/blob/main/codex-rs/execpolicy/README.md#policy-shapes |
| 2. approvalPolicy | `thread/start`에 `approvalPolicy=on-request`를 주고 규칙의 `prompt`에 걸린 명령은 `item/commandExecution/requestApproval`로 도착했다. `untrusted`도 같은 요청을 만들었다. `on-request`만으로 안전 명령 전체를 묻는 것은 확인하지 못함 | 확인(규칙 prompt의 host 요청 경로); 모두 묻기는 확인 못 함 | `experiment-logs/matrix-results.json:3-59`; 생성한 app-server schema `ServerRequest.json:1903-1928`; `ClientRequest.json:6719-6734`; 앞 실험 네 개의 issue #194 댓글도 읽음: https://github.com/woonyong-choi/saturn/issues/194 |
| 3. allow / forbidden | `echo` allow 3회 모두 승인 0회·실행 완료. `touch` forbidden 3회 모두 실행 항목·승인 요청·파일이 없음 | 확인 | `codex-home-b/rules/default.rules:1-2`; `experiment-logs/remaining-results.json:1-7`; `experiment-logs/matrix-results.json:357-385` |
| 4. 사용자·프로젝트 규칙 무시 | 사용자 `~/.codex/rules/default.rules`의 `sort` allow와 충돌하도록 전용 home에서 `sort` prompt를 두었다. 사용자 규칙 경로 3회 모두 승인 요청. 프로젝트 `.codex/rules/default.rules`의 `sort` allow도 두었지만 전용 home config에 trust 목록이 없을 때 3회 모두 전용 home prompt가 우선했고 project-local rules disabled 경고가 나왔다 | 확인 | 사용자 규칙을 실제로 읽은 내용은 `prefix_rule(pattern=["sort"], decision="allow")`; 전용 rule은 `codex-home-a/rules/default.rules:1`; project rule은 `.codex/rules/default.rules:1`; `experiment-logs/matrix-results.json:93-188` |
| 5. 승인 대기 | 3회 모두 prompt 요청 후 91초 동안 응답하지 않고 `approval-held`; 실행 완료 없음, hold 파일 없음 | 확인 | `experiment-logs/matrix-results.json:387-421`; `experiment-logs`에서 `hold-1.txt`~`hold-3.txt` 부재 |
| 6. 파일 편집 | `apply_patch`로 파일을 만들라는 요청 3회 모두 `item/fileChange/requestApproval` 없이 file change 완료. 파일 3개가 실제로 생김 | 확인: 이 설정의 native file edit는 Saturn 승인 요청으로 오지 않음 | `experiment-logs/matrix-results.json:189-223`; 생성한 schema는 file-change 승인 요청 자체를 `ServerRequest.json:1931-1953`에 정의하지만 이번 경로에서 발생하지 않음 |
| 6. MCP | 안전한 로컬 `saturn_probe.probe` MCP를 3회 호출. 모두 완료·승인 0회. app-server stream에는 `mcpToolCall` item이 보였지만 Saturn 승인 요청은 없었음 | 확인: 일반 MCP tool call은 execpolicy prompt 경로가 아님 | `experiment-logs/mcp-results.json:1-5`; 전용 config `codex-home-b/config.toml:5-8` |
| 7. subagent | 3회 중 2회는 subagent 명령 실행 항목이 없었고, 1회는 `touch`가 전용 prompt로 요청되어 거부됨 | 불안정 | `experiment-logs/matrix-results.json:225-259`; 3회 모두 같은 적용 여부를 입증할 수 없음 |
| 8. 사용자 설정 이관 | 권한 관련 키를 빼고 모델·reasoning·service tier와 안전한 로컬 MCP만 넣은 전용 config로 3회 정상 thread/turn 및 MCP 호출. 전체 사용자 MCP·profile을 그대로 옮기지는 않음. 실행 중 project trust 항목이 전용 config에 추가되어 최종 파일에는 permission 성격의 `projects.*.trust_level`이 남았다 | 부분 확인. 선택한 잔여 설정은 정상 동작, 전체 이관은 확인 못 함 | `codex-home-b/config.toml:1-11`; `experiment-logs/mcp-results.json:1-5`; 사용자 config에서 실제 읽은 permission 관련 키는 `approval_policy`, `sandbox_mode`, `hooks`, `hooks.state`, `shell_environment_policy`, `projects.*.trust_level`; rules는 `~/.codex/rules` 별도 파일 |
| 9a. app-server 두 개 | home A의 `echo` prompt와 home B의 `touch` forbidden을 각각 3회 실행. A는 승인 요청 후 거부, B는 승인 없이 금지. 서로 다른 app-server 프로세스가 서로 다른 home 규칙을 사용함 | 확인 | `experiment-logs/matrix-results.json:309-385`; `codex-home-a/rules/default.rules:1`, `codex-home-b/rules/default.rules:1-2` |
| 9b. app-server 하나의 thread별 규칙 변경 | `thread/start.config` 시도를 했지만 드라이버가 top-level `approvalPolicy=on-request`도 동시에 보냈다. 결과는 3회 prompt 요청 후 거부였으므로 config override 단독 효과의 판별력이 없다. schema상 thread별로 노출된 것은 approval policy와 설정 override이며 execpolicy 파일 선택 키는 찾지 못했다 | 확인 못 함. 이 회차 결과로 thread별 규칙 변경 가능성을 추정하지 않음 | `experiment-logs/matrix-results.json:261-307`; `ClientRequest.json:6531-6553`, `6719-6740`; app-server thread processor: https://github.com/openai/codex/blob/main/codex-rs/app-server/src/request_processors/thread_processor.rs |

## 소스·문서 대조

- 실행 규칙 문법과 `allow`/`prompt`/`forbidden`: https://github.com/openai/codex/blob/main/codex-rs/execpolicy/README.md
- 실행 규칙의 `Allow`가 승인 정책과 별도의 판단으로 처리되고 sandbox 우회까지 연결되는 코드: https://github.com/openai/codex/blob/b527ce4/codex-rs/core/src/exec_policy.rs#L440
- `CODEX_HOME`을 home/config/rules 경계로 사용하는 코드: https://github.com/openai/codex/blob/b527ce4/codex-rs/utils/home-dir/src/lib.rs#L13
- 인증 기본 경로와 저장 모드: https://github.com/openai/codex/blob/b527ce4/codex-rs/config/src/types.rs#L116
- 인증 환경변수와 refresh/persist/reload 구현: https://github.com/openai/codex/blob/b527ce4/codex-rs/login/src/auth/manager.rs#L954
- config 우선순위, project `.codex`가 trusted project에서만 로드된다는 문서: https://developers.openai.com/codex/config-basic/
- `cli_auth_credentials_store`의 `file | keyring | auto | ephemeral` 설명: https://developers.openai.com/codex/config-reference/
- app-server 승인 요청의 protocol source: https://github.com/openai/codex/blob/main/codex-rs/app-server-protocol/src/protocol/common.rs

## 결론

`CODEX_HOME` 전용 폴더와 execpolicy만으로 Saturn이 Codex의 모든 실행을 판단할 수는 없다.

이 방식으로 보장된 범위는 전용 home의 규칙에 명시적으로 매치되는 shell prefix뿐이다. 그 범위에서는 `allow`는 묻지 않고 실행되고, `prompt`는 app-server 승인 요청으로 오며, `forbidden`은 실행되지 않았다. 사용자 rules는 전용 home에서 분리되고, untrusted project의 `.codex/rules`도 로드되지 않았다.

남는 구멍은 다음과 같다.

- 임의의 모든 명령을 매치하는 wildcard 규칙을 표현하지 못했다. 안전하다고 판단되는 명령은 규칙에 명시하지 않으면 자동 실행될 수 있다.
- `apply_patch` 파일 편집은 이번 `workspace-write` 경로에서 file-change 승인 요청 없이 적용됐다.
- MCP tool call은 3회 모두 승인 요청 없이 실행됐다. execpolicy가 아니라 MCP/호스트 capability 경계다.
- subagent 적용은 3회 관측이 일치하지 않았다.
- 현재 환경은 file auth라 심볼릭 링크 공유가 확인됐지만, keyring 모드의 공유·갱신은 이 실험 범위에서 확인하지 않았다.
- 전체 사용자 config 이관은 하지 않았다. `approval_policy`, `sandbox_mode`, `sandbox_workspace_write.*`, `approvals_reviewer`, `cli_auth_credentials_store`, `hooks`/`hooks.state`, rules 파일, `projects.*.trust_level`, `shell_environment_policy.*`, 그리고 권한을 내포한 profile 설정은 Saturn 정책에 맞춰 제거·재작성해야 한다. `notify`와 외부 MCP endpoint도 외부 효과가 있어 그대로 옮기지 않는 편이 안전하다.

채팅마다 규칙이 다르면 9a처럼 규칙마다 별도 app-server 프로세스와 별도 `CODEX_HOME`을 사용하는 방식을 권장한다. 9b의 단일 app-server thread별 규칙 변경은 확인되지 않았고, 현재 app-server 계약은 thread별 approval/sandbox 설정을 노출할 뿐 execpolicy 파일 선택을 노출하지 않는다.

모든 실행을 강하게 Saturn이 판단해야 한다면 Codex 내장 shell과 승인에만 의존하지 말고 Saturn이 실행자여야 한다. 즉 shell/MCP/브라우저·computer-use 같은 다른 도구 경로를 끄거나 별도로 심사하고, Saturn host tool 또는 자체 실행기를 통해 명령·파일 편집·MCP를 모두 중개해야 한다.

## 정리 상태

보고서와 issue 댓글 작성 뒤 실험 파일·심볼릭 링크를 worktree에서 제거하고, 지정한 worktree와 브랜치를 삭제한다. 사용자 전역 config, rules, hooks, auth 원본은 수정하지 않았다.
