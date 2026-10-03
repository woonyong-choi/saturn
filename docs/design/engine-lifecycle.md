# engine 수명과 복구

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [engine을 상주 프로세스로 두고 TUI는 JSON-RPC로 붙는 클라이언트로 만든다](../decisions/2026-09-29-engine-centered-json-rpc.md), [외부 효과가 없음을 증명할 때만 크래시 뒤 자동으로 이어 간다](../decisions/2026-09-29-proof-based-auto-resume.md), [engine만 router를 부르고 자식 프로세스 환경에서 router 키를 지운다](../decisions/2026-09-29-engine-as-router-proxy.md) |

## 요약

`engine`은 사용자마다 하나만 도는 상주 프로세스다. TUI와 `cli`는 Unix 소켓 위 JSON-RPC로 `engine`에 붙는 클라이언트이고, 여러 TUI가 한 `engine`에 동시에 붙는다. TUI를 닫아도 `engine`은 기본으로 접수된 입력을 계속 처리하고, 할 일이 없어지면 유예 뒤 스스로 끝난다. 설정 `on_exit`로 닫을 때 작업을 멈추거나 묻게 할 수 있다. `engine`이 비정상 종료되면 다음 실행에서 효과 범위 `effect_scope`를 보고 자동으로 이어 갈 실행과 보류할 실행을 나눈다.

## 동기

사용자는 긴 작업을 맡긴 뒤 TUI를 닫거나 다른 터미널에서 같은 채팅을 열고 싶어 한다. 작업이 TUI 프로세스 안에 있으면 TUI를 닫는 순간 작업도 멈추고, TUI마다 provider 연결과 기록 쓰기가 중복된다. 기록 저장소는 파일 하나이고 쓰는 쪽도 하나여야 하므로, 쓰는 프로세스를 하나로 모을 자리가 필요하다.

크래시는 다른 문제를 만든다. provider와 subagent는 네트워크나 외부 도구로 작업 폴더 밖에 효과를 낼 수 있다. 끝나지 않은 실행을 무조건 다시 보내면 이미 반영된 외부 효과가 한 번 더 일어난다. 반대로 모두 멈춰 두면 로컬에서만 일하던 작업까지 사용자가 일일이 이어야 한다.

## 예시

### TUI를 닫아도 작업이 계속된다

1. 사용자가 작업 A를 실행 중이고 입력 C가 A 다음 차례로 대기 줄에 있다.
2. 사용자가 TUI를 닫는다.
3. `engine`은 `on_exit`가 기본값 `background`임을 확인하고 A를 계속 실행하고, 터미널에는 `작업 2개 계속 실행 중 · saturn으로 다시 여세요` 한 줄이 남는다.
4. A가 끝나면 `engine`은 대기 중이던 C를 이어서 보낸다.
5. 실행 중 provider가 허가 요청이나 입력 요청을 보내면 `engine`은 답하지 않고 요청을 보관한다.
6. 사용자가 TUI를 다시 열면 보관된 허가 요청 창과 입력 요청 창이 가장 먼저 뜬다.

### 닫을 때 작업을 멈춘다

1. 사용자가 `on_exit = "ask"`로 두고 작업 A를 실행 중이고 입력 C가 대기 줄에 있다.
2. 사용자가 `Ctrl+D`를 누르면 TUI가 `PrepareExit`을 보내고 `engine`이 작업 2개와 함께 `Ask`로 답한다.
3. TUI가 `작업 2개 실행 중` 창을 띄우고, 사용자가 `멈추기`를 고른다.
4. TUI가 `StopAll`을 보낸 뒤 닫고, `engine`이 A와 C를 보류한다.
5. 사용자가 다시 열면 보류 재개 질문이 뜨고, 멈춘 작업은 자동으로 이어지지 않는다.

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

`engine`은 사용자마다 하나만 돌고, 잠금으로 이를 지킨다. 기록 저장소 `~/.saturn/saturn.db`에 쓰는 프로세스를 하나로 두어 쓰기 충돌을 막기 위해서다. router 호출도 `engine`만 한다. router 키가 TUI나 provider 자식 프로세스로 새는 일을 막기 위해서다. 자세한 router 호출 규칙은 [router](router.md)에 있다.

TUI와 `cli`는 `engine` crate에 의존하지 않고, 이 경계는 Cargo 의존성으로 강제한다. TUI가 provider를 몰라도 Saturn 용어만으로 화면을 그리게 하기 위해서다.

### engine 시작 순서

![engine은 잠금, 스키마 이관, session 복원, 설정 병합, router 확인, 소켓 접속 순서로 시작한다](../assets/engine-start.svg)

