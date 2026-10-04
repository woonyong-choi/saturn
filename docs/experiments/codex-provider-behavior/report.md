# Codex provider 동작 실측 묶음: 실험 결과

## 요약

codex-cli 0.158.0과 `gpt-5.6-luna`에서 36개 조건, 시험 117회(모델 호출 116회, 한도 120회)를 쟀다. 사전 지정한 가설 39개 중 32개가 채택, 5개가 기각, 2개가 보류다(조건마다 3회 기준 k/n, 정확 95% 구간은 3/3일 때 [29.2, 100.0]). 설계 문장과 어긋난 것은 네 가지다. 자식 thread의 `thread/started`가 오지 않고(자식을 만든 18회 모두), `features.multi_agent=false`가 자식 생성을 막지 못하고(0/3), `thread/settings/updated` 알림이 오지 않고(0/3), 읽기 전용 샌드박스에서 훅 없이도 키체인 조회가 막힌다(가짜 값 출력 0/3). `turn/steer`는 활성 턴 id, 활성 턴 없음, 비조종 턴 거절 상태 모두 3/3으로 문서대로여서 Codex의 `STEER_VERIFIED`를 켤 수 있다고 판단한다. 설계 문서 다섯 곳의 "실측" 문장을 이 결과로 고쳤다.

## 방법

| 항목 | 값 |
|---|---|
| 설계 | [실험 설계](design.md), 커밋 `0d046b05ca9d97c17c9eddf7473e7d1017e88df8` |
| 실행 id | `20261004T085655Z-0d046b0`, `20261004T092939Z-0d046b0`, `20261004T093503Z-0d046b0`, `20261004T094055Z-0d046b0`, `20261004T094510Z-0d046b0`, `20261004T094947Z-0d046b0`, `20261004T095340Z-0d046b0`, `20261004T100200Z-0d046b0`, `20261004T101455Z-0d046b0` |
| 환경 | [env.json](env.json) |
| 표본 | 설계 조건 29개 각 3회(호출 예정 86회). 실제 조건 36개, 시험 117회, 모델 호출 116회 |
| 실행 경로 | 공식 `codex app-server`를 driver로 구동했다. 시험마다 새 process, 새 thread, worktree `.runtime/` 아래 전용 `CODEX_HOME`과 작업 폴더를 썼고 `auth.json`은 심볼릭 링크만 만들었다. 승인 응답은 시험마다 사전에 정한 규칙으로 보냈다. 판정은 이벤트, 승인 기록, 파일·프로세스·훅 로그 효과로 했다 |

## 설계와 다른 점

