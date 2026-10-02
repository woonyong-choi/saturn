# 입력 요청

| 항목 | 값 |
|---|---|
| 상태 | 구현 |
| 관련 실험 | [결정 전 provider 실측](../experiments/provider-decision-checks/report.md) 실험 3 |

## 요약

입력 요청은 provider가 도구 호출 도중에 사용자에게 묻는 질문이다. Codex의 MCP elicitation과 에이전트 질문, Claude의 `AskUserQuestion` 세 형식을 `engine`이 받아 하나의 공통 입력 요청으로 바꾸고, TUI가 입력 창으로 보여 준 뒤 사용자의 답을 provider 형식으로 돌려준다. 답이 올 때까지 provider는 그 호출에서 멈춰 있다. 허가 요청과 같은 방식으로 보관되고 같은 대기 표시를 쓴다.

## 동기

provider는 작업 중에 모르는 값을 사용자에게 직접 묻는다. MCP 서버는 폼이나 링크로 입력을 요구하고, 에이전트는 선택지를 줘서 방향을 묻는다. Saturn이 이 요청을 전달하지 않으면 provider는 답을 영원히 기다리거나 빈 답으로 진행한다. 2026-10-02 실측에서 세 형식의 원문 모양은 서로 달랐고, 각각 다른 응답 모양이 필요했다. 요청이 전달되지 않으면 사용자는 작업이 왜 멈췄는지 알 수 없다.

## 예시

### Claude가 프로젝트 이름을 묻는다

1. Claude가 `AskUserQuestion`으로 질문 하나와 선택지 둘을 보낸다.
2. TUI가 질문과 선택지를 입력 창으로 띄우고 상태판 실행 줄을 `입력 기다림`으로 바꾼다.
3. 사용자가 `↓`와 `Enter`로 둘째 선택지를 고른다.
4. `engine`이 질문 글을 키로 한 `answers`를 요청 `input`에 붙여 허용 응답으로 보낸다.
5. Claude가 답을 받아 턴을 이어 가고 작업이 `실행 중`으로 돌아온다.

### MCP 서버가 폼을 요구한다

1. Codex가 `mcpServer/elicitation/request`로 문자열, 정수, 불리언, 단일 선택, 다중 선택 칸을 가진 폼을 보낸다.
2. TUI가 칸마다 맞는 입력 방식으로 그린다. 사용자가 `Tab`으로 칸을 옮기며 채우고 마지막 칸에서 `Enter`를 누른다.
3. `engine`이 `{"action":"accept","content":{...}}`로 응답한다.
4. 사용자가 `Ctrl+D`를 누르면 `decline`, `Esc`를 누르면 `cancel`로 응답한다.

### MCP 서버가 링크를 열라고 한다

1. Codex가 `mode`가 `url`인 요청을 보낸다.
2. TUI가 설명과 링크를 보이고 링크를 열지 않는다. 사용자가 직접 연 뒤 `Enter`로 계속하면 `accept`, `d`로 거절하면 `decline`을 보낸다.

## 상세 설계

### 요청 형식

| provider | 원문 | 공통 입력 요청으로 옮기는 값 | 사용자 답을 보내는 값 |
|---|---|---|---|
| Codex | `mcpServer/elicitation/request`, `_meta.codex_approval_kind` 없음, `mode: "form"` | `message`, `requestedSchema.properties`의 칸 | `{"action":"accept","content":{칸: 값}}`, `{"action":"decline"}`, `{"action":"cancel"}` |
| Codex | 같은 요청, `mode: "url"` | `message`, `url` | `{"action":"accept"}`, `decline`, `cancel` |
| Codex | `item/tool/requestUserInput` | `questions`의 질문마다 칸 하나 | `{"answers":{질문 id: {"answers":[글]}}}`. 거절과 취소는 `{"answers":{}}` |
| Claude | `control_request`의 `can_use_tool`, `tool_name: "AskUserQuestion"` | `input.questions`의 질문마다 칸 하나 | `{"behavior":"allow","updatedInput":{...요청 input, "answers":{질문 글: 답 글}}}`. 거절과 취소는 `{"behavior":"deny","message":"<고정 문구>"}` |

