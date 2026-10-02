# provider 권한 정본: 실험 결과

> [!NOTE]
> 사후 기록이다. 이슈 [#194](https://github.com/woonyong-choi/saturn/issues/194)의 결과 댓글 10개와 Codex 에이전트 보고서 6개를 근거로 썼고, 수치는 그 근거에 있는 것만 옮겼다. 실험 9의 원문 로그에서 직접 센 값은 표에 따로 밝혔다.

## 요약

Claude Code는 `--permission-prompt-tool stdio`와 `--settings`로 Bash 호출을 모두 호스트로 받았다(시도 1회씩, 사용자 `bypassPermissions`에서도). Codex는 전용 `CODEX_HOME`, `untrusted`, 읽기 전용 샌드박스, MCP 설정 번역을 합친 조합에서 셸, 파일 편집, subagent 명령이 승인 요청으로 왔다(3/3~5/5). MCP 묻기는 실험 6~8에서 불안정했다(실험 6: 2/3, 실험 7: 1/5, 실험 8: 시도 10/10 중 요청 6/10). 실험 9는 시도한 31회 모두 요청이 도착했지만 도착 시점이 도구 호출 시작 뒤 약 0~145초로 갈렸고 원인을 확정하지 못했다. 실험 10이 원인을 드라이버의 읽기 버그(`select()` 뒤 텍스트 `readline()` 버퍼링)로 확인했다. 실험 9의 늦은 16회는 모두 150초 마감의 `turn/interrupt` 1~4ms 뒤에 "보였다". 고친 드라이버에서는 MCP 승인 요청이 10/10회 도구 시작 직후(0.228~8.839ms) 도착했고 거부 5/5는 미실행, 승인 5/5는 실행됐다. 셸 승인도 3/3회 즉시(0, 0, 0.288ms) 왔다. 실험 8과 9의 "누락과 지연"은 Codex의 동작이 아니라 측정 도구의 결함이었다. 이제 Codex는 셸, 파일 편집, subagent, MCP 모두 승인 요청으로 받을 수 있음을 확인했다. 설계 문서와 결정 기록에는 반영하지 않았다.

## 방법

| 항목 | 값 |
|---|---|
| 설계 | [실험 설계](design.md). 사전 등록 없는 사후 기록이라 설계 커밋은 없다. |
| 실행 id | 없음. 실험 10개를 2026-10-02에 이슈 댓글 순서로 실행했고, 실행 id 형식의 원자료 파일은 없다. |
| 환경 | [env.json](env.json) |
| 표본 | 설계 크기는 [실험별 방법](design.md) 표. 실제 크기는 아래 결과 표의 회차 수. |

## 설계와 다른 점

사전 등록이 없어 설계와 비교할 수 없다. 지시서나 직전 실험의 계획과 다르게 진행된 것만 적는다.

| 변경 | 시점 | 이유 | 결론에 미친 영향 |
|---|---|---|---|
| 실험 5가 모델 호출 상한 40회를 넘겼다. 실제 44회(본시험 39회, 드라이버 수정 전 탐색 5회) | 수집 중 | 드라이버 수정 전 탐색 호출. 보고서가 스스로 적었다. | 호출 수만 공개한다. 판정에 쓴 회차 범위는 보고서에 적혀 있지 않다. |
| 실험 5의 9b(thread별 규칙 변경)에서 드라이버가 `thread/start.config`와 함께 최상위 `approvalPolicy=on-request`도 보냈다. | 수집 중 | 드라이버 구현 | 3회 모두 거부 요청이 왔지만 config 단독 효과를 가릴 수 없어 `확인 못 함`으로 판정했다. |
| 실험 5의 2번에서 `pattern=["*"]`는 어떤 명령도 매치하지 않았고 `pattern=[]`는 파싱 오류였다. | 수집 중 | execpolicy 문법 제약 | 모두 묻는 규칙을 표현하지 못해 `확인 못 함`. 실험 6이 `untrusted`로 보완했다. |
| 실험 6 지시서는 "안전 목록을 소스에서 찾아 모두 prompt로 적는다"였지만 소스에 고정 목록이 없었다. | 수집 중 | 전제 불일치 | 규칙에 적은 대표 명령(`ls`, `cat`, `head`, `wc`, `sort`, `node -e`, `touch`)으로 시험했다. 목록 전부는 `확인 못 함`. |
| 실험 8에서 decline 2에 보충 재시도 1회를 추가했다. | 수집 중 | 요청이 오지 않은 회차를 다시 확인했다. | 보충 재시도도 요청 없이 시간 초과였다. 회차 판정은 최초 5회 기준이다. |
| 실험 3과 4의 에이전트가 항목을 건너뛰었다. 상세는 한계 절 | 수집 중 | 에이전트 실수 | 건너뛴 항목은 `확인 못 함`으로 남았다. |
| 실험 9가 모델 호출 상한 30회를 넘겼다. 실제 36회(preliminary 4회, baseline 12회, `all_requests` 20회) | 수집 중 | `all_requests` 실행기가 이름 슬롯 10개마다 thread를 2개씩 남겼다. 보고서는 확인한 뒤 추가 호출을 멈췄다. | 호출 수만 공개한다. 호출이 늘어난 `all_requests` 후보 처리는 정식 판정을 내리지 않았다. |
| 실험 9의 지시서 3번(Codex 소스와 대조)을 수행하지 못했다 | 수집 중 | 보고서는 지시서가 네트워크와 저장소 사본을 금지했다고 해석해 소스를 열지 않았다. 설치된 0.158.0 바이너리로 생성한 schema만 확인했다. | 원인 분석이 로그 비교와 schema 확인에 그쳤다. 소스 경로와 줄은 없다. |
| 실험 9의 지시서 4번(거부 5회, 승인 5회 정식 재확인)의 판정을 내리지 않았다 | 수집 중 | 원인을 찾지 못했고 후보 처리는 실행기 중복과 호출 상한 초과가 겹쳤다. | 해결 확인 판정 없음. |
| 실험 9의 preliminary 4회 중 1회는 드라이버를 수동 종료한 불완전 로그라 정식 비교 표본에서 뺐다. | 수집 중 | 수동 종료 | 모델 호출 수에는 넣었고 판정 표본에서만 뺐다. |
| 실험 10의 지시서는 기준선 5회로 "지연이 다시 나오는지"를 보고, 이어서 원인에 맞는 해결 후보 5회를 돌리라고 했다. 기준선은 고친 드라이버로만 쟀고, 해결 후보는 따로 돌리지 않았다. 원래 드라이버로 지연을 다시 만들어 보지 않았다. | 수집 중 | 원인이 드라이버였다. 드라이버 수정이 곧 해결이라 Codex 설정은 바꾸지 않았다. 호출 13회로 끝났다(상한 30회). | 원인 판정이 실험 9 원문 로그의 도착 순서와 고친 드라이버의 즉시 도착 관측에 기댄다. 같은 조건의 대조는 없다. 한계 절에 적었다. |

## 결과

### 흐름

| 실험 | 모델 호출 | 비고 |
|---|---|---|
| 1. 실행 인자로 항상 묻기 | Claude 8회, Codex 8회 | 댓글 1 |
| 2. Codex 우회 방법 | Codex 6회 | 소스 조사와 실측. 댓글 2 |
| 3. 훅 신뢰성 | 기록 없음 | 에이전트가 한 번 멈췄다. 댓글 3 |
| 4. A안 | 기록 없음 | 댓글 4 |
| 5. 전용 `CODEX_HOME` | 44회 | 상한 40회 초과. 댓글 5 |
| 6. 구멍 막기 | 53회 | 부모 thread 48회, subagent 자식 thread 5회. 상한 60회 안. 댓글 6 |
| 7. MCP 준비 시점 | 60회 | 정식 반복 40회, 번역 실험과 초기 드라이버 확인 20회. 상한 70회 안. 댓글 7 |
| 8. MCP 묻기 경로 | 11회 | decline 5회, decline 2 보충 1회, accept 5회. 모델 호출 전에 끝난 준비 실행 2회는 제외. 상한 20회 안. 댓글 8 |
| 9. MCP 승인 요청 원인 찾기 | 36회 | preliminary 4회, baseline 12회, `all_requests` 20회(`turn/start` 기준). 상한 30회 초과. 댓글 9 |
| 10. 승인 요청 지연 원인 | 13회 | MCP 거부 5회와 승인 5회, 셸 승인 3회. thread 13개, 중복 없음. 상한 30회 안. 댓글 10 |

실험 6의 호출 수는 전용 home에 남은 rollout 기록을 한 번의 모델 호출로 센 값이다(보고서의 가정). 제외한 실행은 없다. 모델이 도구를 시도하지 않은 회차(시도 없음)는 아래 표에 따로 적었다.

### 실험 1. 실행 인자로 항상 묻기

[댓글 1](https://github.com/woonyong-choi/saturn/issues/194#issuecomment-5934801019). 시도마다 1회였다. 설정 파일은 고치지 않았고 폴더 설정은 실험 worktree 안에만 만들고 지웠다.

Claude Code 2.1.285.

| 시도한 인자 | 설정 허용 목록 | 물었나 | 결과 |
|---|---|---|---|
| 인자 없음, 사용자 `defaultMode=bypassPermissions`, 폴더 `allow Bash(echo:*)` | 있다 | 아니오 | 실행됨(`permissionMode=bypassPermissions`) |
| `--permission-mode manual` | 있음(폴더) | 아니오 | 실행됨. 폴더 allow가 모드보다 우선 |
| `--setting-sources "" --permission-mode manual` | 없음 | 아니오 | 실행됨. `echo`는 읽기 전용으로 자동 허용돼 판별력 없음 |
| `--permission-mode manual --settings '{"permissions":{"ask":["Bash"]}}'` | 있다 | 호스트에는 안 옴 | 자동 거부. `--permission-prompt-tool` 없이는 요청이 호스트로 오지 않는다 |
| `--permission-mode manual --permission-prompt-tool stdio --settings '{"permissions":{"ask":["Bash"]}}'` | 있음(`node`) | 예 | `control_request` `can_use_tool` 도착. `deny` 응답 시 미실행, `allow` 응답 시 실행(`1` 출력) |
| 위에서 `--permission-mode`를 뺌(사용자 bypass 유지) | 있음, bypass | 예 | `permissionMode=bypassPermissions`인데도 요청 도착, `deny`로 미실행 |

Codex 0.158.0(app-server, stdio JSON-RPC).

| 시도 | 허용 규칙 | 물었나 | 결과 |
|---|---|---|---|
| 사용자 설정 그대로(`on-request`, `workspace-write`), `node -e` | 없음 | 아니오 | 샌드박스 안에서 바로 실행 |
| 위와 같고 폴더 규칙 `node -e` allow | 폴더 | 아니오 | 실행 |
| `thread/start`의 `approvalPolicy:"untrusted"`, `node -e` | 없음 | 아니오 | 실행. `untrusted`가 모든 명령을 묻는 것은 아니다 |
| `untrusted`, `touch experiment-logs/probe.txt` | 없음 | 예 | `item/commandExecution/requestApproval` 도착, `decline` 시 `status:"declined"`, 파일 안 생김 |
| `untrusted`, `touch`, 폴더 규칙 `touch` allow | 폴더 | 아니오 | 실행됨, 파일 생김 |
| `untrusted`, `sort README.md`, 사용자 규칙 `["sort"]` allow | 사용자 | 아니오 | 실행됨 |
| `codex exec --ignore-rules -c approval_policy=untrusted` | 해당 없음 | 해당 없음 | `approval_policy = "untrusted" is no longer supported` 오류. 설정 키로는 불가, `thread/start` 파라미터로만 가능 |

판정: Claude는 조건부로 가능했다(묻고 싶은 도구 이름을 `ask`에 나열해야 한다). Codex는 사용자나 폴더 허용 규칙에 걸린 명령을 묻지 않고 실행했고 app-server에는 규칙을 무시하는 인자가 없었다.

승인 요청 메시지 예(Codex)는 `item/commandExecution/requestApproval`이고 `availableDecisions`에 `decline`이 없고 `cancel`이 있었다. `decline`도 받아들여져 `declined`로 처리됐다. Claude 요청 예는 `can_use_tool`이고 `decision_reason_type`이 `rule`이었다.

### 실험 2. Codex 우회 방법 조사와 실측

[댓글 2](https://github.com/woonyong-choi/saturn/issues/194#issuecomment-5935512797). 소스는 `openai/codex` `b527ce4`(2026-10-01 main)였고 저장소는 clone하지 않았다. 모델을 부른 Codex 호출은 6회였다.

| 후보 | 근거와 실측 | 허용 규칙 명령도 호스트가 판단 | 거부하면 막힘 |
|---|---|---|---|
| 1. 규칙 끄기(app-server 인자, `-c`, `thread/start.config`) | `--ignore-rules`는 `exec`에만 있다. 내부 플래그는 app-server 인자나 config 키로 노출되지 않는다. `codex features list` 전수에 규칙 관련 키 없음. 소스만 | 아니오 | 해당 없음 |
| 2. `approvalPolicy` 변경, `granular`, `approvalsReviewer` | 규칙이 `allow`면 정책과 무관하게 무승인 실행이고 샌드박스도 우회한다. 관리형 `requirements.toml`은 이기지만 전체 와일드카드가 없고 세션 단위가 아니다. 소스만 | 아니오 | 해당 없음 |
| 3. `CODEX_HOME` 분리 | 빈 `CODEX_HOME`으로 `account/read` 호출(모델 호출 없음)이 `account: null, requiresOpenaiAuth: true`. 인증 공유가 필요해 멈췄다. 분리하면 `.codex` 폴더 config, hooks, rules가 신뢰 전까지 꺼진다는 경고가 나왔다. | 미시험 | 미시험 |
| 4. 내장 셸 끄기와 `dynamicTools` | 호출 3회와 도구 목록 질의 1회. `features.shell_tool=false`와 `dynamicTools`로 폴더 규칙 `touch` allow가 있어도 모델의 `touch probe-e1.txt`가 `item/tool/call`로 호스트에 도착했다. 호스트가 실패 응답을 주면 파일이 안 생겼다. | 예 | 예 |
| 5. 세션 훅 | 호출 1회. `-c hooks.PreToolUse=[...]`와 `-c hooks.state={<key>={trusted_hash=<hash>}}`로 훅이 폴더 규칙 allow `touch`를 가로챘다. deny하자 파일이 안 생기고 `Command blocked by PreToolUse hook: denied by saturn hook`가 전달됐다. | 예 | 예 |
| 6. OpenCode 방식 | 소스만. Codex CLI나 app-server를 띄우지 않고 모델 API를 직접 호출하며, 자체 `shell` 도구가 매번 `ctx.ask`로 권한을 확인한다. 자체 OAuth와 `chatgpt.com/backend-api/codex/responses` 호출에 의존한다. | 해당 없음 | 해당 없음 |

후보 4의 채널 누수(모델이 셸이 막히자 시도한 다른 길)는 다음과 같았다.

| 채널 | 관찰 |
|---|---|
| 사용자 MCP 서버 `node_repl`의 `js` 도구 | `require('node:child_process').execSync('touch ...')` 시도. `require is not defined`로 실패. 승인 요청 없이 호출됨. `config.mcp_servers.<id>.enabled=false`로 끌 수 있었고 플러그인이 만든 서버(`cua_repl`)는 같은 방식이 `invalid transport`로 실패했다. |
| Computer Use(`cua_repl`) | 터미널 앱을 조작해 같은 명령을 시도했고 앱 안전 목록이 막았다. 기능 키 세 개를 꺼도 도구 목록에서 `mcp__cua_repl.js`가 남았다. |
| `apply_patch` | `item/fileChange/requestApproval`로 호스트에 도착, 거부하면 막혔다. |
| 코드 모드 `exec`(JS) | 동적 도구가 코드 모드 안쪽 도구로 노출된 단서가 있었으나 이유는 확인하지 못했다. 호출은 `item/tool/call`로 도착했다. |
| 서브에이전트 `spawn_agent` | 도구 목록에 있었다. 설정 상속은 시험하지 않았다. |

훅 방식의 함정은 점 표기 `hooks.state."<key>".trusted_hash=...`가 키의 `.` 때문에 쪼개져 `ignored` 경고와 함께 무시되고 인라인 테이블로만 적용된다는 점이었다. 사용자의 `~/.codex/hooks.json`(Orca 훅)과 플러그인 훅이 함께 실행됐고 훅 간 결과 합성 규칙은 확인하지 못했다. 공식 문서는 훅을 "a useful guardrail, not a complete enforcement boundary"라고 적는다.

판정: 방법 둘(훅, 내장 셸 끄기와 `dynamicTools`)이 규칙 allow 명령까지 호스트가 판단했다. 이 실험은 방법 둘을 각각 `touch` 한 가지 명령으로만 확인했다. 권장은 훅이었다. 사용자 규칙(`~/.codex/rules/`) 경로는 건드리지 않았다.

### 실험 3. 훅 신뢰성 반복 실측

[댓글 3](https://github.com/woonyong-choi/saturn/issues/194#issuecomment-5935746843). Haiku 에이전트가 훅을 `-c`로 주입하고 조건마다 3회 반복했다. Codex 0.158.0, macOS 25.5.0.

| 확인할 것 | 반복 | 결과 | 근거 |
|---|---|---|---|
| 1. 훅 대기 시간 초과 | 3회 | 실행됨 | `timeout=5초`, 훅 `sleep 30`. 3회 모두 시간 초과 뒤 도구가 실행됐다(fail-open). |
| 2a. 훅 오류: 종료 코드 1 | 3회 | 실행됨 | 종료 코드 2만 deny. 1은 정의되지 않아 실행됐다. |
| 2b. 훅 오류: 잘못된 JSON | 3회 | 실행됨 | JSON 파싱이 실패해도 도구가 실행됐다. |
| 2c. 훅 오류: 출력 없음 | 3회 | 실행됨 | 종료 코드 0에 출력 없음. 3회 모두 실행. 공식 정의는 없다. |
| 3. 훅 적용 확인 | 3회 | 감지됨 | `hooks/list` API로 훅 상태(신뢰 해시 일치) 확인 가능 |
| 4. subagent(`spawn_agent`) | 3회 | 실행됨(차단 불가) | `spawn_agent`가 PreToolUse 훅 이벤트를 지원하지 않는다(openai/codex#49736). |
| 5. 사용자 규칙 `sort` allow | 3회 | 막힘 | deny 훅이 allow 규칙을 이겼다. 3회 모두 같았다. |

통과 조건 5개 중 2개(적용 확인, 사용자 규칙)만 통과했다. 대기 초과, 오류, subagent는 막지 못했다. 확인 못 한 것은 `apply_patch` 훅 범위(openai/codex#16732, 훅 미지원 기록)와 코드 모드 `exec`(openai/codex#23411, 훅 미발화 기록)였고, 두 이슈 기록은 읽은 것이며 실행으로 확인하지 않았다. 권장사항으로 훅 주입 형식(인라인 테이블), Saturn engine의 추가 시간 제한 관리, `hooks/list`로 세션마다 등록 확인을 적었다.

### 실험 4. A안: 훅 ask에서 Codex 승인 경로로

[댓글 4](https://github.com/woonyong-choi/saturn/issues/194#issuecomment-5935985982). A안은 훅이 Saturn 규칙으로 즉시 답하고 `묻기`일 때 `permissionDecision: "ask"`를 돌려 Codex 기본 승인 요청(시간 제한 없음)으로 넘기는 경로였다. Haiku 에이전트가 시험했다.

| 확인할 것 | 결과 | 근거 |
|---|---|---|
| PreToolUse 훅의 ask 응답 | 미지원 | 스키마에 `Ask`가 정의돼 있으나 처리 코드가 "PreToolUse hook returned unsupported permissionDecision:ask"로 처리한다(`codex-rs/hooks/src/engine/output_parser.rs` 458~459행, main 기준). 지원 요청 이슈 openai/codex#28437이 열려 있다. |
| 폴더나 사용자 허용 규칙 명령의 ask 경로, 승인 대기 | `확인 못 함` | ask가 지원되지 않아 경로가 없다. |
| 즉시 허용과 거부, 오류 감싸기(종료 코드 2), 파일 편집(`apply_patch`) 훅 | `확인 못 함` | ask와 무관하게 시험할 수 있었으나 에이전트가 잘못 건너뛰었다. |
| subagent 안 명령의 훅 | `확인 못 함` | `spawn_agent`에 훅이 걸리지 않는다(openai/codex#49736, 실험 3). |

다음 후보로 훅이 즉시 답하고 `묻기`면 거부한 뒤 사람이 허락하면 한 번 허용으로 기록하고 다시 시키는 방식(C), `approvalPolicy: untrusted`와 함께 써서 허용 규칙에 걸린 명령에만 C를 적용하는 조합을 적었다. 두 후보는 시험하지 않았다.

### 실험 5. 전용 `CODEX_HOME`과 execpolicy 번역

[댓글 5](https://github.com/woonyong-choi/saturn/issues/194#issuecomment-5937218533), [보고서](data/raw/codex-194-home-report.md), 요약 줄은 [핵심 줄](data/raw/key-lines.md). `gpt-5.6-luna` 에이전트가 전용 폴더 두 개로 시험했다. 모델 호출은 44회였다. 조건마다 3회 반복했고 여러 하위 경로가 있는 조건은 경로별 3회로 셌다. 로그인은 `auth.json`을 복사하지 않고 원본을 가리키는 심볼릭 링크로 두 폴더에 연결했다.

| 확인할 것 | 회차별 관측 | 결과 |
|---|---|---|
| 1. 로그인 공유 | `account/read` 3회 모두 `account.type=chatgpt`, `planType=pro`, `requiresOpenaiAuth=true`. `auth.json` 두 개 모두 원본 링크 | 확인. 현재 계정은 파일 저장 방식. 키체인 방식은 시험하지 않았다 |
| 2. 모두 묻기(규칙) | `ls`, `cat`, `sort`, `touch`, `echo`, `node`를 대안 목록으로 묶으면 해당 6개만 `prompt`가 됐다. `pattern=["*"]`는 읽히지만 어떤 명령도 매치하지 않았고 `pattern=[]`는 파싱 오류였다. 모델이 명령을 묶거나 일부만 실행한 회차가 있어 명령별 3회가 아니다. | 확인 못 함(임의의 모든 명령 표현). 명시한 6개의 `prompt` 경로는 부분 확인 |
| 2. `approvalPolicy` | `on-request`에서 규칙 `prompt`에 걸린 명령이 `item/commandExecution/requestApproval`로 왔고 `untrusted`도 같았다. `on-request`만으로 안전 명령 전체를 묻는지는 확인하지 못했다. | 확인(규칙 prompt의 호스트 요청 경로). 모두 묻기는 `확인 못 함` |
| 3. `allow`와 `forbidden` | `echo` allow 3회 모두 승인 0회, 실행 완료. `touch` forbidden 3회 모두 실행 항목, 승인 요청, 파일이 없었다. | 확인 |
| 4. 사용자와 프로젝트 규칙 무시 | 사용자 `sort` allow와 충돌하도록 전용 home에 `sort` prompt를 두자 3회 모두 승인 요청이 왔다. 프로젝트 `.codex/rules`의 `sort` allow도 전용 home config에 신뢰 목록이 없을 때 3회 모두 전용 home prompt가 우선했고 project-local rules disabled 경고가 났다. | 확인 |
| 5. 승인 대기 | 3회 모두 prompt 요청 뒤 91초 동안 응답하지 않았다. `approval-held`, 실행 완료 없음, hold 파일 없음 | 확인 |
| 6. 파일 편집 | `apply_patch`로 파일을 만들게 한 3회 모두 `item/fileChange/requestApproval` 없이 완료. 파일 3개가 실제로 생겼다. | 확인: 이 설정(`workspace-write`)에서 파일 편집은 승인 요청으로 오지 않는다 |
| 6. MCP | 안전한 로컬 `saturn_probe.probe`를 3회 호출. 모두 완료, 승인 0회. `mcpToolCall` item은 보였으나 승인 요청은 없었다. | 확인: 일반 MCP 호출은 execpolicy prompt 경로가 아님 |
| 7. subagent | 3회 중 2회는 subagent 명령 실행 항목이 없었고 1회는 `touch`가 전용 prompt로 요청돼 거부됐다. | 불안정 |
| 8. 사용자 설정 이관 | 권한 키를 뺀 전용 config(모델, reasoning, service tier, 안전한 로컬 MCP)로 3회 정상 thread와 turn, MCP 호출. 실행 중 project trust 항목이 전용 config에 추가돼 `projects.*.trust_level`이 남았다. | 부분 확인. 전체 이관은 `확인 못 함` |
| 9a. app-server 둘 | home A의 `echo` prompt와 home B의 `touch` forbidden을 각각 3회. A는 요청 뒤 거부, B는 승인 없이 금지. 프로세스가 서로 다른 home 규칙을 썼다. | 확인 |
| 9b. app-server 하나의 thread별 규칙 | 드라이버가 최상위 `approvalPolicy=on-request`도 보내 3회 모두 요청 뒤 거부. config override 단독 효과는 판별력이 없다. execpolicy 파일 선택 키는 찾지 못했다. | `확인 못 함` |

사용자 config에서 읽은 권한 관련 키는 `approval_policy`, `sandbox_mode`, `hooks`, `hooks.state`, `shell_environment_policy`, `projects.*.trust_level`이었고 규칙은 `~/.codex/rules` 별도 파일이었다. 보고서는 전용 config로 옮기지 말아야 할 키로 `approval_policy`, `sandbox_mode`, `sandbox_workspace_write.*`, `approvals_reviewer`, `cli_auth_credentials_store`, `hooks`, `hooks.state`, 규칙 파일, `projects.*.trust_level`, `shell_environment_policy.*`, 권한을 내포한 profile 설정을 적었고 `notify`와 외부 MCP endpoint도 외부 효과 때문에 그대로 옮기지 않는 편이 안전하다고 적었다.

결론(보고서): 전용 `CODEX_HOME`과 execpolicy만으로 Codex의 모든 실행을 판단할 수 없었다. 보장된 범위는 전용 home 규칙에 명시적으로 매치되는 셸 prefix였다. 남은 구멍은 모든 명령을 매치하는 규칙 부재, `apply_patch`, MCP, subagent 불일치, 키체인 로그인 미확인이었다. 채팅마다 규칙이 다르면 규칙마다 별도 app-server와 별도 `CODEX_HOME`을 권장했다.

### 실험 6. 구멍 막기

[댓글 6](https://github.com/woonyong-choi/saturn/issues/194#issuecomment-5937919511), [보고서](data/raw/codex-194-plug-report.md). 전용 home 설정은 `approvalPolicy=untrusted`(`thread/start`), 전용 `config.toml`의 `approval_policy="on-request"`, `sandbox_mode="workspace-write"`, MCP 서버와 도구별 `prompt` 설정, 명령을 모두 `prompt`로 적은 `rules/default.rules`였다. 모델 호출은 53회(부모 48회, subagent 자식 5회)였다.

| 확인할 것 | 회차별 관측 | 결과 |
|---|---|---|
| 1. 안전 목록과 무해 명령 | 소스에 고정된 known-safe 명령 목록이 없었다. 규칙에 적은 `ls`, `cat`, `head`, `wc`, `sort`는 각각 3/3회 `requestApproval`이 왔다. `node -e "console.log(1)"`과 worktree 안 `touch`도 각각 3/3회 요청이 왔고 모두 거부했다. | 목록 전부는 확인 못 함. 규칙에 적은 대표 명령은 확인(각 3/3) |
| 2. 파일 편집과 읽기 전용 샌드박스 | `read-only`에서 `apply_patch`는 3/3회 `item/fileChange/requestApproval`, 거부 뒤 파일 미생성. 같은 샌드박스의 `touch`도 3/3회 요청, 거부 뒤 파일 미생성. 일반 읽기와 테스트의 통과 여부는 독립 표본으로 재지 않았다. | 편집과 셸 쓰기 차단은 확인. 일반 작업 불편 정도는 `확인 못 함` |
| 3. MCP | 소스에 서버 기본 승인 모드와 도구별 `approval_mode`가 있었다. `mcp/1`, `mcp/2`는 `mcpServer/elicitation/request`가 왔고 거부됐다. `mcp/3`은 도구 접근 불가로 끝났다. 서버 ready를 기다린 보충 `mcp-controlled/1-3`도 모두 도구 접근 불가였고 중계 서버의 `tools/call`은 한 번도 도달하지 않았다. | 불안정(2/3 요청, 1/3 도구 없음). 중계 거부까지의 실행은 `확인 못 함` |
| 4. subagent | 5회 모두 자식 thread가 생성됐다. 5/5회 자식이 실행한 `touch`에 `requestApproval`이 왔고 모두 거부됐다. `subagent-*.txt`는 생성되지 않았다. 실행 없음 회차는 0회 | 확인(5/5) |
| 5. 종합 시나리오 | `composite/1`은 셸 읽기, 셸 쓰기, 파일 편집이 요청되고 MCP는 요청 없음. `composite/2`는 네 종류 모두 요청. `composite/3`은 `composite/1`과 같았다. 요청된 항목은 모두 거부됐고 파일은 생성되지 않았다. | 네 종류가 매 회 요청된다는 조건은 불안정(네 종류 모두 1/3). 요청된 셸, 편집, MCP는 거부로 막혔다 |

소스 확인에서 읽은 것은 다음과 같다. 미매칭 명령은 `UnlessTrusted`(`untrusted`)에서 승인 요청이 되고 `OnRequest`는 샌드박스, 위험도, 경로 조건으로 허용될 수 있다. `UnlessTrusted`에서 patch는 즉시 `AskUser`다. MCP에는 `AppToolApproval::{Auto, Prompt, Writes, Approve}`와 서버 기본 모드, 도구별 `approval_mode`가 있다. 자식 설정 생성 코드는 부모의 approval policy, cwd, permission profile을 반영한다. multi-agent 기능 플래그(`[multi_agent].enabled`, `features.multi_agent`, `features.multi_agent_v2`)는 끄지 않고 기록만 했다.

결론(보고서): 이 설정 조합만으로 셸, 파일 편집, MCP, subagent를 모두 매번 판단한다고 확인할 수 없었다. 남는 구멍은 MCP 도구 목록과 시작 시점에 따른 접근 불안정, 소스에 고정 안전 목록이 없다는 전제 불일치, 읽기 전용에서 일반 작업 통과성을 측정하지 않은 점이었다.

### 실험 7. MCP 준비 시점

[댓글 7](https://github.com/woonyong-choi/saturn/issues/194#issuecomment-5938826077), [보고서](data/raw/codex-194-mcp-report.md), 회차 줄은 [핵심 줄](data/raw/key-lines.md). 기본 공용 준비 유예 `mcp_optional_startup_grace_ms`는 1000(1초)이었고, 그 안에 서버가 도구 목록을 반환하지 않으면 첫 `thread/start`와 첫 턴에서 도구가 빠질 수 있었다. 모델 호출은 60회였고 조건마다 5회 반복했다. 사용자 MCP 서버의 도구 호출은 0회였다. 사용자 MCP 설정은 전용 config에 넣되 `enabled_tools=[]`, `default_tools_approval_mode="prompt"`, `required=false`를 적용했다.

| 조건 | 회차별 관측 | 결과 |
|---|---|---|
| 기준선, 3초 지연 | 5회 모두 서버 process 1, `ready` 1, `tools/list` 1. 첫 턴의 fixture 호출은 0/5 | 준비 상태와 첫 턴 도구 사용이 연결되지 않아 확인 불가 |
| 기준선, 10초 지연 | `ready`와 `tools/list`가 2/5, 나머지 3/5는 서버가 준비되지 않은 상태로 관측. 첫 턴 fixture 호출 0/5 | 불안정 |
| 후보 A(준비 조회), 3초 지연 | 첫 턴 전에 thread 없는 `mcpServerStatus/list`. 5/5회 `echo_tool`, `write_like_tool` 목록 확인. 준비 조회용과 thread용 프로세스가 각각 떠 fixture process 2개. `echo_tool` 호출 5/5 | 확인 |
| 후보 A, 10초 지연 | 5/5회 상태 조회에서 두 도구 목록 확인. 모델의 `echo_tool` 호출은 2/5 | 서버 준비는 확인, 모델의 매번 호출은 불안정 |
| 후보 B(`mcp_optional_startup_grace_ms=12000`), 10초 지연 | 5/5회 `ready`와 `tools/list` 확인. 모델의 `echo_tool` 호출은 4/5 | 서버 준비는 확인, 모델의 매번 호출은 불안정 |
| 후보 C | 더 안정적인 후보를 찾지 못했다. 준비 조회와 공용 유예 조합을 권장안으로 골랐다. | `확인 못 함` |

번역표 실측(5회씩, 준비 조회를 켠 상태)은 다음과 같았다.

| Saturn 규칙 | MCP 설정 | 5회 관측 | 결과 |
|---|---|---|---|
| 허용 | `default_tools_approval_mode="approve"` 또는 도구별 `approval_mode="approve"` | 준비 조회 뒤 호스트의 `mcpServer/tool/call` 5/5 성공, elicitation 0회. 모델이 `echo_tool`을 고른 것은 3/5 | 설정과 호스트 경로는 확인. 모델 선택은 불안정 |
| 묻기 | 도구별 `approval_mode="prompt"` | 모델 전용 5회에서 `mcpServer/elicitation/request` 1/5, fixture 호출 0/5. 요청은 드라이버가 거부했다. | 불안정. 매번 묻기는 `확인 못 함` |
| 거부 | `disabled_tools=["write_like_tool"]` 또는 `enabled_tools` allow-list에서 제외 | 5/5회 상태 목록에서 `write_like_tool`이 빠졌고, 직접 호출은 5/5회 disabled 오류, fixture 호출 0회 | 확인 |

보조 실험에서 `prompt`를 둔 `write_like_tool`에 호스트가 `mcpServer/tool/call`로 직접 5회 호출했고 5회 모두 실행됐다. 테스트 드라이버가 승인 경계를 우회한 관측이고, 모델 경로에서 승인 없이 실행됐다는 뜻이 아니다. 승인 없이 실행된 MCP 호출은 사용자 MCP 서버 0회, 정상 모델 경로의 `prompt` 도구 0회, 직접 호출 우회 5회, `approve` 경로의 의도된 무승인 실행 5회였다. 권장 동작은 첫 `thread/start` 전에 서버별 `mcpServerStatus/list`로 `runtimeStatus`, `toolsError`, `tools`를 확인하고 일치할 때만 첫 턴을 보내는 것이었다. 직접 tool-call API를 외부 입력에 노출하지 않고 모델 경로의 `mcpServer/elicitation/request`를 중계하라고 적었다.

### 실험 8. MCP 묻기 경로

[댓글 8](https://github.com/woonyong-choi/saturn/issues/194#issuecomment-5941681428), [보고서](data/raw/codex-194-prompt-path-report.md). 전용 `CODEX_HOME`, `approvalPolicy="untrusted"`, 첫 턴 전 `mcpServerStatus/list`, `mcp_optional_startup_grace_ms=12000`, `startup_timeout_sec=30`을 썼다. 사용자 MCP 서버는 넣지 않았고 `mcpServer/tool/call` 직접 호출도 하지 않았다. 모델 호출은 11회였다.

| 회차 | 시도 여부 | 승인 요청 | 답 | 실행 기록 | 판정 |
|---|---|---|---|---|---|
| decline 1 | 있다 | 있다 | decline | 없음 | 요청 후 미실행 |
| decline 2 | 있다 | 없음 | 없음(시간 초과) | 없음 | 승인 경계 관측 불가 |
| decline 3 | 있다 | 있다 | decline | 없음 | 요청 후 미실행 |
| decline 4 | 있다 | 없음 | 없음(시간 초과) | 없음 | 승인 경계 관측 불가 |
| decline 5 | 있다 | 있다 | decline | 없음 | 요청 후 미실행 |
| accept 1 | 있다 | 없음 | 없음(시간 초과) | 없음 | 승인 경계 관측 불가 |
| accept 2 | 있다 | 없음 | 없음(시간 초과) | 없음 | 승인 경계 관측 불가 |
| accept 3 | 있다 | 있다 | accept | 있다 | 실행 |
| accept 4 | 있다 | 있다 | accept | 있다 | 실행 |
| accept 5 | 있다 | 있다 | accept | 있다 | 실행 |

시도 없음은 0회였다. 시도한 회차 기준으로 decline은 승인 요청 3/5회, accept도 3/5회였고, 요청이 온 경우에는 decline 3/3회 미실행, accept 3/3회 실행이었다. 판정은 불안정이다(도구 시도 10/10회, 승인 요청 6/10회). 요청이 오지 않은 회차에서도 도구는 실행되지 않았다(실행 기록 없음).

### 실험 9. MCP 승인 요청 원인 찾기

[댓글 9](https://github.com/woonyong-choi/saturn/issues/194#issuecomment-5942476740), [지시서](data/raw/codex-194-rootcause-prompt.md), [보고서](data/raw/codex-194-rootcause-report.md), 회차 요약은 [결과 요약](data/raw/codex-194-rootcause-results.jsonl)과 [thread 요약](data/raw/codex-194-rootcause-thread-summary.json), 원문 줄은 [핵심 줄](data/raw/key-lines.md). `gpt-5.6-luna` 에이전트가 실험 8과 같은 방법(전용 `CODEX_HOME`, `approvalPolicy="untrusted"`, 첫 턴 전 `mcpServerStatus/list`, `mcp_optional_startup_grace_ms=12000`, `write_like_tool` 하나뿐인 서버)으로 시험하면서 app-server와 주고받은 모든 JSON-RPC 원문과 stderr를 회차마다 기록하고 모든 서버 요청의 응답 여부를 적었다. 사용자 MCP 서버와 `mcpServer/tool/call` 직접 호출은 쓰지 않았다. 모델 호출은 36회였다.

| 구분 | 회차 | 도구 시도 | 승인 요청 도착 | 호스트 응답 | fixture 실행 | 주요 순서와 판정 |
|---|---|---|---|---|---|---|
| baseline decline | 1~5 | 5/5 | 5/5 | 5/5 decline | 0/5 | `mcpToolCall` 시작, `waitingOnApproval`, `mcpServer/elicitation/request`, decline 순서. 요청 뒤 완료까지 간 회차와 시간 초과 회차가 섞였다. |
| baseline accept | 1~5 | 3/5 | 3/3 | 3/3 accept | 기록 있다 | accept 2와 5는 도구 시도 없이 idle로 끝났다. 시도한 회차는 모두 요청이 도착했다. |
| baseline 보충 accept | 6~7 | 2/2 | 2/2 | 2/2 accept | 0/2 | 완료 전 150초 경계에서 끝난 회차. 요청 자체는 도착했다. |
| `all_requests` 후보 검증 | decline 1~5, accept 1~5 | 19/20 | 19/19 | 19/19 | 일부 completed | 미응답 서버 요청 0건. 슬롯 10개마다 thread가 2개 남아 20회로 기록됐다. |

시도한 회차 기준으로 승인 요청은 정식 비교 표본 31회 모두 도착했다(보고서 값). 요청이 없던 정식 회차는 모델이 `mcpToolCall`을 시작하지 않고 idle로 끝난 회차였다. 실험 8의 "도구 시도 뒤 요청 없이 시간 초과" 4회는 재현되지 않았다.

도착 시점은 도구 호출 시작 직후부터 약 0~145초까지 갈렸다. baseline decline 공식 5회는 약 `141510, 143478, 1, 143635, 1`ms였고 보충 accept 2회는 약 `123786, 127521`ms였다.

요청 종류와 응답 여부는 다음과 같다.

| 항목 | 관측 |
|---|---|
| 승인 요청 이름 | 실제 도구 시도 회차에서 `mcpServer/elicitation/request` 하나뿐이었다. |
| 다른 이름의 승인 요청 | `item/commandExecution/requestApproval`, `item/fileChange/requestApproval` 모두 0건 |
| 서버 요청 응답 | 관측한 서버 요청은 모두 id `0`이었고 모두 호스트 응답이 있었다. 미응답 서버 요청 0건 |
| 서버 준비 | thread마다 `mcpServer/startupStatus/updated`가 `ready`였고 준비 조회에 `write_like_tool`이 있었다. startup 미완료는 관측되지 않았다. |
| 결과 처리 | decline 응답 뒤 fixture 실행 기록은 없었다. accept 회차는 일부만 `item/completed.status=completed`와 fixture 기록이 남았고, 일부는 승인 뒤 시간 초과 경계에서 completed가 남지 않았다. |

원문 로그에서 센 도착 순서(보고서에 없는 값)는 다음과 같다. `experiment-logs/baseline/`과 `experiment-logs/all_requests/`의 회차별 로그에서 도구를 시도한 thread마다 `mcpToolCall` 시작, 승인 요청 도착, 드라이버가 보낸 `turn/interrupt`의 시각을 비교했다.

| 구분 | 도구를 시도한 thread | 요청이 `turn/interrupt` 없이 도착 | 요청이 드라이버의 `turn/interrupt` 1~4ms 뒤 도착 |
|---|---:|---:|---:|
| baseline | 10 | 3 | 7 |
| `all_requests` | 19 | 10 | 9 |

- `turn/interrupt` 없이 도착한 13회는 도구 시작 뒤 0~2ms에 왔고, `all_requests` 1회만 약 7.9초였다.
- 드라이버는 150초 마감에 닿으면 `threadId`만 담은 `turn/interrupt`를 보냈다. 이 요청은 모두 `Invalid request: missing field turnId` 오류로 거부됐다(19/19).
- 도착이 100초 넘게 늦은 회차는 모두 이 `turn/interrupt` 1~4ms 뒤에 도착했다. `turn/interrupt` 바로 앞 줄은 `waitingOnApproval` 상태 변경(12회)이나 도구 시작(4회)이었고, 그 줄과 `turn/interrupt` 사이에 다른 줄이 없었다. 줄 예시는 [핵심 줄](data/raw/key-lines.md)이다.
- 이 표의 합 29는 보고서 본문의 정식 시도 31회와 맞지 않는다. 보고서는 차이를 설명하지 않았고, 2회가 어느 로그인지 확인하지 못했다.

원인 분석 결과는 다음과 같다.

| 확인한 것 | 결과 |
|---|---|
| 다른 이름의 승인 요청을 놓쳤는가 | 증거 없음 |
| 호스트가 응답하지 않은 다른 서버 요청이 막았는가 | 증거 없음. 미응답 서버 요청 0건 |
| 서버 준비 미완료 | 관측 없음 |
| 실험 8의 4회 누락 재현 | 재현 못 함. 시도한 회차는 모두 요청이 도착했다. |
| 늦게 도착한 요청의 원인 | `확인 못 함`(실험 10에서 드라이버 버그로 확인) |

늦은 도착의 원인 후보는 세 가지이고 어느 것도 시험하지 않았다(실험 10에서 둘째 후보인 드라이버 읽기 버그로 확인했다. [실험 10](#실험-10-승인-요청-지연-원인)).

- 실험 8의 드라이버 시간 초과가 이 지연(최대 약 145초)보다 짧았을 가능성이 있다. 다만 실험 8 accept 1의 에이전트 메모는 180초 대기로 적혀 있어 이 가능성과 맞지 않고, 나머지 3회의 대기 시간은 기록이 없다. 보고서도 원인을 확정하지 않았다.
- 실험 9 드라이버가 stdout을 `selectors`로 기다린 뒤 텍스트 모드 `readline()`으로 읽는 구조라, 이미 도착한 줄이 읽기 버퍼에 남아 다음 입력이 올 때까지 처리되지 않았을 가능성이 있다. 늦은 도착이 모두 호스트 쓰기 직후라는 관측과 맞지만, 드라이버 코드를 읽어 세운 후보이고 확인하지 않았다. 실험 8 드라이버는 유실돼 같은 구조였는지 알 수 없다.
- Codex 쪽 내부 지연일 가능성이 있다. 소스를 열지 못해 배제도 확인도 못 했다.

Codex 소스(`codex-rs/core/src/mcp_tool_call.rs`, `codex-rs/codex-mcp/`, app-server 승인과 elicitation 구현)는 열지 못했다. 생성 schema에서 확인한 계약은 다음과 같다.

| schema | 내용 |
|---|---|
| `ServerRequest.json` 1905~1945줄 | 명령, 파일 승인 요청 |
| `ServerRequest.json` 1980~2002줄 | `mcpServer/elicitation/request` |
| `McpServerElicitationRequestResponse.json` | 응답의 필수 `action`, 값은 `accept`, `decline`, `cancel` |
| `CommandExecutionRequestApprovalResponse.json`, `FileChangeRequestApprovalResponse.json` | 다른 승인 요청의 응답 계약 |

후보 처리 재확인은 모든 서버 요청에 즉시 JSON-RPC 응답을 보내는 `all_requests` 드라이버로 했다. 미응답 요청은 0건이었고 elicitation은 도구를 시도한 19/19회에 도착했다. 이 후보가 누락을 고쳤다고 확인할 수 없다. 실행기 중복으로 슬롯 10개가 thread 20개가 됐고 호출 상한도 넘어, 거부 5회와 승인 5회의 정식 확인 판정은 내리지 않았다.

판정: 확인 못 함. 원자료는 비공개 폴더에 있고 경로와 SHA-256은 [데이터](data/README.md)에 적었다. 실험 10에서 원인이 드라이버의 읽기 버그로 확인됐다. 이 절의 "원인 미확정" 서술은 실험 9 시점의 기록으로 남긴다.

### 실험 10. 승인 요청 지연 원인

[댓글 10](https://github.com/woonyong-choi/saturn/issues/194#issuecomment-5942820548), [지시서](data/raw/codex-194-delay-prompt.md), [보고서](data/raw/codex-194-delay-report.md), [결과 요약](data/raw/codex-194-delay-results-summary.json), 드라이버 발췌는 [핵심 줄](data/raw/key-lines.md). `gpt-5.6-luna` 에이전트가 실험 9 원문 로그에서 얻은 단서(늦은 16회가 모두 `turn/interrupt` 1~4ms 뒤에 도착)를 먼저 검증했다. 실험 9 드라이버(`run_trial_original.py`)를 비차단 바이트 읽기와 줄 큐로 고치고(`run_trial.py`), 회차 마감을 150초에서 300초로 늘리고, 줄마다 원시 바이트 수신 시각(`rawByteReceivedAtNs`)을 기록했다. 나머지 조건은 실험 9와 같았다(전용 `CODEX_HOME`, `untrusted`, `write_like_tool` 하나뿐인 서버, 유예 12초). 모델 호출은 13회(MCP 10회, 셸 3회), 회차마다 thread 하나였고 중복 thread는 없었다. Codex 소스는 읽기만 했고(`openai/codex` `064c6b8`, 태그 `rust-v0.158.0`) clone하지 않았다.

실험 9의 늦은 요청 재확인(보고서 값).

| 항목 | 값 |
|---|---|
| 늦게 도착한 승인 요청 | 16회 |
| 도구 시작부터 도착까지 | 119,608~145,452ms |
| 드라이버의 `turn/interrupt` 뒤 도착까지 | 모두 1~4ms |

MCP 승인, 고친 드라이버. 지연은 `item/started`의 `mcpToolCall`에서 `mcpServer/elicitation/request`까지이고 원시 바이트 수신 시각 기준이다.

| 회차 | 거부 지연(ms) | 거부 후 실행 | 승인 지연(ms) | 승인 후 실행 |
|---:|---:|---|---:|---|
| 1 | 8.839 | 없음 | 0.583 | 있다 |
| 2 | 0.253 | 없음 | 0.312 | 있다 |
| 3 | 0.228 | 없음 | 0.270 | 있다 |
| 4 | 0.308 | 없음 | 0.357 | 있다 |
| 5 | 0.240 | 없음 | 0.738 | 있다 |
| 합계 | 승인 요청 5/5 | 0/5 | 승인 요청 5/5 | 5/5 |

미응답 서버 요청은 0건이었다. 거부는 fixture 호출 기록이 없었고, 승인은 `tool-calls.jsonl`에 기록이 있었다.

셸 명령 승인, 같은 드라이버.

| 회차 | 요청 이름 | `commandExecution` 시작부터 요청까지(ms) | 응답 | 실행 |
|---:|---|---:|---|---|
| 1 | `item/commandExecution/requestApproval` | 0.000 | accept | 실행 |
| 2 | `item/commandExecution/requestApproval` | 0.000 | accept | 실행 |
| 3 | `item/commandExecution/requestApproval` | 0.288 | accept | 실행 |

MCP 전체의 지연은 0.228~8.839ms, 셸은 0~0.288ms였다. 모두 100ms 미만이다(보고서의 "즉시" 기준).

소스 근거는 보고서가 열어 본 경로와 줄이다(`064c6b8`).

| 경로와 줄 | 확인 내용 |
|---|---|
| [`core/src/mcp_tool_call.rs:263-292`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/mcp_tool_call.rs#L263-L292) | `mcpToolCall` 시작 알림을 보낸 뒤 바로 승인 경로(`maybe_request_mcp_tool_approval`)로 들어간다. |
| [`core/src/mcp_tool_call.rs:1476-1525`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/mcp_tool_call.rs#L1476-L1525), [`1542-1585`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/mcp_tool_call.rs#L1542-L1585), [`1653-1693`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/mcp_tool_call.rs#L1653-L1693) | 승인 reviewer를 계산하고, 자동 승인이 아니면 `ApprovalAction::McpToolCall`로 `request_approval`을 기다리고, elicitation을 구성해 `request_mcp_server_elicitation`을 호출한다. |
| [`core/src/tools/approvals.rs:505-569`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/tools/approvals.rs#L505-L569), [`572-676`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/tools/approvals.rs#L572-L676) | 승인 순서는 hook, Guardian 조건부 검토, 사용자 승인이다. Guardian 경로는 결정이 날 때까지 기다린다. |
| [`ext/guardian-reviewer/src/routing.rs:54-62`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/ext/guardian-reviewer/src/routing.rs#L54-L62), [`lib.rs:40-42`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/ext/guardian-reviewer/src/lib.rs#L40-L42) | `on-request`나 granular 정책에서 `approvals_reviewer=auto_review`일 때만 Guardian으로 간다. 검토 기한은 90초다. |
| [`core/src/config/config_tests.rs:11840-11852`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/config/config_tests.rs#L11840-L11852) | 설정에 `approvals_reviewer`가 없으면 기본값은 `user`다(upstream 테스트가 확인). |
| [`core/src/session/mcp.rs:557-628`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/core/src/session/mcp.rs#L557-L628), [`app-server/src/outgoing_message.rs:330-444`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/app-server/src/outgoing_message.rs#L330-L444) | app-server 이벤트를 보내고 응답 channel을 기다린다. 서버 요청을 보내는 경로에 사용자 응답 전의 140초 대기는 없다. |
| [`codex-mcp/src/rmcp_client.rs:339-397`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/codex-mcp/src/rmcp_client.rs#L339-L397), [`656-680`](https://github.com/openai/codex/blob/064c6b8c737f5b41d171fdda80bd9ef10ad06eb3/codex-rs/codex-mcp/src/rmcp_client.rs#L656-L680) | 서버 시작은 `startup_timeout_sec`로 제한된다. 도구 목록 조회는 별도 `tools/list` 경로이고 승인 요청 이후 지연의 근거가 아니다. |

가설별 판정은 다음과 같다(보고서 값).

| 가설 | 판정 | 근거 |
|---|---|---|
| H-a 자동 검토자가 먼저 검토한 뒤 사용자에게 전달 | 기준선의 원인 아님 | 소스상 `auto_review`일 때만 가능하다. 이번 설정은 기본값 `user`이고, 10/10회 즉시 도착했고 90초 Guardian 기한도 관측되지 않았다. |
| H-b MCP 서버 응답, `tool_timeout_sec`, 연결 확인 대기 | 기준선의 원인 아님 | 시작과 목록 경로는 별도 시간 제한이다. `mcpToolCall` 시작 뒤 승인 요청까지 10회 모두 0.228~8.839ms였다. |
| H-c 모델 turn과 stream 순서 | 기준선의 원인 아님 | 소스 순서가 시작, 승인 경로, elicitation이고 고친 읽기에서 매회 즉시 도착했다. |
| 드라이버의 `select()`와 텍스트 `readline()` 버퍼링 | 확인 | 늦은 16회가 모두 `turn/interrupt` 1~4ms 뒤에 도착했고, 바이트 단위 비차단 읽기에서 지연이 사라졌다. |

버그 구조는 다음과 같다. 원래 드라이버는 `select()`가 읽을 수 있다고 알릴 때마다 텍스트 모드 `readline()`을 한 번 불렀다. `readline()`이 한 번의 OS read로 여러 줄을 파이썬 버퍼에 미리 읽으면, 다음 줄이 이미 도착했어도 fd에는 읽을 것이 없어 `select()`가 깨어나지 않는다. 그 줄은 다음 입력이 올 때까지 처리되지 않았고, 실험 9에서는 150초 마감에 드라이버가 `turn/interrupt`를 쓰면서 그 입력이 생겼다. 이 메커니즘은 보고서의 설명이고, 원래 드라이버로 지연을 다시 만들어 본 것은 아니다. 발췌한 코드 줄은 [핵심 줄](data/raw/key-lines.md)이다.

고친 설정은 드라이버 읽기 방식과 마감(300초)뿐이다. Codex 설정은 바꾸지 않았다. 이번 fixture 설정은 `approval_mode="prompt"`, `startup_timeout_sec=30`, `tool_timeout_sec=30`, `mcp_optional_startup_grace_ms=12000`이었다. 보존한 원자료와 SHA-256은 [데이터](data/README.md)다. 인증 원본은 복사·이동·수정하지 않았다.

판정: 확인. 지연의 원인은 Codex가 아니라 실험 9 드라이버다.

## 방향이 바뀐 이유

| 단계 | 시도 | 실패하거나 모자란 것 | 다음 시도 |
|---|---|---|---|
| 1에서 2 | 실행 인자로 항상 묻기 | Codex는 사용자나 폴더 허용 규칙에 걸린 명령을 묻지 않고 실행했고 app-server에 규칙 무시 인자가 없었다. | 규칙 allow 명령까지 판단하는 방법을 소스와 실측으로 조사 |
| 2에서 3 | 후보 6개 조사 | 훅과 `dynamicTools`는 됐지만 훅의 대기 초과, 오류, subagent, `apply_patch`는 측정하지 않았다. `CODEX_HOME` 분리는 인증 공유가 필요해 멈췄다. | 훅 신뢰성을 조건마다 3회 반복 |
| 3에서 4 | 훅 신뢰성 | 훅이 시간 초과와 오류에서 fail-open이었고 subagent 생성에는 적용되지 않았다(통과 2/5). | 훅이 `ask`로 Codex 승인 경로에 넘기는 A안 확인 |
| 4에서 5 | A안 | 훅 `ask`가 소스상 지원되지 않았다. 후보 C(거부 뒤 한 번 허용)는 시험하지 않았다. | 실험 2에서 보류한 `CODEX_HOME` 분리를 시험. 로그인 심볼릭 링크는 지시서에 사용자 승인으로 적혀 있다. |
| 5에서 6 | 전용 `CODEX_HOME`과 execpolicy | 모두 묻는 규칙을 표현하지 못했고, `apply_patch`와 MCP가 승인 없이 실행됐고, subagent가 불안정했다. | `untrusted`, 읽기 전용 샌드박스, MCP 서버별 `approval_mode`, subagent 5회로 구멍 4개를 막는 시험 |
| 6에서 7 | 구멍 막기 | MCP가 3회 중 1회 도구 접근 불가였다. | MCP 서버 시작과 도구 목록 시점을 조사하고 준비 조회, 유예 늘리기, 번역표 시험 |
| 7에서 8 | MCP 준비 시점 | 준비는 5/5 확인했지만 `prompt` 묻기가 모델 전용 5회에서 1/5였다. | 준비 조회와 유예 12초를 켠 상태에서 묻기 경로만 거부 5회와 승인 5회로 시험 |
| 8에서 9 | MCP 묻기 경로 | 도구 시도 10/10회, 승인 요청 6/10회로 불안정 | 모든 JSON-RPC 원문을 기록하고 재현해 원인을 찾는 실험 9 |
| 9에서 10 | 원인 찾기 | 시도한 31회 모두 승인 요청이 도착해 4회 누락을 재현하지 못했다. 도착 시점이 약 0~145초로 갈린 원인은 `확인 못 함`. 원문 로그를 다시 세니 늦은 16회가 모두 드라이버의 `turn/interrupt` 1~4ms 뒤에 도착했다. | 드라이버를 비차단 바이트 읽기로 고쳐 거부 5회, 승인 5회, 셸 3회를 다시 재고 Codex 소스로 승인 경로를 대조하는 실험 10 |
| 10 이후 | 지연 원인 | 원인은 드라이버의 읽기 버그로 확인했다. 고친 드라이버에서 MCP 승인 요청 10/10회와 셸 3/3회가 즉시 도착했고 거부와 승인이 맞게 동작했다. | 이 시리즈는 여기서 마친다. 남은 것 절에 후속 시험을 적었다. |

### 확인 분석

| 가설 | 판정 | 근거 |
|---|---|---|
| H1 | Claude 채택(조건부), Codex 기각 | Claude 시도 2건에서 모든 Bash 호출이 `can_use_tool`로 왔다. Codex는 허용 규칙 명령을 묻지 않고 실행 |
| H2 | 채택 | 방법 둘(세션 훅, 내장 셸 끄기와 `dynamicTools`)이 각 1가지 명령으로 확인됨 |
| H3 | 기각 | 통과 조건 5개 중 2개(각 3회) |
| H4 | 기각 | 훅 `ask` 미지원. 나머지 항목은 `확인 못 함` |
| H5 | 기각 | 보장 범위는 규칙에 명시한 셸 prefix뿐. 모든 실행 판단 불가 |
| H6 | 보류 | 셸 대표 명령 3/3, 파일 편집과 subagent 확인. MCP 2/3 불안정, 종합 1/3. 실험 10 이후 MCP 누락은 드라이버 결함일 가능성이 있으나 실험 6의 종합 시나리오는 다시 재지 않아 판정을 바꾸지 않았다. |
| H7 | 부분 채택 | 준비 5/5, 허용과 거부 5/5, 묻기 1/5 불안정 |
| H8 | 채택(실험 10 기준. 실험 9까지는 보류였다) | 실험 8은 시도 10/10, 요청 6/10이었고 실험 9는 도착 31/31이지만 시점이 약 0~145초로 갈려 보류였다. 실험 10의 고친 드라이버에서 시도한 회차 모두 요청이 즉시 도착했다(MCP 10/10, 0.228~8.839ms). 거부 5/5는 미실행, 승인 5/5는 실행이라 판정 기준(시도한 거부와 승인 각 5회 이상이 모두 요청, 거부 시 미실행, 승인 시 실행)을 만족한다. 시도 없음 0회. |
| H9 | 원인 확인: 드라이버 버그 | 다른 이름의 승인 요청 0건, 미응답 서버 요청 0건, 서버 준비 미완료 관측 없음은 실험 9 그대로다. 실험 9의 늦은 도착 16회는 모두 `turn/interrupt` 뒤 1~4ms에 왔고 실험 10이 읽기 버그로 확인했다(소스 `064c6b8` 대조 포함). 실험 8의 4회 누락은 실험 9와 10에서 재현되지 않았지만, 실험 8 드라이버가 유실돼 같은 버그였는지 직접 확인하지 못했다. |
| H10 | 확인 | 늦은 16회 모두 `turn/interrupt` 1~4ms 뒤, 고친 드라이버에서 MCP 10/10과 셸 3/3이 100ms 미만. 가설 H-a(자동 검토자), H-b(MCP 서버 대기), H-c(모델 turn 순서)는 기준선의 원인 아님. |

## 논의

### 해석

Claude Code와 Codex 모두 실행 인자나 전용 설정으로 Saturn이 요청을 받는 경로는 있었다. Codex는 provider 규칙이 인자를 이기기 때문에 전용 `CODEX_HOME`으로 사용자와 폴더 규칙을 제거해야 했고, 그 위에 `untrusted`와 읽기 전용 샌드박스가 셸과 파일 편집을 요청으로 바꿨다. MCP 묻기는 실험 6~9까지 불안정해 보였다. 요청이 오지 않은 회차에서도 도구가 실행되지 않았으므로 요청 누락이 승인 없는 실행으로 이어진 관측은 없었다. 실험 10이 이 불안정의 원인을 Codex가 아니라 측정 도구로 돌렸다. 실험 9 드라이버가 이미 도착한 줄을 읽지 못해 요청이 150초 마감 때에야 "보였고", 읽기를 고치자 MCP 승인 요청이 도구 시작 0.228~8.839ms 뒤에 매번 왔다. 소스에서도 도구 시작 뒤 승인 경로와 elicitation이 바로 이어지고, 사용자 응답 전의 별도 대기는 없다. 자동 검토자(Guardian)는 `approvals_reviewer=auto_review`일 때만 타고 기본값은 `user`다. 그래서 Saturn이 생성하는 config에는 `approvals_reviewer="user"`를 명시해 사용자 기본값이 바뀌어도 검토 경로가 끼어들지 않게 하는 것이 맞다(시험하지 않은 제안이다). 실험 8의 4회 누락은 실험 9와 10에서 재현되지 않았고, 실험 8 드라이버가 유실돼 같은 버그였는지는 직접 확인하지 못했다. 사용자 대기 시간에 주는 영향은 재지 않았다.

### 타당성 위협

| 종류 | 위협 | 이 실험에서 |
|---|---|---|
| 내적 | 모델 응답에 의존한다. | 실험 6의 `mcp/3`, 실험 7의 모델 선택 2/5, 실험 8의 요청 6/10처럼 같은 설정에서 회차마다 결과가 달랐다. 실험 8의 요청 6/10은 드라이버 결함이 원인이었을 가능성이 크지만 실험 8 드라이버가 유실돼 확인하지 못했다. 실험 6과 7의 MCP 결과(2/3, 1/5)도 드라이버가 유실돼 같은 영향이 있었는지 알 수 없다. |
| 내적 | 실험 9의 호출 수와 실행기가 계획을 벗어났다. | 상한 30회를 넘긴 36회가 들었고, `all_requests` 실행기 중복으로 슬롯 10개가 thread 20개가 됐다. 호출 수는 공개했고 후보 처리는 정식 판정에서 뺐다. |
| 내적 | 실험 9의 드라이버 읽기 방식과 `turn/interrupt` 호출이 관측에 영향을 줬다. | 늦게 도착한 요청 16회가 모두 `turn/interrupt` 직후였고 이 호출은 19/19 오류로 거부됐다. 실험 10이 드라이버를 고쳐 지연이 사라지는 것을 확인했다. 실험 9 시점에는 후보였다. |
| 내적 | 실험 10은 원래 드라이버로 지연을 다시 만들지 않았다. | 원인 판정은 실험 9 원문 로그의 도착 순서(16/16)와 고친 드라이버의 즉시 도착(13/13)에 기댄다. 같은 조건의 대조는 없다. |
| 구성 | 실험 10의 "즉시"는 에이전트가 정한 기준(원시 바이트 수신 시각 100ms 미만)이다. 실제 관측은 모두 9ms 이하라 기준 선택이 결론을 바꾸지 않는다. | 지연을 그대로 표에 적었다. |
| 외적 | 실험 10도 한 장치, 한 버전(Codex 0.158.0, 소스 `064c6b8`)이고 `approvals_reviewer`는 기본값(`user`)이었다. | `auto_review` 설정에서는 Guardian 경로가 있어 이 결과가 적용되지 않는다. 시험하지 않았다. |
| 구성 | 정식 시도 수의 출처가 맞지 않는다. | 보고서 본문 31회, 원문 로그에서 센 값 29회. 차이를 풀지 못했다. |
| 외적 | 실험 9에서 Codex 소스를 열지 못했다. | 생성 schema만 확인했다. 실험 10에서 소스(`064c6b8`)를 열어 승인 경로를 대조했고 내부 지연의 근거는 없었다. |
| 내적 | 실험 에이전트의 실수다. | 실험 3의 인용 오류, 실험 4의 건너뛴 항목, 실험 5의 호출 상한 초과와 판별력 없는 9b가 있었다. |
| 구성 | `asked`는 호스트에 온 요청이고 보안 경계의 완전성이 아니다. | 거부 뒤 파일 생성과 호출 기록으로 실행 여부를 따로 봤다. 요청이 오지 않은 MCP 회차는 `승인 경계 관측 불가`로만 셌다. |
| 구성 | 호스트의 직접 호출 경로(`mcpServer/tool/call`)는 승인 경계를 우회한다. | 실험 7에서 5/5 실행됐고 실험 8은 이 경로를 쓰지 않았다. |
| 외적 | 한 사용자, 한 장치, 한 버전이다. | 사용자 규칙은 `sort` allow만 직접 확인했다. 키체인 로그인과 관리형 설정은 시험하지 않았다. Codex 0.158.0, Claude Code 2.1.285 기준(2026-10-02 확인) |

### 한계

- 에이전트 실수가 섞였다. 실험 3은 에이전트가 한 번 멈췄고, 인용 일부가 틀렸다. 실행 전 훅 근거로 실행 후 훅 이슈(openai/codex#34289, #46455)를 인용했다. 같은 댓글의 `apply_patch`와 코드 모드 훅 항목(#16732, #23411)은 이슈 기록을 읽은 것이고 실행하지 않았다. 훅 대기 초과와 오류 항목은 공식 문서 인용과 3회 실측이 함께 적혔으나, 실측만 판정 근거로 읽어야 한다.
- 실험 4는 즉시 허용과 거부, 오류 감싸기(종료 코드 2), 파일 편집 훅 항목을 에이전트가 잘못 건너뛰었다. 이 항목들은 지금도 확인 못 함이다.
- 원자료 일부가 유실됐다. 실험 1~4의 회차별 원문 메시지와 스크립트, 실험 5~8의 드라이버와 회차별 관측 파일(`matrix-results.json`, `observations.jsonl`, `results.jsonl` 등)은 실험 worktree와 함께 지워졌다. 보고서가 인용한 줄 번호는 지금 확인할 수 없다. 남은 것은 [데이터](data/README.md)에 적었다.
- 실험 5가 모델 호출 상한 40회를 넘겼다(44회).
- 실험 9도 모델 호출 상한 30회를 넘겼다(36회). `all_requests` 실행기가 슬롯마다 thread를 2개씩 남겨 후보 처리 표본이 슬롯 10개가 아니라 thread 20개로 기록됐다. 이 처리의 정식 확인 판정은 내리지 않았다.
- 실험 9는 Codex 소스를 열지 못해 어디서 멈추는지 소스로 대조하지 못했다. 지연된 도착의 원인은 실험 9 시점에는 확인 못 함이었고 실험 10에서 드라이버 버그로 확인했다.
- 실험 8과 9의 "누락과 지연" 관측은 측정 도구(드라이버)의 결함이었다. 실험 9의 지연 16회는 확인했다. 실험 8의 요청 누락 4회는 드라이버가 유실돼 같은 결함이었는지 직접 확인하지 못했고, 실험 9와 10에서 재현되지 않았다. 실험 6과 7의 MCP 결과도 같은 영향이 있었는지 알 수 없다. 그러므로 실험 6~9의 MCP 불안정 수치(2/3, 1/5, 6/10, 지연 0~145초)는 Codex의 동작을 재는 근거로 쓰기 어렵다. 실험 9의 지연만 원인을 확인했고 나머지는 미확인이다.
- 실험 10은 원래 드라이버로 지연을 다시 만들지 않았다. 버그 메커니즘(`select()` 뒤 `readline()` 버퍼링)은 보고서의 설명과 코드 읽기에 기댄다.
- 실험 10은 `approvals_reviewer`를 기본값으로 두었고 `auto_review`는 시험하지 않았다. 셸 승인은 3회뿐이다.
- 실험 9의 정식 시도 수 31회는 원문 로그에서 센 29회와 다르다. `turn/interrupt` 직후 도착 비율은 원문 로그 기준 값이고 보고서와 댓글에는 없다.
- 실험 9의 드라이버가 늦은 도착에 영향을 줬는지는 실험 10에서 확인했다. 실험 8의 드라이버는 유실돼 같은 구조였는지 알 수 없다.
- 반복이 3~10회다. 통계 검정과 신뢰구간을 계산하지 않았다.
- 실험 6의 모델 호출 수는 rollout 기록을 한 번으로 센 값이다. 같은 실험의 실행 로그에는 `safe-command/ls/1`이 요청 0건으로 중단된 앞선 시도(종료 코드 130)가 남아 있고, 이 시도가 53회에 들어갔는지는 보고서에 없다. 보고서의 3/3은 이후 재실행이다.
- 실험 2의 에이전트 모델은 기록이 없고, 실험 3과 4의 호출 수도 기록이 없다.

## 지금까지의 결론

| provider | 구성 | 상태 |
|---|---|---|
| Codex | 전용 `CODEX_HOME`(사용자와 폴더 규칙 제외, 로그인은 심볼릭 링크), `thread/start`의 `approvalPolicy="untrusted"`, 읽기 전용 샌드박스, MCP 설정 번역(허용 `approve`, 묻기 `prompt`, 거부 `disabled_tools`나 `enabled_tools`), 첫 턴 전 `mcpServerStatus/list`와 유예 늘리기 | 셸 대표 명령, 파일 편집, subagent는 확인(3/3~5/5). MCP 묻기도 확인: 고친 드라이버에서 요청 10/10이 도구 시작 직후(0.228~8.839ms) 도착했고 거부 5/5 미실행, 승인 5/5 실행(실험 10). 셸 승인 3/3 즉시. 이전의 MCP 불안정과 지연은 드라이버 읽기 버그였다. 이제 셸, 파일 편집, subagent, MCP 모두 확인했다. 네 경로를 한 시나리오에서 함께 다시 재지는 않았다. |
| Claude Code | `--permission-prompt-tool stdio`와 `--settings '{"permissions":{"ask":[...]}}'` | Bash는 확인(사용자 bypass에서도). 다른 도구는 시험하지 않았다 |

채팅마다 규칙이 다르면 규칙마다 별도 app-server와 별도 `CODEX_HOME`을 쓴다(실험 5의 9a 확인). 단일 app-server의 thread별 규칙 변경은 확인하지 못했다.

## 남은 것

- Claude의 편집과 쓰기 도구, MCP 도구, `ask: ["*"]` 와일드카드 시험
- 읽기 전용 샌드박스의 일반 작업 불편 정도 측정
- 키체인 로그인(`cli_auth_credentials_store`)에서의 로그인 공유와 토큰 갱신
- Saturn이 생성하는 Codex config에 `approvals_reviewer="user"`를 명시하고 자동 검토자 경로가 끼지 않는지 확인
- 실험 6의 종합 시나리오(네 경로 한 번에)를 고친 드라이버로 다시 측정
- 모든 명령을 매치하는 규칙의 표현 방법과 안전 목록 전체 묻기
- 훅의 `apply_patch`와 코드 모드 `exec` 범위 실측
- 관리형(managed) 설정이 있을 때의 결과

## 재현

원본 스크립트와 회차별 원자료가 유실돼 `run.sh`, `scripts/`, `results/`가 없고 `./run.sh verify`와 `./run.sh analyze`를 실행할 수 없다. 재현 절차는 [실험별 방법](design.md)의 표다. 남은 근거 파일의 무결성만 확인할 수 있다.

```sh
cd docs/experiments/provider-permission-gating/data
shasum -a 256 -c SHA256SUMS
```

| 파일 | SHA-256 |
|---|---|
| `data/SHA256SUMS` | `5f37c26298c7d29d31c59a22f5759055ec99c64103f85eaa2306c7c3c67c53e1` |

## 결론

| 가설 | 판정 | 반영한 문서 |
|---|---|---|
| H1 | Claude 채택, Codex 기각 | 없음 |
| H2 | 채택 | 없음 |
| H3 | 기각 | 없음 |
| H4 | 기각 | 없음 |
| H5 | 기각 | 없음 |
| H6 | 보류 | 없음 |
| H7 | 부분 채택 | 없음 |
| H8 | 채택(실험 10) | 없음 |
| H9 | 원인 확인: 드라이버 버그(실험 10) | 없음 |
| H10 | 확인 | 없음 |

실험 시리즈는 여기서 마친다. H6(보류)과 H7(부분 채택)은 실험 10에서 다시 재지 않았고, 키체인 로그인과 Claude의 다른 도구 등 남은 것이 있어 설계 문서와 결정 기록에는 반영하지 않았다.