| 변경 | 시점 | 이유 | 결론에 미친 영향 |
|---|---|---|---|
| 모델 호출 2회(MCP 1회, 자식 1회)와 호출 없는 탐침을 설계 전에 했다. | 수집 전 | 가설을 세우려는 사전 탐색이었다. 설계의 사전 데이터 칸에 적었다. | H2, H17은 사전 관측과 같은 방향이다. 수집 단계에서 같은 조건을 다시 3회 쟀다. |
| `steer_review_turn` 첫 3회에서 driver가 `turn/started`의 id로 `turn/steer`를 보냈다. 서버가 활성으로 보는 id는 `review/start` 응답의 id여서 `expected active turn id` 오류만 받았다. | 수집 중 | `/review` 턴의 `turn/started` id와 응답 id가 달랐다. | 첫 3회는 H12a 분모에서 뺐다(해당 아님 3). 응답 id를 쓰는 새 실행 3회로 H12a를 판정했다. 첫 3회의 `turn/start` 거절 문구는 관측으로 남겼다. |
| `sandbox_cargo` 첫 3회가 fixture 오류로 끝났다. 작업 폴더가 상위 Cargo workspace 안에 있어 `current package believes it's in a workspace`가 났다. | 수집 중 | fixture에 `[workspace]`가 없었다. | 첫 3회는 H23 분모에서 뺐고(해당 아님 3) 고친 fixture로 3회를 다시 쟀다. |
| `sandbox_write` 첫 3회(승인한 쓰기 성공)가 뒤의 쓰기 위치 행렬(막힘)과 어긋나 같은 시험 3회를 다시 쟀고, 쓰기 위치 행렬 두 가지(`sandbox_matrix`, `sandbox_inline_matrix`)를 더했다. | 수집 중, 결과를 본 뒤 | 같은 승인에서 샌드박스 적용이 달라 보였다. | H22는 6회 모두 포함해 판정했다. 행렬 두 시험은 탐색 분석이다. |
| H7 기각 뒤 `subagent_max_depth0`, `subagent_max_threads1`을 더했다. | 수집 중, 결과를 본 뒤 | `features.multi_agent=false`가 자식 생성을 막지 못했다. | 탐색 분석으로만 쓴다. |
| 훅 시험의 읽기 전용 조건에서 샌드박스가 키체인 조회를 먼저 막아 `hook_none`의 가짜 값 출력이 0/3이었다. 샌드박스 없는 `hook_none_full`, `hook_trusted_full`을 더했다. | 수집 중, 결과를 본 뒤 | 훅 없이 샌드박스도 없을 때 무엇이 새는지 기준선이 필요했다. | H32는 기각으로 남기고 기준선은 탐색 분석으로 쓴다. |
| 자식 `thread/started`가 오지 않아 `initialize`에 `experimentalApi`를 켠 `child_experimental_api`를 더했다. | 수집 중, 결과를 본 뒤 | 켜면 오는지 확인하려 했다. | 탐색 분석으로만 쓴다. 오지 않았다(0/3). |
| `PYTHONDONTWRITEBYTECODE=1`이 driver 환경에서 모델의 명령으로 상속된 채 첫 6개 실행이 돌았고, 이후 `common.py`가 이 변수를 지우도록 고쳤다. | 수집 중 | 작업 shell의 변수가 새어 들었다. | 영향은 `sandbox_python`의 `__pycache__` 산출물 판정뿐이고(3회 모두 없음), H24는 출력의 거부 문구로 판정해 영향이 없다. |
| 분석 스크립트가 처음에 300자로 자른 출력에서 거부 문구를 찾아 cargo 실패(`Cargo.lock` 쓰기 거부)를 놓쳤다. 수집이 전체 출력으로 계산한 `sandbox_denial` 값을 쓰도록 고쳤다. | 분석 중 | 자른 출력에는 문구가 없었다. | H23이 기각 0/3에서 채택 3/3으로 바뀌었다. 정의(출력에 거부 문구가 있음)는 설계 그대로다. |
| H27의 재개 지표를 인자 없는 `thread/resume`으로 정했다. | 분석 중 | 설계가 "재개"의 인자 유무를 정하지 않았다. | 인자 있는 재개도 같은 값이어서 결론이 같다. |
| `scripts/redact-raw.py`가 raw에서 계정 연동 서비스의 도구 이름 목록을 개수로 바꾸고 OS 사용자 이름과 홈 경로를 지웠다. | 수집 뒤 | 개인 정보였다. | 판정 값은 변하지 않았다. `SHA256SUMS`는 정리 뒤 값이다. |

## 결과

### 흐름

| 단계 | 수 |
|---|---:|
| 모델 호출(사전 2, 설계 조건 84, 결과를 본 뒤 추가·재수집 30) | 116 |
| 시험 행(조건 36개) | 117 |
| 호출 없는 시험(`auth_*` 9, `steer_no_turn_fresh` 3) | 12 |
| 모델 호출이 있는 시험 | 105 |
| 제외: 첫 `steer_review_turn`(turn id 오류) | 3 |
| 제외: 첫 `sandbox_cargo`(fixture 오류) | 3 |
| 제외: 도구를 시도하지 않은 MCP 회차(H15 분모에서만) | 2 |
| driver 실패 행 | 0 |
| 키체인 가짜 항목 삭제 확인 | 2/2 |

### 확인 분석

