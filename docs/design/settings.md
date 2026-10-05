# 설정

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [provider 설정과 subagent 사용은 사용자 설정을 따르고 Saturn은 추적만 한다](../decisions/2026-09-29-minimal-provider-control.md), [권한 판단의 정본은 Saturn 설정의 `permission` 규칙 하나로 둔다](../decisions/2026-10-02-saturn-permission-authority.md), [provider는 열린 id의 어댑터로 붙이고 확장은 Saturn 저장소에 설치해 session을 열 때 주입한다](../decisions/2026-10-04-open-providers-and-saturn-extensions.md) |

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

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/settings-layers.ko.dark.svg">
  <img src="../assets/settings-layers.ko.light.svg" alt="설정은 기본값, 사용자, 폴더, 채팅, 실행 층을 차례로 병합하고 뒤 층의 값이 앞 층을 이긴다" width="100%">
</picture>

뒤 층의 값이 앞 층의 값보다 우선한다.

| 순서 | 층 | 위치 |
|---|---|---|
| 1 | 기본값 | Saturn 안의 기본값 |
| 2 | 사용자 | `<Saturn 홈>/config.toml`(기본 `~/.saturn/config.toml`) |
| 3 | 폴더 | `<작업 폴더>/.saturn/config.toml` |
| 4 | 채팅 | 채팅마다 둔 값 |
| 5 | 실행 | 이번 실행의 `-c` 값 |

