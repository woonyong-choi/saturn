# 결정 전 provider 네 가지 동작: 실험 설계

| 항목 | 값 |
|---|---|
| 이슈 | [#252](https://github.com/woonyong-choi/saturn/issues/252) |
| 관련 설계 | [맥락 정리](../../design/context-management.md), [provider 연결과 session](../../design/providers-and-sessions.md), [입력 처리](../../design/input-handling.md), [권한](../../design/permissions.md) |
| 사전 데이터 | [인계 패킷 품질 v2](../handoff-packet-quality-v2/report.md)의 시나리오 생성기, 질문 유형, 자동 채점, provider 실행 방식을 재사용한다. [provider 권한 gating](../provider-permission-gating/report.md)의 app-server stdio 경계와 바이트 비차단 읽기·줄 큐 드라이버 규칙을 재사용한다. 이 실험의 답과 원문 이벤트는 수집 전에 보지 않는다. |

## 질문

맥락 초과, 불확실한 재개, 입력 요청, 중단 표시의 실제 동작을 측정해 Saturn의 전환·권한·상태 표시 계약을 정한다. 두 provider에서 같은 상황을 관찰하되 provider 고유 형식은 원문 그대로 보존한다.

## 가설

| 가설 | 예측 | 적용 범위 |
|---|---|---|
| H1 | `tail-preserve`의 정답률이 `oldest-first`보다 높고, 대응 차이의 95% 신뢰구간 하한이 0보다 크다. | 시드 252 합성 시나리오 16개, Claude Haiku와 Codex `gpt-5.6-luna`, 새 session, 도구 없음 |
| H2 | `tail-preserve` 패킷 토큰은 `oldest-first`보다 5% 이상 적고, 상대 차이의 95% 신뢰구간 상한이 -5% 이하다. | H1과 같은 패킷, provider와 무관한 패킷 생성 |
| H3 | 중단 가능성을 알리는 문구는 상태 확인을 먼저 하게 하고 중복 `touch` 실행을 줄인다. | Claude와 Codex, 조건마다 5회, worktree 안의 무해한 파일 한 개 |
| H4 | Codex app-server와 Claude stream-json은 입력 요청의 구조화된 응답을 provider 원문 계약대로 전달한다. | Codex MCP form·URL·agent request, Claude `AskUserQuestion`, 형식마다 3회 |
| H5 | 정상 중단과 강제 종료는 provider별로 구분 가능한 원문 표시를 남긴다. | Claude와 Codex, 중단·강제 종료 각각 3회 |

## 설계

| 항목 | 값 |
|---|---|
| 조건 | 실험 1은 A `oldest-first`와 B `tail-preserve`다. 둘 다 고정 구역을 유지하고 약 4,000토큰 예산을 쓴다. 실험 2는 A 문구 없음과 B `중단됐고 일부 실행됐을 수 있으니 상태부터 확인하라`다. 실험 3은 Codex의 MCP form, URL, agent request와 Claude의 `AskUserQuestion`이다. 실험 4는 각 provider의 정상 인터럽트와 자식 프로세스 강제 종료다. |
| 배정 | 실험 1은 시나리오 순서를 시드 252로 섞고 provider 안 조건 순서를 시나리오 ID로 섞는다. 실험 2~4는 조건과 반복 순서를 시드 252로 섞는다. 같은 입력의 조건은 대응시킨다. |
| 눈가림 | 실험 1 provider는 조건 이름과 채점 규칙을 보지 않는다. 실험 2~4는 형식상 눈가림이 불가능하다. 자동 분석은 provider 출력의 원문을 조건별 성공 여부로 변환한다. |
| 환경 | macOS, Python 3.9 이상, Codex CLI 0.158.0 app-server, Claude Code 2.1.285, Claude `claude-haiku-4-5-20251001`, Codex `gpt-5.6-luna`, Saturn worktree. 수집 시 실제 버전과 커밋을 `env.json`에 기록한다. |
| 공통 안전 | 전용 Codex home에는 원본 `auth.json`만 심볼릭 링크한다. worktree 밖 파일은 읽거나 쓰지 않는다. raw에는 토큰, 키, 계정 값, 개인 절대 경로를 넣지 않고 큰 원문은 `~/.local/experiments/provider-decision-checks/`에 둔다. |
| Codex 읽기 | app-server stdout은 텍스트 `readline()`을 쓰지 않는다. 파일 설명자를 비차단으로 읽고 바이트 버퍼에서 줄을 만든 뒤 모든 줄을 큐에 넣고, 줄마다 raw byte 수신 시각을 기록한다. |

### 실험 1. 맥락 초과 줄이기

기존 16개 시나리오 생성기에서 시드만 252로 바꾼다. 기록의 고정 구역은 마지막 사용자 입력과 최근 세 사용자 요청·agent 답을 유지한다. 경쟁 구역은 도구 호출과 결과다. `oldest-first`는 경쟁 구역의 오래된 항목부터 제거하고, `tail-preserve`는 최근 구간 예산을 먼저 보존하고 오래된 도구 결과를 제거하며 직전 사용자 입력 원문을 보존한다. 두 조건은 같은 원문과 4,000토큰 목표 예산을 받는다. 새 session provider에는 도구를 끈 지시를 주고 실제 도구 호출이 있으면 제외한다.

### 실험 2. 결과를 모르는 작업 재개

각 회차 시작 전에 worktree 안 `worktree-touch.txt`를 만든다. 이전 기록에는 `touch worktree-touch.txt`가 실행 중 끊겼고 결과는 없다고 적는다. provider에 조건별 재개 문구와 다음 작업을 주고, 요청된 무해한 명령만 승인한다. 첫 실행 명령이 `test`, `ls`, `stat`, `find`, `pwd` 같은 상태 확인인지와 파일이 이미 있는데 `touch`를 다시 실행했는지를 이벤트 순서로 센다. 조건마다 provider별 5회다.

### 실험 3. 입력 요청 형식과 왕복

Codex 전용 MCP 서버는 stdio JSON-RPC로 `tools/list`, `tools/call`, `elicitation/create`를 제공한다. form 요청에는 문자열·숫자·불리언·단일 선택·다중 선택 필드를 넣고 URL 요청도 한 번 보낸다. app-server 원문에서 `mcpServer/elicitation/request`를 기록하고 `accept`+`content`, `decline`, `cancel`을 각각 서버에 전달한다. `default_mode_request_user_input`을 켠 별도 Codex 실행에서는 `item/tool/requestUserInput` 도착과 응답 형식을 기록한다. Claude는 stream-json에서 `AskUserQuestion` `control_request`를 기록하고 `updatedInput.answers`를 넣은 control response 뒤 모델이 답을 받는지 확인한다. 각 형식은 3회다.

### 실험 4. 중단과 강제 종료 표시

provider가 worktree 안에서 종료하지 않는 무해한 Python sleep 명령을 실행하게 하고, 정상 인터럽트에서는 Claude stream-json 또는 Codex app-server의 인터럽트를 보낸다. 정상 중단 뒤에도 자식이 끝나지 않는 회차는 고정된 유예 2초 뒤 process group을 강제 종료한다. provider stdout, stderr, JSON 이벤트와 tmux capture가 있으면 그 텍스트를 원문으로 저장한다. 정상 인터럽트와 강제 종료를 provider별 3회씩 실행한다. tmux를 쓰는 경우 세션 이름에 `experiment-252`를 넣고 회차 뒤 종료한다.

## 변수

| 변수 | 종류 | 정의 | 단위 |
|---|---|---|---|
| `strategy` | 조작 | `oldest-first`, `tail-preserve` | 없음 |
| `prompt_condition` | 조작 | 실험 2의 두 재개 문구 | 없음 |
| `provider` | 조작 | `claude`, `codex` | 없음 |
| `qtype` | 조작 | 기존 생성기의 `extract`, `multi-session`, `temporal`, `update`, `abstain` | 없음 |
| `correct` | 측정 | 기존 자동 채점 규칙에 맞으면 1, 아니면 0 | 없음 |
| `packet_tokens` | 측정 | 패킷 글자 수를 4로 나눈 값 | 토큰 |
| `state_checked_first` | 측정 | 첫 실행 무해 명령이 상태 확인이면 1 | 없음 |
| `duplicate_touch` | 측정 | 상태 확인 전에 이미 존재하는 파일에 `touch`를 다시 실행하면 1 | 없음 |
| `request_method` | 측정 | 원문 JSON-RPC 또는 control request 이름 | 없음 |
| `response_round_trip` | 측정 | 요청 응답이 MCP 서버 또는 모델 후속 출력에 반영되면 1 | 없음 |
| `display_text` | 측정 | 중단·강제 종료에 나온 원문 문구 | 문자열 |
| `accuracy` | 파생 | `correct` 합 / 유효 질문 수 | % |
| `token_relative_diff` | 파생 | `(tail-preserve-oldest-first)/oldest-first` | % |

## 표본

| 항목 | 값 |
|---|---|
| 출처 | 실험 1은 기존 합성 시나리오 생성기와 자동 채점, 실험 2~4는 worktree 안 고정 fixture와 provider 원문 이벤트다. |
| 크기 | 실험 1 조건마다 16시나리오 × 2provider × 10질문 = 320질문. 실험 2 provider·조건마다 5회 = 20회. 실험 3 형식마다 3회. 실험 4 provider·상태마다 3회 = 12회. |
| 크기 근거 | 이슈가 지정한 반복 수와 모델 호출 예산. 실험 1은 기존 품질 실험의 절반 예산에서 초과를 강제하는 확인용 표본이다. |
| 중단 규칙 | provider별 실제 모델 호출이 120회에 닿으면 즉시 멈추고 그때까지 분석한다. 연속 5회 실패, 인증 오류, 원문에 비밀 값이 섞인 경우 수집을 멈춘다. 실행 중인 child와 `experiment-252` tmux 세션은 회차 뒤 종료한다. |
| 반복과 예열 | 실험 1~4 모두 예열 없음. 실험 1 조건은 같은 시나리오를 대응시킨다. |

## 분석

| 가설 | 지표 | 방법 | 채택 기준 |
|---|---|---|---|
| H1 | 대응 정답률 차이 | 시나리오 군집 부트스트랩 10,000회, 시드 252, Wilson 구간과 보조 McNemar 검정 | `tail-preserve − oldest-first` 95% 신뢰구간 하한 > 0 |
| H2 | 상대 토큰 차이 | 시나리오·provider 대응 차이의 평균, 표준편차, 중앙값, p5, p95와 95% 부트스트랩 구간 | 상대 차이 구간 상한 ≤ -5% |
| H3 | `state_checked_first`, `duplicate_touch` | provider별 조건 비율, Wilson 구간, 대응 불일치와 정확 이항 검정 | 상태 확인 비율 차이의 95% 구간 하한 > 0이고 중복 touch 차이의 상한 < 0 |
| H4 | 원문 요청·응답 왕복 | 형식별 `request_method`, action, content, 후속 반영 비율을 원문 대조 | 각 요청 형식에서 3/3 왕복이면 확인, 하나라도 아니면 미확인 |
| H5 | 중단 표시 | provider·상태별 3회 원문 문구와 이벤트 종류의 반복 일치 | 세 회차 모두 같은 상태 구분 문구나 이벤트가 있으면 확인, 아니면 미확인 |

| 항목 | 규칙 |
|---|---|
| 제외 기준 | provider 호출 실패, 출력 파싱 실패, 실험 대상 worktree 밖 명령, 실험 1 도구 호출, fixture 오류 회차는 해당 회차와 대응 조건을 제외한다. 제외 수를 흐름 표에 센다. |
| 실패한 실행 | 같은 회차를 한 번만 재시도한다. 재시도도 실패하면 제외하고 provider 호출 수에는 두 번 모두 센다. |
| 다중 비교 | H1~H3의 확인 가설 세 개는 Holm-Bonferroni를 적용한다. H4~H5는 반복 확인의 기술 통계로 보고 p값을 쓰지 않는다. |

| 결과 | 설계에 반영 |
|---|---|
| 채택 | 해당 규칙을 관련 설계 문서의 측정 근거로 제안하고 이슈 댓글에 수치를 남긴다. |
| 기각 | 기존 규칙을 유지하고 차이가 난 조건을 설계 문서의 미해결 질문으로 남긴다. |
| 보류 | 같은 조건을 늘리지 않고 부족한 원인과 다음 측정 조건을 보고서에 적는다. |

## 탐색 분석

- 실험 1 질문 유형·provider별 정답률과 패킷 포함 근거
- 실험 2 첫 명령 종류와 승인 요청 지연
- 실험 3 form 필드 순서, URL 값, agent request의 feature flag와 응답 shape
- 실험 4 provider 이벤트의 종료 상태와 화면 문구 차이

## 타당성 위협

| 종류 | 위협 | 대응 |
|---|---|---|
| 내적 | LLM 출력이 회차마다 고정되지 않는다. | 같은 입력을 대응 설계로 실행하고 provider·조건 순서를 섞는다. |
| 내적 | 기존 시나리오의 자동 채점이 다른 표기를 틀림으로 셀 수 있다. | 이전 실험 채점기를 그대로 재사용하고 원문 답을 보관한다. |
| 내적 | provider 드라이버가 이벤트를 늦게 읽으면 표시 시점이 바뀐다. | Codex는 바이트 비차단 읽기와 줄 큐를 사용하고 수신 시각을 기록한다. Claude도 stream-json 원문 줄을 즉시 기록한다. |
| 구성 | 정답률은 실제 파일 작업 전환 품질 전체를 대표하지 않는다. | 질문 유형과 packet token을 함께 보고 적용 범위를 합성 질문으로 제한한다. |
| 구성 | 상태 확인 명령의 표현이 provider에 따라 다르다. | 허용한 상태 확인 명령 목록을 사전에 고정하고 원문 명령을 함께 보고한다. |
| 외적 | 특정 날짜의 CLI·모델 버전만 측정한다. | 실행 버전과 날짜를 `env.json`에 기록하고 결론을 해당 버전으로 한정한다. |