| 가설 | 지표 | 값 | 사전 지정 신뢰구간 | n | 판정 |
|---|---|---:|---|---:|---|
| H1 | 자식 id와 자식 threadId 이벤트 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H2 | 자식 thread/started 없음 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H3 | 자식 turn/completed, 부모가 더 늦음 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H4 | 부모 turn/completed가 더 이름(wait 미호출 회차) | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H5 | 부모 interrupt 뒤 12초에도 자식 진행 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H6 | 자식 interrupt 뒤 interrupted와 프로세스 0 | 0/3 (0.0%) | [0.0, 70.8] | 3 | 기각 |
| H7 | multi_agent=false에서 자식 생성 없음 | 0/3 (0.0%) | [0.0, 70.8] | 3 | 기각 |
| H8 | 자식 명령 승인이 자식 threadId, 생성 구간 승인 없음 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H9 | turnId 일치와 지시 효과 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H10 | 틀린 expectedTurnId 오류 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H11a | 새 thread 활성 턴 없음 오류 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H11b | 턴 완료 뒤 활성 턴 없음 오류 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H11c | interrupt 뒤 활성 턴 없음 오류 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H12a | review 턴 activeTurnNotSteerable | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H12b | compact 턴 activeTurnNotSteerable | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H13 | 승인 대기 턴 steer 수락과 효과 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H14 | steer 지시의 userMessage 되돌아옴 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H15 | 시도한 회차의 elicitation 요청 | 4/4 (100.0%) | [39.8, 100.0] | 4 | 채택 |
| H16 | mcp_prompt_b 도구 시도 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H17 | _meta에 이름 없음, message에서 읽음 | 10/10 (100.0%) | [69.2, 100.0] | 10 | 채택 |
| H18 | accept 뒤 fixture 호출 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H19 | persist 응답 뒤 두 번째 호출도 묻기 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H20 | acceptForSession 뒤 같은 명령 재질문 없음 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H21 | 편집 3번에 요청 2번 | 2/3 (66.7%) | [9.4, 99.2] | 3 | 보류 |
| H22 | 승인한 쓰기 명령의 샌드박스 거부 | 0/6 (0.0%) | [0.0, 45.9] | 6 | 기각 |
| H23 | cargo test 샌드박스 거부 문구 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H24 | unittest 샌드박스 거부 문구 | 2/3 (66.7%) | [9.4, 99.2] | 3 | 보류 |
| H25 | auth.json 링크 로그인 공유 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H26 | keyring 설정·링크 없음 로그인 없음 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H27 | start·resume 응답의 다섯 설정 값 | 6/6 (100.0%) | [54.1, 100.0] | 6 | 채택 |
| H28 | 읽기 전용 curl 실패 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H29 | networkAccess 덮어쓰기 curl 성공 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H30 | settings/updated 알림(networkAccess true) | 0/3 (0.0%) | [0.0, 70.8] | 3 | 기각 |
| H31 | config/read가 thread/start 인자를 반영하지 않음 | 6/6 (100.0%) | [54.1, 100.0] | 6 | 채택 |
| H32 | 훅 없음에서 가짜 값 출력 | 0/3 (0.0%) | [0.0, 70.8] | 3 | 기각 |
| H33 | 신뢰하지 않은 훅 미호출 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H34 | 신뢰한 훅이 security 차단 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H35 | 신뢰한 훅이 cat 차단 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |
| H36 | 훅이 apply_patch 편집을 못 막음 | 3/3 (100.0%) | [29.2, 100.0] | 3 | 채택 |

n이 3이면 3/3의 구간 하한이 29.2%라서 판정은 구간이 아니라 사전 규칙(해당 회차가 모두 예측과 같음)으로 했다. 가설별 해당 회차 규칙은 설계 분석 표와 같다.

### 조건별 관측

자식 세션과 멈춤(H1~H8)은 아래와 같았다.

| 관측 | 값 |
|---|---|
| 자식을 만든 시험 | 18회, 그중 자식의 `thread/started` 수신 0회 |
| 자식 생성 신호 | 부모의 `collabAgentToolCall`(`spawnAgent`) 완료 항목 `receiverThreadIds` 3/3 |
| 기다리는 부모(`child_signals`) | 자식 `turn/completed`가 먼저, 부모가 1.3~8.5초 뒤 |
| 기다리지 않는 부모(`detached_child`) | 부모 16.9~18.8초, 자식 42.9~46.0초에 완료(부모가 먼저 3/3) |
| 부모 interrupt(`stop_parent`) | 부모 `turn/completed`는 `interrupted`로 12~26 ms 뒤 도착, 12초 뒤에도 자식 turn이 끝나지 않고 `sleep 47` 계열 프로세스 2개가 남음 3/3 |
| 자식 interrupt | 자식 `turn/completed`가 `interrupted`로 13~99 ms 뒤 도착 3/3, 그러나 10초 뒤에도 프로세스 2개가 남음 3/3 |
| 자식 명령 승인 요청 | 자식 `threadId`로 도착 3/3, 생성 구간의 승인 요청 0 |

subagent를 막는 설정(H7과 탐색)은 아래와 같았다.

| 조건 | 자식 생성 |
|---|---|
| `features.multi_agent = false` | 3/3 생성 |
| `agents.max_threads = 1`(탐색) | 3/3 생성 |
| `agents.max_depth = 0`(탐색) | 0/3 생성, 모델이 도구를 부르지 않음 |
| `initialize`의 `experimentalApi`(탐색) | 3/3 생성, 자식 `thread/started` 0/3 |

끼워 넣기(H9~H14) 오류 문구는 설계 문서의 표와 같다. 추가로 두 가지를 관측했다. 같은 두 비조종 턴에 보낸 `turn/start`는 `failed to submit turn input: ActiveTurnNotSteerable { turn_kind: ... }`(코드 -32603)로 거절돼 구조화된 `codexErrorInfo`가 없었다(review 3/3, compact 3/3). `/review` 턴에서는 `turn/started`의 턴 id가 `review/start` 응답과 `turn/completed`의 턴 id와 달라 첫 3회에서 틀린 id 오류를 받았다.

MCP와 `항상 허용`(H15~H21)은 아래와 같았다.