`saturn`을 실행했을 때 `engine`이 없으면 `cli`가 `saturn-engine`을 띄운다. `engine`은 다음 순서로 시작한다.

1. `rpc`가 사용자당 `engine` 잠금을 얻는다.
2. `store`가 스키마 버전이 올라갔는지 보고, 올라갔으면 스키마를 이관한다.
3. `sessions`가 끝나지 않은 메인 session과 보관 session의 마지막 턴 값을 `store`에서 되살린다.
4. `settings`가 기본값, 사용자, 실행 층을 병합하고 설정 번호를 확정한다. 폴더 층과 채팅 층은 채팅이 붙을 때 채팅마다 병합한다.
5. `routers`가 판단 방식이 쓰는 router가 응답하는지 확인한다.
6. `rpc`가 Unix 소켓에서 JSON-RPC 접속을 받기 시작한다.
7. `rpc`가 여러 TUI의 접속을 동시에 유지한다.

앞 단계가 실패하면 뒤 단계를 하지 않는다. 판단 방식에 맞는 router를 만들 수 없으면(허용 호스트가 아닌 주소, 설정이 없는 판단 방식) 소켓을 열지 않고 끝낸다. 키를 받아도 확인할 수 없기 때문이다.

등록된 router 키가 확인되면 화면이 있어도 키를 묻지 않는다. 키가 없거나 틀려 확인에 실패하면 `engine`은 `SATURN_KEY` 환경 변수, 비밀번호 관리자 명령(`router.key.command`) 순서로 키를 받아 다시 확인한다. 표준 입력으로는 키를 받지 않는다. 그래도 실패하면 소켓은 연다. 다만 router를 확인하기 전에는 `SubmitRouterKey`, `Attach`, `Detach`만 받고, 나머지 요청은 오류 번호 `-32001`(초안)의 오류 응답으로 거절한다. TUI가 붙으면 시작 정보와 기록 뒤에 `RouterKeyRequired`를 보내고, TUI가 보낸 키로 다시 확인해 성공하면 일반 요청을 받기 시작한다. 실패하면 오류 응답과 함께 `RouterKeyRequired`를 다시 보낸다. `engine`은 터미널에서 직접 숨김 입력을 받지 않는다. 사용자당 하나인 상주 프로세스라 키를 물을 터미널을 갖지 않기 때문이다. router 시작 확인은 [router](router.md)에, 키 요청과 저장 절차는 [router 키 보호](router-key-security.md)에 있다.

폴더 설정은 TUI가 붙을 때 그 채팅의 작업 폴더로 병합한다. 처음 보거나 내용이 바뀐 폴더 설정은 빼고 병합하고, 그 TUI에 폴더 설정 신뢰 창을 보낸다. 키를 기다리는 동안에는 키를 받은 뒤 보낸다. TUI는 창을 한 번에 하나만 띄우기 때문이다. 적용을 고르면 신뢰를 기록하고 그 채팅의 작업 폴더로 설정을 다시 병합해 같은 채팅에 붙은 모든 TUI에 `SettingsApplied`를 보내고, 고르지 않으면 그 채팅은 폴더 설정 없이 계속한다. 병합 결과에 경고가 있으면 붙을 때 `SettingsApplied`로 알린다. 설치 검증 테스트만을 위해 router 확인을 건너뛰는 설정이 있고, 이 설정은 도움말에 보이지 않는다. router 없이 설치만 검증하기 위해서다. 설정 층과 병합 규칙은 [설정](settings.md)에 있다.

### 스키마 이관

새 버전을 처음 실행할 때 스키마 버전이 올라갔으면 `store`가 다음 순서로 이관한다.

1. 이관 직전 기록 저장소의 백업을 `~/.saturn/backup/`에 만든다.
2. 이전 버전의 백업을 지운다.
3. 스키마를 자동으로 이관한다.
4. 이관했다는 사실을 stderr 로그에 한 줄로 남기고, 처음 붙는 TUI에 상태판 알림 `Alert::SchemaMigrated { from, to }`를 보낸다. TUI는 알림 줄에 `기록 저장소 v{to}로 옮김`을 보인다.
5. 만든 지 14일이 지난 백업을 자동으로 지운다.

백업은 가장 최근 하나만 두고 14일 뒤 지운다. 백업은 이관 규칙의 버그에 대비한 임시본이기 때문이다.

### 여러 TUI 동시 접속

TUI와 `cli`는 Unix 소켓 위 JSON-RPC로 `engine`에 붙는다. 한 `engine`에 여러 TUI가 동시에 붙을 수 있다. TUI를 닫은 뒤에도 작업을 이어 가고 여러 TUI를 한 `engine`에 붙이기 위해 이 구조를 골랐다. 허가 요청 창은 다른 클라이언트가 먼저 답하면 사라진다.

