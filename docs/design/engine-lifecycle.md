# engine 수명과 복구

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [engine을 상주 프로세스로 두고 TUI는 JSON-RPC로 붙는 클라이언트로 만든다](../decisions/2026-09-29-engine-centered-json-rpc.md), [외부 효과가 없음을 증명할 때만 크래시 뒤 자동으로 이어 간다](../decisions/2026-09-29-proof-based-auto-resume.md), [engine만 judge를 부르고 자식 프로세스 환경에서 judge 키를 지운다](../decisions/2026-09-29-engine-as-judge-proxy.md) |

## 요약

`engine`은 사용자마다 하나만 도는 상주 프로세스다. TUI와 `cli`는 Unix 소켓 위 JSON-RPC로 `engine`에 붙는 클라이언트이고, 여러 TUI가 한 `engine`에 동시에 붙는다. TUI를 닫아도 `engine`은 접수된 입력을 계속 처리하고, 할 일이 없어지면 유예 뒤 스스로 끝난다. `engine`이 비정상 종료되면 다음 실행에서 효과 범위 `effect_scope`를 보고 자동으로 이어 갈 실행과 보류할 실행을 나눈다.

## 동기

사용자는 긴 작업을 맡긴 뒤 TUI를 닫거나 다른 터미널에서 같은 채팅을 열고 싶어 한다. 작업이 TUI 프로세스 안에 있으면 TUI를 닫는 순간 작업도 멈추고, TUI마다 provider 연결과 기록 쓰기가 중복된다. 기록 저장소는 파일 하나이고 쓰는 쪽도 하나여야 하므로, 쓰는 프로세스를 하나로 모을 자리가 필요하다.

크래시는 다른 문제를 만든다. provider와 subagent는 네트워크나 외부 도구로 작업 폴더 밖에 효과를 낼 수 있다. 끝나지 않은 실행을 무조건 다시 보내면 이미 반영된 외부 효과가 한 번 더 일어난다. 반대로 모두 멈춰 두면 로컬에서만 일하던 작업까지 사용자가 일일이 이어야 한다.

## 예시

### TUI를 닫아도 작업이 계속된다

1. 사용자가 작업 A를 실행 중이고 입력 C가 A 다음 차례로 대기 줄에 있다.
2. 사용자가 TUI를 닫는다.
3. `engine`은 `on_exit`가 기본값 `background`임을 확인하고 A를 계속 실행한다.
4. A가 끝나면 `engine`은 대기 중이던 C를 이어서 보낸다.
5. 실행 중 provider가 허가 요청을 보내면 `engine`은 답하지 않고 요청을 보관한다.
6. 사용자가 TUI를 다시 열면 보관된 허가 요청 창이 가장 먼저 뜬다.

### 크래시 뒤 다시 켜면 보류 목록을 본다

1. 작업 A는 로컬 파일만 고치고 있었고, 작업 E는 네트워크를 쓸 수 있는 설정으로 실행 중이었다.
2. `engine`이 비정상 종료된다.
3. 사용자가 `saturn`을 실행하면 `cli`가 `engine`이 없음을 보고 새로 띄운다.
4. `engine`은 A의 `effect_scope`가 `proven-by-observation`이므로 파일 상태를 확인한 뒤 새 입력으로 A를 이어 간다.
5. E의 `effect_scope`는 `network-possible`이므로 `engine`은 E를 보류하고 `/continue E` 제안을 한 줄로 보낸다.
6. TUI는 보류 목록을 보여 주고, 사용자는 E를 이어 갈지 직접 고른다.

### 두 TUI가 같은 engine에 붙는다

1. 사용자가 터미널 두 개에서 `saturn`을 실행한다.
2. 두 번째 `saturn`은 이미 도는 `engine`을 찾아 같은 Unix 소켓에 접속한다.
3. 허가 요청이 오면 두 TUI 모두 허가 요청 창을 띄운다.
4. 한쪽에서 답하면 다른 쪽의 창은 사라진다.

## 상세 설계

### 사용자당 engine 하나

`engine`은 사용자마다 하나만 돌고, 잠금으로 이를 지킨다. 기록 저장소 `~/.saturn/saturn.db`에 쓰는 프로세스를 하나로 두어 쓰기 충돌을 막기 위해서다. judge 호출도 `engine`만 한다. judge 키가 TUI나 provider 자식 프로세스로 새는 일을 막기 위해서다. 자세한 judge 호출 규칙은 [judge](judge.md)에 있다.