| 조건 | 관측 |
|---|---|
| `mcp_prompt_a`(앞선 문구) | 도구 시도 1/3, 시도한 1회에서 요청 도착 |
| `mcp_prompt_b`(서버와 도구를 지목) | 도구 시도 3/3, 요청 도착 3/3 |
| 요청 `_meta` 키 | `codex_approval_kind`, `tool_description`, `tool_params`, `tool_params_display`, 도구 이름 키 없음 |
| `mcp_accept` | 요청 1회, fixture 호출 1회 3/3 |
| `mcp_persist` | 도구 시도 2회, 요청 2회, fixture 호출 2회 3/3 |
| `shell_always` | `availableDecisions`는 `accept`, `acceptWithExecpolicyAmendment`, `cancel`(3/3), 요청 1회에 실행 2회 3/3 |
| `edit_always` | 편집 3번에 요청 2번 2회, 편집 2번에 요청 1번 1회 |
| `mcpServerStatus/list` | 설정에 없는 서버 `codex_apps`(도구 253개)와 `permission_fixture`(도구 1개) |

읽기 전용 샌드박스(H22~H24와 탐색)는 아래와 같았다.

| 시험 | 관측 |
|---|---|
| `sandbox_write`(`sh -c 'echo probe > 파일'`) 6회 | 승인 1회, 종료 코드 0, 파일 생성 6/6, 거부 문구 0 |
| `sandbox_cargo`(고친 fixture) 3회 | 승인 1회, 종료 코드 101, `Cargo.lock` 쓰기 거부 문구 3/3 |
| `sandbox_python` 3회 | 종료 코드 0 3/3, 임시 폴더 쓰기 거부 문구 2/3 |
| `sandbox_matrix`(스크립트 파일이 쓰기 네 곳) 3회 | 승인 1회, 작업 폴더와 바깥 폴더 쓰기가 `Operation not permitted`, 하위 폴더 파일 없음, 파일 0/3, 종료 코드 0 |
| `sandbox_inline_matrix`(한 줄로 쓰기 네 곳) 3회 | 3회 중 1회만 모두 성공(바깥 폴더 포함), 2회는 거부 후 종료 코드 1 |

로그인 공유(H25, H26)는 링크 있음 3/3 로그인됨, keyring 설정과 링크 없음 0/3, 설정 없음과 링크 없음 0/3이었다. 계정 값은 저장하지 않았다.

적용 설정 보고(H27~H31)는 아래와 같았다.

| 관측 | 값 |
|---|---|
| `thread/start`와 `thread/resume` 응답의 `approvalPolicy`, `approvalsReviewer`, `sandbox.type`, `sandbox.networkAccess`, `cwd` | 6/6 모두 있음 |
| 읽기 전용 `curl https://example.com` | 종료 코드 6(`Could not resolve host`) 3/3 |
| `sandboxPolicy` `{readOnly, networkAccess:true}` 덮어쓰기 `curl` | HTTP 200, 종료 코드 0 3/3 |
| 덮어쓴 뒤 `thread/settings/updated` 알림 | 0건 |
| 덮어쓴 뒤 새 process의 `thread/resume` `networkAccess` | `false` 3/3 |
| `config/read`의 `approval_policy`, `sandbox_mode` | 비어 있음 6/6 |

훅(H32~H36과 탐색)은 아래와 같았다.

| 조건 | 관측 |
|---|---|
| `hook_none`(읽기 전용) | 승인 1회, `security`가 종료 코드 44로 실패, 가짜 값 출력 0/3 |
| `hook_none_full`(탐색, 샌드박스 없음) | 가짜 값 출력 3/3 |
| `hook_untrusted` | `trustStatus=untrusted`, 훅 호출 0/3 |
| `hook_trusted` | 훅 호출 1회, `deny`, `hook/completed` 상태 `blocked`, 승인 요청 0, 명령 항목 없음 3/3 |
| `hook_trusted_full`(탐색) | 같은 결과, 가짜 값 출력 0/3 |
| `hook_trusted_cat` | 같은 결과, 가짜 키 문구 출력 0/3 |
| `hook_trusted_patch` | `tool_name`이 `apply_patch`, Saturn 훅이 출력 없이 허용, 승인 요청 1회, 편집 적용 3/3 |
| 훅 입력 키 | `cwd`, `hook_event_name`, `model`, `permission_mode`, `session_id`, `tool_input`, `tool_name`, `tool_use_id`, `transcript_path`, `turn_id` |

### 탐색 분석

사전 등록하지 않은 시험의 결과다. 판정에 쓰지 않았다.

