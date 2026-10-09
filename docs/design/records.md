# 기록 저장과 보존

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [engine을 상주 프로세스로 두고 TUI는 JSON-RPC로 붙는 클라이언트로 만든다](../decisions/2026-09-29-engine-centered-json-rpc.md), [판단 기록은 로컬에 쌓고 동의한 레코드만 서버로 올린다](../decisions/2026-09-29-local-first-judgment-collection.md) |

## 요약

기록 저장과 보존은 Saturn의 모든 기록을 SQLite 파일 하나에 남기고 지우는 기능이다. 입력, 실행, session, 사용량, 판단 기록, 설정 스냅샷을 engine 하나만 쓴다. 새 버전은 스키마를 자동으로 옮기고 그 직전 백업을 14일 둔다. 기록은 기본으로 지우지 않고, 사용자가 정리를 요청하거나 설정으로 켤 때만 지운다.

## 동기

Saturn은 provider가 바뀌어도 채팅을 이어 가려고 모든 입력과 결과를 자체 기록으로 남긴다. 이 기록은 크래시 뒤 복구, 맥락 전달, router 학습의 재료다. 여러 프로세스가 같은 파일에 쓰면 쓰기가 충돌한다. provider가 보고하지 않은 값을 0으로 채우면 사용량 계산이 틀어진다. 기록이 계속 쌓이면 디스크를 차지하지만, 함부로 지우면 진행 중인 작업이나 학습 데이터를 잃는다. 이 기능은 쓰는 쪽을 하나로 두고, 무엇을 언제 지울 수 있는지 정한다.

## 예시

### 새 버전을 처음 실행할 때

1. 사용자가 스키마 버전이 올라간 새 Saturn을 처음 실행한다.
2. `store`가 이관 직전 백업을 `~/.saturn/backup/`에 만들고 이전 버전의 백업을 지운다.
3. `store`가 스키마를 자동으로 옮기고 이관 사실을 한 줄로 알린다.
4. 14일이 지나면 `store`가 그 백업을 자동으로 지운다.

### 오래된 채팅을 정리할 때

1. 사용자가 `--yes` 없이 정리 명령을 실행한다.
2. engine은 아무것도 지우지 않고 지울 대상만 미리 보인다.
3. 사용자가 미리보기가 알려 준 확인 번호를 붙여 `--yes --plan <번호>`로 다시 실행한다.
4. engine이 미리보기에 있던 채팅만 대상으로 삼고, `store`가 열린 입력과 활성 session이 있는 항목은 빼고 나머지를 지운다.
5. `store`가 지운 채팅의 ID, 해시, 삭제 시각을 삭제 흔적으로 남긴다.
6. `store`가 WAL을 비우고 `VACUUM`으로 파일 크기를 줄인다.

### 판단 기록을 내보낼 때

1. 사용자가 채점을 한 번도 하지 않은 상태에서 판단 기록 내보내기를 실행한다.
2. engine이 판단 기록을 JSONL 파일로 쓴다.
3. 일반 정리를 실행해도 판단 기록은 그대로 남는다.

## 상세 설계

### 기록 저장소

기록 저장소는 `~/.saturn/saturn.db` SQLite 파일 하나다. engine은 `sqlx`로 이 파일에 접근한다.