TUI와 `cli`는 `engine` crate에 의존하지 않고, 이 경계는 Cargo 의존성으로 강제한다. TUI가 provider를 몰라도 Saturn 용어만으로 화면을 그리게 하기 위해서다.

### engine 시작 순서

`saturn`을 실행했을 때 `engine`이 없으면 `cli`가 `saturn-engine`을 띄운다. `engine`은 다음 순서로 시작한다.

1. `rpc`가 사용자당 `engine` 잠금을 얻는다.
2. `store`가 스키마 버전이 올라갔는지 보고, 올라갔으면 스키마를 이관한다.
3. `settings`가 설정 층을 병합하고 설정 번호를 확정한다.
4. `judges`가 판단 방식이 쓰는 judge가 응답하는지 확인한다.
5. `rpc`가 Unix 소켓에서 JSON-RPC 접속을 받기 시작한다.
6. `rpc`가 여러 TUI의 접속을 동시에 유지한다.

앞 단계가 실패하면 뒤 단계를 하지 않는다. 판단 방식에 맞는 judge를 만들 수 없으면(허용 호스트가 아닌 주소, 설정이 없는 판단 방식) 소켓을 열지 않고 끝낸다. 키를 받아도 확인할 수 없기 때문이다.

judge 키가 없거나 틀려 확인에 실패하면 `engine`은 환경 변수, 비밀번호 관리자 명령 순서로 키를 받아 다시 확인한다. 그래도 실패하면 소켓은 연다. 다만 judge를 확인하기 전에는 `SubmitJudgeKey`, `Attach`, `Detach`만 받고, 나머지 요청은 오류 번호 `-32001`(초안)의 오류 응답으로 거절한다. TUI가 붙으면 시작 정보와 기록 뒤에 `JudgeKeyRequired`를 보내고, TUI가 보낸 키로 다시 확인해 성공하면 일반 요청을 받기 시작한다. 실패하면 오류 응답과 함께 `JudgeKeyRequired`를 다시 보낸다. `engine`은 터미널에서 직접 숨김 입력을 받지 않는다. 사용자당 하나인 상주 프로세스라 키를 물을 터미널을 갖지 않기 때문이다. judge 시작 확인은 [judge](judge.md)에, 키 요청과 저장 절차는 [judge 키 보호](judge-key-security.md)에 있다.

처음 보거나 내용이 바뀐 폴더 설정은 빼고 시작하고, 처음 붙는 TUI에 폴더 설정 신뢰 창을 보낸다. 키를 기다리는 동안에는 키를 받은 뒤 보낸다. TUI는 창을 한 번에 하나만 띄우기 때문이다. 적용을 고르면 신뢰를 기록하고 설정을 다시 병합해 `SettingsApplied`를 모든 TUI에 보내고, 고르지 않으면 이번 실행 동안 폴더 설정 없이 계속한다. 설치 검증 테스트만을 위해 judge 확인을 건너뛰는 설정이 있고, 이 설정은 도움말에 보이지 않는다. judge 없이 설치만 검증하기 위해서다. 설정 층과 병합 규칙은 [설정](settings.md)에 있다.

### 스키마 이관

새 버전을 처음 실행할 때 스키마 버전이 올라갔으면 `store`가 다음 순서로 이관한다.

1. 이관 직전 기록 저장소의 백업을 `~/.saturn/backup/`에 만든다.
2. 이전 버전의 백업을 지운다.
3. 스키마를 자동으로 이관한다.
4. 이관했다는 사실을 한 줄로 알린다.
5. 만든 지 14일이 지난 백업을 자동으로 지운다.

백업은 가장 최근 하나만 두고 14일 뒤 지운다. 백업은 이관 규칙의 버그에 대비한 임시본이기 때문이다.

### 여러 TUI 동시 접속

TUI와 `cli`는 Unix 소켓 위 JSON-RPC로 `engine`에 붙는다. 한 `engine`에 여러 TUI가 동시에 붙을 수 있다. TUI를 닫은 뒤에도 작업을 이어 가고 여러 TUI를 한 `engine`에 붙이기 위해 이 구조를 골랐다. 허가 요청 창은 다른 클라이언트가 먼저 답하면 사라진다.