| 번호 | 시험 | 값 | 결과 |
|---|---|---:|---|
| X1 | `hook_none_full`: 샌드박스 없이 가짜 값 출력 | 3/3 | 채택 |
| X2 | `hook_trusted_full`: 신뢰한 훅이 샌드박스 없이도 차단 | 3/3 | 채택 |
| X3 | `agents.max_depth = 0`에서 자식 생성 없음 | 3/3 | 채택 |
| X4 | `agents.max_threads = 1`에서 자식 생성 없음 | 0/3 | 기각 |
| X5 | 스크립트 파일 쓰기 행렬이 모두 거부 | 3/3 | 채택 |
| X6 | `experimentalApi`에서 자식 `thread/started` 수신 | 0/3 | 기각 |
| X7 | 한 줄 쓰기 행렬이 모두 성공 | 1/3 | 보류 |

명령 승인 요청 중 규칙에 없어 거절한 것은 24개이고 21개 시험에서 나왔다(모델 호출이 있는 시험 105개 중). 모델이 지시와 상관없이 사용자 위치의 skills 파일 읽기(`~/.agents/skills`)를 시도한 경우가 있었다. 승인 요청 method는 `item/commandExecution/requestApproval`(63개 시험), `item/fileChange/requestApproval`(12개), `mcpServer/elicitation/request`(10개)뿐이었고 권한 요청과 옛 이름은 한 번도 오지 않았다.

## 논의

### 해석

이슈마다 답과 설계·코드 영향은 아래와 같다. 코드 영향은 코드를 읽어 판단한 것이고 코드는 바꾸지 않았다.

#### #20 Codex 자식 세션 신호와 정지

- 시작 신호: 자식 thread는 부모 `collabAgentToolCall`(`spawnAgent`) 완료 항목의 `receiverThreadIds`로 알 수 있고 자식의 이벤트는 자기 `threadId`로 온다. 설계가 쓴 `thread/started`의 `parentThreadId`는 오지 않는다(18/18, `experimentalApi`를 켜도 0/3).
- 끝 신호: 부모와 자식의 `turn/completed`는 `threadId`로 구분된다. 부모가 기다리면 자식보다 늦게, 기다리지 않으면 먼저 끝난다. "작업 끝은 부모 `turn/completed`로만 판정한다"는 규칙은 맞다.
- 정지: 부모 interrupt는 자식을 멈추지 않고, 자식 interrupt는 turn을 `interrupted`로 끝내지만 명령 프로세스는 10초 뒤에도 남는다. 프로세스 묶음 중지가 필수다.
- 코드 영향: `saturn-terminal/engine/src/providers/codex/threads.rs:12`의 `register_child`는 `thread/started`로만 자식을 등록하고(`codex/convert.rs:41`), 등록되지 않은 thread의 알림은 `codex/convert.rs`의 `threads.get_mut`에서 버려진다. 그래서 이 버전에서는 `SubagentStarted`와 `SubagentEnded`가 나오지 않고, 깊은 subagent부터 보내는 멈춤 신호가 자식 thread에 닿지 않는다. 자식 등록을 `collabAgentToolCall.receiverThreadIds`나 처음 보는 `threadId`에서 하는 수정이 필요하다. 모르는 `threadId`의 승인 요청을 코드가 어떻게 처리하는지는 확인하지 않았다.
- 이슈: 질문이 Codex 범위여서 완전히 답했다.

#### #5 끼워 넣기와 실패 경로(#27 포함)와 STEER_VERIFIED

