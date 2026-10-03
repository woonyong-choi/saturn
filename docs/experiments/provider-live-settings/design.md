# provider 설정 즉시 적용: 실험 설계

| 항목 | 값 |
|---|---|
| 이슈 | 없음: 사용자가 이슈를 만들지 않도록 지정했다 |
| 관련 설계 | [권한](../../design/permissions.md), [provider 연결과 session](../../design/providers-and-sessions.md), [설정](../../design/settings.md) |
| 사전 데이터 | 없음. 지정한 기존 문서와 코드만 읽고, provider 실행 결과와 schema는 수집 전 열람하지 않는다 |

## 질문

실행 중인 Claude Code stream-json session과 Codex app-server thread가 provider를 다시 시작하지 않고 설정 변경을 받아들이는 공식 요청·인자·파일 재읽기 경로가 있는지 확인한다. 각 경로가 실제 도구 호출의 허용·거부, 입력 요청, 다음 턴 반영으로 이어지는지도 함께 확인해 Saturn의 즉시 적용 범위를 정한다.

## 가설

| 가설 | 예측 | 적용 범위 |
|---|---|---|
| H1 | Claude의 공식 control request 중 `set_permission_mode`는 실행 중 session에서 성공하고, 다음 도구 호출의 권한 판단이 바뀐다. | Claude Code stream-json, `claude-haiku-4-5-20251001`, 2026-10-03 실행 버전 |
| H2 | Claude에는 `set_permission_mode` 외에 실행 중 설정·도구 허용·거부 목록을 바꾸는 공식 control subtype이 없거나, 있어도 같은 요청의 실제 행동 변경으로 이어지지 않는다. | CLI help·설치 bundle 문자열·control request, Claude Code 2026-10-03 실행 버전 |
| H3 | Claude의 특정 도구는 설정 재읽기 없이도 `can_use_tool` 응답을 호출마다 `deny`·`allow`로 바꾸어 막고 다시 열 수 있다. | `AskUserQuestion`, 같은 stream-json process, 3회 |
| H4 | Codex `turn/start`의 schema가 허용하는 턴 덮어쓰기 값은 해당 턴의 행동을 바꾸지만 thread의 다음 턴 설정을 영구 변경하지 않는다. | Codex app-server, `gpt-5.6-luna`, 전용 `CODEX_HOME`, 2026-10-03 실행 버전 |
| H5 | Codex app-server의 설정 읽기·쓰기 또는 reload 요청이 실행 중 thread의 `config.toml`, feature, execpolicy, MCP 승인 설정을 다시 읽게 하지 않는다. | 생성 schema에 나타난 설정 관련 요청과 실제 실행 행동, Codex 2026-10-03 |
| H6 | Codex `config.toml`과 `rules/default.rules`의 외부 파일 변경은 같은 app-server의 다음 턴에 반영되지 않고, 새 process에서만 반영된다. | 전용 `CODEX_HOME`, 파일 변경 뒤 같은 thread의 다음 턴과 재시작 뒤 첫 턴 |

## 설계

| 항목 | 값 |
|---|---|
| 조건 | Claude: 초기 권한 mode, `set_permission_mode` control request, 발견한 설정·도구 허용·거부 subtype, `AskUserQuestion` 호출별 거부·허용. Codex: `turn/start` override 후보, schema의 설정 관련 request, `config.toml`·`rules/default.rules` 변경 뒤 다음 턴, process 재시작 뒤 첫 턴 |
| 배정 | 각 경로의 3회는 고정된 trial 순서로 실행한다. provider의 출력 변동이 목적이 아니므로 무작위화하지 않고, trial id와 요청 순서를 기록한다 |
| 눈가림 | 해당 없음. 요청 subtype과 파일 변경 조건을 driver가 알아야 한다. 행동 판정은 원문 provider 이벤트와 fixture의 marker 파일·호출 기록을 함께 대조한다 |
| 환경 | macOS, Python 3.9 이상, Saturn worktree, 설치된 Claude Code와 Codex CLI의 실제 버전, Claude `claude-haiku-4-5-20251001`, Codex `gpt-5.6-luna`. 실행 시 `env.json`에 실제 값을 기록한다 |
| 공통 안전 | 실험 전용 Codex home은 worktree `.runtime/` 아래에 만들고 원본 `~/.codex/auth.json`만 심볼릭 링크한다. 사용자 `~/.codex`·`~/.claude` 설정은 쓰지 않는다. 모든 fixture 파일은 worktree 안에 둔다. 토큰·인증값·개인 절대 경로는 raw에서 가린다 |
| Codex 읽기 | app-server stdout은 텍스트 `readline()`을 쓰지 않고 파일 설명자를 비차단으로 읽어 바이트 버퍼에서 줄을 만들며, 한 번 읽은 바이트에서 생긴 모든 줄을 큐에 넣고 원시 바이트 수신 시각을 기록한다 |

