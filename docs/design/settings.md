# 설정

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [provider 설정과 subagent 사용은 사용자 설정을 따르고 Saturn은 추적만 한다](../decisions/2026-09-29-minimal-provider-control.md), [권한 판단의 정본은 Saturn 설정의 `permission` 규칙 하나로 둔다](../decisions/2026-10-02-saturn-permission-authority.md) |

## 요약

설정은 여러 층의 설정 파일과 값을 합쳐 Saturn의 동작 값을 정하는 기능이다. engine의 `settings`가 기본값, 사용자, 폴더, 채팅, 실행 층을 차례로 병합하고 검사한다. 검사를 통과한 결과는 설정 번호가 붙은 스냅샷으로 기록 저장소에 남는다. 입력은 접수 때 고정한 설정 번호로 끝까지 처리한다.

## 동기

사용자는 모든 작업에 같은 값을 쓰기도 하고, 저장소마다 다른 값을 쓰기도 한다. 한 번만 값을 바꿔 실행하고 싶을 때도 있다. 층이 없으면 저장소마다 사용자 설정 파일을 고쳐야 한다. 저장소에 들어 있는 폴더 설정을 묻지 않고 적용하면 남이 만든 설정이 router 주소나 키 참조를 바꿀 수 있다. 처리 중에 설정이 바뀌면 한 입력의 앞뒤가 다른 값으로 처리되고, 나중에 어떤 값으로 처리했는지 알 수 없다. 이 기능은 층별 우선순위, 폴더 설정 신뢰, 입력마다 고정한 설정 번호로 이 문제를 막는다.

## 예시

### 처음 보는 폴더 설정을 만날 때

1. 사용자가 `.saturn/config.toml`이 들어 있는 저장소에서 `saturn`을 실행한다.
2. `settings`가 작업 폴더에서 git 맨 위까지 올라가며 폴더 설정을 찾는다.
3. 처음 보는 폴더 설정이라 TUI가 폴더 설정 신뢰 창을 연다.
4. 창은 파일 경로, 지문, 적용되는 항목, 무시되는 항목을 보인다.
5. 사용자가 `y`로 적용하고 계속을 고르고 `Enter`로 확정한다.
6. `settings`가 경로와 지문으로 신뢰를 기록하고 병합을 이어 간다.

### 폴더 설정에 오류가 있을 때

1. 사용자가 실행 중에 폴더 설정 파일의 7번째 줄을 잘못 고친다.
2. `settings`가 병합 결과를 검사하다 실패한다.
3. engine은 이전 설정 번호 12를 그대로 쓴다.
4. TUI가 `폴더 설정 오류 · 이전 설정 번호 12로 계속 · 줄 7: ...`를 보인다.

### 처리 중에 설정이 바뀔 때

1. 사용자가 입력을 보내고, engine이 접수 때 설정 번호 12를 고정한다.
2. 작업이 도는 중에 사용자가 사용자 설정을 고쳐 새 설정 번호 13이 생긴다.
3. 진행 중인 입력은 끝까지 설정 번호 12의 값으로 처리된다.
4. 그 뒤에 접수한 입력은 설정 번호 13을 쓴다.

## 상세 설계

### 설정 층

뒤 층의 값이 앞 층의 값보다 우선한다.

| 순서 | 층 | 위치 |
|---|---|---|
| 1 | 기본값 | Saturn 안의 기본값 |
| 2 | 사용자 | `~/.saturn/config.toml` |
| 3 | 폴더 | `<작업 폴더>/.saturn/config.toml` |
| 4 | 채팅 | 채팅마다 둔 값 |
| 5 | 실행 | 이번 실행의 `-c` 값 |