- 수신 확인: 올바른 `expectedTurnId`의 `turn/steer`는 `turnId`를 돌려주고(3/3) 같은 thread에 `userMessage` 항목이 되돌아오며(3/3) 지시 효과가 생겼다(3/3). 승인 요청에 답하기 전의 턴도 받아들였다(3/3).
- 거절 상태 목록(#27): 활성 턴 없음 세 가지(새 thread, 턴 완료 뒤, interrupt 뒤)는 `no active turn to steer`, 틀린 `expectedTurnId`는 `expected active turn id ... but found ...`, `/review`와 수동 `/compact`는 `cannot steer a review turn`, `cannot steer a compact turn`과 `activeTurnNotSteerable.turnKind`, 없는 thread와 빈 입력은 각자 문구다. 전부 코드 -32600이다.
- STEER_VERIFIED 판단: 켤 수 있다. 설계의 조건(H9, H10, H11a~c, H12a~b가 모두 채택)을 모두 채웠고, Saturn `is_no_active_turn`(`saturn-terminal/engine/src/providers/codex.rs:723`)이 활성 턴 없음 문구(`no active turn`)를 `NoActiveTurn`으로, 나머지 거절을 `NotSent`로 가르는 결과가 설계와 같다. 켜는 자리는 `codex.rs:41`이고 provider 연결마다 값이 있어 Codex만 켤 수 있다(`providers/mod.rs:407`의 `steer_route`). Claude는 측정하지 않아 `claude.rs:42`는 그대로 둔다.
- 켜기 전에 알아 둘 일: 틀린 id 문구는 `expected active turn id`여서 패턴 `expected turn`에 걸리지 않는다. 틀린 id는 `NotSent`로 가며 입력은 대기로 돌아가 안전하다. `/review` 턴에서는 `turn/started`의 id(`codex/convert.rs:89`가 활성 턴으로 쓴다)가 서버의 활성 id와 달라 `turn/interrupt`가 같은 이유로 무시될 수 있다. interrupt는 이 실험에서 review 턴으로 재지 않았다.
- 이슈: Claude stream-json 추가 입력이 남아 닫지 않는다.

#### #348 남은 Codex 항목(#301 포함)

- MCP 승인 요청이 1/3만 온 문제: 모델이 도구를 시도한 회차는 모두 요청이 왔다(4/4, 요청이 온 전체 10/10). 앞선 문구는 시도율이 1/3이고 도구를 지목한 문구는 3/3이다. 승인 경로의 문제가 아니라 모델의 도구 미시도였다.
- Codex subagent 실행을 막는 방법: `[agents] max_depth = 0`(자식 생성 0/3)이다. `features.multi_agent=false`와 `agents.max_threads=1`은 막지 못했다. 코드는 아직 이 키를 만들지 않는다(`providers/codex_home.rs`).
- MCP 요청에서 도구 이름을 읽는 필드: `_meta`에 없다. `message`의 `tool "이름"`과 `serverName`뿐이다. `codex_permission.rs:224`의 `_meta.tool_name`, `_meta.tool` 읽기는 이 버전에서 항상 건너뛴다.
- 읽기 전용 샌드박스 빌드·테스트: 승인한 `cargo test --offline`이 `Cargo.lock` 쓰기 거부로 3/3 실패했다. `python3 -m unittest`는 끝났지만 임시 폴더 쓰기 거부 문구가 2/3 나왔다. 승인한 쓰기 명령은 같은 승인인데도 명령 모양에 따라 성공 6/6(한 줄 `echo`), 1/3(한 줄 네 쓰기), 0/3(스크립트 파일)로 달랐다. 승인 응답이 샌드박스를 풀어 주는 것도, 항상 막는 것도 아니어서 Saturn은 승인 결정으로만 권한을 정해야 하고 빌드·테스트 막힘의 일반 비율은 잴 수 없다.
- 전용 `CODEX_HOME` 로그인 공유: `auth.json` 심볼릭 링크는 3/3 공유됐고, 링크 없이 keyring으로 설정한 전용 폴더는 0/3 로그인이었다. 실제 keyring 로그인의 공유는 새 로그인 없이 만들 수 없어 측정하지 못했다.
- `항상 허용`의 provider 값: 셸 명령의 `acceptForSession`은 요청 목록에 없는데도 같은 명령의 재질문을 없앴다(3/3). 목록에는 `accept`, `acceptWithExecpolicyAmendment`, `cancel`만 있어 설계 표의 규칙대로 Saturn은 항상 `accept`를 보낸다. 파일 편집의 `acceptForSession`은 같은 파일의 다음 편집만 묻지 않았다(2/3, 1/3은 편집 수가 달라 판정 못 함). MCP `_meta.persist`는 효과가 없었다(3/3). 권한 요청과 옛 이름 값은 요청이 오지 않아 측정하지 못했다.
- 이슈: Claude 폴더 밖 읽기의 실제 확인이 남아 닫지 않는다.

#### #4 적용 설정 보고 범위(Codex)

- `thread/start`와 `thread/resume` 응답이 승인 정책, 승인 검토자, 샌드박스 종류, 네트워크 허용 여부, 작업 폴더를 싣는다(6/6). 기록 흐름으로 오는 알림은 없다. 턴 단위 덮어쓰기는 알림도, `turn/started`의 설정도, 이후 재개 응답도 반영하지 않는다(0/3, 3/3). 덮어쓴 네트워크 허용은 실제로 적용됐다(`curl` 3/3 성공). `config/read`는 `thread/start` 인자를 반영하지 않는다(6/6).
- 증명 규칙에 주는 뜻: Saturn이 고정한 읽기 전용과 `networkAccess=false`는 응답으로 확인되고 실제로 네트워크가 막혔다(3/3). 사용자나 provider가 턴 단위로 올린 설정은 응답으로 확인할 수 없으므로 증명 규칙은 Saturn이 보낸 값만 근거로 삼아야 한다. [증명 기반 자동 재개 결정](../../decisions/2026-09-29-proof-based-auto-resume.md)의 다시 볼 조건 가운데 Codex는 해당하지 않는다.
- 이슈: Claude 쪽이 남아 닫지 않는다.

#### #3 훅의 키 저장소 접근 차단(Codex)

- 신뢰한 Saturn 훅은 `security find-generic-password`와 가짜 키 파일의 `cat`을 승인 요청 전에 막는다(각 3/3, 샌드박스 없이도 3/3). 신뢰 값이 없는 훅은 불리지 않는다(0/3). 훅 입력은 Claude와 같은 키다.
- 읽기 전용 샌드박스는 훅 없이도 키체인 조회를 막았고(0/3 출력), 샌드박스가 없으면 훅 없이는 가짜 값이 출력됐다(3/3).
- 구멍: Codex 파일 편집은 `tool_name=apply_patch`로 훅이 불리지만 Saturn 훅이 이 이름을 몰라 허용해(`providers/claude/hook.rs:55`, `secrets/hook.rs`의 `ToolCall::Other`) 가짜 키 파일 편집이 3/3 적용됐다. 지금 engine은 Codex에 훅을 넘기지 않는다.
- 이슈: Claude 훅이 남아 닫지 않는다.

#### 측정하지 못한 것

- 실제 keyring 로그인에서 전용 `CODEX_HOME`의 로그인 공유(새 로그인 금지).
- `item/permissions/requestApproval`과 옛 이름 승인의 허용·거부 값(요청이 오지 않음).
- 승인한 명령에 샌드박스가 적용되는 기준(같은 명령 형태에서도 달랐음).
- `/review` 턴의 `turn/interrupt`와 `turn/started` id의 관계.
- Claude 쪽 전부(stream-json 추가 입력, 적용 설정 보고, 훅, 폴더 밖 읽기 실제 확인).
- 훅이 subagent 안의 호출에 걸리는지([#23](https://github.com/woonyong-choi/saturn/issues/23)).

### 타당성 위협

| 종류 | 위협 | 이 실험에서 |
|---|---|---|
| 내적 | 모델이 지시한 도구를 시도하지 않거나 순서를 바꿀 수 있다. | MCP `a` 문구에서 2/3회 시도하지 않았다. H15, H18~H20은 시도한 회차만 분모로 했다. `edit_always` 1회는 편집을 2번만 해 H21이 보류가 됐다. |
| 내적 | 사용자 위치의 skills 등이 모델의 행동에 끼어든다. | 모델이 사용자 skills 파일 읽기를 시도한 명령 24개를 거절했다(21개 시험). 시험의 효과 판정에는 영향이 없었지만 모델의 행동 순서에는 섞였을 수 있다. |
| 내적 | 같은 짧은 지시를 3번 반복하므로 시험끼리 독립이 아닐 수 있다. | 시험마다 새 thread와 새 홈을 썼다. 같은 지시문과 모델 캐시의 영향은 배제하지 못했다. |
| 내적 | 읽기 전용 샌드박스 거부, 승인 거절, 훅 차단을 같은 실행 안 됨으로 오판한다. | 승인 요청, driver 응답, 명령 항목 상태, 거부 문구, 훅 로그를 따로 기록해 가렸다. 그래도 `hook_none`은 샌드박스가 가짜 값을 막아 훅 기준선이 되지 못해 샌드박스 없는 조건을 더했다. |
| 구성 | `sleep`, `echo`, `curl`, 작은 crate 한 개가 실제 부하를 대표하지 않는다. | 막힘의 일반 비율은 결론으로 내지 않았다. 같은 승인의 샌드박스 적용이 명령 모양에 따라 달라 한 명령의 결과도 일반화하지 못한다. |
| 구성 | 훅 시험은 Saturn 훅 명령과 Codex 훅 입력을 직접 이었을 뿐 engine이 Codex에 훅을 주입하는 코드는 시험하지 않는다. | engine은 지금 Codex에 훅을 넘기지 않는다. 결론을 신뢰된 훅 설정이 있을 때 Codex가 훅을 부르고 Saturn 판정을 따르는지로 한정했다. |
| 구성 | 가짜 키 항목은 사용자의 실제 router 키 항목과 접근 제어 목록이 다를 수 있다. | 판정은 훅 호출, 명령 실행 여부, 출력으로 했고 키체인 접근 창은 쓰지 않았다. 가짜 항목은 끝에 삭제했다(2/2). |
| 외적 | Codex CLI, 모델, app-server 스키마가 바뀌면 결과가 달라진다. | 버전 0.158.0, 모델, 실행 날짜, 원문 JSON-RPC를 기록했다. |
| 외적 | 키체인 로그인 사용자의 공유 동작은 새 로그인 없이는 만들 수 없다. | H26은 링크 없는 keyring 설정 홈까지만 확인했다. 실제 keyring 로그인 공유는 측정 불가로 남겼다. |
| 외적 | Claude 쪽 남은 항목(폴더 밖 읽기 실제 확인)은 이 실험 밖이다. | #348은 닫지 않았다. |

### 한계

- n이 조건마다 3이라 구간이 넓다. 3/3의 하한이 29.2%여서 "항상"이 아니라 "관측한 3회 모두"로 읽어야 한다.
- 모델 호출 116회 안에서 결과를 본 뒤 추가한 시험(30회)은 판정 규칙 없이 관측만 했다.
- driver가 승인 응답을 규칙으로 대신 보냈으므로 Saturn engine의 저장과 TUI 전달은 재지 않았다.
- `stop_parent`의 프로세스 수는 app-server 자손의 `ps` 표를 읽은 값이다. 종료 시각은 재지 않았고 10초 뒤까지만 봤다.
- 샌드박스 적용의 비일관성은 원인을 가리지 못했다. 모델의 호출 인자(login 셸 여부)는 원인으로 보이지 않았다(`/bin/zsh -c`에서도 성공과 거부가 갈렸다).

## 재현

```sh
./run.sh verify
./run.sh analyze
```

수집(`./run.sh collect`)은 실제 Codex를 호출하므로 호출 상한 120회를 쓰는 환경에서만 다시 돌린다. `analyze`는 `raw/`를 입력으로 전처리와 분석을 다시 실행하고 `results/`를 같은 바이트로 만든다.

| 파일 | SHA-256 |
|---|---|
| `data/SHA256SUMS` | `3bf62063c8d57e52db3fbb3cb45e046709bda24abd95c85ce5f913521771ccbd` |

## 결론

| 가설 | 판정 | 반영한 문서 |
|---|---|---|
| H1, H3, H8 | 채택: 자식 id와 threadId, 완료 순서, 승인 요청 3/3 | [provider 연결과 session](../../design/providers-and-sessions.md) |
| H2 | 채택: 자식 `thread/started` 0/3(자식을 만든 18회 모두 0) | [provider 연결과 session](../../design/providers-and-sessions.md) |
| H4, H5 | 채택: 기다리지 않는 부모가 먼저 끝남 3/3, 부모 interrupt는 자식을 멈추지 않음 3/3 | [provider 연결과 session](../../design/providers-and-sessions.md) |
| H6 | 기각: 자식 interrupt 뒤 프로세스 남음 3/3 | [provider 연결과 session](../../design/providers-and-sessions.md) |
| H7 | 기각: `multi_agent=false`에서 자식 생성 3/3, 탐색에서 `agents.max_depth=0`이 막음 | [권한](../../design/permissions.md) |
| H9~H14 | 채택: 끼워 넣기 수락, 거절 상태, 수신 확인 각 3/3 | [provider 연결과 session](../../design/providers-and-sessions.md), [입력 처리](../../design/input-handling.md) |
| H15~H17 | 채택: 시도한 MCP 호출 4/4 요청 도착, 도구 이름은 `message`에서만 10/10 | [권한](../../design/permissions.md) |
| H18, H19, H20 | 채택: MCP `accept` 3/3, `_meta.persist` 무효 3/3, 명령 `acceptForSession` 3/3 | [권한](../../design/permissions.md) |
| H21 | 보류: 편집 3번에 요청 2번 2/3 | [권한](../../design/permissions.md) |
| H22 | 기각: 승인한 `sh -c` 한 줄 쓰기가 6/6 성공, 탐색에서 모양에 따라 달랐음 | [권한](../../design/permissions.md) |
| H23, H24 | H23 채택: `cargo test` 거부 3/3, H24 보류: `unittest` 거부 문구 2/3 | [권한](../../design/permissions.md) |
| H25, H26 | 채택: 링크 공유 3/3, 링크 없는 keyring 설정 공유 안 됨 3/3 | [권한](../../design/permissions.md) |
| H27, H31 | 채택: 시작·재개 응답의 설정 값 6/6, `config/read` 미반영 6/6 | [provider 연결과 session](../../design/providers-and-sessions.md) |
| H28, H29 | 채택: 읽기 전용 `curl` 실패 3/3, 덮어쓰기 성공 3/3 | [provider 연결과 session](../../design/providers-and-sessions.md) |
| H30 | 기각: `thread/settings/updated` 알림 0/3 | [provider 연결과 session](../../design/providers-and-sessions.md) |
| H32 | 기각: 읽기 전용 샌드박스가 가짜 값을 막아 훅 없이 출력 0/3, 샌드박스 없으면 3/3 | [router 키 보호](../../design/router-key-security.md) |
| H33~H35 | 채택: 신뢰하지 않은 훅 미호출 3/3, 신뢰한 훅의 `security`·`cat` 차단 각 3/3 | [router 키 보호](../../design/router-key-security.md), [확장](../../design/extensions.md) |
| H36 | 채택: `apply_patch` 편집이 훅을 통과 3/3 | [router 키 보호](../../design/router-key-security.md) |