### Claude 실행

각 trial은 새 stream-json process를 하나만 띄우고 session id를 고정한다. help와 bundle에서 control subtype 목록을 수집한 뒤, `set_permission_mode`와 설정·도구 허용·거부를 뜻하는 후보를 각각 3회 실제 control request로 보낸다. `set_permission_mode`는 권한 mode를 바꾼 뒤 Bash 호출을 유도해 `can_use_tool`의 허용 결과와 marker 실행을 확인하고, 동일 요청을 턴 진행 중과 턴 경계에서 나누어 보낸다. `AskUserQuestion`은 첫 호출을 `deny`, 다음 호출을 `allow`로 응답해 process 재시작 없이 도구가 막혔다가 다시 열리는지 확인한다.

### Codex 실행

`codex app-server generate-json-schema` 결과에서 `turn/start`의 추가 필드와 설정 관련 request schema를 수집한다. `turn/start` override는 필드별로 3회 실행하고, approval·sandbox 차이가 실제 승인 요청과 marker 실행에 나타나는지 확인한다. 설정 관련 request는 전용 home 안의 `config.toml`·`rules/default.rules`만 대상으로 3회 보내며, 성공 응답만으로 재읽기를 판정하지 않고 다음 턴의 실행·승인·MCP 호출 행동을 확인한다. 파일 변경 조건은 시작 후 파일을 바꾼 뒤 같은 thread에서 다음 턴, 이어 새 app-server process에서 같은 요청을 보내 비교한다.

## 변수

| 변수 | 종류 | 정의 | 단위 |
|---|---|---|---|
| `provider` | 조작 | `claude`, `codex` | 없음 |
| `route` | 조작 | 공식 request·override·파일 변경 경로의 이름 | 없음 |
| `trial_id` | 조작 | route별 1, 2, 3 | 없음 |
| `process_id` | 측정 | 설정 변경 전후 provider OS process 식별자 | 문자열 |
| `request_result` | 측정 | control/RPC 요청의 성공 응답·오류·미지원 | 값 목록 |
| `tool_request` | 측정 | 설정 변경 뒤 provider가 실제로 보낸 도구 호출 | 문자열 |
| `tool_decision` | 측정 | `allow`, `deny`, `not_requested`, `approval` | 값 목록 |
| `fixture_effect` | 측정 | marker 파일 또는 tool-call 기록의 실제 변경 | 불리언 |
| `next_turn_effect` | 측정 | 변경 직후 다음 턴의 행동이 달라졌는지 | 불리언 |
| `restart_effect` | 측정 | provider process를 새로 띄운 뒤 행동이 달라졌는지 | 불리언 |
| `repeat_consistent` | 파생 | 같은 route의 세 회차 결과가 모두 같으면 1 | 불리언 |
| `classification` | 파생 | 세 회차가 모두 같으면 `확인`, 하나라도 다르면 `불안정`, 실행할 수 없으면 `확인 못 함` | 값 목록 |

## 표본

| 항목 | 값 |
|---|---|
| 출처 | 설치된 provider CLI help·bundle, `codex app-server` 생성 schema, worktree 안의 고정 fixture와 provider 원문 이벤트 |
| 크기 | 확인 대상 route마다 3회. Claude 모델 호출은 40회, Codex 모델 호출은 40회를 상한으로 둔다 |
| 크기 근거 | 사용자가 지정한 반복 수와 provider 모델 호출 예산 |
| 중단 규칙 | Claude 또는 Codex 모델 호출이 각각 40회에 닿으면 해당 provider 수집을 멈춘다. 인증 오류, 토큰 누출, 전용 home 밖 쓰기, 연속 3회 process 시작 실패가 발생하면 해당 route를 `확인 못 함`으로 남기고 안전하게 종료한다 |
| 반복과 예열 | 예열 없음. 각 trial은 새 Claude process이며, Codex 파일 변경 비교는 한 app-server process 안에서 세 번 반복하고 재시작 비교 process를 별도로 둔다 |

## 분석