TUI가 `Attach`로 채팅에 붙으면 `engine`은 `StartInfo`, `HistoryChunk`, 답을 기다리는 허가 요청과 입력 요청 순서로 보낸 뒤 `Attach`에 응답한다. `chat`이 없으면 새 채팅을 만든다. `HistoryChunk`는 그 채팅에 접수한 입력과 provider 이벤트를 시각 순서로 합친 끝 50개(초안)이고, 기록에 없는 작업 글자와 처리 방식은 비운다. provider는 첫 입력 때 연결하므로 `StartInfo`의 provider 버전은 비어 있다. `LoadHistory`는 한 번에 500개(초안)까지 보낸다.

붙은 TUI가 같은 연결로 `Attach`를 다시 보내면 `engine`은 연결을 끊지 않고 붙은 채팅만 바꾸며, 떠난 채팅의 작업은 계속된다. 채팅 이동은 마지막 TUI 이탈이 아니므로 `on_exit`를 적용하지 않는다. 연결을 끊고 마지막 TUI 이탈로 세는 것은 `Detach`와 연결 끊김뿐이다. TUI의 채팅 이동 순서는 [TUI](tui.md#채팅-이동)에 있다.

`Attach`에는 그 TUI의 작업 폴더와 환경 변수(`env`)가 들어 있다. TUI는 터미널마다 따로 뜨고 터미널마다 PATH와 환경이 다르기 때문에, 상주 프로세스인 `engine`의 환경 대신 붙은 TUI의 값을 쓴다. `engine`은 새 채팅을 TUI가 넘긴 작업 폴더로 만들고 그 폴더를 채팅 기록에 고정한다. 이미 있는 채팅에 붙을 때는 TUI가 다른 폴더를 넘겨도 채팅의 폴더, 폴더 설정 층, 폴더 설정 신뢰 판단에 처음 폴더를 그대로 쓰고, 환경 변수만 그 채팅에 가장 최근에 붙은 TUI의 값으로 바꿔 저장한다. 채팅의 provider 실행 환경은 이 환경 변수로 정한다. 넘기는 변수는 `PATH`, `HOME`, `SHELL`, 로캘, 프록시 같은 실행에 필요한 것으로 한정하고(목록은 `saturn-protocol`의 `ATTACH_ENV_NAMES`, 초안), 넘겨받은 환경에 router 키 변수가 있어도 provider 자식 환경에는 넣지 않는다. 처음 친 `saturn`이 `engine`을 띄우고 붙으며, 이후의 `saturn`은 붙기만 한다.

`cli`는 소켓에 먼저 붙어 보고, 받는 `engine`이 없을 때만 `saturn-engine`(`saturn`과 같은 폴더, 없으면 `PATH`)을 새 프로세스 그룹으로 띄워 `saturn`이 끝나도 남게 한다. `engine`은 로그를 `~/.saturn/logs/engine-YYYY-MM-DD.log`에 직접 쌓고(로컬 날짜, 하루 한 파일), `cli`는 `engine`의 stderr를 버린다. 오래 도는 `engine`이 날짜가 바뀌면 새 파일로 옮겨 쓰려면 `engine`이 파일을 쥐고 있어야 하기 때문이다. 시작할 때와 날짜가 바뀔 때 30일 지난 날짜의 파일을 지운다. 지우는 대상은 `engine-YYYY-MM-DD.log` 형식 이름뿐이고 날짜는 이름으로 읽는다. 30일은 Claude Code `cleanupPeriodDays` 기본값을 따랐다. 크기 상한과 돌려 쓰기는 두지 않는다. 크기 안전 상한은 실제 하루 사용량을 잰 뒤 정한다. 날짜별 파일 이전의 `~/.saturn/engine.log`와 `~/.saturn/logs/engine.log`, `engine.log.N`은 `engine`이 시작할 때 지운다. `engine`이 로그 파일을 열기 전에 끝나면 사유가 남지 않는다. 소켓이 열릴 때까지 30초(초안)를 기다린다. 두 `saturn`이 동시에 띄워 한쪽 `engine`이 잠금을 얻지 못하고 끝나면, 소켓이 열리기를 2초(초안) 더 기다려 먼저 뜬 `engine`에 붙는다. 그래도 붙지 못하면 가장 최근 로그 파일의 끝 줄과 함께 오류로 끝낸다. 실행 층 `-c`는 `engine`을 띄울 때 넘기지 않고 `Attach`의 `overrides`로만 넘긴다. 이미 도는 `engine`에도 같은 방식으로 적용하고, 다른 접속에는 영향을 주지 않게 하기 위해서다. `saturn --resume <채팅 id>`는 그 id를 `Attach`의 `chat`으로 넘긴다. `saturn --continue`는 `Attach` 전에 현재 폴더를 `LatestChat`으로 `engine`에 물어 받은 채팅 id를 `Attach`의 `chat`으로 넘긴다. 폴더에 채팅이 없으면 새 채팅을 열지 않고 `이 폴더에 이어 열 채팅이 없습니다 · saturn으로 새 채팅을 시작하세요`(영어 `No chat to continue in this folder · Start a new chat with saturn`)를 보이고 오류 종료한다. 최근 채팅의 기준은 `Usage`의 `chat` 범위와 같다([기록](records.md)). `LatestChat`은 `Attach`하지 않는 접속에서도 쓰고 router 키를 기다리는 동안에도 받는다. 키 입력은 이어 열린 채팅의 TUI가 받는다. 채팅 id 없는 `--resume`과 `--resume all`의 목록은 `engine`에 채팅 목록 요청이 생기기 전까지 오류로 끝낸다([#161](https://github.com/woonyong-choi/saturn/issues/161)).

메시지는 JSON-RPC 2.0이고 소켓 한 줄에 하나씩 쓴다. 메서드 이름은 `saturn-protocol` 타입의 variant 이름, `params`는 그 필드다. 클라이언트의 요청에는 모두 `id`가 붙고, `engine`은 요청마다 같은 `id`의 응답 하나(`result: null` 또는 `error`)를 돌려준다. 조회 결과와 화면 갱신은 `id` 없는 알림으로 보낸다. 해석하지 못한 줄에는 읽어 낸 `id`(없으면 `null`)로 오류 응답을 보내고 연결은 유지한다. 오류 문구에는 입력 원문을 넣지 않는다. router 키가 들어 있을 수 있기 때문이다. 메시지의 JSON Schema와 TypeScript 타입은 `saturn-protocol/generated/`에 있고 `cargo run -p saturn-protocol --example codegen`으로 다시 만든다.

사용자당 잠금은 `~/.saturn/engine.lock`의 `flock`이다. 프로세스가 죽으면 운영체제가 풀기 때문에 남은 잠금 파일을 지울 필요가 없다. 소켓은 `~/.saturn/engine.sock`이고 권한은 0600이다. 남은 소켓 파일은 잠금을 얻은 뒤에만 지운다. 답을 기다리는 허가 요청은 TUI가 붙어 있어도 답이 올 때까지 보관한다. 나중에 붙는 TUI도 같은 창을 띄우게 하기 위해서다. 클라이언트마다 보낼 메시지를 1024개까지 쌓고, 넘치면 기다리지 않고 그 연결을 끊는다. 느린 TUI 하나가 `engine`을 멈추지 않게 하기 위해서다. 끊긴 TUI는 다시 붙어 기록으로 화면을 되살린다. 파일 이름, 권한, 쌓는 개수는 초안이다.

### 채팅 폴더와 이어 열기

채팅을 만든 폴더가 그 채팅의 기본 폴더다. 폴더 설정, 폴더 설정 신뢰, 작업 폴더는 기본 폴더 하나만 따른다. 다른 폴더의 설정이 섞이지 않게 하기 위해서다. 사용자는 기본 폴더 밖의 폴더를 더해 같은 채팅에서 다룰 수 있다.

| 명령 | 동작 |
|---|---|
| `saturn --add-dir <폴더>` | 새 채팅을 열면서 폴더를 더한다. 여러 번 쓸 수 있다. |
| TUI `/add-dir <폴더>` | 열린 채팅에 폴더를 더한다. |
| `saturn --continue` | 현재 폴더에서 가장 최근에 쓴 채팅을 연다. |
| `saturn --resume [채팅 id]` | 채팅 id가 있으면 그 채팅을, 없으면 현재 폴더의 채팅 목록에서 골라 연다. |
| `saturn --resume all` | 모든 폴더의 채팅 목록에서 골라 연다. |

- 더한 폴더는 채팅 기록에 저장하고(표 `chat_dirs`, [기록 저장과 보존](records.md)), 채팅을 이어 열면 그대로 되살린다. 채팅마다 폴더가 달라도 이어 열기 전에 다시 더하지 않게 하기 위해서다. 붙을 때마다 기록에서 읽어 메모리 사본을 새로 만든다.
- 더한 폴더는 그 채팅의 모든 provider session에 넘긴다. provider를 바꿔도 같은 폴더를 다루게 하기 위해서다. 넘기는 방식은 provider마다 다르고 `providers`가 맡는다(초안). `engine`은 session을 열 때마다(새 session, 재개, provider 전환) 그 시점의 더한 폴더를 넘긴다.
- 쓰는 중에 더한 폴더는 이미 열린 session에 넣지 못하고 다음에 여는 session부터 적용한다. 열린 session에 폴더를 넣는 방법을 두 provider에서 확인하지 못했기 때문이다(초안). 열린 메인 session이 있으면 안내에 그 사실을 덧붙인다.
- `--add-dir`는 `saturn`이 링크를 푼 절대 경로로 바꿔 `Attach`에 싣는다. 폴더가 없거나 폴더가 아니면 engine을 띄우기 전에 거절한다. 이미 있는 채팅을 `--resume`으로 열 때 준 `--add-dir`도 그 채팅에 더한다(초안). `engine`도 같은 검사를 하고, 틀린 경로가 있으면 채팅을 만들기 전에 `Attach`를 거절한다.
- 폴더 경로는 링크를 푼 절대 경로로 저장한다. 같은 폴더를 다른 이름으로 두 번 더하지 않기 위해서다. 채팅의 기본 폴더와 같거나 이미 더한 폴더는 더하지 않고 안내도 보내지 않는다.
- 더한 폴더의 `.saturn/config.toml`은 읽지 않는다. 폴더 설정은 기본 폴더 것만 쓴다([설정](settings.md)).
- 이어 열기는 provider의 재개 명령이 아니라 Saturn이 채팅 기록으로 직접 처리한다. 채팅은 여러 provider session을 잇는 단위이므로 provider 하나의 session으로 채팅을 정할 수 없기 때문이다. provider의 이어 열기와 새 대화 명령은 provider 명령 목록에서 뺀다([provider 연결과 session](providers-and-sessions.md)).
- 다른 폴더의 채팅을 이어 열면 그 채팅의 기본 폴더에서 일한다. 현재 폴더로 바꾸지 않으며, 시작 화면이 그 폴더를 보인다([TUI](tui.md)).
- TUI의 작업 목록은 기본으로 현재 폴더의 채팅만 보인다. 모든 폴더의 채팅은 `--resume all`이나 작업 목록 필터로 본다.
- 이어 열기 명령은 짧은 이름 없이 긴 이름만 쓴다. 설정의 실행 층 `-c KEY=VALUE`와 겹치지 않게 하고 이어 열기 명령끼리 일관되게 하기 위해서다. 채팅 id는 숫자라 `all`과 겹치지 않는다.

### TUI 종료 뒤 동작

TUI를 닫을 때 설정 `on_exit` 값이 닫은 뒤의 처리를 정한다. 값은 `background`, `stop`, `ask` 세 가지이고 기본값은 `background`다. 기본값을 `background`로 둔 것은 TUI를 닫아도 작업을 계속하게 하기 위해서다. 값은 닫는 TUI의 채팅 기준으로 폴더 층과 채팅 층까지 합쳐 읽는다.

`on_exit`는 마지막 TUI가 떨어질 때 적용한다. 마지막 TUI는 `Attach`한 접속 중 마지막이고, `saturn usage`처럼 `Attach`하지 않는 접속은 세지 않는다. 다른 TUI가 붙어 있는 동안에는 TUI를 닫아도 작업이 계속되므로 묻지도 안내하지도 않는다. 이 규칙에서 작업은 실행 중인 작업과 보내기 전에 판단하거나 기다리는 입력이고, 보류는 세지 않는다. 작업이 없으면 어느 값이든 묻지도 안내하지도 않고 닫는다.

| 값 | 닫으려 할 때 | 마지막 TUI가 떨어질 때 | 터미널 |
|---|---|---|---|
| `background` | 묻지 않는다. | 작업을 계속한다. | 작업이 있으면 `작업 N개 계속 실행 중 · saturn으로 다시 여세요` 한 줄 |
| `stop` | 묻지 않는다. | 모든 채팅의 작업을 멈춤과 같은 규칙으로 보류하고 끝낸다. | 없음 |
| `ask` | 작업이 있으면 `계속 실행`과 `멈추기`를 묻는 창을 띄운다. `Esc`는 닫기를 취소한다. | `계속 실행`이면 `background`와 같고, `멈추기`면 닫기 전에 모든 채팅을 멈춰 둔다. | `계속 실행`이면 `background`와 같은 한 줄 |

TUI는 닫기 전에 `PrepareExit`을 보내고, `engine`은 `ExitPlan`으로 답한다. `ExitPlan`은 그냥 닫는 `Close`, 묻는 `Ask`, 한 줄을 남기고 닫는 `Notice` 중 하나이고 작업 수 `running`을 싣는다. `ask`에서 `멈추기`를 고르면 TUI가 `StopAll`을 보내고 닫는다. 여러 채팅의 작업을 한 번에 멈추기 위해서다. 터미널을 닫거나 TUI가 강제로 끝나면 질문할 수 없으므로 `ask`도 `background`처럼 계속한다.

`stop`과 `ask`의 `멈추기`는 입력 처리의 멈춤 규칙을 작업이 있는 모든 채팅에 그대로 적용한다. 멈춘 작업은 자동으로 이어 가지 않고, 다시 열면 보류 재개 질문이 뜬다([입력 처리](input-handling.md#멈춤과-보류)). 채팅마다 따로 정하지 않고 모든 채팅을 함께 멈추는 것은, 앞서 닫은 TUI의 채팅이 사용자 몰래 계속 도는 일을 막기 위해서다.

TUI가 없는 동안 허가 요청, 입력 요청, 유예 종료는 값마다 다음과 같다.

| 항목 | `background`, `ask`의 `계속 실행` | `stop`, `ask`의 `멈추기` |
|---|---|---|
| 허가 요청과 입력 요청 | 사용자 확인을 기다리는 상태로 보관하고, TUI가 다시 붙으면 가장 먼저 보낸다. | 멈춤이 끝날 때 그 작업의 대기 요청을 지우고 보관하지 않는다. |
| 피드백 질문 | 건너뛴다. | 건너뛴다. |
| 완료 알림(macOS) | 켠 경우에만 보낸다. | 켠 경우에만 보낸다. |
| 보류 | 그대로 둔다. | 멈춘 작업을 보류로 두고 그대로 둔다. |
| 유예 종료 | 에이전트 트리 전체가 끝난 뒤 5분을 기다려 session을 닫고 `engine`을 끝낸다. | 멈춤으로 트리가 모두 유휴가 된 뒤 같은 5분을 기다려 끝낸다. |

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
| `~/.saturn/logs/engine-YYYY-MM-DD.log` | `engine` 로그. 로컬 날짜별 파일 하나, 30일 지난 파일은 시작 때와 날짜가 바뀔 때 삭제 | `engine` |
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

![크래시 뒤에는 효과 범위가 증명된 실행만 자동으로 이어 가고, 나머지는 보류해 /continue를 제안한다](../assets/crash-recovery.svg)

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
| 판단 방식에 맞는 router를 만들 수 없음 | 원인을 한 줄로 보이고 소켓을 열지 않고 끝낸다. |
| router 키가 없거나 틀려 시작 확인 실패 | 소켓을 열고 TUI가 키를 보낼 때까지 `SubmitRouterKey`, `Attach`, `Detach` 밖의 요청을 오류 응답으로 거절한다. |
| 화면이 없는 환경(파이프, CI)의 router 확인 실패 | `engine`은 키를 기다리고, `cli`가 키를 묻지 않고 끝내며 `SATURN_KEY` 환경 변수와 `router.key.command` 설정 방법을 안내한다. |
| 시작 때 이전 설정 번호 없음 | 실행하지 않는다. |
| 설정 검사 실패 | 이전 설정 번호를 유지하고 경고한다. |
| `engine` 비정상 종료 | 다시 시작한 `engine`이 `effect_scope`로 자동 재개와 보류를 나눈다. |
| provider 흐름의 관찰 중단 | `effect_scope`를 `unobserved`로 기록하고 자동으로 이어 가지 않는다. |
| 크래시 뒤 증명되지 않은 `effect_scope` | 자동으로 재개하지 않고 보류한 뒤 재개를 한 줄로 제안한다. |
| 에이전트가 실행한 중첩 `saturn` | 실행을 거절한다. |
| 한 채팅의 provider 요청이 느리거나 응답하지 않음 | 요청은 연결 작업이 실행하고 요청 처리 루프는 결과 메시지만 받으므로, 다른 채팅의 요청과 같은 채팅의 멈춤 요청은 기다리지 않고 처리한다([provider 요청 작업](providers-and-sessions.md#provider-요청-작업)). 응답 대기에는 요청 종류별 제한(바로 돌아와야 하는 요청 10초, 시작·열기 요청 60초, 초안)을 두고 턴은 자동으로 끊지 않는다. 응답 없는 요청의 처리는 [provider 오류 처리](providers-and-sessions.md#오류-처리)를 따른다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| `engine`은 사용자당 하나만 실행된다. | 같은 사용자로 `engine`을 두 번 띄우면 두 번째가 잠금을 얻지 못하고 기존 `engine`에 붙는지 확인 |
| `engine` 로그는 `~/.saturn/logs/engine-YYYY-MM-DD.log`에 로컬 날짜별로 하루 한 파일씩 쌓이고, 날짜가 바뀌면 새 파일을 연다. 30일 지난 `engine-YYYY-MM-DD.log`만 시작 때와 날짜가 바뀔 때 지우고, 옛 `engine.log`와 `engine.log.N`은 시작 때 지운다. | `saturn-terminal/engine/src/engine_log.rs`의 `engine_log_writes_to_a_file_named_after_the_day`, `engine_log_opens_a_new_file_when_the_day_changes`, `engine_log_start_removes_only_files_older_than_30_days`, `engine_log_removes_files_older_than_30_days_when_the_day_changes`, `engine_log_start_removes_the_old_engine_log_files`, `saturn-terminal/cli/src/launch.rs`의 `engine_log_tail_reads_the_latest_dated_file` |
| TUI를 닫아도 `engine`은 접수된 입력을 계속 처리한다. | TUI 연결을 끊은 뒤 대기 입력이 순서대로 provider에 전달되는지 확인 |
| `on_exit`가 `stop`이면 마지막 TUI가 떨어질 때 모든 채팅의 작업을 보류하고, 다른 TUI가 붙어 있는 동안에는 멈추지 않는다. 멈춘 작업은 자동으로 이어 가지 않는다. | `saturn-terminal/engine/src/lifecycle/exit.rs`의 `stop_holds_the_work_of_every_chat_only_when_the_last_tui_detaches`, `stop_on_exit_keeps_waiting_input_held_after_the_turn_ends` |
| 붙은 TUI가 같은 연결로 다른 채팅에 `Attach`하면 연결이 유지되고 붙은 채팅만 바뀌며, `on_exit`가 `stop`이어도 떠난 채팅의 작업을 멈추지 않는다. 그 뒤 `Detach`는 마지막 TUI 이탈로 센다. | `saturn-terminal/engine/src/lifecycle/exit.rs`의 `attach_to_another_chat_on_the_same_connection_is_not_a_detach` |
| `on_exit`가 `ask`나 `background`이면 TUI가 떨어져도 작업을 멈추지 않는다. | `saturn-terminal/engine/src/lifecycle/exit.rs`의 `ask_and_background_keep_running_after_the_last_tui_detaches` |
| 닫기 전 계획은 `on_exit`, 계속될 작업 수, 다른 TUI의 유무로 정한다. 작업이 없거나 다른 TUI가 붙어 있으면 묻지 않는다. | `saturn-terminal/engine/src/lifecycle/exit.rs`의 `exit_plan_follows_on_exit_work_and_other_tuis`, `prepare_exit_for_a_chat_the_client_is_not_attached_to_is_refused` |
| `ask`에서 `멈추기`를 고르면 모든 채팅의 작업을 멈춘다. | `saturn-terminal/engine/src/lifecycle/exit.rs`의 `stop_all_holds_the_work_of_every_chat` |
| router를 확인하기 전에는 router 키 제출과 채팅 붙기 밖의 요청을 받지 않는다. | router 키가 없는 환경에서 일반 요청이 오류 응답으로 거절되고, 키를 보낸 뒤에는 처리되는지 확인 |
| 채팅의 폴더, 폴더 설정 층, 폴더 설정 신뢰 판단은 채팅을 만든 폴더로 정하고, provider 실행 환경은 가장 최근에 붙은 TUI의 환경으로 정한다. | 다른 폴더에서 같은 채팅에 붙어도 폴더와 신뢰 창이 처음 폴더 그대로이고 환경만 바뀌는지 확인 |
| 더한 폴더는 채팅 기록에 저장돼 이어 열 때 되살아나고, 모든 provider session에 넘어가며, 그 폴더의 설정 파일은 읽지 않는다. | `saturn-terminal/engine/src/lifecycle/add_dir.rs`의 `add_dir_from_attach_is_saved_and_kept_when_the_chat_is_opened_again`, `add_dir_is_read_from_records_not_from_memory_when_a_chat_is_reopened`, `add_dir_reaches_the_session_spec_of_every_provider`, `add_dir_settings_file_is_not_read` |
| 쓰는 중에 더한 폴더는 열린 session을 건드리지 않고 다음 session부터 적용하며 TUI에 알린다. | `saturn-terminal/engine/src/lifecycle/add_dir.rs`의 `add_dir_while_a_session_is_open_applies_from_the_next_session`, `add_dir_without_an_open_session_applies_at_once`, `add_dir_twice_or_the_base_folder_changes_nothing` |
| 폴더가 아니거나 붙지 않은 채팅이면 더하지 않고, 틀린 `--add-dir`는 채팅을 만들지 않는다. | `saturn-terminal/engine/src/lifecycle/add_dir.rs`의 `add_dir_refuses_other_clients_and_paths_that_are_not_folders`, `add_dir_with_a_bad_attach_path_creates_no_chat`, `saturn-terminal/cli/src/commands/chat.rs`의 `add_dir_resolves_to_absolute_folders_and_rejects_files_and_missing_paths` |
| 다른 폴더의 채팅을 이어 열면 그 채팅의 기본 폴더에서 일한다. | `--resume all`로 다른 폴더의 채팅을 열어 작업 폴더가 처음 폴더인지 확인 |
| 넘겨받은 환경의 router 키 변수는 provider 자식 환경에 들어가지 않는다. | `Attach`의 `env`에 `SATURN_KEY`를 넣어도 그 채팅의 provider 환경에 없는지 확인 |
| 판단 방식에 맞는 router를 만들 수 없으면 Saturn을 실행하지 않는다. | 허용 호스트가 아닌 router 주소로 `engine`을 띄워 소켓을 열지 않고 끝나는지 확인 |
| 스키마를 올리기 전 백업 하나를 남긴다. | 옛 스키마 저장소로 새 버전을 실행한 뒤 백업이 하나만 남고 스키마가 올라갔는지 확인 |
| 크래시 뒤 자동 재개는 효과 범위가 설정이나 관찰로 증명된 실행에만 한다. | [공급자 적용 설정의 보고 범위 측정](https://github.com/woonyong-choi/saturn/issues/4), [하위 에이전트 외부 효과 경로 측정](https://github.com/woonyong-choi/saturn/issues/22) |
| 자동으로 이어 갈 때 같은 패킷을 다시 보내지 않는다. | 자동 재개 때 파일 상태로 만든 새 입력만 전송되는지 확인 |
| 강제 종료 뒤 subagent가 있던 session을 재개할 때의 동작을 확인한다. | [강제 종료 뒤 세션 재개 동작 측정](https://github.com/woonyong-choi/saturn/issues/24), [실측 결과](../experiments/crash-resume/report.md) |
| 에이전트가 실행한 `saturn`은 거절한다. | [하위 에이전트 훅 적용 범위 측정](https://github.com/woonyong-choi/saturn/issues/23) |
| 한 채팅의 provider 요청이 느리거나 응답하지 않아도 다른 채팅의 입력과 조회, 같은 채팅의 멈춤 요청을 바로 처리한다. | `saturn-terminal/engine/src/lifecycle/provider_stall.rs`의 `a_slow_start_request_does_not_stall_other_chats_or_stop`, `a_silent_provider_request_does_not_stall_other_chats_or_stop`, `a_provider_that_stops_reading_input_does_not_stall_other_chats_or_stop` |

## 대안

- TUI 프로세스 안에 `engine`을 넣는 방식은 TUI를 닫으면 작업이 멈추고 TUI마다 provider 연결과 기록 쓰기가 중복되어 버렸다([결정 기록](../decisions/2026-09-29-engine-centered-json-rpc.md)).
- 로컬 작업을 추정해 크래시 뒤 자동 재개하는 방식은 외부 효과가 중복될 수 있어 버렸다([결정 기록](../decisions/2026-09-29-proof-based-auto-resume.md)).
- Saturn이 provider의 네트워크 차단 설정을 고정하는 방식은 사용자의 provider 설정을 바꿔서 버렸다([결정 기록](../decisions/2026-09-29-proof-based-auto-resume.md)).

## 미해결 질문

- 에이전트가 실행한 자식 Saturn을 부모 `engine` 소켓에 자식으로 붙일지, 독립 `engine`으로 띄우고 결과 파일로 돌려받을지 ([#33](https://github.com/woonyong-choi/saturn/issues/33))
- 크래시 뒤 파일 상태 확인에 쓰는 수정 파일 목록을 실행 경계의 파일 상태 차이로 계산할지, provider 이벤트로 계산할지 ([#65](https://github.com/woonyong-choi/saturn/issues/65))
- 크래시 복구 때 실행 중으로 남은 subagent와 provider가 다시 불러오는 자식 session을 Saturn이 정리할지, 끊김 표시만 하고 provider 재개 동작은 그대로 둘지 ([#66](https://github.com/woonyong-choi/saturn/issues/66))
- `ListTasks`, `Usage`, `LoadHistory` 같은 조회 요청의 결과를 알림으로 보낼지, 응답 `result`에 담을지 ([#177](https://github.com/woonyong-choi/saturn/issues/177))
- 새 TUI나 `cli`가 버전이 다른 상주 `engine`에 붙을 때 그대로 붙을지, 거절할지, 옛 `engine`을 끝내고 새로 띄울지 ([#178](https://github.com/woonyong-choi/saturn/issues/178))