- Saturn 홈은 engine이 소켓, 로그, 기록 저장소, 사용자 설정을 두는 폴더다. 환경 변수 `SATURN_HOME`이 있으면 그 폴더이고, 없거나 비어 있으면 `~/.saturn`이다. `SATURN_HOME`이 상대 경로이면 현재 폴더 기준 절대 경로로 바꿔 쓴다. engine의 `--home`이 있으면 그 값이 이기고, `cli`는 engine을 띄울 때 자기가 찾은 홈을 `--home`으로 넘겨 `cli`, TUI, engine이 같은 폴더를 보게 한다. 위치는 설정 파일의 키가 아니므로 설정 층을 읽기 전에 정해진다([이름 규칙](#이름-규칙)).
- 설정 원본은 파일이고, 기록 저장소에는 적용된 설정의 스냅샷만 둔다. 입력마다 그때 쓴 설정을 번호로 남기기 위해서다.
- 잠깐 들여다본 다른 폴더의 설정은 적용하지 않고 참고 자료로만 읽는다. 작업 폴더가 아닌 폴더의 설정이 실행에 섞이는 일을 막기 위해서다. 같은 이유로 `--add-dir`와 `/add-dir`로 더한 폴더의 설정 파일은 읽지 않고, 폴더 층은 채팅의 기본 폴더 것만 쓴다([engine 수명과 복구](engine-lifecycle.md#채팅-폴더와-이어-열기)).
- 보조 에이전트는 부모 채팅의 채팅 층을 물려받는다. 보조 에이전트가 메인 에이전트와 같은 설정으로 실행되게 하기 위해서다.
- router 기준값은 설정 층에 둔다. 릴리스 없이 사용자 층과 폴더 층에서 기준값을 조정하기 위해서다. 기준값은 자동으로 조정하지 않고 설정 변경으로만 바꾼다([정책 고정](router.md#정책-고정)). 기록으로 기준값을 계산하는 규칙은 [router 학습](router-training.md)에 있다.

### 이름 규칙

설정 키 이름은 Saturn이 정한 이 규칙 하나를 따른다. 어느 provider의 설정 형식도 따르지 않고, 기능의 뜻만 다른 도구가 같은 개념에 쓰는 말과 맞춘다. 키는 소문자 영문, 숫자, 밑줄(`_`), 점(`.`)으로만 쓰고, 점은 묶음과 키를 나눈다. 이 절이 정본이고 스키마 시험이 아래 규칙을 지키는지 확인한다.

| 번호 | 규칙 | 예 |
|---|---|---|
| 1 | 묶음 없는 키는 두지 않는다. 키는 항상 `묶음.키` 모양이다. | `tui.on_exit` |
| 2 | 묶음 이름은 단수다. 맨 앞 묶음에만 적용하고, `router.thresholds`처럼 이름을 키로 갖는 안쪽 표는 복수로 쓴다. | `agent.worktree`, `permission.shell` |
| 3 | 켜고 끄는 값은 켜짐을 뜻하는 이름을 쓰고 기본은 끔이다. `enable_`, `is_`, `use_`, `has_` 접두사는 쓰지 않는다. | `notify.on_done`, `retention.auto_prune` |
| 4 | 단위가 있는 값은 단위 접미사를 쓴다. 접미사는 `_days`, `_sec`, `_ms`, `_bytes`, `_percent`뿐이다. `_percent` 키는 정수 퍼센트다. | `retention.max_age_days`, `context.safety_percent` |
| 5 | 고르는 값은 소문자 영문이고 두 단어는 `-`로 잇는다. | `read-only`, `hardened` |
| 6 | 동작 방식을 고르는 키는 `mode`다. `method`, `type`, `strategy`, `style`은 쓰지 않는다. | `router.mode`, `context.mode` |
| 7 | 보안과 비용에 걸린 키는 사용자 설정에서만 정한다. 폴더 층에서는 무시한다([폴더 층에서 바꿀 수 없는 항목](#폴더-층에서-바꿀-수-없는-항목)). | `router.endpoint`, `retention.max_age_days` |
| 8 | 위치는 설정 키가 아니라 환경 변수 `SATURN_HOME`으로 정한다. `_dir`, `_path`, `_file`로 끝나는 키는 두지 않는다. | `SATURN_HOME` |

- 규칙을 바꾸기 전에 만든 키는 옛 이름을 별칭으로 계속 읽는다. 별칭은 층마다 병합 전에 새 이름으로 옮기고, 옮길 때마다 한 줄 경고(`옛 설정 이름을 새 이름으로 읽음 · on_exit → tui.on_exit`)를 TUI와 engine 로그에 남긴다. 같은 층에 새 이름이 있으면 새 이름이 이긴다. 새 이름만 설정 번호별 스냅샷에 저장하고, 옛 스냅샷은 옛 이름 그대로 읽는다. 값 이름도 같다. router 키 출처는 소문자(`stored`, `env`, `command`)로 쓰고 옛 `Stored`, `Env`, `Command`도 읽는다.
- 안쪽 표의 이름은 규칙 2의 예외가 아니라 키의 일부다. 기준값 이름(`router.thresholds.<이름>`)은 질문 이름을 그대로 쓰므로 규칙 3의 접두사 금지는 불리언 값에만 적용한다.
- 나눗수 키는 두지 않는다. 비율은 퍼센트로 쓴다.

#### 옛 이름과 새 이름

| 옛 이름 | 새 이름 | 값 변환 | 상태 |
|---|---|---|---|
| `on_exit` | `tui.on_exit` | 없음 | 구현 |
| `router.method` | `router.mode` | 없음 | 구현 |
| `context.packet_hard_divisor` | `context.packet_hard_percent` | 나눗수 `d`를 `100 / d`로(5는 20). 나누어떨어지지 않으면 버림 | 구현 |
| `context.constraint_slot_divisor` | `context.constraint_slot_percent` | 나눗수 `d`를 `100 / d`로(4는 25) | 구현 전([#380](https://github.com/woonyong-choi/saturn/issues/380)). 키는 구현했고 이 옛 이름을 읽는 별칭은 아직 없다. 이 이름으로 저장한 스냅샷은 없다 |
| `agents.worktree` | `agent.worktree` | 없음 | 구현 전([#335](https://github.com/woonyong-choi/saturn/issues/335)). 키와 함께 별칭도 구현 |
| `router.key.info.source`의 `Stored`, `Env`, `Command` | `stored`, `env`, `command` | 소문자로 | 구현. 경고 없이 읽음 |
| `context.<id>.<키>` | `provider.<id>.context.<키>` | 없음 | 구현(아래 [provider 설정 키](#provider-설정-키)) |

### 설정 키

아래 표가 키 이름과 기본값의 1판이고, 이름은 위 [이름 규칙](#이름-규칙)을 따른다. 측정으로 정할 값은 초안 표시를 유지한다. 병합 결과에 아래 표에 없는 키가 있으면 검사에 실패한다. 기능과 함께 구현할 키는 표에 "구현 전"으로 적고, 그 키는 구현하기 전까지 검사에서 모르는 키로 실패한다. 표시와 스키마가 어긋나지 않는지는 시험이 문서를 읽어 확인한다.

| 키 | 값 | 기본값 |
|---|---|---|
| `tui.on_exit` | `background`, `stop`, `ask` | `background` |
| `tui.keymap` | `saturn`, `claude`, `codex`, `gemini`, `opencode` | `saturn`. TUI 키 묶음이고 `/keymap`은 이 TUI만 바꾼다([TUI](tui.md#키-해석-계층)) |
| `tui.screen` | `auto`, `full`, `plain` | `auto`. 단순 방식은 [TUI](tui.md#단순-방식) |
| `notify.on_done` | 참·거짓 | 거짓. 키만 받는다. 알림을 보내는 동작은 [#151](https://github.com/woonyong-choi/saturn/issues/151) |
| `agent.worktree` | 참·거짓 | 거짓. 구현 전([#335](https://github.com/woonyong-choi/saturn/issues/335)) |
| `permission.mode` | `ask`, `edit`, `read-only`, `full` | `edit`(초안) |
| `permission.shell`, `permission.edit`, `permission.read`, `permission.mcp`, `permission.subagent` | `allow`, `ask`, `deny` 또는 패턴 → 값 표 | 모드를 따름 |
| `model.default` | `<provider>/<model>` 문자열 | 없음. 처음 고르기 창이 사용자 설정 파일에 쓴다([기본 모델과 선택 방식](providers-and-sessions.md#기본-모델과-선택-방식)) |
| `model.mode` | `auto`, `manual` | `auto` |
| `router.mode` | `jev`, `saturn`, `collect` | `jev` |
| `router.endpoint` | 문자열 | `https://api.typesafe.ai` |
| `router.model` | 문자열 | `jev-1.13.0` |
| `router.local.endpoint`, `router.local.version` | 문자열 | 없음 |
| `router.skip_check` | 참·거짓(도움말에 없음) | 없음 |
| `router.key.info` | `source`(`stored`, `env`, `command`), `last4` | 없음 |
| `router.key.command` | 문자열 배열 | 없음 |
| `router.key.storage` | `standard`, `hardened` | `standard` |
| `router.thresholds.<이름>` | 0~1 실수 | [router](router.md) 표의 값 |
| `grading.model` | 문자열 | 없음 |
| `consent.share_with_server` | 참·거짓 | 거짓 |
| `child.max_depth` | 0 이상 정수 | 2. 하위 접속의 깊이 상한이고 0이면 하위 접속을 받지 않는다([하위 접속](child-sessions.md#상한과-대기열)) |
| `child.max_concurrent` | 1 이상 정수 | 5. 한 채팅이 동시에 거느리는 하위 접속 수 |
| `child.max_total` | 1 이상 정수 | 10. `engine` 전체의 동시 하위 접속 수 |
| `constraint.auto_apply` | 참·거짓 | 거짓. 참이면 router의 `is_constraint` 판단이 지속 제약을 자동 등록하거나 묻고 `constraint_change` 판단이 제약을 해제하거나 예외를 건다. 거짓이면 판단은 기록만 하고 제약을 만들지도 바꾸지도 않는다. 사용자의 명시 해제는 이 키와 상관없다. 사용자 층에서만 정한다([제약](constraints.md#식별-순서)) |
| `debug.provider_events` | 참·거짓 | 거짓. 켜면 provider 연결이 받은 원시 메시지의 모양(방법 이름, 순서, ID, 필드 이름)을 값 없이 `logs/provider-events-<날짜>.log`에 남긴다([provider 연결과 session](providers-and-sessions.md#원시-이벤트-관측-기록)) |
| `retention.max_age_days` | 1 이상 정수 | 없음(무제한 보존). `saturn prune`과 `/prune`이 오래된 채팅을 정하는 기준이기도 하다([기록](records.md)) |
| `retention.auto_prune` | 참·거짓 | 거짓. 참이고 `max_age_days`가 있을 때만 engine 시작 때 한 번 그 기한보다 오래 쓰지 않은 채팅을 지운다. 삭제라 `max_age_days`만으로 켜지지 않는다([기록](records.md#보존과-정리)) |
| `context.safety_percent` | 0~100 정수 | 70 |
| `context.mode` | `saturn`, `provider` | `saturn` |
| `context.packet_hard_percent` | 1~100 정수 | 20. engine이 읽어 패킷 크기 상한에 적용한다 |
| `context.item_cap_percent` | 1~100 정수 | 30. engine이 읽어 경쟁 항목 길이 상한에 적용한다 |
| `context.constraint_slot_percent` | 1~100 정수 | 25(초안). engine이 읽어 패킷 제약 칸 상한에 적용한다 |
| `context.select.rrf_k` | 0 이상 정수 | 60 |

- `tui.on_exit`는 TUI를 닫을 때 작업을 어떻게 할지 정한다. `background`는 계속하고, `stop`은 모든 채팅의 작업을 멈춤과 같게 보류하고, `ask`는 작업이 있으면 닫기 전에 묻는다. 규칙은 [engine 수명과 복구](engine-lifecycle.md#tui-종료-뒤-동작)에 있다.
- `model.default`는 새 작업을 보낼 기본 모델이고, `model.mode`는 새 작업의 모델을 router가 고를지(`auto`) 사용자가 정한 모델로만 보낼지(`manual`) 정한다. 두 키 모두 사용자 층, 폴더 층, 실행 층에서 정할 수 있고, 입력은 접수 때 고정한 설정 번호의 값을 쓴다. `model.default`가 `<provider>/<model>` 모양이 아니거나 등록하지 않은 provider id면 고르지 않은 것으로 보고 처음 고르기 창을 다시 연다. 모델 창은 값을 사용자 설정 파일에 쓰므로, 폴더 설정이나 `-c`가 같은 키를 정했으면 그 값이 이기고 창은 병합 결과를 보인다. 규칙은 [기본 모델과 선택 방식](providers-and-sessions.md#기본-모델과-선택-방식)에 있다.
- `tui.screen`은 대화 화면을 그리는 방식이다. `auto`는 터미널이면 전체 화면, 파이프나 CI면 plain이고, `full`과 `plain`은 고정한다. `plain`은 [단순 방식](tui.md#단순-방식)이다. 옵션 `--plain`과 환경 변수 `NO_COLOR`가 이 설정보다 앞서고, 터미널이 아니면 값과 관계없이 plain이다.
- `notify.on_done`은 작업이 모두 끝났을 때 알림을 보낼지 정한다. 이 문서는 키 이름과 기본값(거짓)만 정하고, 알림 동작은 [#151](https://github.com/woonyong-choi/saturn/issues/151)이 정한다. 지금은 키를 받아 저장만 한다.
- `agent.worktree`가 거짓이면 보조 에이전트는 같은 폴더에서 한 번에 하나씩 쓴다. 참이면 git 저장소일 때만 보조 에이전트의 쓰기를 별도 worktree에서 병렬로 하고, git 저장소가 아니면 거짓일 때와 같다. 쓰기 격리를 사용자가 켠 뒤에만 하기 위해서다. 규칙은 [입력 처리](input-handling.md)에 있다.
- `permission.mode`는 기본 규칙 묶음이다. `edit`는 작업 폴더 안 편집을 허용하고 나머지는 묻는다. `ask`는 모두 묻고, `read-only`는 읽기만 허용하고, `full`은 `deny` 규칙을 뺀 모두를 허용한다. 모드의 뜻과 provider 대응은 [권한](permissions.md)에 있다. `full`이면 에이전트 질문 기능도 두 provider에서 끈다. 질문만 따로 정하는 키는 없다([입력 요청](input-requests.md#에이전트-질문-설정)). 층 병합에서 폴더 층의 `permission.mode`는 낮은 쪽부터 `read-only`, `ask`, `edit`, `full` 순서일 때 앞 층까지 합친 모드보다 낮은 값만 적용하고, 같거나 높은 값은 무시해 신뢰 창의 무시되는 항목(`permission.mode`)에 보이고, 병합 때 한 줄 경고에도 남는다. 채팅 층의 `/permissions`와 실행 층 `-c`에는 이 제한이 없다.
- `permission.shell` 같은 개별 규칙은 셸 명령, 파일 편집, 파일 읽기(Claude는 폴더 밖 읽기 도구, Codex는 읽기로 분류된 명령의 경로에 닿는다. 폴더 안의 읽기와 분류되지 않은 명령에는 닿지 않는다), MCP 도구, subagent 실행의 허용, 묻기, 거부 규칙이다. 문자열 하나면 그 도구 전체에 적용하고, 패턴 표를 주면 패턴마다 값을 준다. 모드 기본 규칙 뒤에 사용자 층 규칙, 폴더 층 규칙, 채팅 층 규칙, 실행 층 규칙을 잇고 마지막으로 일치한 규칙이 이긴다. 단 어느 층이든 `deny`가 하나라도 일치하면 거부한다(채팅 층과 실행 층까지 넣은 것은 초안). 같은 층 안의 순서는 파일에 적힌 순서다(초안). 병합 결과의 `permission`에는 합친 모드와 이은 규칙 목록이 들어가고, 입력은 접수 때 고정한 설정 번호의 목록을 쓴다. `permission` 아래 모르는 키, 모르는 모드, `allow`, `ask`, `deny`가 아닌 값은 검사에 실패한다. 패턴 문법과 판정 흐름, provider별 번역은 [권한](permissions.md)에 있다.
- `context.mode`가 `provider`이면 `sessions`는 compaction과 유휴 복귀를 판정하지 않고 provider 실행 인자에 자동 압축 안전망 값을 넣지 않는다. 규칙은 [맥락 정리](context-management.md#정리-모드)에 있다. 기본은 `saturn`이다.
- `context.select.rrf_k`는 router가 답하지 못한 항목의 순서와 같은 확률인 항목의 순서에만 쓴다.
- 기준값 이름은 `keep_current`, `is_actionable`, `min_confidence`, `resume_held`, `file_present`, `file_absent`, `context_gate`, `injection`, `progressing`, `feedback_cause`, `is_constraint`, `constraint_ask`, `constraint_release`다.
- `is_constraint`는 자동 등록 기준값(기본 0.8), `constraint_ask`는 등록 질문의 묻는 하한(기본 0.7)이다. `constraint.auto_apply`가 거짓(기본)이면 두 값 모두 적용하지 않는다. 켠 상태에서 권한 모드가 `full`이면 제약 질문을 묻지 않는다([제약](constraints.md#묻지-않고-진행하는-권한-모드)).
- `constraint_release`는 해제·예외 판단을 적용하는 기준값(기본 0.8)이고 종류 확률의 하한으로도 쓴다. `constraint.auto_apply`가 거짓(기본)이면 해제·예외 질문도 하지 않는다.
- 제약 칸 상한은 `P_max`의 `context.constraint_slot_percent`%다. 규칙은 [제약](constraints.md#패킷의-제약-칸)에 있다.
- 되돌릴 수 없는 행동의 기준값 `keep_current`, `resume_held`는 0.8 미만이면 검사에 실패한다(목록은 초안).
- 실행 층 `-c key=value`의 값은 TOML 값 문법으로 읽고, 같은 키가 여러 번 오면 뒤 값이 이긴다. TOML 값으로 읽을 수 없는 따옴표 없는 한 단어(`permission.mode=read-only`)는 문자열로 읽는다. 공백이나 따옴표가 든 값은 TOML 문법을 지켜야 한다.

### 설정 키가 없는 고정 상수

아래 값은 설정 키가 아니라 고정 상수다. 사용자가 바꿀 이유가 측정으로 드러나기 전에는 키를 만들지 않는다. 키가 늘면 이름 규칙과 검사 비용이 함께 늘기 때문이다(초안).

| 상수 | 값 | 정하는 곳 |
|---|---|---|
| router 재시도 간격 | 5초 | `saturn_core::routers::failure`의 `RETRY_INTERVAL`, [router 실패](router.md#router-실패) |
| router 재시도 횟수 | 첫 호출 뒤 2회 | `MAX_RETRIES` |
| router 재시도 전체 마감 | 첫 실패부터 10초 | `RETRY_DEADLINE` |
| router 응답 대기 | 시도마다 5초 | `routers::remote`의 `RetryPolicy` |
| 설정 파일 확인 주기 | 0.5초 | [설정 변경 감지](#설정-변경-감지) |
| engine 로그 보관 | 30일 | [engine 수명과 복구](engine-lifecycle.md) |

router 버전 표기는 설정 키가 아니다. `saturn router use`와 `saturn router train --from`은 `v3`처럼 `v`와 1 이상 정수를 받고, `v`를 빼고 `3`만 써도 `v3`으로 읽는다. 다른 모양은 오류다([router 학습](router-training.md#router-버전)).

### provider 설정 키

provider 고유 설정 키는 `provider.<id>.*` 열린 이름공간에 둔다. `<id>`는 어댑터 레지스트리의 provider id이고, 키 목록과 기본값은 그 어댑터의 설명자가 알린다([provider id와 설명자](providers-and-sessions.md#provider-id와-설명자)). 키 이름공간, 옛 키 별칭, 어댑터 설명자의 `window`와 `cache_write` 기본값은 구현했고, 어댑터마다 다른 키 목록과 레지스트리에 없는 id의 키 처리는 구현 전이다([#412](https://github.com/woonyong-choi/saturn/issues/412)). 지금은 형식이 맞는 id(소문자 영문, 숫자, `-`)의 `context.*` 네 키만 허용한다. 설명자가 없는 id는 `window` 200000, `cache_write` 1.25로 예산을 계산한다.

| 키 | 값 | 기본값 |
|---|---|---|
| `provider.<id>.context.t_abs` | 1 이상 정수 | 200000 |
| `provider.<id>.context.window` | 1 이상 정수 | 어댑터 설명자의 값(codex 272000, claude 1000000) |
| `provider.<id>.context.cache_read`, `cache_write` | 0 이상 실수 | 0.1, 어댑터 설명자의 값(codex 1.0, claude 1.25) |

- 옛 키 `context.<id>.<키>`는 새 키 `provider.<id>.context.<키>`의 별칭으로 계속 읽는다. 같은 값이 둘 다 있으면 새 키가 이긴다. `context.select`처럼 `context` 아래에 이미 있는 표는 별칭으로 읽지 않는다. 별칭은 층마다 병합 전에 새 키로 옮기므로 높은 층의 옛 키가 낮은 층의 새 키를 이긴다. 기존 설정 파일을 고치지 않고도 같은 값으로 동작하게 하기 위해서다(초안). 명령으로 설정을 쓸 때는 새 키로 쓰고, 설정 번호별 스냅샷은 새 키 이름으로 저장한다. 옛 스냅샷은 옛 키 이름 그대로 읽는다.
- 병합 결과 검사는 `provider.<id>.*` 키를 그 어댑터가 알린 키로만 허용하고 모르는 키는 실패로 본다. 레지스트리에 없는 id의 키는 실패 대신 무시하고 경고한다. 저장소에 다른 사용자가 쓰는 provider의 설정이 있어도 이 사용자의 실행이 막히지 않게 하기 위해서다(초안).
- `permission.<종류>`는 위 표의 다섯 종류에 더해 어댑터가 알린 권한 종류도 허용한다([권한](permissions.md#권한-규칙)).
- provider 실행 파일이나 인자를 바꾸는 키는 설명자가 사용자 층 전용으로 표시하고, 폴더 층에서 바꾸지 못한다. 저장소가 사용자 모르게 다른 프로그램을 실행시키는 일을 막기 위해서다.

### 폴더 층에서 바꿀 수 없는 항목

아래 항목은 사용자 전용이라 폴더 층에서 바꿀 수 없다. 폴더 설정 신뢰 창은 이 항목을 무시되는 항목에 표시한다.

| 항목 | 다루는 문서 |
|---|---|
| router 주소와 키 참조 | [router 키 보호](router-key-security.md) |
| 채점 모델 | [router 학습](router-training.md) |
| 데이터 공유 동의 | [router 학습](router-training.md) |
| 판단 방식 | [router](router.md) |
| 보존 기간과 자동 삭제 | [기록](records.md#보존과-정리) |
| 하위 접속 상한 | [하위 접속](child-sessions.md#상한과-대기열) |
| provider 원시 메시지 관측 기록 | [provider 연결과 session](providers-and-sessions.md#원시-이벤트-관측-기록) |

사용자 전용 키는 `router.endpoint`, `router.key`, `grading.model`, `consent`, `router.mode`, `retention`, `child`, `debug`, `constraint.auto_apply`와 같거나 그 아래 키다(초안). 옛 이름 `router.method`도 새 이름으로 옮긴 뒤 같은 규칙을 받는다. `retention`은 보존 기간과 자동 삭제라 비용과 삭제가 걸려 있어 사용자만 정한다. `child`는 하위 접속 상한이라 저장소가 provider 프로세스 수를 늘리지 못하게 사용자만 정한다. `debug`는 홈 폴더에 파일을 쌓는 기록이라 저장소가 남의 홈에 기록을 늘리지 못하게 사용자만 정한다. `constraint.auto_apply`는 검증하지 않은 판단이 지속 제약을 만드는 정책이라 저장소가 켜지 못하게 사용자만 정한다. router 키 자체는 설정 파일에 두지 않는다. 설정에는 키의 출처와 끝 4자리만 남는다([router 키 보호](router-key-security.md)).

### 병합과 설정 번호

engine이 시작하면 사용자당 잠금을 얻고 스키마 이관을 마친 뒤 기본값, 사용자, 실행 층을 병합한다. router 시작 확인은 이 설정 번호가 확정된 뒤에 한다. 폴더 층과 채팅 층은 TUI가 채팅에 붙을 때 채팅마다 병합한다. 채팅은 처음 만든 폴더에 묶이므로, 이미 있는 채팅에 다른 폴더의 TUI가 붙어도 처음 폴더로 병합한다. 아래 절차의 3~5단계가 그때 돈다.

1. `settings`가 기본값 층을 읽는다.
2. `settings`가 사용자 층 `<Saturn 홈>/config.toml`을 읽는다.
3. `settings`가 작업 폴더에서 git 맨 위까지 올라가며 폴더 층 `<작업 폴더>/.saturn/config.toml`을 찾는다.
4. 처음 보거나 내용이 바뀐 폴더 설정이면 `settings`가 한 번 묻고 경로와 지문으로 신뢰를 기록한다.
5. `settings`가 채팅 층과 실행 `-c` 층까지 병합하고, 뒤 층의 값을 우선한다. 실행 층은 engine 시작 `-c` 뒤에 그 접속이 Attach로 보낸 `-c`를 붙여 만든다.
6. `settings`가 병합 결과를 검사한다.
7. 검사를 통과하면 `store`가 병합 결과와 층 목록을 새 설정 번호의 스냅샷으로 기록한다.
8. 같은 내용의 스냅샷이 이미 있으면 `store`는 기존 설정 번호를 다시 쓴다.

- 같은 내용의 설정은 기존 설정 번호를 다시 쓴다. 여러 프로세스가 같은 설정에 번호를 중복으로 만드는 일을 막기 위해서다.

#### 적용 범위

- 병합 결과는 채팅 하나와 접속의 `-c` 목록을 합친 범위마다 따로 정한다. 접속의 `-c`는 그 접속이 접수하는 입력에만 적용하고, engine 전체나 채팅의 영구 설정(채팅 층)으로 올리지 않는다. 같은 채팅에 붙은 두 접속이 다른 `-c`를 쓰면 입력마다 자기 접속의 값이 고정된다.
- 검사에 실패하면 그 범위에서 마지막으로 성공한 설정 번호로 계속하고, 그 범위에 성공한 적이 없으면 engine 시작 때 확정한 설정 번호로 계속한다. 다른 채팅이나 다른 접속이 만든 번호로 돌아가지 않는다. engine 시작 때 사용자 층 병합이 실패하면 채팅과 접속 설정 없이 마지막으로 성공한 번호(기록 저장소에 따로 남긴다)로 시작하고, 그런 번호가 없으면 시작하지 않는다. 접속이나 채팅 층이 섞인 번호로 시작하면 설정 파일 오류가 접속별 `-c`의 권한(예 `permission.mode=full`)을 그대로 이어받기 때문이다. 잘못된 접속 `-c`는 그 접속에서만 경고로 나타난다.
- 파일 지문(마지막 병합 때 본 설정 파일)도 범위마다 기억한다. 같은 폴더의 채팅 둘이 서로의 변경을 소비하지 않고, 파일이 바뀌면 범위마다 한 번씩 다시 병합한다.
- 접속을 특정할 수 없는 일(종료 때 `on_exit` 읽기)은 그 채팅에 붙은 첫 접속의 범위를 쓴다. 입력 없이 채팅의 설정을 읽는 곳(허가 판정, 하위 접속 만들기, 맥락 정리 판단과 정리 뒤 session 다시 열기)은 그 에이전트가 가장 나중에 시작한 입력의 번호를 먼저 쓰고, 없으면 그 채팅에서 마지막에 적용한 번호를 쓴다. 모델 창과 기본 모델 알림은 그 접속이 접수할 입력의 번호를 쓴다. engine 전체 값(보관 기간, 시작 확인의 router, 하위 접속 상한)만 engine 시작 때 번호를 쓴다. 실행 중인 입력의 설정은 그 입력이 끝날 때까지 고정이고, 이후 접수하는 입력부터 바뀐 값을 쓰므로 맥락 정리 판단도 바뀐 값은 다음 입력의 시작부터 따른다.
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
- 확인 주기 0.5초는 상수 하나이고 설정 키가 없다(초안, [고정 상수](#설정-키가-없는-고정-상수)). 같은 지문을 연달아 두 번 볼 때만 적용하므로 저장 뒤 적용까지 1초 안팎이다.
- 두 번 연달아 같을 때만 적용하는 것은 편집기가 파일을 쓰는 도중의 반쯤 쓴 내용을 읽고 검사 실패로 오작동하는 일을 막기 위해서다. 쓰는 도중 파일이 계속 바뀌면 적용은 멈춘 뒤로 미룬다.
- 검사에 실패한 파일은 이전 설정 번호를 유지하고 경고를 한 번 알린다. 같은 내용으로는 다시 적용하지 않는다.
- 감시는 engine만 하고 `core`는 파일을 읽지 않는다. 채팅에 붙은 TUI가 없으면 확인하지 않고, 다음에 붙거나 입력을 접수할 때 알아챈다.

### 폴더 설정 신뢰

폴더 설정은 저장소와 함께 들어오므로 사용자가 한 번 확인한 뒤에만 적용한다. `settings`는 신뢰를 설정 파일의 경로와 지문으로 기록한다.

- 신뢰한 폴더 설정의 내용이 바뀌면 다시 묻고 새 지문으로 신뢰를 기록한다. 확인하지 않은 변경이 적용되는 일을 막기 위해서다.
- 폴더 설정 신뢰는 채팅마다, 그 채팅을 처음 만든 폴더 기준으로 묻는다. 적용을 고르지 않으면 그 채팅은 폴더 설정 없이 계속한다.
- 실행 중에 폴더 설정이 바뀌면 신뢰 창은 engine이 변경을 알아챈 때 그 채팅에 붙은 TUI에 열린다. 그 전에 입력을 접수하면 접수 전에 열린다.
- 신뢰 기록은 `<Saturn 홈>/trusted.json`(경로 → 지문, 권한 0600)에 두고, 지문은 파일 내용의 SHA-256 hex다(초안).
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
| 판단 기준값도 입력의 설정 번호에서 읽고, 설정을 바꿔도 접수한 입력의 값은 그대로이며 옛 설정으로 되돌리면 옛 번호를 다시 쓴다. | `saturn-terminal/engine/src/lifecycle/policy.rs`의 `inputs_keep_the_policy_they_were_accepted_under_across_swap_rollback_and_restart` |
| 입력은 접수 때 고정한 설정 번호로 끝까지 처리한다. | 처리 중 설정을 바꿔도 그 입력의 provider 실행 값이 접수 때 값인지 확인한다. |
| 검사에 실패한 설정은 적용하지 않고 이전 설정 번호를 쓴다. | 잘못된 폴더 설정으로 바꾼 뒤 이전 설정 번호가 유지되는지 확인한다. |
| 시작 때 사용자 층 병합이 실패하면 접속·채팅 층이 섞이지 않은 마지막 번호로 시작한다. | 접속 `-c permission.mode=full`을 적용한 뒤 사용자 설정을 깨뜨려 다시 시작하고 시작 번호의 권한이 사용자 설정 값인지 확인한다(`saturn-terminal/engine/src/lifecycle/start.rs`의 `restart_with_broken_user_settings_does_not_adopt_a_connection_layer`). |
| 맥락 정리 판단과 모델 알림은 입력의 접속 설정을 따른다. | 접속 `-c`로 `context.mode`와 맥락 기준을 바꾼 채팅의 판정과 다른 채팅의 기준이 서로 다른지 확인한다(`saturn-terminal/engine/src/lifecycle/turn_end.rs`, `model_mode.rs`의 `connection_layer_model_mode_reaches_the_tui_of_that_connection`, `model_settings_notice_follows_the_settings_of_each_connection_of_the_chat`). |
| 설정 파일을 바꾸면 입력 없이도 engine이 알아채 provider에 적용한다. 쓰는 도중의 파일은 적용하지 않고, 잘못된 파일은 이전 설정을 유지하며 경고한다. | `saturn-terminal/engine/src/lifecycle/settings_watch.rs`의 `settings_watch_restarts_an_idle_chat_without_any_input`, `settings_watch_waits_for_the_turn_end_when_the_chat_is_running`, `settings_watch_ignores_a_file_that_is_still_being_written`, `settings_watch_keeps_the_connection_and_warns_when_the_file_is_invalid`, `settings_watch_runs_in_the_serve_loop` |
| `model.default`와 `model.mode`를 알려진 키로 읽고 값을 검사하며, 기본 모델이나 방식을 바꾸면 사용자 설정 파일에 쓰고 붙은 TUI에 알린다. | `saturn-terminal/engine/src/lifecycle/model_mode.rs`의 `choosing_the_default_model_writes_the_user_config_and_tells_the_tui`, `changing_the_mode_writes_the_user_config_and_applies_to_the_next_input` |
| 명령으로 설정 파일을 고쳐도 주석이 남는다. | 주석이 있는 파일을 명령으로 고친 뒤 주석이 그대로인지 확인한다. |
| 설정 파일은 읽은 버전을 확인한 뒤 쓴다. | 읽기와 쓰기 사이에 파일을 고쳐도 그 변경이 사라지지 않는지 확인한다. |
| 옛 `context.<id>.*` 키를 새 `provider.<id>.context.*` 키의 별칭으로 읽고, 새 키를 알려진 이름으로만 허용한다. | `saturn-terminal/engine/src/settings/layers.rs`의 `provider_context_key_reads_from_the_new_name`, `old_context_key_is_an_alias_of_the_new_key`, `new_key_wins_over_the_old_alias_in_the_same_layer`, `higher_layer_old_key_beats_lower_layer_new_key`, `old_snapshot_keeps_its_old_key_name`, `provider_keys_accept_any_well_formed_id_and_reject_unknown_names`. 어댑터가 알린 키로만 허용하는 부분은 구현 전(#412) |
| 레지스트리에 없는 id의 `provider.<id>.*` 키는 검사에 실패하지 않고 경고한다. | 구현 전(#412). 등록하지 않은 id의 키를 폴더 설정에 두고 실행이 계속되는지 확인한다. |