| 가설 | 지표 | 방법 | 채택 기준 |
|---|---|---|---|
| H1 | `set_permission_mode` 성공률과 다음 도구 행동 | 세 회차 원문 response, process id, `can_use_tool` 결정, marker 효과를 대조 | 세 회차 모두 요청 성공과 행동 변경이면 `확인`, 결과가 갈리면 `불안정`, 요청을 보낼 수 없으면 `확인 못 함` |
| H2 | 발견 subtype별 response와 행동 | help·bundle·실제 요청을 세 회차씩 대조 | 세 회차가 모두 같은 subtype 목록·같은 행동이면 `확인`, 다르면 `불안정`, 실행 불가면 `확인 못 함` |
| H3 | `AskUserQuestion` 거부·허용과 실제 호출 효과 | 첫 호출과 두 번째 호출의 control request·response·후속 marker를 세 회차 대조 | 세 회차 모두 첫 호출은 막히고 두 번째는 열리면 `확인`, 아니면 `불안정` 또는 `확인 못 함` |
| H4 | override별 응답·승인·marker 효과·다음 턴 효과 | schema와 세 회차 실제 turn 이벤트를 대조 | 세 회차가 모두 같은 필드별 효과를 보이면 `확인`, 다르면 `불안정`, 실행 불가면 `확인 못 함` |
| H5 | 설정 관련 request 결과와 행동 | request response만이 아니라 다음 턴 도구 결정과 fixture 효과를 대조 | 세 회차 모두 재읽기 행동이 없으면 H5 `확인`, 일부만 있으면 `불안정`, request schema·실행이 없으면 `확인 못 함` |
| H6 | 같은 process 다음 턴과 새 process 첫 턴의 차이 | process id, 파일 hash, 승인 요청, marker 효과를 세 회차 비교 | 세 회차 모두 다음 턴 미반영·재시작 후 반영이면 `확인` |

| 항목 | 규칙 |
|---|---|
| 제외 기준 | provider 실행 실패, 원문 파싱 실패, fixture 오류, worktree 밖 쓰기, 비밀값 검출 회차는 제외하지 않고 실패 흐름에 센다. 실패한 route는 판정에서 `확인 못 함`으로 둔다 |
| 실패한 실행 | 같은 trial을 재시도하지 않는다. 원문과 종료 상태를 보존하고 다음 trial로 진행한다 |
| 다중 비교 | 수치 추정이나 유의성 검정을 하지 않는다. route별 반복 일치 판정만 사용한다 |

| 결과 | 설계에 반영 |
|---|---|
| 확인 | 보고서의 provider별 설정 적용 표와 Saturn 설계 문서의 적용 시점 제안에 반영한다 |
| 불안정 | provider 버전 범위와 불안정 조건을 설계 문서에 남기고 즉시 적용을 보장하지 않는다 |
| 확인 못 함 | 확인되지 않은 공식 경로로 표시하고 추가 실행 조건을 보고서에 남긴다 |

## 탐색 분석

- Claude control response의 subtype·error message와 bundle에만 있는 후보 이름
- Codex schema의 request/notification 이름, `turn/start` override 필드와 실제 응답의 적용값
- 파일 변경 직후 provider가 읽은 설정 fingerprint 또는 행동 순서
- provider process id가 설정 변경 전후에 유지되는지

## 타당성 위협

| 종류 | 위협 | 대응 |
|---|---|---|
| 내적 | 모델이 요청한 도구를 매번 호출하지 않을 수 있다. | 실제 provider 이벤트와 fixture 효과를 함께 기록하고, 도구를 호출하지 않은 회차는 설정 적용 성공으로 세지 않는다 |
| 내적 | 대화 중 설정 변경과 턴 경계 변경이 섞일 수 있다. | 변경 시점을 원문 시각과 turn id로 기록하고 조건을 분리한다 |
| 내적 | driver의 stdout 버퍼링이 승인 요청 시점을 바꿀 수 있다. | Codex는 비차단 바이트 읽기·줄 큐를 쓰고 Claude도 줄 원문과 수신 시각을 기록한다 |
| 구성 | 한 도구의 행동이 전체 설정 재읽기를 대표하지 않을 수 있다. | Claude는 permission mode와 AskUserQuestion, Codex는 approval·sandbox·execpolicy·MCP 후보를 별도로 보고 범위를 제한한다 |
| 외적 | CLI와 모델 버전이 바뀌면 공식 schema와 bundle이 달라질 수 있다. | 실제 버전·실행 날짜·schema hash를 기록하고 결론을 해당 환경으로 한정한다 |
| 외적 | 전용 Codex home과 인증 심볼릭 링크가 일반 사용자 home과 다르다. | Saturn 설계와 같은 전용 home 조건을 명시하고 사용자 설정 파일은 수정하지 않는다 |