| 담는 것 | 예 |
|---|---|
| 입력 | 접수한 입력과 전달 상태 |
| 실행 | 실행마다의 `effect_scope`, 원시 기록, 실행에 붙이지 못한 원시 줄 |
| session | 닫은 session의 provider session ID, 마지막 턴의 활성 맥락과 끝 시각 |
| 사용량 | 사용량 보고 원값, 범위, 대상 에이전트, 모델 |
| 판단 기록 | router 호출의 보낸 원문, 받은 원문, 질문별 답, 비용, 시간, 물은 확률 q, 결과 신호, 물은 답 |
| 설정 스냅샷 | 설정 번호별 병합 결과와 층 목록 |
| 제약 | 규칙 한 줄과 적용 범위, 변경 이벤트, 묻는 중인 확인, 전환마다 패킷에 넣은 제약 |
| provider CLI 버전 | provider마다 마지막으로 확인한 CLI 버전과 확인 시각 |
| 전달 패킷 | provider에 보낸 인계 패킷의 시도마다의 본문 해시와 크기, 받은 session, 입력·실행, 설정과 제약 revision, 정책 지문, 들어가거나 빠진 항목 |
| 근거 조회 | 에이전트 작업이 기록을 찾거나 원문을 다시 읽은 시도의 종류, 기록 번호, 결과, 돌려준 양 |
| 확장 | 설치한 확장의 이름, 출처, 부분별 provider 사용 가능 판정, 옮기자는 질문의 거절(거절은 구현 전, [#412](https://github.com/woonyong-choi/saturn/issues/412)) |

- 기록 저장소에 쓰는 쪽은 engine 하나다. 쓰기 충돌을 막기 위해서다.
- engine은 사용자당 하나이고 잠금으로 지킨다. 쓰는 프로세스를 하나로 유지하기 위해서다.
- 한 번의 변경은 한 거래로 처리하고, provider가 보고하지 않은 값은 0이 아닌 NULL로 둔다. 쓰기 충돌과 지어낸 값을 막기 위해서다.
- 설정 원본은 파일이고 기록 저장소에는 적용된 설정의 스냅샷만 둔다([설정](settings.md)).
- TUI의 입력 기록 `~/.saturn/history`는 기록 저장소 밖의 권한 0600 파일이고 TUI가 쓴다.
- 첫 스키마(V1)는 옛 스키마 위 변경분이 아니라 전체 정의로 쓴다. 공개 저장소만으로 스키마 전체를 읽기 위해서다.
- 표 이름은 `chats`, `inputs`, `runs`, `sessions`, `events`, `usage`이고 Saturn 용어(채팅, 입력, 실행, session)를 따른다.
- 하위 접속으로 만든 하위 채팅은 보통 채팅과 같은 표에 남고 정리 대상도 같다. 부모와의 연결은 기록 저장소에 두지 않고 `engine` 메모리에만 있다([하위 접속](child-sessions.md)).
- `sessions` 표는 마지막 활성 맥락 `last_active`(토큰)와 마지막 턴 끝 시각 `last_turn_ended_at`(unix 밀리초) 열을 둔다. engine이 턴이 끝날 때 쓰고 시작할 때 읽어 보관 session의 재개 판정을 재시작 뒤에도 같게 한다([provider 연결과 session](providers-and-sessions.md#그-provider로-돌아가기)). 두 열은 스키마 V2에서 더했고 이관 전 행은 NULL이며, NULL이면 재개로 판정한다.
- `sessions` 표의 `model`은 session을 열 때 고른 모델이다. 고르지 않았거나 이관 전 행은 NULL이고, 입력의 모델과 다르면 새 메인 session을 연다([모델 고르기](providers-and-sessions.md#모델-고르기)). 스키마 V6에서 더했다.
- `chats` 표의 `pinned_model`은 `/model`로 고른 채팅의 고정 모델(`<provider>/<model>`)이다. 고르지 않았으면 NULL이고 입력 접수 때 읽어 `inputs.pinned_model`에 남긴다. 고정하지 않은 입력은 새 작업으로 판단되어 router가 고른 모델(`<provider>/<model>`)을 판단을 적용할 때 같은 열에 쓴다. 스키마 V6에서 더했다.
- `chat_dirs` 표는 채팅에 더한 폴더를 채팅 `chat_id`, 링크를 푼 절대 경로 `path`, 더한 시각 `added_at`(unix 밀리초)으로 둔다. 같은 채팅의 같은 경로는 한 행이고 행 번호 순서가 더한 순서다. 채팅을 지우면 함께 지운다. 스키마 V5에서 더했다([engine 수명과 복구](engine-lifecycle.md#채팅-폴더와-이어-열기)).
- `chats` 표의 `name`은 사용자가 붙인 채팅 이름, `group_name`은 작업 목록의 묶음 이름이다. 붙이지 않았으면 NULL이고 이관 전 채팅도 NULL이다. 앞뒤 공백을 지우고 비면 NULL로 저장하며, 줄바꿈 같은 제어 문자가 들어 있으면 저장하지 않고 요청을 거절한다(초안). 채팅을 지우면 함께 지운다. 스키마 V8에서 더했다.
- `sessions`와 `runs` 표의 `provider`는 열린 provider id를 담는다. 지금 값 `Codex`, `Claude`는 대소문자를 가리지 않고 id `codex`, `claude`로 읽고 옛 행은 고치지 않는다. `chats.pinned_model`과 `inputs.pinned_model`의 `<provider>/<model>`은 provider 자리가 이미 id와 같아 그대로 읽힌다([provider 연결과 session](providers-and-sessions.md#provider-id와-설명자)). 새 행은 id 글자로 쓴다. `meta`의 캐시 유지 시간 키는 `cache_ttl_secs_<id>`라 옛 키가 그대로 읽힌다.
- `extensions` 표는 설치한 확장을 이름, 출처, 설치 시각, 부분별 판정으로 둔다. 확장 원본 파일은 `~/.saturn/extensions/`에 두고 이 표에 넣지 않는다. 열은 `name`(기본 키), `source`, `installed_at`(unix 밀리초), `parts`(부분별 판정 JSON)다. 스키마 V12에서 더했고 이관은 표만 비어 있게 더하며, 정리 대상이 아니다([기능 목록과 확장](extensions.md#확장-저장소)).
- `held_tasks` 표는 멈출 때 실행 중이던 보류 작업을 작업 번호 `task_id`(첫 입력 번호라 engine을 다시 켜도 같다), 채팅 `chat_id`, 에이전트 `agent_id`, 멈출 때 진행 중이던 실행을 연 입력 `input_id`로 둔다. 멈춤이나 크래시 복구가 보류할 때 쓰고, 재개하거나 닫으면 지운다. 채팅이나 입력을 지우면 함께 지운다. 입력과 에이전트가 없는 보류(보내기 전에 멈춘 입력)는 이 표에 남기지 않고 입력 행의 상태 `Held`로만 남긴다. 멈춤이 보류한 입력은 그때 `inputs.state`에 `Held`로 쓰고, 다시 켠 `engine`은 끝 상태가 아닌 입력을 접수 순서로 읽어 대기열에 되살린다([입력 처리](input-handling.md#재시작-뒤-입력-복원)). 스키마 V7에서 더했다([engine 수명과 복구](engine-lifecycle.md#크래시-뒤-복구)).
- `run_changes` 표는 실행이 시작 뒤 바꾼 파일을 실행 `run_id`, 채팅 `chat_id`, 경로 `path`(절대 경로), 종류 `kind`(`added`, `modified`, `deleted`), 수정 주체 `actors`(줄바꿈으로 이은 글, 비면 이벤트에 없는 수정)로 둔다. `runs` 표의 `changes_state`는 측정했으면 `complete`, 폴더가 커서 일부만 훑었으면 `partial`이고, 측정하지 못한 실행(크래시, 보내기 전 실패, 이관 전 행)은 NULL이다. 실행이나 채팅을 지우면 함께 지운다. 스키마 V11에서 더했고 이관은 열과 표만 비어 있게 더한다([provider 연결과 session](providers-and-sessions.md#수정-파일-목록)).
- `direct_installs` 표는 provider에 직접 설치된 항목 중 옮길지 물은 것을 provider id `provider`, 종류 `kind`(`skill`, `command`, `mcp_server`, `plugin`), 이름 `name`, 상태 `state`(`asked`, `moved`), 처음 본 시각 `seen_at`(unix 밀리초)으로 둔다. 기본 키는 앞의 세 열이다. 한 번 물은 항목은 행이 남아 다시 묻지 않고, 사용자가 옮기면 `moved`가 된다. 스키마 V14에서 더했고 이관은 표만 비어 있게 더하며, 정리 대상이 아니다([기능 목록과 확장](extensions.md#provider에-직접-설치한-것)).
- `runs` 표의 `evidence_state`, `evidence_reason`, `evidence_events`는 끝난 실행의 완료 검사 근거다. 상태는 `Verified`, `Unverified`, `NotApplicable`, 까닭은 `Unverified`일 때의 `UnverifiedReason`, 근거 이벤트는 `Verified`일 때 검사 결과 이벤트의 기록 번호를 담은 JSON 배열이다. 근거를 가리지 않은 실행(이관 전, 멈춘 실행, 중간에 끊긴 실행)은 NULL이다. 스키마 V18에서 더했고 이관은 열만 비어 있게 더한다([provider 연결과 session](providers-and-sessions.md#완료-검사-근거)).
- `raw_unattributed` 표는 어느 실행의 것인지 정하지 못한 provider 원시 줄을 채팅 `chat_id`, provider id `provider`, 에이전트 `agent_id`(모르면 NULL), 줄에 적힌 provider session 식별자 `provider_session`(없으면 NULL), JSON 여부 `is_json`, 줄 `bytes`, 받은 시각 `received_at`(unix 밀리초)으로 둔다. 스키마 V15에서 더했고 이관은 표만 비어 있게 더하며, 채팅을 지우면 함께 지워진다([provider 원시 응답 수집](#provider-원시-응답-수집)).
- `handoff_packets` 표는 provider에 보낸 인계 패킷의 시도마다 한 행이다. 종류 `kind`(`Switch`, `Return`, `Restart`), 시도 번호 `attempt`와 줄이기 전 시도 `reduced_from`, 받는 session `session_id`, 보낸 입력 `input_id`(맥락 정리는 NULL), 입력이 연 실행 `run_id`, provider와 받은 provider session `provider_session`, 설정 번호 `settings_revision`, 패킷이 담은 기록의 마지막 번호 `chat_revision`, `constraint_revision`, 정책 지문 `policy`, 보낸 글의 SHA-256 `body_hash`와 바이트 수 `body_bytes`, 추정 토큰 `estimated_tokens`, 상태 `state`(`Prepared`, `Sent`, `NotSent`, `Unknown`)를 둔다. `handoff_packet_items` 표는 시도마다 구역 `zone`(`Constraints`, `Open`, 대화 본문의 `User`, `Steer`, `Assistant`, `Competing`), 번호 `ref_id`(제약 칸은 제약 번호, `Steer`는 입력 번호, 나머지는 기록 번호), 고른 방식 `selector`(대화 본문은 `protected`, 경쟁 구역은 순위 순서면 `rank`, router `compact` 판단 순서면 `compact`), 들어간 모양 `form`, 빠진 이유 `reason`, 대화 본문 원문의 SHA-256 `body_hash`(본문 행만, 나머지는 NULL)를 둔다. 대화 본문 행은 실제 순서로 쌓인다. 스키마 V16에서 더했고 `body_hash`는 V21에서 열로 더했으며, 이관은 표만 비어 있게 더하거나 열을 비워 두고, 채팅을 지우면 함께 지워진다([전달 패킷 근거](#전달-패킷-근거)).
- `model_shadows` 표는 모델 판단 그림자의 판단마다 한 행이다. 판단 기록 `judgment_id`(유일), 입력 `input_id`, 설정 번호 `settings_revision`, 판단을 시작한 채팅 revision `chat_revision`, 정책 지문 `policy_digest`, 모델 목록 버전 `catalog_version`, 질문 세트 `question_set`, 후보 지문 `candidates_hash`, 후보별 `model`·품질 확정 여부 `quality`·충분할 확률 `probability`를 담은 JSON `candidates`, 상태 `status`(`answered`, `invalid`, `no-answer`, `superseded`), 실제로 적용한 모델 `applied_model`(어긋난 판단이거나 정하지 않았으면 NULL), 그림자 질문이 요청에 더한 바이트 `request_bytes`를 둔다. 원문은 저장하지 않는다. 스키마 V17에서 더했고 이관은 표만 비어 있게 더한다. 판단 기록 전용 정리가 판단 기록과 함께 지운다([모델 판단 그림자](router.md#모델-판단-그림자)).
- `model_selections` 표는 새 작업의 모델을 정한 규칙의 기록이고 입력 판단마다 한 행이다. 판단 기록 `judgment_id`(유일), 입력 `input_id`, 정한 규칙 `source`(`pinned`, `router`, `preference`, `default`, `current`), 정한 모델 `model`(현재 모델이나 provider 기본값이면 NULL), router 선택을 쓰지 못한 이유 `reason`(없으면 NULL. `manual`, `no-candidates`, `router-failed`, `invalid`, `fallback`, `unsupported`), 건너뛴 선호 `skipped_preferences`(JSON), 후보 지문 `candidates_hash`, 정책 지문 `policy_digest`, 모델 목록 버전 `catalog_version`, 적용 여부 `applied`(어긋난 판단은 0)를 둔다. 스키마 V19에서 더했고 이관은 표만 비어 있게 더한다. 판단 기록 전용 정리가 판단 기록과 함께 지운다([기본 모델과 선택 방식](providers-and-sessions.md#기본-모델과-선택-방식)).
- `evidence_lookups` 표는 에이전트 작업의 근거 조회(`saturn evidence`) 시도마다 한 행이다. 채팅 `chat_id`, 종류 `kind`(`Search`, `Read`), 읽은 기록 번호 `record_id`(검색은 NULL), 결과 `outcome`(`Ok`, `NotFound`, `Stale`, `Scope`, `Unreachable`), 돌려준 양 `units`(검색은 후보 수, 읽기는 글자 수), 시각 `created_at`(unix 밀리초)을 둔다. 스키마 V20에서 더했고 이관은 표만 비어 있게 더하며, 채팅을 지우면 함께 지워진다([근거 조회 기록](#근거-조회-기록)).
- `provider_versions` 표는 provider id `provider`마다 마지막으로 확인한 CLI 버전 `version`과 확인 시각 `checked_at`(unix 밀리초)을 한 행으로 둔다. engine이 시작할 때 설치된 CLI의 버전을 읽어 다르면 덮어쓴다([provider 연결과 session](providers-and-sessions.md#직접-연결과-acp-어댑터)). 스키마 V9에서 더했고 이관은 표만 비어 있게 더한다. 채팅 정리의 대상이 아니다.
- `interrupted_subagents` 표는 크래시로 끊긴 하위 에이전트를 채팅 `chat_id`, 메인 에이전트 `agent_id`, provider의 하위 에이전트 번호 `subagent`, provider에 정리를 넘겼는지 `cleaned`로 둔다. 같은 에이전트의 같은 하위 에이전트는 한 행이다. 에이전트의 session이 끝나거나 보류를 닫으면, 채팅을 지우면 함께 지운다. 스키마 V7에서 더했다.
- `permission_allows` 표는 항상 허용을 작업 폴더 `workdir`, 도구 `tool`, 패턴 `pattern`, 저장 시각 `created_at`(unix 밀리초)으로 둔다. 같은 `workdir`, `tool`, `pattern`은 한 행이다. 스키마 V4에서 더했고 채팅과 상관없이 작업 폴더 단위로 쓴다([권한](permissions.md#항상-허용-저장)).
- `constraints` 표는 제약 한 건을 `constraint_id`(다시 쓰지 않는 번호), 채팅 `chat_id`, 제약이 나온 입력 `input_id`, 입력 안 조각 번호 `line`(나누지 않았으면 0), 규칙 한 줄 `rule`, 적용 범위 `scope`(줄바꿈으로 이은 경로, NULL이면 전체), 상태 `state`(`Candidate`, `Active`, `Released`), 만든 시각 `created_at`(unix 밀리초)로 둔다. 행은 지우지 않고 채팅을 지울 때만 함께 지운다([제약](constraints.md#제약의-모양)).
- `constraint_events` 표는 제약의 변경 내역이다. 사용자가 `AddConstraint`로 직접 등록한 제약은 입력에서 나오지 않아 `constraints.input_id`가 0이고 등록 이벤트의 `input_id`는 NULL이다. 이벤트 번호 `event_id`, `chat_id`, `constraint_id`, 종류 `kind`(`Added`, `Released`, `Excepted`, `Resumed`, `Restored`), 주체 `actor`(`Router`, `User`, `Engine`), 사유 `reason`(NULL, `Mistaken`, `InputCanceled`, `Declined`, `Unconfirmed`), 변경을 일으킨 입력 `input_id`(사용자가 직접 바꿨거나 작업 끝 때문이면 NULL), 판단 기록 `judgment_id`, 되돌린 이벤트 `undoes`, 시각 `created_at`을 둔다. `Unconfirmed`는 권한 모드 `full`이라 묻지 않고 정한 변경이다. 채팅의 제약 revision은 그 채팅의 마지막 `event_id`(없으면 0)이고 대화 기록의 제약 줄은 이 표에서 그린다. 행은 이어 쓰기만 한다.
- `constraint_asks` 표는 사용자에게 묻는 중이거나 끝난 확인이다. `ask_id`, `chat_id`, 종류 `kind`(`Register`, `ExceptionKind`), 대상 `constraint_id`, `judgment_id`, 물은 시각 `asked_at`, 답 `answer`(NULL은 대기, `Yes`, `No`, 예외 종류 답 `Keep`, `Once`, `Permanent`, 대상이 바뀌어 닫은 `Void`), 답한 시각 `answered_at`을 둔다. `answer`가 NULL인 행은 engine을 다시 켜면 TUI에 되살린다.
- `constraint_exceptions` 표는 제약에 걸린 예외다. `exception_id`, `chat_id`, `constraint_id`, 종류 `kind`(`Once`, `Scoped`), 건 이벤트 `event_id`, 이번 작업 예외의 작업 `task_id`(아니면 NULL), 조건 문장 `condition`(`Scoped`만, 입력 원문의 연속된 글), 닫은 이벤트 `ended_event_id`(NULL이면 유효)를 둔다. 한 제약에 열린 예외는 하나다.
- `packet_constraints` 표는 전환마다 패킷에 넣은 제약을 새 session `session_id`, `constraint_id`, 단계 `tier`(`All`, `Scope`, `Relevance`, `Omitted`)로 둔다. 같은 session의 같은 제약은 한 행이고 session을 지우면 함께 지운다. 못 넣은 제약도 `Omitted`로 남긴다.
- `inputs` 표의 `steered_run`과 `steered_after`는 실행 중에 끼워 넣어 적용한 입력이 들어간 실행 `runs.id`와, 그때까지 채팅에 쌓인 마지막 `events.seq`다. 두 열은 입력 상태 `Applied`와 한 번의 쓰기로 함께 저장하므로 연결 없는 `Applied`가 남지 않고, 저장에 실패해도 이미 보낸 입력은 다시 보내지 않는다. 적용한 입력만 쓰고(끝 상태가 `Applied`일 때 읽는다), 실행을 연 입력, 거절되어 대기로 돌아온 입력, 이관 전 입력은 NULL이다. 인계 패킷이 끼워 넣은 입력을 사용자 입력으로 읽는 재료이고 기록 조회(`ledger_since`)는 그대로다([맥락 관리](context-management.md#패킷-구성)). 스키마 V13에서 더했고 이관은 열만 NULL로 더한다.
- 제약 표 다섯 개는 스키마 V10에서 더했다. 이관은 표만 비어 있게 더하고 이관 전 채팅의 입력을 소급해 판단하지 않는다. 이관 직전 백업은 아래 스키마 이관 규칙을 따른다.
- 패킷과 돌아온 session의 변경분은 `events` 행을 그 이벤트를 연 실행의 session과 입력 원문, 기록 시각에 이어 읽어 만든다. 이벤트 행에 session을 따로 저장하지 않기 위해서다. `sessions.delivered`는 턴이 끝날 때와 session을 열거나 바꿀 때 저장한다.
- session과 에이전트 번호는 `meta` 표에 마지막으로 준 번호를 두고, 기록에 있는 가장 큰 번호보다 큰 값을 한 거래로 새로 준다. 번호를 다시 쓰지 않기 위해서다.
- 기록 저장소 파일 권한은 0600이다(초안). 입력 원문과 판단 기록이 들어 있기 때문이다.

### 실행별 완료 검사 근거

실행을 `Completed`로 닫을 때 가린 완료 검사 근거를 위 `runs`의 근거 열에 남긴다. 판정 규칙은 [완료 검사 근거](providers-and-sessions.md#완료-검사-근거)에 있다. 같은 실행을 다시 기록하면 바꿔 쓴다. 이벤트는 실행을 닫은 뒤 압축되므로 근거는 닫기 전에 가려 남긴다.

### 실행별 수정 파일

수정 파일 목록은 실행 시작과 끝의 폴더 상태 차이로 센 결과를 위 `run_changes` 표에 남긴다. 계산 규칙, 상한, 쓰는 자리는 [수정 파일 목록](providers-and-sessions.md#수정-파일-목록)에 있다. 같은 실행을 다시 기록하면 앞선 목록을 바꿔 쓴다. 바뀐 파일이 없는 실행도 `changes_state`가 남아 측정했음을 알 수 있다.

### 사용량 조회 범위

`/usage`의 `day`는 지금부터 24시간 전부터, `week`는 7일 전부터이고 모든 채팅을 합친다. provider가 알려 준 사용량은 보여 주는 범위와 상관없이 모두 기록하고, 범위는 조회할 때만 가른다. 붙은 채팅이 없는 `saturn usage`의 `chat` 범위와 `saturn --continue`의 `LatestChat`은 요청에 실린 폴더에서 마지막 입력 접수가 가장 늦은 채팅(입력이 없으면 만든 시각)을 쓴다. `ListChats`는 같은 기준으로 정렬한 채팅 목록을 돌려주고, 폴더를 주지 않으면 모든 폴더의 채팅이며 각 줄에 채팅 첫 입력 원문을 싣는다. session 누적 보고가 직전 누적보다 작거나 그사이 보고 없는 실행이 있으면 그 행을 여러 턴에 걸친 값으로 표시한다. `/usage` 응답은 provider·모델마다 한 행과 router 한 행으로 묶고, 여러 턴을 합친 행은 protocol `UsageRow.turns`에 턴 수를 싣는다. 비용, 맥락 정리, 채점은 저장하지 않아 `None`으로 보낸다. 묶음 규칙과 표시는 [TUI](tui.md)에 있다.

### 판단 기록

engine은 모든 router 호출의 원문, 답, 비용, 시간을 판단 기록으로 남긴다. 판단 기록은 판단 방식과 관계없이 전부 남긴다. 어느 방식에서도 학습과 비교용 기록을 쌓기 위해서다.

- 저장하기 전에 보낸 원문과 받은 원문에서 비밀값을 가린다([router 키 보호](router-key-security.md)).
- 기록을 끈 채팅(`/record off`)에서는 판단 기록을 저장하지 않는다.
- 판단 기록에는 질문 버전과 설정 번호를 남긴다. 버전이 바뀐 뒤에도 옛 기록을 다시 해석하기 위해서다.
- 판단 기록에는 그때의 기준값과 물은 확률 q(`asked_with`)를 남기고, 결과 신호(`signal`)와 물은 답(`asked_answer`)이 생기면 같은 기록에 채운다. 느린 조정이 모든 판단 기록으로 위험 곡선을 다시 계산하기 위해서다.
- `signal`은 스키마 V3에서 더한 칸이고 `Wrong`(틀림), `Missed`(놓침), `Unconfirmed`(반응 없음)이다. 값이 NULL이면 관찰 시간(다음 입력 3개 또는 10분)이 끝나지 않은 판단이다. engine이 관찰이 끝난 뒤 한 번만 쓰고 이미 쓴 값은 바꾸지 않는다.
- `asked_answer`는 스키마 V3에서 더한 칸이고 `Correct`나 `Wrong`이다. 물은 판단(`asked_with`가 NULL이 아닌 판단)에만 쓰고 첫 답만 남긴다. NULL이면 묻지 않았거나 답이 없는 판단이다.
- 이관 전 행은 두 칸이 NULL이다. NULL 신호인 판단은 느린 조정의 기록에 들지 않는다.
- engine이 다시 시작하면 관찰 중이던 판단의 신호는 NULL로 남는다. 관찰 상태를 메모리에만 두기 때문이다.
- 사용자가 동의한 레코드만 서버로 올리고, 동의 버전과 삭제 id를 기록한다([결정 기록](../decisions/2026-09-29-local-first-judgment-collection.md)).
- 삭제를 요청한 레코드는 다음 학습부터 뺀다.
- router 호출 규칙은 [router](router.md), 학습에 쓰는 방식은 [router 학습](router-training.md)에 있다.

### 스키마 이관

engine이 시작하면 사용자당 잠금을 얻은 직후 스키마를 확인한다.

1. 새 버전을 처음 실행할 때 스키마 버전이 올라갔으면 `store`가 이관 직전 백업을 만든다.
2. `store`가 이전 버전의 백업을 지운다.
3. `store`가 스키마를 자동으로 옮긴다.
4. `store`가 이관 사실을 한 줄로 알린다.
5. `store`가 만든 지 14일이 지난 백업을 자동으로 지운다.

- 이관 직전 백업은 가장 최근 1개만 두고 14일 뒤 지운다. 이관 규칙의 버그에 대비한 임시본이기 때문이다.
- 백업은 `~/.saturn/backup/`에 둔다. 파일 이름은 `saturn-v<이전 버전>-<unix 밀리초>.db`이고(초안), 이 형식의 파일만 백업으로 보고 지운다.
- 백업 폴더 권한은 0700, 백업 파일 권한은 0600이다.
- 이관은 모든 단계를 한 거래로 실행한다. 한 단계라도 실패하면 스키마 버전과 표를 이관 전 그대로 두고 백업을 남긴다.

### 원시 기록 압축

1. 실행이 끝나면 `store`가 그 실행의 원시 기록을 gzip으로 압축해 저장한다.
2. 압축한 원시 기록은 읽을 때 자동으로 푼다.

- 실행 중인 원시 기록은 압축하지 않고, 원본 해시와 크기는 압축 전 값으로 기록한다. 압축이 기록 내용과 대조 값을 바꾸지 않게 하기 위해서다.
- 해시는 SHA-256 hex다(초안). 끝나 압축한 실행의 원시 기록에는 더 쓰지 않는다.

### provider 원시 응답 수집

provider가 연결로 보낸 줄은 변환하기 전 모습 그대로 원시 기록에 쌓는다. 우리가 보낸 패킷과는 별개이고, 보낸 패킷의 근거는 [#538](https://github.com/woonyong-choi/saturn/issues/538)이 맡는다.

1. 어댑터의 읽기 작업이 줄을 읽는 즉시 router 키를 가려 engine으로 보낸다. 줄이 만드는 이벤트보다 먼저 보낸다.
2. engine이 줄 안의 에이전트로 그 에이전트의 열린 실행을 찾아 `raw_chunks`에 줄바꿈과 함께 이어 쓴다. 쓰는 쪽은 engine 하나다.
3. 실행이 끝나면 위의 압축 절차를 따른다. 줄은 이벤트보다 먼저 도착하므로 턴 끝 줄은 실행이 닫히기 전에 쌓인다.

- 에이전트는 어댑터가 provider의 session·thread 식별자로 찾는다. 자식 thread나 subagent의 줄은 부모 에이전트의 실행에 쌓인다.
- 에이전트를 찾지 못했거나 그 에이전트에 열린 실행이 없으면 `raw_unattributed`에 쌓는다. 채팅, provider, 줄에 적힌 session 식별자, 받은 시각을 함께 남긴다. 지금 열린 실행이라는 이유로 추정해 붙이지 않는다.
- Codex의 연결과 session을 여는 응답, 첫 턴을 시작하는 응답처럼 thread가 적히지 않았거나 실행이 시작되기 전에 온 줄은 미귀속으로 남는다. 실제 Claude와 Codex 확인에서 Codex 연결 한 번에 21줄이 이렇게 남았다.
- JSON으로 읽지 못한 줄도 버리지 않고 같은 곳에 쌓고 `is_json`을 거짓으로 표시한다.
- router 키와 일치하는 글자는 저장 전에 가린다. 줄에서 가린 결과와 값 단위로 가린 결과가 다르면 값 단위로 가린 쪽을 다시 써서 쌓는다.
- 원시 기록은 실행 단위다. 이벤트와의 연결은 실행 번호로 한다.
- 저장하지 못해도 이벤트 처리는 막지 않고 로그만 남긴다.
- 끝난 실행에 늦게 도착한 줄은 실행에 붙이지 않고 `raw_unattributed`로 간다.

### 전달 패킷 근거

provider에 실제로 보낸 인계 패킷은 시도마다 근거를 남긴다. [provider 원시 응답](#provider-원시-응답-수집)은 받은 쪽이고 이 기록은 보낸 쪽이다. 둘은 같은 저장소를 쓰지만 서로를 대신하지 않는다.

1. engine이 패킷을 보내는 요청(새 session 열기, 보관 session 다시 열기, 이미 열린 session에 변경분 턴 보내기, 맥락 정리의 새 session 열기)을 provider에 맡기기 직전에 `Prepared`로 시도를 쓴다.
2. 결과를 받으면 상태를 확정한다. 성공은 `Sent`와 받은 provider session이다. 거절과 맥락 한도 초과는 보내지 않았음이 확정이라 `NotSent`다. 그 밖의 실패(연결 끊김, 결과 불명)는 보냈는지 모르므로 `Unknown`이고 다시 보내지 않는다.
3. 입력이 실행을 시작하면 그 입력을 위해 보낸 `Sent` 패킷에 실행 번호를 붙인다. 패킷 턴의 답은 그 실행의 기록에 남는다.
4. 맥락 한도로 거절돼 줄여 다시 보내면 새 시도 행을 만든다. `attempt`가 늘고 `reduced_from`이 거절된 시도를 가리키며 받는 session도 새 번호다. 항목은 시도마다 따로 쌓여 어느 항목이 어느 전송에 들어갔는지 섞이지 않는다.
5. engine이 크래시 뒤 시작하면 `Prepared`로 남은 시도를 `Unknown`으로 확정한다.

- `body_hash`는 provider에 넘긴 글 전체의 SHA-256이다. 같은 글이 provider가 받은 바이트와 맞는지 대조하는 값이다.
- 본문과 원문 항목은 복제하지 않는다. 항목은 `ref_id`로 기록(`events`)이나 제약을 가리킨다. 그래서 기록 원문을 가린 값이 이 기록에 다시 나타날 수 없다.
- 제약 칸 항목은 제약 번호와 단계(`All`, `Scope`, `Relevance`)이고, 칸이 차서 빠진 제약은 이유 `slot_full`이다. 새 session의 제약 단계는 기존 `packet_constraints`에도 남는다. 열린 항목과 경쟁 구역 항목은 기록 번호만 가지므로, 제약은 제약 칸 한 곳에서만 전달된다는 것을 이 표의 `Constraints` 행과 다른 구역 행으로 조회한다.
- 대화 본문은 줄이거나 빼지 않으므로 `form`은 늘 `Full`이고 빠진 이유가 없다. 행마다 역할(`zone`), 번호(`ref_id`), 원문 해시(`body_hash`)가 있고 행 순서가 패킷의 실제 순서라, 기록된 입력과 글을 보낸 본문과 번호·해시·역할·순서로 대조할 수 있다. 보내기 직전에 engine이 보낼 글이 이 본문을 같은 내용과 순서로 모두 담았는지 검사해 어긋나면 시도를 쓰지 않고 보내지 않는다. 경쟁 구역은 `Full`, `Digest`, `Path`, `Summary`로 모양을, `budget`, `provider_doc`으로 빠진 이유를 남긴다.
- 정책 지문은 설정 번호의 기준값과 router 모델의 지문이다([판단 정책 고정](router.md#정책-고정)).
- 기록하지 못해도 전송은 막지 않고 로그만 남긴다. 이 기록은 전송의 전제가 아니라 전송의 근거다.

### 근거 조회 기록

전달 패킷은 보호 본문의 역할, 번호, 원문 해시와 순서를 기존 패킷 항목에 남긴다. 실제 발신 글은 따로 해시를 남기고, 전송 전 만든 패킷과 일치하는지 검사한다. 도구 기록의 `selector`는 새 패킷에서 `recent`다. 이전 시도의 `rank`와 `compact` 값은 기록 해석을 위해 그대로 읽는다. 원시 이벤트, 패킷 구성, 실제 전달을 구분하고 결과 불명과 사용량 결측을 성공이나 0으로 바꾸지 않는다([보존 우선 경로](context-management.md#보존-우선-경로)).

에이전트가 패킷에서 생략된 기록을 번호로 다시 읽은 일은 기록 저장소에 남는다. provider가 낸 도구 호출 이벤트와 별개로, Saturn 경로(`saturn evidence`)로 실제 조회가 일어났는지와 얼마나 돌려줬는지를 세기 위해서다.

1. engine이 출입증으로 채팅을 정하고 검색이나 읽기를 처리한다.
2. 처리가 끝나면 종류, 기록 번호, 결과, 돌려준 양을 한 행으로 쓴다. 거절도 쓴다. 거절한 행의 양은 0이다. 샌드박스가 소켓 접속을 막아 요청이 engine에 오지 못한 시도는 engine이 도구 결과 첫머리의 오류 표지(`saturn evidence unreachable (read 13)`)를 읽어 `Unreachable`로 쓴다([근거 검색과 원문 조회](context-selection.md#근거-검색과-원문-조회)).
3. 쓰지 못해도 조회는 막지 않고 로그만 남긴다.

- 원문과 검색어는 이 표에 두지 않는다. 기록 번호로 `events`를 가리킬 뿐이고, 원문은 가려진 채 거기에만 있다.
- 사용량은 기존 `usage`에 provider가 보고한 값이 이미 쌓인다. 이 표는 그 사용량이 조회를 거쳤는지 맞춰 보는 재료다.
- 기록 번호는 채팅마다 센다. 그래서 `record_id`는 `chat_id`와 함께 읽는다.

### 보존과 정리

기록은 기본으로 기한 없이 보존하고, 자동 정리는 설정 `retention.auto_prune`을 켤 때만 engine 시작 때 한 번 실행한다. 기록을 재현과 Saturn 모델 학습에 쓰고 삭제는 되돌릴 수 없어, `max_age_days`만 있다고 켜지지 않고 별도 스위치를 둔다.

- 정리 명령은 `--yes` 없이는 지우지 않고 미리보기만 한다. 미리보기는 확인 번호를 함께 돌려준다.
- `saturn prune`은 `retention.max_age_days`보다 오래 쓰지 않은 채팅(마지막 활동이 그 기한보다 이른 채팅)을 대상으로 한다. 이 설정이 없으면 대상을 정할 수 없어 아무것도 지우지 않고 거절한다(초안). 마지막 활동은 채팅 생성, 입력 접수, 실행 시작과 끝, 이벤트 중 가장 늦은 시각이다.
- `engine`은 `Prune`에 `yes`가 거짓이면 `PrunePreview`, 참이면 `Pruned`를 응답 `result`로 돌려준다. 둘 다 지울(지운) 채팅(번호, 폴더, 이름, 마지막 활동, 첫 입력), 남긴 채팅과 이유, 지울(지운) 채팅의 행 수(입력, 실행, 이벤트, 사용량, session. 판단 기록과 설정 스냅샷은 세지 않는다)를 싣는다.
- 남기는 이유는 열린 입력, 열린 실행, 멈춤 처리 중, 열린 session, 보관한 session과 `engine`의 `TUI에 붙어 있음`이다. TUI가 붙은 채팅은 열린 항목이 없어도 지우지 않는다. 붙은 TUI가 지워진 채팅에 입력을 보내는 일을 막기 위해서다(초안). 삭제는 `--yes`일 때만 한다.
- 확인 번호(`plan`)는 미리보기가 정한 채팅 목록을 engine 메모리에 기억해 두는 열쇠다. `Prune { yes: true, plan }`은 그 목록의 채팅 가운데 지금도 기한을 넘기고 열린 항목이 없고 TUI가 붙어 있지 않은 것만 지운다. 미리본 뒤에 기한 안으로 돌아온 채팅은 `UsedSincePreview`, 열린 입력이 생겼거나 TUI가 붙은 채팅은 각각의 이유로 남기고, 목록에 없던 채팅은 후보가 되어도 지우지 않는다. 열린 항목은 삭제 거래 안에서 다시 확인한다. 미리보기에서 정한 대로만 지워지게 하기 위해서다.
- 확인 번호는 한 번만 쓸 수 있고 10분 뒤 만료되며, engine은 최근 8개만 기억한다. 모르거나 만료됐거나 이미 쓴 번호는 `INVALID_PARAMS`와 `NotFound` 종류로 거절하고 아무것도 지우지 않는다(CLI 종료 코드 66). 번호는 접속과 무관하게 engine 안에서 유효하다. `saturn prune`의 미리보기와 `--yes`가 서로 다른 접속이기 때문이다. 번호는 맞히기 어렵게 만든다(초안).
- `plan` 없이 `yes`만 보내면 거절하고 지우지 않는다. 미리보기 확인을 모르는 옛 클라이언트의 `yes`가 지금 대상 전체의 삭제로 읽히지 않게 하기 위해서다. 요청 순간의 기준으로 대상을 정해 바로 지우려면 `all`을 함께 보내야 하고, 미리보기를 거치지 않는 `saturn prune --yes`가 이 방식이다. 시작 때 자동 정리는 요청이 아니라 engine 안의 호출이라 번호를 요구하지 않는다. 이 변경으로 protocol 판을 2로 올렸다. 새 `cli`는 판 1 engine을 교체하고, 판 1 클라이언트가 새 engine에 보낸 `yes`는 위처럼 거절된다. 클라이언트는 번호가 있는 요청의 응답을 읽지 못하면 기다리지 않고 그 요청의 실패로 끝낸다.
- 자동 정리는 `retention.auto_prune`이 참이고 `retention.max_age_days`가 있을 때만 한다. engine이 크래시 복구와 보내지 않은 입력 되살림을 마친 뒤 소켓 요청을 처리하기 전에 한 번 `max_age_days`보다 오래 쓰지 않은 채팅을 `saturn prune --yes`와 같은 규칙(열린 항목이 있는 채팅은 남김)으로 지운다. 그 뒤에는 engine이 도는 동안 다시 하지 않는다. 시작 때라 붙은 TUI가 없다.
- 자동 정리가 지운 채팅이 있으면 stderr 로그에 `자동 정리: 채팅 N개 · 기록 M행`을 한 줄 남기고, 처음 붙는 TUI에 상태판 알림 `Alert::AutoPruned { chats, rows }`를 보낸다. TUI는 알림 줄에 `오래된 채팅 N개를 지웠습니다`를 보인다. 지울 채팅이 없으면 로그도 알림도 없다. `auto_prune`이 참인데 `max_age_days`가 없으면 아무것도 지우지 않고 `자동 정리를 건너뜀: retention.max_age_days가 없음`을 로그에 한 줄 남긴다.
- 자동 정리가 실패하면(기록 저장소 오류) 삭제는 한 거래라 일부만 지워지지 않고, engine은 시작을 멈추지 않고 이어 간다. 오류를 로그에 남기고 처음 붙는 TUI에 `Alert::AutoPruneFailed`를 보내며, TUI는 `자동 정리에 실패했습니다 · 로그를 확인하세요`를 보인다. 다시 시도하지 않고 다음 시작 때 한 번 더 한다. 정리 실패로 기록이 늘어나는 일은 기록을 잃는 것보다 낫기 때문이다.
- `saturn prune`의 출력은 `지울 채팅 N개 · 기록 M행`과 채팅 줄, `남긴 채팅 N개`와 이유 줄, 미리보기이면 `아무것도 지우지 않았습니다 · 지우려면 saturn prune --yes --plan <번호> 으로 실행하세요`다. 지운 결과에는 번호가 없다. TUI의 `/prune` 창은 같은 `Prune` 요청을 쓰고, 지울 채팅 줄의 이름, 마지막 사용 날짜, 크기(채팅의 기록 행 수)를 `PrunePreview`로 받는다. 그래서 `PrunePreview`와 `Pruned`의 지울(지운) 채팅 줄마다 그 채팅의 행 수를 싣는다. 창은 미리보기 번호를 들고 있다가 `y`로 그 번호를 실어 확정하고 `Esc`로 취소한다([TUI](tui.md#기록-정리-창)). 기준 설정이 없으면 거절 응답 앞에 `Alert::PruneNeedsRetention`을 요청한 접속에 보내 TUI가 안내를 보이게 한다.
- 실제 삭제에서는 제외 조건 확인과 삭제를 같은 거래에서 처리한다.
- `~/.claude`, `~/.codex`의 provider 기록은 지우지 않는다. Saturn 소유가 아니기 때문이다.

### 삭제 대상 제외

어떤 명령으로 지워도 `store`는 아래 항목을 삭제 대상에서 뺀다.

- 열린 입력
- 열린 실행
- 처리 중인 중지 요청
- 활성 session과 대기 session

열린 항목을 포함한 삭제 요청이 오면 engine은 열린 항목을 지우지 않는다. 사용자가 먼저 그 작업을 끝내거나 멈추게 한다.

### 삭제 절차

1. `store`가 `secure_delete=ON` 상태로 대상을 지운다.
2. `store`가 지운 채팅의 ID, 해시, 삭제 시각을 삭제 흔적으로 기록한다.
3. `store`가 `PRAGMA wal_checkpoint(TRUNCATE)`를 실행한다.
4. `store`가 `VACUUM`을 실행한다.

- `secure_delete=ON`으로 지운 자리를 덮어쓴다. 지운 내용이 파일에 남는 일을 막기 위해서다.
- 채팅을 지워도 삭제 흔적을 남긴다. 같은 ID가 다시 들어오면 알아보기 위해서다.

### 판단 기록 정리와 내보내기

- 판단 기록은 일반 정리에서 빼고 판단 기록 전용 정리로만 지운다. 판단 기록을 Saturn 모델 학습에 쓰기 때문이다.
- 판단 기록은 채점하지 않아도 JSONL로 내보낼 수 있다.
- 내보내기 한 줄에는 모델 판단 그림자가 있으면 `model_shadow`(없으면 null)가, 모델 정하기 기록이 있으면 `model_selection`이, 입력이 낳은 결과 `applied`가 붙는다. `applied`는 입력에 남은 모델 `input_model`, 입력 원문의 SHA-256 `input_sha256`, 입력이 연 실행별 provider·session·session 모델과 입력·캐시 읽기·캐시 쓰기 토큰 합(`runs`), 받은 인계 패킷(`packets`: 종류, 상태, session, 채팅 revision, 설정 번호, 정책 지문, 본문 해시)이다. 같은 입력 번호로 읽기 전용으로 이어 붙이므로 따로 저장하지 않는다.
- 내보내기 한 줄의 필드는 `id`, `chat`, `input`, `method`, `router`, `model`, `question_sets`(`name@major.minor`), `settings`, `sent`, `received`, `answers`, `fallbacks`, `tokens`(없으면 null), `started_at`(unix 밀리초), `elapsed_ms`, `outcome`이다(초안).
- 내보내기 파일은 새로 만들고 권한 0600으로 둔다(초안). 이미 있는 파일에는 쓰지 않는다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| 열린 항목을 포함한 삭제 요청 | 열린 항목은 지우지 않고, 먼저 해당 작업을 끝내거나 멈추게 한다. |
| 사용량 중간 보고 누락 | 차이가 여러 턴에 걸친다고 표시하고 0으로 채우지 않는다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 기록 저장소에는 engine 하나만 쓴다. | 두 번째 engine이 잠금을 얻지 못해 쓰지 못하는지 확인한다. |
| 근거 조회는 검색과 읽기, 거절을 모두 종류와 결과, 양과 함께 채팅별로 남긴다. 스키마 V19 파일은 채팅을 보존한 채 표만 더해 이관한다. | `saturn-terminal/engine/src/lifecycle/evidence.rs`의 `an_unknown_pass_is_refused_and_every_lookup_is_counted`, `saturn-terminal/engine/src/store/schema.rs`의 `v19_file_migrates_to_evidence_lookups_keeping_chats` |
| provider가 보고하지 않은 값은 NULL로 남는다. | 사용량 일부가 빠진 보고를 넣어 빈 값이 0이 아닌 NULL인지 확인한다. |
| 스키마를 올리기 전 백업 하나를 남긴다. | 옛 스키마 파일로 새 버전을 실행해 백업 1개와 이관된 스키마가 생기는지 확인한다. |
| 스키마 V2 이관은 session 행을 보존하고 마지막 턴 열을 NULL로 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 `v1_file_migrates_to_last_turn_columns_keeping_sessions`, `migration_keeps_only_latest_backup_and_removes_old_ones` |
| 마지막 턴 값은 저장하고 되살린다. | `saturn-terminal/engine/src/store/sessions.rs`의 `record_last_turn_round_trips_through_live_mains`, `record_last_turn_overwrites_and_survives_session_upsert` |
| 스키마 V3 이관은 판단 기록 행을 보존하고 결과 신호와 물은 답 칸을 NULL로 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 `v2_file_migrates_to_outcome_columns_keeping_judgments` |
| 스키마 V4 이관은 채팅 행을 보존하고 항상 허용 표를 비어 있게 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 `v3_file_migrates_to_permission_allows_keeping_chats` |
| 스키마 V5 이관은 채팅 행을 보존하고 더한 폴더 표를 비어 있게 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 `v4_file_migrates_to_chat_dirs_keeping_chats` |
| 스키마 V8 이관은 채팅 행을 보존하고 이름과 묶음 칸을 비어 있게 더한다. 이관 직전 백업은 하나만 남는다. | `saturn-terminal/engine/src/store/schema.rs`의 `v7_file_migrates_to_chat_name_and_group_keeping_chats` |
| 채팅 이름과 묶음은 앞뒤 공백을 지워 저장하고, 비면 지우며, 없는 채팅이나 제어 문자가 든 값은 거절하고 바꾸지 않는다. 이름은 채팅 목록에 보인다. | `saturn-terminal/engine/src/lifecycle/chat_labels.rs`의 `rename_and_group_are_saved_trimmed_and_shown_in_the_chat_list`, `blank_name_and_no_group_clear_the_labels`, `unknown_chat_and_control_characters_are_refused_without_changing_anything` |
| 스키마 V9 이관은 채팅 행을 보존하고 provider 버전 표를 비어 있게 더한다. 이관 직전 백업은 하나만 남는다. | `saturn-terminal/engine/src/store/schema.rs`의 `v8_file_migrates_to_provider_versions_keeping_chats`, `saturn-terminal/engine/src/store/versions.rs`의 `recorded_version_is_read_back_and_overwritten_per_provider` |
| 스키마 V11 이관은 실행 행을 보존하고 수정 파일 표와 측정 상태 열을 비어 있게 더한다. 이관 직전 백업은 하나만 남는다. | `saturn-terminal/engine/src/store/schema.rs`의 `v10_file_migrates_to_run_changes_keeping_runs` |
| 실행별 수정 파일은 주체와 부분 표시까지 저장하고 되살리며, 다시 기록하면 바꿔 쓴다. | `saturn-terminal/engine/src/store/changes.rs`의 `run_changes_round_trip_with_actors_and_partial_mark`, `recording_again_replaces_the_list_and_empty_runs_are_not_returned` |
| 스키마 V7 이관은 채팅 행을 보존하고 보류 작업 표와 끊긴 하위 에이전트 표를 비어 있게 더한다. 이관 직전 백업은 하나만 남는다. | `saturn-terminal/engine/src/store/schema.rs`의 `v6_file_migrates_to_recovery_tables_keeping_chats_and_backing_up` |
| 스키마 V10 이관은 채팅 행을 보존하고 제약 표 다섯 개를 비어 있게 더한다. 이관 직전 백업은 하나만 남는다. | `saturn-terminal/engine/src/store/schema.rs`의 `constraint_v9_file_migrates_to_empty_constraint_tables_keeping_chats` |
| 제약 변경은 상태와 이벤트를 한 거래로 쓰고 이벤트는 이어 쓰기만 한다. 제약 revision은 마지막 이벤트 번호다. | `saturn-terminal/engine/src/store/constraints.rs`의 `constraint_active_registration_writes_rows_and_events_in_one_transaction`, `constraint_revision_is_the_last_event_number_and_zero_without_events`, `constraint_registration_that_fails_midway_writes_nothing`, `constraint_candidate_gets_one_ask_and_answers_apply_to_the_whole_input` |
| 묻는 중인 확인은 engine을 다시 켜도 되살아나고, 대상이 바뀌면 `Void`로 닫힌다. | 다시 열기는 `saturn-terminal/engine/src/store/constraints.rs`의 `constraint_open_asks_survive_reopening_the_store`, `saturn-terminal/engine/src/lifecycle/constraints.rs`의 `constraint_ask_reaches_a_tui_that_attaches_later_and_after_a_restart`. 입력 취소로 닫고 늦은 답을 거절하는 것은 같은 파일의 `constraint_ask_is_closed_when_its_input_is_canceled`. 해제와 예외로 대상이 바뀌는 경우는 구현 전(#379) |
| 채팅을 지우면 제약, 이벤트, 확인, 패킷 제약이 함께 지워진다. | 제약, 이벤트, 확인은 `saturn-terminal/engine/src/store/constraints.rs`의 `constraint_rows_are_removed_only_with_their_chat`. 패킷 제약은 `saturn-terminal/engine/src/store/schema.rs`의 외래 키 `ON DELETE CASCADE`이고 기록은 `saturn-terminal/engine/src/lifecycle/constraint_handoff.rs`의 `packet_constraints_records_what_the_new_session_got` |
| 보류 작업은 작업 번호 순서로 읽히고 같은 작업은 덮어쓰며 채팅을 지우면 함께 지워진다. 끊긴 하위 에이전트는 정리를 넘겼다는 표시를 유지하며 에이전트가 끝나면 지워진다. | `saturn-terminal/engine/src/store/recovery.rs`의 `held_task_is_saved_replaced_and_deleted`, `interrupted_subagents_keep_cleaned_flag_until_the_agent_ends`, `recovery_rows_follow_the_chat_when_it_is_deleted` |
| 더한 폴더는 채팅마다 더한 순서대로 읽히고 같은 경로는 한 번만 저장되며 채팅을 지우면 함께 지워진다. | `saturn-terminal/engine/src/store/chat_dirs.rs`의 `add_dir_is_kept_per_chat_in_added_order_without_duplicates`, `add_dir_for_a_missing_chat_is_refused`, `add_dir_rows_follow_the_chat_when_it_is_deleted` |
| 항상 허용은 작업 폴더마다 저장 순서대로 읽히고 같은 행은 한 번만 저장된다. | `saturn-terminal/engine/src/store/permissions.rs`의 `allows_are_kept_per_workdir_in_saved_order`, `saving_the_same_allow_twice_keeps_one_row` |
| 결과 신호와 물은 답은 같은 판단 기록에 저장하고, 신호는 한 번 확정하면 바꾸지 않으며, 물은 답은 묻지 않은 판단에 쓰지 않는다. | `saturn-terminal/engine/src/store/outcomes.rs`의 `observations_carry_signal_answer_and_q_of_the_judgment`, `record_signal_keeps_first_confirmed_value`, `record_asked_answer_keeps_first_answer`, `record_asked_answer_for_unasked_judgment_returns_not_found` |
| 이관 백업은 14일이 지나면 지운다. | 만든 지 14일이 지난 백업이 다음 시작 때 사라지는지 확인한다. |
| 원시 기록의 해시와 크기는 압축 전 값이다. | 압축 뒤 기록한 해시가 원본의 해시와 같은지 확인한다. |
| provider가 보낸 줄은 그 에이전트의 열린 실행에 받은 순서대로 쌓이고, 끝난 실행에 늦게 온 줄과 에이전트를 모르는 줄과 JSON이 아닌 줄은 실행에 붙지 않고 미귀속으로 남는다. | `saturn-terminal/engine/src/lifecycle/events.rs`의 `raw_lines_go_to_the_run_of_their_agent_in_order_and_late_ones_are_kept_apart`, `raw_lines_without_a_known_agent_are_kept_apart_and_never_attached_to_the_open_run`, `saturn-terminal/engine/src/lifecycle/crash_recovery.rs`의 `raw_lines_received_before_a_crash_survive_and_recovery_seals_them` |
| 어댑터는 줄을 변환 전에 router 키를 가려 보내고, 줄의 에이전트를 provider 식별자로 정한다. | `saturn-terminal/engine/src/providers/claude/tests.rs`의 `stdout_hides_router_key_before_emitting_events`, `saturn-terminal/engine/src/providers/codex/tests.rs`의 `a_child_is_registered_from_the_spawn_completion_without_thread_started` |
| 스키마 V19 이관은 채팅 행을 보존하고 모델 정하기 표를 비어 있게 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 `v18_file_migrates_to_model_selections_keeping_chats` |
| 스키마 V18 이관은 실행 행을 보존하고 완료 검사 근거 열을 비어 있게 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 `v17_file_migrates_to_completion_evidence_columns_keeping_runs` |
| 완료 검사 근거는 상태, 까닭, 근거 이벤트 번호까지 저장하고 되살리며, 다시 기록하면 바꿔 쓴다. | `saturn-terminal/engine/src/store/evidence.rs`의 `completion_round_trips_and_is_absent_until_recorded` |
| 패킷 시도는 보낸 글의 해시와 일치하고, 제약 칸 항목이 `packet_constraints`와 같으며, 입력의 실행과 받은 provider session이 붙는다. | `saturn-terminal/engine/src/lifecycle/constraint_handoff.rs`의 `long_chat_constraints_reach_the_new_sessions_packet_and_record` |
| 맥락 한도로 줄여 다시 보내면 시도마다 행이 따로 쌓이고, 거절된 시도는 `NotSent`, 다시 보낸 시도는 `Sent`로 줄이기 전 시도를 가리키며, 줄인 항목은 원문에서 요약본(`Digest`)으로 바뀐다. 맥락 정리도 같다. | `saturn-terminal/engine/src/lifecycle/packet_overflow.rs`의 `packet_overflow_rejection_resends_once_without_the_lowest_items`, `packet_overflow_after_the_reduced_resend_stops_and_tells_the_user`, `compaction_overflow_rejection_resends_once_without_the_lowest_items` |
| 보내지 않음이 확정인 오류만 `NotSent`이고 연결 끊김과 결과 불명은 `Unknown`이다. 크래시로 결과를 받지 못한 시도는 `Unknown`이 되고 다시 보내지 않는다. | `saturn-terminal/engine/src/packets.rs`의 `only_a_confirmed_refusal_is_not_sent_and_every_other_failure_is_unknown`, `saturn-terminal/engine/src/lifecycle/crash_recovery.rs`의 `a_packet_without_a_result_before_the_crash_becomes_unknown_and_is_not_sent_again` |
| 패킷의 항목은 구역별로 들어간 모양과 빠진 이유를 남기고, 대화 본문은 역할·번호·원문 해시를 실제 순서로 남긴다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_fixed_zone_in_order_and_tool_results_only_in_competing`, `build_packet_over_soft_limit_keeps_every_turn_whole_and_empties_competing`, `build_packet_fixed_over_limit_allows_hard_limit_without_competing`, `saturn-terminal/engine/src/lifecycle/dialogue_preserved.rs`의 `four_reads_after_a_correction_do_not_push_it_out_of_the_switch_packet` |
| 스키마 V21 이관은 전달 패킷 항목에 본문 해시 열을 비워 두고 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 이관 시험 전체(`SCHEMA_VERSION` 대조) |
| 스키마 V17 이관은 채팅 행을 보존하고 모델 판단 그림자 표를 비어 있게 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 `v16_file_migrates_to_model_shadows_keeping_chats` |
| 스키마 V16 이관은 채팅 행을 보존하고 전달 패킷 표 두 개를 비어 있게 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 `v15_file_migrates_to_handoff_packets_keeping_chats` |
| 스키마 V15 이관은 채팅 행을 보존하고 미귀속 원시 줄 표를 비어 있게 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 `v14_file_migrates_to_raw_unattributed_keeping_chats` |
| 열린 입력, 열린 실행, 활성 session은 어떤 명령으로도 지우지 않는다. | 열린 항목이 있는 채팅을 지워 그 항목이 남는지 확인한다. |
| 자동 정리는 `retention.auto_prune`이 참일 때만 시작 때 한 번 오래된 채팅을 지우고, 기본(거짓)이거나 `max_age_days`가 없으면 아무것도 지우지 않는다. | `saturn-terminal/engine/src/lifecycle/prune.rs`의 `auto_prune_on_start_does_nothing_unless_the_switch_is_on_with_a_max_age`, `auto_prune_on_start_deletes_old_finished_chats_and_tells_the_first_tui_only` |
| 자동 정리는 열린 항목이 있는 채팅을 지우지 않고, 지운 수를 처음 붙는 TUI에 알리며, 실패해도 engine은 시작하고 실패를 알린다. | `saturn-terminal/engine/src/lifecycle/prune.rs`의 `auto_prune_on_start_deletes_old_finished_chats_and_tells_the_first_tui_only`, `auto_prune_failure_keeps_every_chat_and_still_tells_the_first_tui` |
| `PrunePreview`와 `Pruned`는 지울(지운) 채팅마다 행 수를 싣는다. | `saturn-terminal/engine/src/lifecycle/prune.rs`의 `prune_lines_carry_the_row_count_of_each_chat_and_add_up_to_the_total` |
| 정리 명령은 `--yes` 없이는 미리보기만 하고, 미리보기는 지울 채팅과 남길 채팅과 이유와 행 수를 보인다. | `saturn-terminal/engine/src/lifecycle/prune.rs`의 `prune_without_yes_previews_old_chats_and_deletes_nothing`, `saturn-terminal/cli/src/commands/prune.rs`의 `preview_lists_what_would_be_deleted_and_what_stays_and_says_nothing_was_deleted`, `run_without_yes_asks_for_preview_only_and_reads_the_preview` |
| `--yes`일 때만 오래된 채팅 가운데 열린 항목이 없는 것만 지우고 결과를 알리며, 기준 설정이 없으면 거절한다. | `saturn-terminal/engine/src/lifecycle/prune.rs`의 `prune_with_yes_deletes_only_the_old_finished_chats_and_reports_them`, `prune_without_a_retention_setting_is_refused_and_deletes_nothing`, `saturn-terminal/cli/src/commands/prune.rs`의 `run_with_yes_deletes_and_reads_the_result`, `run_without_a_retention_setting_explains_how_to_set_it` |
| 미리보기 번호를 실은 확인은 미리보기에 있던 채팅만 지우고, 미리본 뒤 다시 쓰이거나 열린 항목이 생겼거나 TUI가 붙은 채팅은 남긴다. | `saturn-terminal/engine/src/lifecycle/prune.rs`의 `prune_confirmation_does_not_delete_a_chat_that_was_not_previewed`, `prune_confirmation_keeps_a_previewed_chat_that_was_used_again`, `prune_confirmation_keeps_a_previewed_chat_that_got_an_open_input`, `prune_confirmation_keeps_a_previewed_chat_a_tui_attached_to_afterwards` |
| 확인 번호는 한 번만 쓸 수 있고 만료되며, 모르는 번호는 아무것도 지우지 않고 거절한다. 다른 접속이 미리본 번호도 같은 목록만 지운다. 번호 없는 확인은 거절하고, `all`을 함께 보낸 확인만 요청 순간의 대상을 지운다. 응답을 읽지 못하는 옛 engine에는 기다리지 않고 실패한다. | `saturn-terminal/engine/src/lifecycle/prune.rs`의 `a_prune_plan_works_once_and_a_made_up_one_deletes_nothing`, `a_prune_plan_from_another_connection_deletes_only_the_previewed_chats`, `confirming_without_a_plan_decides_the_targets_at_that_moment`, `a_confirmation_from_a_client_without_plan_support_deletes_nothing`, `saturn-terminal/cli/tests/exit_codes.rs`의 `prune_against_an_engine_that_answers_without_a_plan_fails_instead_of_waiting`, `saturn-terminal/cli/src/launch.rs`의 `an_engine_of_the_protocol_before_the_prune_plan_is_retired_at_the_same_version`, `saturn-terminal/engine/src/prune.rs`의 `prune_plans_expire_are_single_use_and_keep_a_bounded_number`, `saturn-terminal/cli/src/commands/prune.rs`의 `run_with_a_plan_sends_the_previewed_id_back`, `run_with_an_unknown_plan_says_to_preview_again`, `saturn-terminal/tui/src/app/tests.rs`의 `prune_window_asks_for_the_preview_and_only_y_deletes` |
| TUI가 붙은 채팅은 정리하지 않는다. | `saturn-terminal/engine/src/lifecycle/prune.rs`의 `prune_keeps_a_chat_a_tui_is_attached_to` |
| 지운 채팅은 삭제 흔적을 남긴다. | 채팅을 지운 뒤 ID, 해시, 삭제 시각이 남는지 확인한다. |
| 일반 정리는 판단 기록을 지우지 않는다. | 일반 정리 뒤 판단 기록 수가 그대로인지 확인한다. |
| 채점하지 않아도 판단 기록을 JSONL로 내보낼 수 있다. | 채점 없는 판단 기록을 내보내 줄마다 JSON 한 건인지 확인한다. |
| provider 기록은 지우지 않는다. | 정리 뒤 `~/.claude`, `~/.codex`의 파일이 그대로인지 확인한다. |
| 옛 provider 값 `Codex`, `Claude`가 옛 값 그대로 읽히고 새 값은 id 글자로 쓰이며, 모델 고정 글은 그대로 읽힌다. | `saturn-terminal/engine/src/store/records/tests.rs`의 `old_provider_values_read_as_open_ids`, `new_provider_values_are_written_as_ids`, `saturn-terminal/engine/src/store/sessions.rs`의 `cache_ttl_reads_the_key_an_old_version_wrote`, `saturn-terminal/engine/src/providers/mod.rs`의 `pinned_text_keeps_the_old_provider_prefix` |
| 확장 저장소는 채팅 정리의 대상이 아니다. | `saturn-terminal/engine/src/lifecycle/extensions.rs`의 `pruning_old_chats_leaves_installed_extensions_alone` |
| 스키마 V13 이관은 입력 행을 보존하고 끼워 넣은 입력 열 둘을 NULL로 더한다. 이관 직전 백업은 하나만 남는다. | `saturn-terminal/engine/src/store/schema.rs`의 `v12_file_migrates_to_steered_input_columns_keeping_inputs` |
| 끼워 넣어 적용한 입력은 적용한 순서와 쌓여 있던 기록 번호와 함께 읽히고, 대기로 돌아온 입력과 실행을 연 입력은 읽히지 않는다. | `saturn-terminal/engine/src/store/ledger.rs`의 `steered_inputs_keep_acceptance_order_and_the_sequence_they_arrived_after`, `steered_inputs_leave_out_inputs_that_are_not_applied_and_run_openers` |
| 끼워 넣기 연결 쓰기가 실패하면 입력은 연결 없는 `Applied`로 남지 않는다. | `saturn-terminal/engine/src/lifecycle/intake.rs`의 `steer_is_not_applied_without_its_run_link_when_the_link_write_fails`, `failed_steer_record_keeps_the_input_delivering_tells_the_user_and_retries_only_the_record` |
| 스키마 V14 이관은 채팅 행을 보존하고 직접 설치 표를 비어 있게 더한다. 같은 항목은 한 번만 물은 것으로 남고 옮기면 `moved`가 된다. | `saturn-terminal/engine/src/store/schema.rs`의 `v13_file_migrates_to_direct_installs_keeping_chats`, `saturn-terminal/engine/src/store/direct.rs`의 `an_item_is_asked_once_and_moving_it_keeps_the_row` |
| 스키마 V12 이관은 채팅 행을 보존하고 확장 표를 비어 있게 더한다. 이관 직전 백업은 하나만 남는다. | `saturn-terminal/engine/src/store/schema.rs`의 `v11_file_migrates_to_extensions_keeping_chats`, `saturn-terminal/engine/src/store/extensions.rs`의 `extension_rows_keep_install_order_and_refuse_a_second_row_with_the_same_name` |