- 설정 원본은 파일이고, 기록 저장소에는 적용된 설정의 스냅샷만 둔다. 입력마다 그때 쓴 설정을 번호로 남기기 위해서다.
- 잠깐 들여다본 다른 폴더의 설정은 적용하지 않고 참고 자료로만 읽는다. 작업 폴더가 아닌 폴더의 설정이 실행에 섞이는 일을 막기 위해서다. 같은 이유로 `--add-dir`와 `/add-dir`로 더한 폴더의 설정 파일은 읽지 않고, 폴더 층은 채팅의 기본 폴더 것만 쓴다([engine 수명과 복구](engine-lifecycle.md#채팅-폴더와-이어-열기)).
- 보조 에이전트는 부모 채팅의 채팅 층을 물려받는다. 보조 에이전트가 메인 에이전트와 같은 설정으로 실행되게 하기 위해서다.
- router 기준값은 설정 층에 둔다. 릴리스 없이 사용자 층과 폴더 층에서 기준값을 조정하기 위해서다. 기준값 조정은 [router 학습](router-training.md)에 있다.

### 설정 키

아래 표가 키 이름과 기본값의 1판이다. 측정으로 정할 값은 초안 표시를 유지한다. 병합 결과에 아래 표에 없는 키가 있으면 검사에 실패한다.

| 키 | 값 | 기본값 |
|---|---|---|
| `on_exit` | `background`, `stop`, `ask` | `background` |
| `agents.worktree` | 참·거짓 | 거짓 |
| `permission.mode` | `ask`, `edit`, `read-only`, `full` | `edit`(초안) |
| `permission.shell`, `permission.edit`, `permission.mcp`, `permission.subagent` | `allow`, `ask`, `deny` 또는 패턴 → 값 표 | 모드를 따름 |
| `router.method` | `jev`, `saturn`, `collect` | `jev` |
| `router.endpoint` | 문자열 | `https://api.typesafe.ai` |
| `router.model` | 문자열 | `jev-1.13.0` |
| `router.local.endpoint`, `router.local.version` | 문자열 | 없음 |
| `router.skip_check` | 참·거짓(도움말에 없음) | 없음 |
| `router.key.info` | `source`(`Stored`, `Env`, `Command`), `last4` | 없음 |
| `router.key.command` | 문자열 배열 | 없음 |
| `router.key.storage` | `standard`, `hardened` | `standard` |
| `router.thresholds.<이름>` | 0~1 실수 | [router](router.md) 표의 값 |
| `grading.model` | 문자열 | 없음 |
| `consent.share_with_server` | 참·거짓 | 거짓 |
| `retention.max_age_days` | 1 이상 정수 | 없음(무제한 보존). `saturn prune`이 오래된 채팅을 정하는 기준이기도 하다([기록](records.md)) |
| `context.safety_percent` | 0~100 정수 | 70 |
| `context.<codex\|claude>.t_abs` | 1 이상 정수 | 200000 |
| `context.<codex\|claude>.window` | 1 이상 정수 | codex 272000, claude 1000000 |
| `context.<codex\|claude>.cache_read`, `cache_write` | 0 이상 실수 | 0.1, codex 1.0 · claude 1.25 |
| `context.mode` | `saturn`, `provider` | `saturn` |
| `context.packet_hard_divisor` | 1 이상 정수 | 5 |
| `context.item_cap_percent` | 1~100 정수 | 30 |
| `context.constraint_slot_divisor` | 1 이상 정수 | 4(초안) |
| `context.select.rrf_k` | 0 이상 정수 | 60 |

- `on_exit`는 TUI를 닫을 때 작업을 어떻게 할지 정한다. `background`는 계속하고, `stop`은 모든 채팅의 작업을 멈춤과 같게 보류하고, `ask`는 작업이 있으면 닫기 전에 묻는다. 규칙은 [engine 수명과 복구](engine-lifecycle.md#tui-종료-뒤-동작)에 있다.
- `agents.worktree`가 거짓이면 보조 에이전트는 같은 폴더에서 한 번에 하나씩 쓴다. 참이면 git 저장소일 때만 보조 에이전트의 쓰기를 별도 worktree에서 병렬로 하고, git 저장소가 아니면 거짓일 때와 같다. 쓰기 격리를 사용자가 켠 뒤에만 하기 위해서다. 규칙은 [입력 처리](input-handling.md)에 있다.
- `permission.mode`는 기본 규칙 묶음이다. `edit`는 작업 폴더 안 편집을 허용하고 나머지는 묻는다. `ask`는 모두 묻고, `read-only`는 읽기만 허용하고, `full`은 `deny` 규칙을 뺀 모두를 허용한다. 모드의 뜻과 provider 대응은 [권한](permissions.md)에 있다. `full`이면 에이전트 질문 기능도 두 provider에서 끈다. 질문만 따로 정하는 키는 없다([입력 요청](input-requests.md#에이전트-질문-설정)). 층 병합에서 폴더 층의 `permission.mode`는 낮은 쪽부터 `read-only`, `ask`, `edit`, `full` 순서일 때 앞 층까지 합친 모드보다 낮은 값만 적용하고, 같거나 높은 값은 무시해 신뢰 창의 무시되는 항목(`permission.mode`)에 보이고, 병합 때 한 줄 경고에도 남는다. 채팅 층의 `/permissions`와 실행 층 `-c`에는 이 제한이 없다.
- `permission.shell` 같은 개별 규칙은 셸 명령, 파일 편집, MCP 도구, subagent 실행의 허용, 묻기, 거부 규칙이다. 문자열 하나면 그 도구 전체에 적용하고, 패턴 표를 주면 패턴마다 값을 준다. 모드 기본 규칙 뒤에 사용자 층 규칙, 폴더 층 규칙, 채팅 층 규칙, 실행 층 규칙을 잇고 마지막으로 일치한 규칙이 이긴다. 단 어느 층이든 `deny`가 하나라도 일치하면 거부한다(채팅 층과 실행 층까지 넣은 것은 초안). 같은 층 안의 순서는 파일에 적힌 순서다(초안). 병합 결과의 `permission`에는 합친 모드와 이은 규칙 목록이 들어가고, 입력은 접수 때 고정한 설정 번호의 목록을 쓴다. `permission` 아래 모르는 키, 모르는 모드, `allow`, `ask`, `deny`가 아닌 값은 검사에 실패한다. 패턴 문법과 판정 흐름, provider별 번역은 [권한](permissions.md)에 있다.
- `context.select.rrf_k`는 router가 답하지 못한 항목의 순서와 같은 확률인 항목의 순서에만 쓴다.
- 기준값 이름은 `keep_current`, `is_actionable`, `min_confidence`, `resume_held`, `file_present`, `file_absent`, `context_gate`, `injection`, `progressing`, `feedback_cause`, `is_constraint`, `constraint_replace`, `constraint_conflict`, `constraint_ask`, `constraint_same`, `constraint_release`다. `constraint_conflict`는 대체 질문을 사용자에게 묻는 하한이고 `constraint_ask`는 등록과 해제 질문의 묻는 하한이다. 제약 칸 상한은 `context.constraint_slot_divisor`로 `P_max`를 나눈 값이다([제약](constraints.md)).
- 되돌릴 수 없는 행동의 기준값 `keep_current`, `resume_held`는 0.8 미만이면 검사에 실패한다(목록은 초안).
- 실행 층 `-c key=value`의 값은 TOML 값 문법으로 읽고, 같은 키가 여러 번 오면 뒤 값이 이긴다.

### 폴더 층에서 바꿀 수 없는 항목

아래 항목은 사용자 전용이라 폴더 층에서 바꿀 수 없다. 폴더 설정 신뢰 창은 이 항목을 무시되는 항목에 표시한다.

| 항목 | 다루는 문서 |
|---|---|
| router 주소와 키 참조 | [router 키 보호](router-key-security.md) |
| 채점 모델 | [router 학습](router-training.md) |
| 데이터 공유 동의 | [router 학습](router-training.md) |
| 판단 방식 | [router](router.md) |

사용자 전용 키는 `router.endpoint`, `router.key`, `grading.model`, `consent`, `router.method`와 같거나 그 아래 키다(초안). router 키 자체는 설정 파일에 두지 않는다. 설정에는 키의 출처와 끝 4자리만 남는다([router 키 보호](router-key-security.md)).

### 병합과 설정 번호

engine이 시작하면 사용자당 잠금을 얻고 스키마 이관을 마친 뒤 기본값, 사용자, 실행 층을 병합한다. router 시작 확인은 이 설정 번호가 확정된 뒤에 한다. 폴더 층과 채팅 층은 TUI가 채팅에 붙을 때 채팅마다 병합한다. 채팅은 처음 만든 폴더에 묶이므로, 이미 있는 채팅에 다른 폴더의 TUI가 붙어도 처음 폴더로 병합한다. 아래 절차의 3~5단계가 그때 돈다.

1. `settings`가 기본값 층을 읽는다.
2. `settings`가 사용자 층 `~/.saturn/config.toml`을 읽는다.
3. `settings`가 작업 폴더에서 git 맨 위까지 올라가며 폴더 층 `<작업 폴더>/.saturn/config.toml`을 찾는다.
4. 처음 보거나 내용이 바뀐 폴더 설정이면 `settings`가 한 번 묻고 경로와 지문으로 신뢰를 기록한다.
5. `settings`가 채팅 층과 실행 `-c` 층까지 병합하고, 뒤 층의 값을 우선한다.
6. `settings`가 병합 결과를 검사한다.
7. 검사를 통과하면 `store`가 병합 결과와 층 목록을 새 설정 번호의 스냅샷으로 기록한다.
8. 같은 내용의 스냅샷이 이미 있으면 `store`는 기존 설정 번호를 다시 쓴다.

- 같은 내용의 설정은 기존 설정 번호를 다시 쓴다. 여러 프로세스가 같은 설정에 번호를 중복으로 만드는 일을 막기 위해서다.
- router 판단 기록에는 질문 버전과 함께 설정 번호를 남긴다. 버전이 바뀐 뒤에도 옛 기록을 다시 해석하기 위해서다.

### 입력마다 설정 번호 고정

1. `queue`가 입력을 접수할 때 그 입력의 설정 번호와 권한을 고정한다.
2. engine이 provider를 실행할 때 고정한 설정 번호의 값을 조회한다.
3. 그 입력의 처리가 끝날 때까지 같은 설정 번호를 쓴다.

- 입력은 접수 때 고정한 설정 번호로 끝까지 처리한다. 처리 중 설정이 바뀌어도 한 입력을 한 설정으로 처리하기 위해서다.
- 입력 접수와 처리 흐름은 [입력 처리](input-handling.md)에 있다.

### 설정 변경 감지

engine이 TUI가 붙은 채팅마다 사용자 설정과 폴더 설정 파일의 지문을 주기적으로 확인한다. 바뀌었으면 입력을 기다리지 않고 병합해 그 채팅의 provider 연결에 적용한다. 연결에 적용하는 방법(작업 중이 아니면 바로, 작업 중이면 턴 끝에 다시 시작)은 [권한](permissions.md#codex-구성)과 같다.

1. engine이 0.5초마다 붙은 채팅의 사용자 설정과 폴더 설정 파일 내용을 읽어 지문을 마지막 병합 때 지문과 비교한다.
2. 달라졌으면 지문을 기록만 하고 적용하지 않는다.
3. 다음 확인에서도 지문이 같으면 병합하고, 검사 결과를 입력 접수와 같은 방식으로 알린다.
4. 병합으로 설정 번호가 정해지면 provider 연결에 바뀐 설정을 적용한다.

- 파일 감시 crate 대신 주기 확인을 쓴다(초안). 폴더 설정은 작업 폴더에서 git 맨 위까지 올라가며 찾고 나중에 생길 수 있어 감시할 경로를 미리 정하기 어렵고, 편집기가 임시 파일로 갈아 끼우는 저장과 링크, 신뢰 지문 비교를 이벤트로 따로 다루지 않아도 되며, 의존성이 늘지 않기 위해서다. 이 확인은 채팅마다 파일 둘의 지문만 읽으므로 비용이 작다.
- 확인 주기 0.5초는 상수 하나이고 설정 키가 없다(초안). 같은 지문을 연달아 두 번 볼 때만 적용하므로 저장 뒤 적용까지 1초 안팎이다.
- 두 번 연달아 같을 때만 적용하는 것은 편집기가 파일을 쓰는 도중의 반쯤 쓴 내용을 읽고 검사 실패로 오작동하는 일을 막기 위해서다. 쓰는 도중 파일이 계속 바뀌면 적용은 멈춘 뒤로 미룬다.
- 검사에 실패한 파일은 이전 설정 번호를 유지하고 경고를 한 번 알린다. 같은 내용으로는 다시 적용하지 않는다.
- 감시는 engine만 하고 `core`는 파일을 읽지 않는다. 채팅에 붙은 TUI가 없으면 확인하지 않고, 다음에 붙거나 입력을 접수할 때 알아챈다.

### 폴더 설정 신뢰

폴더 설정은 저장소와 함께 들어오므로 사용자가 한 번 확인한 뒤에만 적용한다. `settings`는 신뢰를 설정 파일의 경로와 지문으로 기록한다.

- 신뢰한 폴더 설정의 내용이 바뀌면 다시 묻고 새 지문으로 신뢰를 기록한다. 확인하지 않은 변경이 적용되는 일을 막기 위해서다.
- 폴더 설정 신뢰는 채팅마다, 그 채팅을 처음 만든 폴더 기준으로 묻는다. 적용을 고르지 않으면 그 채팅은 폴더 설정 없이 계속한다.
- 실행 중에 폴더 설정이 바뀌면 신뢰 창은 engine이 변경을 알아챈 때 그 채팅에 붙은 TUI에 열린다. 그 전에 입력을 접수하면 접수 전에 열린다.
- 신뢰 기록은 `~/.saturn/trusted.json`(경로 → 지문, 권한 0600)에 두고, 지문은 파일 내용의 SHA-256 hex다(초안).
- 신뢰한 뒤 내용이 바뀐 파일은 옛 내용을 두지 않으므로, 바뀐 줄로 빈 줄과 주석을 뺀 모든 줄을 보인다(초안).

폴더 설정 신뢰 창은 파일 경로, 지문, 적용되는 항목, 무시되는 항목, 바뀐 줄을 보인다.

| 키 | 동작 |
|---|---|
| `1`, `y` | 적용하고 계속 강조 |
| `↑`, `↓` | 선택지 이동 |
| `Enter` | 강조한 선택지 확정 |
| `3`, `q`, `Esc`, `Ctrl+C` | 종료 |

### 설정 파일 편집

명령으로 설정 파일을 고칠 때 `settings`는 `toml_edit`로 주석을 보존한다. 파일을 쓰기 전에 읽은 버전이 그대로인지 확인한다. 다른 곳에서 고친 내용과 사용자 주석을 잃지 않기 위해서다.

- 버전은 내용 지문으로 비교하고, 고친 내용은 같은 폴더의 `<파일 이름>.partial`을 새로 만들어 쓴 뒤 이름을 바꿔 갈아 끼운다. 원래 파일 권한은 그대로 두고 새 파일은 0600으로 만든다.
- 고칠 수 있는 파일은 사용자 설정과 폴더 설정(`.saturn/config.toml`)뿐이다.

### provider 설정과의 관계

Saturn 설정은 provider 설정 파일을 바꾸지 않는다. 권한은 `permission` 규칙이 정본이고, Saturn이 provider별 실행 설정으로 번역해 넘긴다([결정 기록](../decisions/2026-10-02-saturn-permission-authority.md)). 권한 외 Saturn 기본값은 사용자 provider 설정에 값이 없을 때만 실행 인자로 넘긴다. 사용자가 정한 provider 설정을 덮어쓰지 않기 위해서다([결정 기록](../decisions/2026-09-29-minimal-provider-control.md)).

### 오류 처리

| 상황 | 동작 |
|---|---|
| 병합 결과 검사 실패 | 이전 설정 번호를 유지하고 `<층> 설정 오류 · 이전 설정 번호 12로 계속 · 줄 7: ...`(검사 실패면 `키: 이유`)로 경고한다. |
| 시작 때 이전 설정 번호 없음 | 실행하지 않는다. |
| 신뢰한 폴더 설정의 내용 변경 | 다시 묻고 새 지문으로 신뢰를 기록한다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 뒤 층의 값이 앞 층의 값보다 우선한다. | 다섯 층에 같은 키를 다른 값으로 두고 `-c` 값이 적용되는지 확인한다. |
| 더한 폴더의 설정 파일은 읽지 않는다. | `saturn-terminal/engine/src/lifecycle/add_dir.rs`의 `add_dir_settings_file_is_not_read` |
| 폴더 층은 router 주소와 키 참조, 채점 모델, 데이터 공유 동의, 판단 방식을 바꾸지 못한다. | 폴더 설정에 이 항목을 넣어도 병합 결과가 사용자 층 값인지 확인한다. |
| 처음 보거나 바뀐 폴더 설정은 사용자 확인 전에는 적용하지 않는다. | 지문이 바뀐 폴더 설정이 신뢰 창을 거치기 전에 병합되지 않는지 확인한다. |
| 같은 내용의 설정은 같은 설정 번호를 쓴다. | 같은 설정으로 두 번 병합해 설정 번호가 하나만 생기는지 확인한다. |
| 입력은 접수 때 고정한 설정 번호로 끝까지 처리한다. | 처리 중 설정을 바꿔도 그 입력의 provider 실행 값이 접수 때 값인지 확인한다. |
| 검사에 실패한 설정은 적용하지 않고 이전 설정 번호를 쓴다. | 잘못된 폴더 설정으로 바꾼 뒤 이전 설정 번호가 유지되는지 확인한다. |
| 설정 파일을 바꾸면 입력 없이도 engine이 알아채 provider에 적용한다. 쓰는 도중의 파일은 적용하지 않고, 잘못된 파일은 이전 설정을 유지하며 경고한다. | `saturn-terminal/engine/src/lifecycle/settings_watch.rs`의 `settings_watch_restarts_an_idle_chat_without_any_input`, `settings_watch_waits_for_the_turn_end_when_the_chat_is_running`, `settings_watch_ignores_a_file_that_is_still_being_written`, `settings_watch_keeps_the_connection_and_warns_when_the_file_is_invalid`, `settings_watch_runs_in_the_serve_loop` |
| 명령으로 설정 파일을 고쳐도 주석이 남는다. | 주석이 있는 파일을 명령으로 고친 뒤 주석이 그대로인지 확인한다. |
| 설정 파일은 읽은 버전을 확인한 뒤 쓴다. | 읽기와 쓰기 사이에 파일을 고쳐도 그 변경이 사라지지 않는지 확인한다. |