TUI가 `Attach`로 채팅에 붙으면 `engine`은 `StartInfo`, `HistoryChunk`, 답을 기다리는 허가 요청 순서로 보낸 뒤 `Attach`에 응답한다. `chat`이 없으면 새 채팅을 만든다. `HistoryChunk`는 그 채팅에 접수한 입력과 provider 이벤트를 시각 순서로 합친 끝 50개(초안)이고, 기록에 없는 작업 글자와 처리 방식은 비운다. provider는 첫 입력 때 연결하므로 `StartInfo`의 provider 버전은 비어 있다. `LoadHistory`는 한 번에 500개(초안)까지 보낸다.

메시지는 JSON-RPC 2.0이고 소켓 한 줄에 하나씩 쓴다. 메서드 이름은 `saturn-protocol` 타입의 variant 이름, `params`는 그 필드다. 클라이언트의 요청에는 모두 `id`가 붙고, `engine`은 요청마다 같은 `id`의 응답 하나(`result: null` 또는 `error`)를 돌려준다. 조회 결과와 화면 갱신은 `id` 없는 알림으로 보낸다. 해석하지 못한 줄에는 읽어 낸 `id`(없으면 `null`)로 오류 응답을 보내고 연결은 유지한다. 오류 문구에는 입력 원문을 넣지 않는다. judge 키가 들어 있을 수 있기 때문이다. 메시지의 JSON Schema와 TypeScript 타입은 `saturn-protocol/generated/`에 있고 `cargo run -p saturn-protocol --example codegen`으로 다시 만든다.

사용자당 잠금은 `~/.saturn/engine.lock`의 `flock`이다. 프로세스가 죽으면 운영체제가 풀기 때문에 남은 잠금 파일을 지울 필요가 없다. 소켓은 `~/.saturn/engine.sock`이고 권한은 0600이다. 남은 소켓 파일은 잠금을 얻은 뒤에만 지운다. 답을 기다리는 허가 요청은 TUI가 붙어 있어도 답이 올 때까지 보관한다. 나중에 붙는 TUI도 같은 창을 띄우게 하기 위해서다. 클라이언트마다 보낼 메시지를 1024개까지 쌓고, 넘치면 기다리지 않고 그 연결을 끊는다. 느린 TUI 하나가 `engine`을 멈추지 않게 하기 위해서다. 끊긴 TUI는 다시 붙어 기록으로 화면을 되살린다. 파일 이름, 권한, 쌓는 개수는 초안이다.

### TUI 종료 뒤 동작

TUI가 끝나면 `rpc`가 설정 `on_exit` 값을 확인한다. 값은 `background`, `stop`, `ask` 세 가지이고 기본값은 `background`다. 기본값을 `background`로 둔 것은 TUI를 닫아도 작업을 계속하게 하기 위해서다.

