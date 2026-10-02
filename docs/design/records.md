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
3. 사용자가 같은 명령을 `--yes`와 함께 다시 실행한다.
4. `store`가 열린 입력과 활성 session이 있는 항목은 빼고 나머지를 지운다.
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
| 실행 | 실행마다의 `effect_scope`, 원시 기록 |
| session | 닫은 session의 provider session ID, 마지막 턴의 활성 맥락과 끝 시각 |
| 사용량 | 사용량 보고 원값, 범위, 대상 에이전트, 모델 |
| 판단 기록 | router 호출의 보낸 원문, 받은 원문, 질문별 답, 비용, 시간, 물은 확률 q, 결과 신호, 물은 답 |
| 설정 스냅샷 | 설정 번호별 병합 결과와 층 목록 |

- 기록 저장소에 쓰는 쪽은 engine 하나다. 쓰기 충돌을 막기 위해서다.
- engine은 사용자당 하나이고 잠금으로 지킨다. 쓰는 프로세스를 하나로 유지하기 위해서다.
- 한 번의 변경은 한 거래로 처리하고, provider가 보고하지 않은 값은 0이 아닌 NULL로 둔다. 쓰기 충돌과 지어낸 값을 막기 위해서다.
- 설정 원본은 파일이고 기록 저장소에는 적용된 설정의 스냅샷만 둔다([설정](settings.md)).
- TUI의 입력 기록 `~/.saturn/history`는 기록 저장소 밖의 권한 0600 파일이고 TUI가 쓴다.
- 첫 스키마(V1)는 옛 스키마 위 변경분이 아니라 전체 정의로 쓴다. 공개 저장소만으로 스키마 전체를 읽기 위해서다.
- 표 이름은 `chats`, `inputs`, `runs`, `sessions`, `events`, `usage`이고 Saturn 용어(채팅, 입력, 실행, session)를 따른다.
- `sessions` 표는 마지막 활성 맥락 `last_active`(토큰)와 마지막 턴 끝 시각 `last_turn_ended_at`(unix 밀리초) 열을 둔다. engine이 턴이 끝날 때 쓰고 시작할 때 읽어 보관 session의 재개 판정을 재시작 뒤에도 같게 한다([provider 연결과 session](providers-and-sessions.md#그-provider로-돌아가기)). 두 열은 스키마 V2에서 더했고 이관 전 행은 NULL이며, NULL이면 재개로 판정한다.
- `sessions` 표의 `model`은 session을 열 때 고른 모델이다. 고르지 않았거나 이관 전 행은 NULL이고, 입력의 모델과 다르면 새 메인 session을 연다([모델 고르기](providers-and-sessions.md#모델-고르기)). 스키마 V6에서 더했다.
- `chat_dirs` 표는 채팅에 더한 폴더를 채팅 `chat_id`, 링크를 푼 절대 경로 `path`, 더한 시각 `added_at`(unix 밀리초)으로 둔다. 같은 채팅의 같은 경로는 한 행이고 행 번호 순서가 더한 순서다. 채팅을 지우면 함께 지운다. 스키마 V5에서 더했다([engine 수명과 복구](engine-lifecycle.md#채팅-폴더와-이어-열기)).
- `permission_allows` 표는 항상 허용을 작업 폴더 `workdir`, 도구 `tool`, 패턴 `pattern`, 저장 시각 `created_at`(unix 밀리초)으로 둔다. 같은 `workdir`, `tool`, `pattern`은 한 행이다. 스키마 V4에서 더했고 채팅과 상관없이 작업 폴더 단위로 쓴다([권한](permissions.md#항상-허용-저장)).
- 패킷과 돌아온 session의 변경분은 `events` 행을 그 이벤트를 연 실행의 session과 입력 원문, 기록 시각에 이어 읽어 만든다. 이벤트 행에 session을 따로 저장하지 않기 위해서다. `sessions.delivered`는 턴이 끝날 때와 session을 열거나 바꿀 때 저장한다.
- session과 에이전트 번호는 `meta` 표에 마지막으로 준 번호를 두고, 기록에 있는 가장 큰 번호보다 큰 값을 한 거래로 새로 준다. 번호를 다시 쓰지 않기 위해서다.
- 기록 저장소 파일 권한은 0600이다(초안). 입력 원문과 판단 기록이 들어 있기 때문이다.

### 사용량 조회 범위

`/usage`의 `day`는 지금부터 24시간 전부터, `week`는 7일 전부터이고 모든 채팅을 합친다. provider가 알려 준 사용량은 보여 주는 범위와 상관없이 모두 기록하고, 범위는 조회할 때만 가른다. 붙은 채팅이 없는 `saturn usage`의 `chat` 범위는 요청에 실린 폴더에서 마지막 입력 접수가 가장 늦은 채팅(입력이 없으면 만든 시각)을 쓴다. session 누적 보고가 직전 누적보다 작거나 그사이 보고 없는 실행이 있으면 그 행을 여러 턴에 걸친 값으로 표시한다. `/usage` 응답은 provider·모델마다 한 행과 router 한 행으로 묶고, 여러 턴을 합친 행은 protocol `UsageRow.turns`에 턴 수를 싣는다. 비용, 맥락 정리, 채점은 저장하지 않아 `None`으로 보낸다. 묶음 규칙과 표시는 [TUI](tui.md)에 있다.

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

### 보존과 정리

기록은 기본으로 기한 없이 보존하고, 자동 정리는 설정으로 켤 때만 engine 시작 때 한 번 실행한다. 기록을 재현과 Saturn 모델 학습에 쓰기 때문이다.

- 정리 명령은 `--yes` 없이는 지우지 않고 미리보기만 한다.
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
| provider가 보고하지 않은 값은 NULL로 남는다. | 사용량 일부가 빠진 보고를 넣어 빈 값이 0이 아닌 NULL인지 확인한다. |
| 스키마를 올리기 전 백업 하나를 남긴다. | 옛 스키마 파일로 새 버전을 실행해 백업 1개와 이관된 스키마가 생기는지 확인한다. |
| 스키마 V2 이관은 session 행을 보존하고 마지막 턴 열을 NULL로 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 `v1_file_migrates_to_last_turn_columns_keeping_sessions`, `migration_keeps_only_latest_backup_and_removes_old_ones` |
| 마지막 턴 값은 저장하고 되살린다. | `saturn-terminal/engine/src/store/sessions.rs`의 `record_last_turn_round_trips_through_live_mains`, `record_last_turn_overwrites_and_survives_session_upsert` |
| 스키마 V3 이관은 판단 기록 행을 보존하고 결과 신호와 물은 답 칸을 NULL로 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 `v2_file_migrates_to_outcome_columns_keeping_judgments` |
| 스키마 V4 이관은 채팅 행을 보존하고 항상 허용 표를 비어 있게 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 `v3_file_migrates_to_permission_allows_keeping_chats` |
| 스키마 V5 이관은 채팅 행을 보존하고 더한 폴더 표를 비어 있게 더한다. | `saturn-terminal/engine/src/store/schema.rs`의 `v4_file_migrates_to_chat_dirs_keeping_chats` |
| 더한 폴더는 채팅마다 더한 순서대로 읽히고 같은 경로는 한 번만 저장되며 채팅을 지우면 함께 지워진다. | `saturn-terminal/engine/src/store/chat_dirs.rs`의 `add_dir_is_kept_per_chat_in_added_order_without_duplicates`, `add_dir_for_a_missing_chat_is_refused`, `add_dir_rows_follow_the_chat_when_it_is_deleted` |
| 항상 허용은 작업 폴더마다 저장 순서대로 읽히고 같은 행은 한 번만 저장된다. | `saturn-terminal/engine/src/store/permissions.rs`의 `allows_are_kept_per_workdir_in_saved_order`, `saving_the_same_allow_twice_keeps_one_row` |
| 결과 신호와 물은 답은 같은 판단 기록에 저장하고, 신호는 한 번 확정하면 바꾸지 않으며, 물은 답은 묻지 않은 판단에 쓰지 않는다. | `saturn-terminal/engine/src/store/outcomes.rs`의 `observations_carry_signal_answer_and_q_of_the_judgment`, `record_signal_keeps_first_confirmed_value`, `record_asked_answer_keeps_first_answer`, `record_asked_answer_for_unasked_judgment_returns_not_found` |
| 이관 백업은 14일이 지나면 지운다. | 만든 지 14일이 지난 백업이 다음 시작 때 사라지는지 확인한다. |
| 원시 기록의 해시와 크기는 압축 전 값이다. | 압축 뒤 기록한 해시가 원본의 해시와 같은지 확인한다. |
| 열린 입력, 열린 실행, 활성 session은 어떤 명령으로도 지우지 않는다. | 열린 항목이 있는 채팅을 지워 그 항목이 남는지 확인한다. |
| 정리 명령은 `--yes` 없이는 미리보기만 한다. | `--yes` 없이 실행한 뒤 기록 수가 그대로인지 확인한다. |
| 지운 채팅은 삭제 흔적을 남긴다. | 채팅을 지운 뒤 ID, 해시, 삭제 시각이 남는지 확인한다. |
| 일반 정리는 판단 기록을 지우지 않는다. | 일반 정리 뒤 판단 기록 수가 그대로인지 확인한다. |
| 채점하지 않아도 판단 기록을 JSONL로 내보낼 수 있다. | 채점 없는 판단 기록을 내보내 줄마다 JSON 한 건인지 확인한다. |
| provider 기록은 지우지 않는다. | 정리 뒤 `~/.claude`, `~/.codex`의 파일이 그대로인지 확인한다. |