- `_meta.codex_approval_kind`가 있는 `mcpServer/elicitation/request`는 MCP 도구 승인이라 [허가 요청](permissions.md#허가-요청-창과-답)으로 간다. 같은 메서드를 이 표시 하나로 가른다.
- 알 수 없는 `mode`의 elicitation과 질문이 없는 `AskUserQuestion`은 입력 요청으로 올리지 않는다. 뒤의 것은 허가 요청으로 간다.
- 폼의 `properties`는 순서 없는 객체로 읽는다. 칸 순서는 `required` 목록 순서가 먼저이고 나머지는 이름순이다.
- 폼 칸은 `type`으로 가른다. `string`은 글, `integer`는 정수, `number`는 숫자, `boolean`은 예와 아니오다. `enum`(이름은 `enumNames`)이나 `oneOf`, `anyOf`(`const`와 `title`)가 있으면 단일 선택이고, `array`의 `items`에 선택지가 있으면 다중 선택이다.
- Codex 질문은 `options`가 없으면 글 칸이고, 있으면 단일 선택이다. `isOther`가 참이면 직접 쓰기를 허용하고 `isSecret`이 참이면 입력을 가린다. 선택지의 `label`이 답 글이다.
- Claude는 질문 글이 답의 키라서 칸 `id`도 질문 글이다. `multiSelect`가 참이면 다중 선택이고 여럿을 고른 답은 `, `로 합쳐 한 글로 보낸다. 모든 질문은 목록에 없는 글을 쓸 수 있다.
- 원문 모양은 실험 3의 원자료 기준이다. 폼의 `requestedSchema`와 Claude `AskUserQuestion`의 응답은 그 원문과 응답 실측이 있다. Codex 질문의 응답 모양(`answers`의 값이 `{"answers":[글]}`)은 2026-10-03에 codex-cli 0.158.0에서 실측했다. 글만 담은 응답은 모델이 빈 `answers`로 받았다.

### 공통 입력 요청

`saturn-protocol`의 `InputRequest`는 provider 고유 이름 없이 요청을 나타낸다. 알림 `InputRequested`와 `InputResolved`, 요청 `AnswerInput`, 이벤트 `ProviderEvent::InputRequested`가 쓴다. 형식은 `saturn-protocol/generated`에 있다.

| 타입 | 값 |
|---|---|
| `InputRequest` | `message`, 칸 목록 `fields`, 링크 `url`. `url`이 있으면 `fields`는 비어 있다 |
| `InputField` | `id`, `title`, `description`, 종류 `kind`, `is_required`, `is_secret` |
| `InputFieldKind` | `Text`, `Integer`, `Number`, `Boolean`, `Choice`, `MultiChoice`. 선택은 선택지 목록과 `allows_other`를 가진다 |
| `InputAnswer` | `Submit`(칸 `id`별 값), `Decline`, `Cancel` |
| `InputValue` | `Text`, `Integer`, `Number`, `Boolean`, `Selected`(고른 값 목록) |

provider 고유 이름(`mcpServer/elicitation/request`, `AskUserQuestion` 같은 값)은 `providers/codex`와 `providers/claude` 안에서만 쓴다. 답은 칸 `id`별로 provider 어댑터가 자기 형식으로 바꾼다. 선택 칸의 값은 선택지의 `value`이고, 직접 쓴 글은 그대로 `Selected`에 든다. 비워 둔 선택 칸은 `Submit`에 싣지 않는다.

### 흐름과 대기

1. provider가 입력 요청을 보낸다. 어댑터가 요청의 번호를 기억하고 `InputRequested` 이벤트로 올린다.
2. `engine`이 이벤트를 기록한 뒤 TUI에 `InputRequested`를 보내고, 작업 상태를 `AwaitingInput`(`입력 기다림`)으로 바꾼다. 경과 시간이 멈춘다.
3. 사용자가 답하면 TUI가 `AnswerInput`을 보낸다. `engine`이 요청 번호로 찾은 요청의 어댑터에 답을 넘긴다.
4. 어댑터가 provider 형식으로 응답하면 `engine`이 다른 TUI의 창을 지우고 작업을 `실행 중`으로 되돌린다.

- 입력 대기는 허가 대기(`AwaitingPermission`, `허가 기다림`)와 다른 작업 상태이고 영어는 `waiting for input`이다. 사용자가 무엇을 기다리는지 틀리게 알지 않게 하기 위해서다. 허가 요청과 입력 요청이 함께 남아 있으면 허가 요청이 먼저 보이고, 하나에 답하면 남은 쪽 상태로 바뀐다.
- 요청은 TUI가 붙어 있지 않아도 답이 올 때까지 보관하고, 나중에 붙는 TUI가 허가 요청과 같은 순서로 받는다([engine 수명과 복구](engine-lifecycle.md)).
- 허가 요청과 입력 요청이 함께 오면 TUI는 허가 요청 창을 먼저 보인다.
- 답이 오기 전에 턴이 끝나거나 흐름이 끊기면 그 요청의 창을 모든 TUI에서 지운다.
- 묻지 않은 요청이나 이미 답한 요청의 답은 provider에 보내지 않고 거절한다. 어댑터가 쓰기에 실패하면 요청을 그대로 두어 다시 답할 수 있다.
- 허가 요청의 답으로 입력 요청을, 입력 요청의 답으로 허가 요청을 닫지 않는다. 어댑터가 요청 종류를 확인한다.
- Codex 질문의 `autoResolutionMs`로 provider가 먼저 요청을 거둔 경우는 알림을 받지 못해 턴이 끝날 때까지 창이 남는다(초안).

### Codex 질문 기능

Codex 에이전트 질문(`item/tool/requestUserInput`)은 `default_mode_request_user_input` 실험 기능을 켰을 때만 온다. Saturn은 전용 `CODEX_HOME`의 생성 설정에서 이 기능을 켜지 않는다. 이유는 셋이다. 실험 기능이라 버전마다 동작이 바뀔 수 있고, 켜면 모델이 전에는 진행하던 자리에서 멈춰 묻게 되어 작업 흐름이 바뀐다. provider 설정은 추적만 한다는 원칙([최소 provider 제어](../decisions/2026-09-29-minimal-provider-control.md))과 맞지 않는다. 사용자가 `~/.codex/config.toml`의 `[features]`에서 켠 값은 권한 외 설정이라 생성 설정으로 옮겨지므로, 사용자가 켜면 질문이 전달된다. Saturn 설정 키로 켜는 방법은 정해지지 않았다.

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| Codex 폼 elicitation이 칸 종류별로 공통 요청이 되고, 답이 `accept`와 `content`, `decline`, `cancel`로 같은 번호에 나가며 답이 없는 동안 턴이 멈춰 있다. | `saturn-terminal/engine/src/providers/codex.rs`의 `elicitation_form_round_trip_answers_accept_with_content`, `elicitation_form_round_trip_answers_decline_and_cancel`, `saturn-terminal/engine/src/providers/codex_input.rs`의 `elicitation_form_fields_follow_the_required_order_and_kinds`, `elicitation_form_reads_titled_options`, `elicitation_answers_use_accept_content_decline_and_cancel` |
| Codex URL 모드 요청이 링크만 담은 공통 요청이 되고 `accept`, `decline`, `cancel`로 답한다. | `saturn-terminal/engine/src/providers/codex.rs`의 `elicitation_url_round_trip_answers_accept_decline_and_cancel`, `saturn-terminal/engine/src/providers/codex_input.rs`의 `elicitation_url_mode_carries_the_link_only` |
| Codex 에이전트 질문이 질문마다 칸이 되고 답이 질문 id별 `answers`로 나간다. | `saturn-terminal/engine/src/providers/codex.rs`의 `elicitation_user_input_round_trip_answers_by_question_id`, `saturn-terminal/engine/src/providers/codex_input.rs`의 `elicitation_user_input_questions_become_fields`, `elicitation_user_input_answers_are_keyed_by_question_id` |
| Claude `AskUserQuestion`이 질문마다 칸이 되고 답이 `updatedInput.answers`로, 거절과 취소가 거부로 나간다. | `saturn-terminal/engine/src/providers/claude.rs`의 `elicitation_ask_user_question_round_trip_returns_answers_in_updated_input`, `elicitation_ask_user_question_decline_and_cancel_deny_the_call`, `saturn-terminal/engine/src/providers/claude_input.rs`의 `ask_user_question_becomes_one_choice_field_per_question`, `ask_user_question_answers_are_keyed_by_the_question_text` |
| 입력 요청과 허가 요청이 서로의 답을 받지 않고, 승인 elicitation은 허가 요청으로 남는다. | `saturn-terminal/engine/src/providers/codex.rs`의 `elicitation_input_and_permission_answers_do_not_cross`, `elicitation_approval_is_still_a_permission_request`, `saturn-terminal/engine/src/providers/claude.rs`의 `elicitation_ask_user_question_is_not_answered_as_a_permission` |
| 입력 요청이 TUI에 오르고 답이 provider에 넘어가며, 턴이 끝나면 답 없는 요청이 지워지고, 나중에 붙는 TUI가 받는다. | `saturn-terminal/engine/src/lifecycle/inputs.rs`의 `elicitation_request_reaches_the_tui_and_the_answer_reaches_the_provider`, `elicitation_decline_and_cancel_reach_the_provider_as_given`, `elicitation_answer_for_a_request_nobody_asked_is_refused`, `elicitation_answer_the_provider_did_not_take_keeps_the_request`, `elicitation_turn_end_withdraws_requests_nobody_answered`, `elicitation_request_waits_for_a_tui_that_attaches_later` |
| TUI 입력 창이 칸 종류별 입력, 필수 칸 검사, 거절과 취소를 처리한다. | `saturn-terminal/tui/src/view/input_form.rs`의 `elicitation_form_collects_every_field_kind_into_one_answer`, `elicitation_boolean_takes_space_and_yes_no_keys`, `elicitation_missing_required_field_blocks_the_answer_and_takes_focus`, `elicitation_optional_field_left_empty_is_not_sent`, `elicitation_number_fields_reject_unreadable_values`, `elicitation_other_row_takes_typed_text_and_clears_the_single_pick`, `elicitation_decline_and_cancel_keys_end_the_request`, `elicitation_link_request_accepts_declines_and_cancels` |
| 입력 창이 한국어와 영어로 보이고, 비밀 칸을 가리고, 링크를 보이되 열지 않으며, 뜬 뒤 1초는 키를 받지 않는다. | `saturn-terminal/tui/src/view/input_request.rs`의 `elicitation_window_shows_title_fields_choices_and_keys`, `elicitation_window_texts_follow_the_language`, `elicitation_secret_text_is_masked_on_screen`, `elicitation_url_window_shows_the_link_and_never_opens_it`, `elicitation_keys_are_ignored_during_the_guard_time`, `saturn-terminal/tui/src/app/tests.rs`의 `elicitation_window_takes_typed_text_and_sends_the_answer`, `elicitation_window_takes_the_keyboard_and_clears_when_resolved_elsewhere`, `elicitation_permission_window_comes_before_the_input_window` |

## 단점

- Codex 질문은 사용자가 기능을 켜야 쓸 수 있다.

## 미해결 질문

- Saturn 설정 키로 Codex 질문 기능을 켜게 할지, 기본값을 무엇으로 할지 ([#271](https://github.com/woonyong-choi/saturn/issues/271))