`on_exit`가 `background`이면 `engine`은 접수된 대기 입력을 provider에 순서대로 계속 보낸다. `stop`과 `ask`의 동작은 아직 정하지 않았다([#70](https://github.com/woonyong-choi/saturn/issues/70)). TUI가 없는 동안 `engine`은 다음 규칙을 따른다.

1. 허가 요청은 사용자 확인을 기다리는 상태로 보관하고, TUI가 다시 붙으면 가장 먼저 보낸다.
2. 피드백 질문을 건너뛴다.
3. 완료 알림(macOS)을 켠 경우에만 알림을 보낸다.
4. 보류는 그대로 둔다.
5. 에이전트 트리 전체가 끝나면 5분을 기다린 뒤 session을 닫고 `engine`을 끝낸다.

TUI가 없는 동안 보류를 그대로 두는 것은 사용자가 멈춘 작업을 자동으로 이어 가지 않기 위해서다. 보류와 대기 규칙은 [입력 처리](input-handling.md)에 있다.

### 프로세스 배치와 수명

| 프로세스 | 시작 주체 | 수명 |
|---|---|---|
| `saturn` | 사용자 | 명령 실행이 끝나거나 TUI를 닫을 때까지 |
| `saturn-engine` | `saturn` | 모든 TUI가 떨어지고 트리 유휴가 된 뒤 5분 유예까지, 사용자당 하나 |
| Codex app-server | `saturn-engine` | 연결 창구로 유지, session은 턴이 끝난 뒤 5분 유예에 정리 |
| Claude Code | `saturn-engine` | 턴 진행 중과 턴이 끝난 뒤 5분 유예까지 |

### 파일 경로

| 경로 | 내용 | 쓰는 구성 요소 |
|---|---|---|
| `~/.saturn/saturn.db` | 기록 저장소 | `engine` |
| `~/.saturn/config.toml` | 사용자 설정 | `engine` |
| `<작업 폴더>/.saturn/config.toml` | 폴더 설정 | `engine` |
| `~/.saturn/backup/` | 스키마 이관 직전 백업 | `engine` |
| `~/.saturn/history` | 입력 기록 | `tui` |

### 효과 범위

`engine`은 실행마다 효과 범위 `effect_scope`를 기록하고, 크래시 뒤에는 이 값만으로 자동 재개와 보류를 나눈다. 전달 여부가 불확실한 입력을 자동으로 다시 보내지 않기 위해서다.

| 값 | 뜻 | 크래시 뒤 처리 |
|---|---|---|
| `proven-by-config` | 적용된 provider 설정을 읽어 트리 전체에서 외부 효과가 불가능함을 증명한 실행 | 자동 재개 |
| `proven-by-observation` | 트리 전체를 끊김 없이 관찰했고 모든 행동이 로컬 전용 목록 안인 실행 | 자동 재개 |
| `network-possible` | 외부 효과 가능성을 배제하지 못한 실행 | 보류 |
| `unobserved` | 관찰이 끊긴 실행 | 보류 |

완료 신호 없이 흐름이 끝나거나, 읽는 도중 종료되거나, 끝 신호가 없는 subagent가 남으면 관찰이 끊긴 것으로 보고 `unobserved`를 기록한다. 관찰하지 못한 사이에 외부 효과가 있었을 수 있기 때문이다.

### 크래시 뒤 복구

`engine`이 비정상 종료된 뒤 사용자가 `saturn`을 실행하면 `cli`가 새 `engine`을 띄우고, `engine`은 다음 순서로 복구한다.

1. `store`가 끝나지 않은 실행의 `effect_scope`를 조회한다.
2. `store`가 `proven-by-config`와 `proven-by-observation` 실행을 자동 재개 대상으로 분류한다.
3. `engine`은 자동 재개 대상의 파일 상태를 확인한 뒤 그 상태로 만든 새 입력을 provider에 보낸다.
4. `store`가 `network-possible`과 `unobserved` 실행을 보류로 기록하고 해당 session도 보류로 둔다.
5. `rpc`가 보류 항목마다 `/continue` 제안을 한 줄씩 보낸다.

자동으로 이어 갈 때 크래시 전에 보낸 패킷을 다시 보내지 않는다. 이미 반영된 입력을 두 번 실행하지 않기 위해서다. 보류한 실행은 사용자가 `/continue`로 이을 때까지 멈춰 있다. TUI가 보류 목록을 묻는 방식은 [TUI](tui.md)에 있다.

### 중첩 saturn 거절

에이전트가 작업 중에 실행한 `saturn`은 거절한다. 이 거절은 자식 Saturn을 부모와 잇는 기능이 생길 때까지 유지한다. 부모와 자식의 연결 규칙 없이 에이전트가 Saturn을 다시 띄우는 일을 막기 위해서다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| 판단 방식에 맞는 judge를 만들 수 없음 | 원인을 한 줄로 보이고 소켓을 열지 않고 끝낸다. |
| judge 키가 없거나 틀려 시작 확인 실패 | 소켓을 열고 TUI가 키를 보낼 때까지 `SubmitJudgeKey`, `Attach`, `Detach` 밖의 요청을 오류 응답으로 거절한다. |
| 입력할 수 없는 환경(파이프, CI)의 judge 확인 실패 | `engine`은 키를 기다리고, `cli`가 키를 묻지 않고 끝내며 환경 변수와 표준 입력 방식을 안내한다. |
| 시작 때 이전 설정 번호 없음 | 실행하지 않는다. |
| 설정 검사 실패 | 이전 설정 번호를 유지하고 경고한다. |
| `engine` 비정상 종료 | 다시 시작한 `engine`이 `effect_scope`로 자동 재개와 보류를 나눈다. |
| provider 흐름의 관찰 중단 | `effect_scope`를 `unobserved`로 기록하고 자동으로 이어 가지 않는다. |
| 크래시 뒤 증명되지 않은 `effect_scope` | 자동으로 재개하지 않고 보류한 뒤 재개를 한 줄로 제안한다. |
| 에이전트가 실행한 중첩 `saturn` | 실행을 거절한다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| `engine`은 사용자당 하나만 실행된다. | 같은 사용자로 `engine`을 두 번 띄우면 두 번째가 잠금을 얻지 못하고 기존 `engine`에 붙는지 확인 |
| TUI를 닫아도 `engine`은 접수된 입력을 계속 처리한다. | TUI 연결을 끊은 뒤 대기 입력이 순서대로 provider에 전달되는지 확인 |
| judge를 확인하기 전에는 judge 키 제출과 채팅 붙기 밖의 요청을 받지 않는다. | judge 키가 없는 환경에서 일반 요청이 오류 응답으로 거절되고, 키를 보낸 뒤에는 처리되는지 확인 |
| 판단 방식에 맞는 judge를 만들 수 없으면 Saturn을 실행하지 않는다. | 허용 호스트가 아닌 judge 주소로 `engine`을 띄워 소켓을 열지 않고 끝나는지 확인 |
| 스키마를 올리기 전 백업 하나를 남긴다. | 옛 스키마 저장소로 새 버전을 실행한 뒤 백업이 하나만 남고 스키마가 올라갔는지 확인 |
| 크래시 뒤 자동 재개는 효과 범위가 설정이나 관찰로 증명된 실행에만 한다. | [공급자 적용 설정의 보고 범위 측정](https://github.com/woonyong-choi/saturn/issues/4), [하위 에이전트 외부 효과 경로 측정](https://github.com/woonyong-choi/saturn/issues/22) |
| 자동으로 이어 갈 때 같은 패킷을 다시 보내지 않는다. | 자동 재개 때 파일 상태로 만든 새 입력만 전송되는지 확인 |
| 강제 종료 뒤 subagent가 있던 session을 재개할 때의 동작을 확인한다. | [강제 종료 뒤 세션 재개 동작 측정](https://github.com/woonyong-choi/saturn/issues/24) |
| 에이전트가 실행한 `saturn`은 거절한다. | [하위 에이전트 훅 적용 범위 측정](https://github.com/woonyong-choi/saturn/issues/23) |

## 대안

- TUI 프로세스 안에 `engine`을 넣는 방식은 TUI를 닫으면 작업이 멈추고 TUI마다 provider 연결과 기록 쓰기가 중복되어 버렸다([결정 기록](../decisions/2026-09-29-engine-centered-json-rpc.md)).
- 로컬 작업을 추정해 크래시 뒤 자동 재개하는 방식은 외부 효과가 중복될 수 있어 버렸다([결정 기록](../decisions/2026-09-29-proof-based-auto-resume.md)).
- Saturn이 provider의 네트워크 차단 설정을 고정하는 방식은 사용자의 provider 설정을 바꿔서 버렸다([결정 기록](../decisions/2026-09-29-proof-based-auto-resume.md)).

## 미해결 질문

- 에이전트가 실행한 자식 Saturn을 부모 `engine` 소켓에 자식으로 붙일지, 독립 `engine`으로 띄우고 결과 파일로 돌려받을지 ([#33](https://github.com/woonyong-choi/saturn/issues/33))
- `codex exec`, `claude -p` 같은 한 번 실행 경로를 상시 연결의 대체 경로로 구현할지, 경로를 하나만 둘지 ([#34](https://github.com/woonyong-choi/saturn/issues/34))
- Codex 자식 session의 승인 요청에 Saturn이 부모와 같은 정책으로 답할지, 사용자에게 따로 보일지, 모두 거절할지 ([#61](https://github.com/woonyong-choi/saturn/issues/61))
- 크래시 뒤 파일 상태 확인에 쓰는 수정 파일 목록을 실행 경계의 파일 상태 차이로 계산할지, provider 이벤트로 계산할지 ([#65](https://github.com/woonyong-choi/saturn/issues/65))
- 크래시 복구 때 실행 중으로 남은 subagent와 provider가 다시 불러오는 자식 session을 Saturn이 정리할지, 끊김 표시만 하고 provider 재개 동작은 그대로 둘지 ([#66](https://github.com/woonyong-choi/saturn/issues/66))
- TUI를 닫을 때 `on_exit`가 `stop`이나 `ask`이면 무엇을 할지 ([#70](https://github.com/woonyong-choi/saturn/issues/70))
