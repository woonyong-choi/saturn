# 요구사항

코딩 에이전트를 여러 개 쓰는 개발자는 에이전트를 바꿀 때마다 맥락이 끊긴다. Saturn은 모든 입력을 로컬 기록에 먼저 접수하고 필요한 맥락만 골라 Codex와 Claude 세션에 넘긴다. 공급자별 세션과 달리 한 채팅 안에서 공급자를 바꿔도 기록과 작업 상태가 이어진다.

## 목표

- 공급자와 무관한 채팅 기록 보존
- 전달 여부가 기록으로 남는 입력 처리
- 공급자 기본 압축보다 적은 토큰의 맥락 유지
- 판단 기록으로 학습한 로컬 판단 모델

## 비목표

- 공급자 설정과 하위 에이전트 사용 제어
- 공급자 자체 기록의 정리와 삭제
- 판단기 지출 상한 관리
- 에이전트 사이 직접 통신

## 사용자

| 사용자 | 쓰는 상황 |
|---|---|
| 개인 개발자 | Codex와 Claude Code를 함께 쓰며 한 저장소에서 여러 작업 진행 |
| 스크립트와 CI 사용자 | 화면 없이 파이프로 입력 전송과 결과 수신 |

## 기능

| 기능 | 하는 일 |
|---|---|
| 입력 접수 | 모든 입력을 로컬 기록에 접수한 뒤 에이전트로 전송 |
| 실행 중 입력 처리 | 실행 중 새 입력을 끼워 넣기, 새 작업, 대기 중 하나로 처리 |
| 대기와 보류 | 대기 입력의 전송과 취소, 멈춘 작업의 보류와 재개 |
| 공급자 전환 | 한 채팅 안에서 Codex와 Claude 세션을 바꿔 가며 작업 |
| 맥락 관리 | 턴마다 맥락 크기 기록, 기준 도달 때 정리 뒤 새 세션 |
| 하위 에이전트 추적 | 공급자 하위 에이전트의 시작, 끝, 사용량 기록 |
| 판단기 | 입력마다 이어 가기, 새 작업, 모델을 확률로 판단 |
| 판단 모델 학습 | 판단 기록 채점, 로컬 판단 모델 학습과 승격 |
| 전체 화면 | 채팅 기록, 실행 영역, 상태판, 입력창, 슬래시 메뉴 |
| 데이터 관리 | 사용량 조회, 기록 정리와 삭제, 판단 기록 내보내기 |

## 기능 요구사항

| 기능 | 요구사항 | 확인 방법 |
|---|---|---|
| 입력 접수 | 입력은 기록 저장소에 접수된 뒤에만 에이전트로 보낸다. | `saturn-terminal/core/tests/queue.rs`의 `acks_before_dispatch` |
| 입력 접수 | 보낸 뒤 결과가 불명인 입력은 자동으로 다시 보내지 않는다. | `saturn-terminal/core/tests/queue.rs`의 `unknown_result_is_not_resent` |
| 실행 중 입력 처리 | 같은 채팅의 입력은 접수 순서대로 하나씩 판단한다. | `saturn-terminal/core/tests/queue.rs`의 `judges_inputs_in_ack_order` |
| 실행 중 입력 처리 | 판단 뒤 채팅 상태가 바뀌었으면 한 번 다시 판단하고, 또 바뀌면 대기로 둔다. | `saturn-terminal/core/tests/queue.rs`의 `revision_conflict_falls_back_to_queue` |
| 실행 중 입력 처리 | 쓰기 권한 에이전트는 같은 작업 폴더에서 한 번에 하나만 실행한다. | `saturn-terminal/core/tests/queue.rs`의 `single_writer_by_default` |
| 대기와 보류 | 취소는 에이전트에 보내기 전 입력에만 적용한다. | `saturn-terminal/core/tests/queue.rs`의 `cancel_only_before_send` |
| 대기와 보류 | 멈춘 작업은 자동으로 이어 가지 않고 보류한다. | `saturn-terminal/core/tests/queue.rs`의 `stopped_task_is_held` |
| 공급자 전환 | 채팅마다 살아 있는 세션은 하나다. | `saturn-terminal/engine/tests/store.rs`의 `one_open_session_per_chat` |
| 공급자 전환 | 세션 교체 뒤 새 세션에는 받지 않은 기록 번호 뒤의 변경분만 넘긴다. | `saturn-terminal/core/tests/sessions.rs`의 `delivers_changes_after_ledger_seq` |
| 맥락 관리 | 맥락 정리는 트리 유휴이고 합칠 대기 입력이 없을 때만 한다. | `saturn-terminal/core/tests/sessions.rs`의 `compacts_only_when_tree_idle` |
| 맥락 관리 | 패킷 크기는 정리 기준의 10분의 1을 넘지 않는다. | `saturn-terminal/core/tests/sessions.rs`의 `packet_within_budget` |
| 하위 에이전트 추적 | 작업 끝은 메인 에이전트와 모든 하위 에이전트가 끝난 때로 판정한다. | `saturn-terminal/core/tests/agents.rs`의 `task_ends_on_tree_idle` |
| 하위 에이전트 추적 | 멈춤 신호는 추적된 하위 에이전트까지 보낸다. | `saturn-terminal/engine/tests/processes.rs`의 `stop_reaches_whole_tree` |
| 하위 에이전트 추적 | 멈춤 신호 10초 뒤 남은 프로세스 묶음에는 중지 신호를 보낸다. | `saturn-terminal/engine/tests/processes.rs`의 `escalates_after_ten_seconds` |
| 판단기 | 판단기 확인에 실패하면 Saturn을 실행하지 않는다. | `saturn-terminal/engine/tests/judges.rs`의 `refuses_to_start_without_judge` |
| 판단기 | 실행 중 판단기 호출이 실패하면 질문별 대체 규칙을 적용한다. | `saturn-terminal/core/tests/judges.rs`의 `fallback_on_judge_failure` |
| 판단기 | 판단 호출의 보낸 원문, 받은 원문, 질문별 답을 모두 기록한다. | `saturn-terminal/engine/tests/store.rs`의 `records_every_judge_call` |
| 판단 모델 학습 | 새 판단 모델은 같은 평가 세트에서 현재 모델보다 나쁘지 않을 때만 승격한다. | [판단 모델 후보별 정확도와 지연 측정](https://github.com/woonyong-choi/saturn/issues/11) |
| 전체 화면 | 작업이 하나이고 대기와 보류가 없으면 이름표를 숨긴다. | `saturn-terminal/tui/tests/render.rs`의 `hides_tag_for_single_task` |
| 전체 화면 | 화면을 닫아도 엔진은 접수된 입력을 계속 처리한다. | `saturn-terminal/engine/tests/rpc.rs`의 `continues_after_screen_exit` |
| 데이터 관리 | 열린 입력, 열린 실행, 활성 세션은 어떤 명령으로도 지우지 않는다. | `saturn-terminal/engine/tests/store.rs`의 `refuses_to_prune_open_items` |
| 데이터 관리 | 정리 명령은 `--yes` 없이는 미리보기만 한다. | `saturn-terminal/engine/tests/store.rs`의 `prune_previews_without_yes` |
| 데이터 관리 | 채점하지 않아도 판단 기록을 JSONL로 내보낼 수 있다. | `saturn-terminal/engine/tests/store.rs`의 `exports_judge_records_without_labels` |

## 품질 요구사항

| 항목 | 요구사항 | 확인 방법 |
|---|---|---|
| 신뢰성 | 크래시 뒤 자동 재개는 효과 범위가 설정이나 관찰로 증명된 실행에만 한다. | [공급자 적용 설정의 보고 범위 측정](https://github.com/woonyong-choi/saturn/issues/4) |
| 신뢰성 | 맥락 정리 방식은 공급자 기본 압축보다 품질을 낮추지 않는다. | [맥락 정리 기준과 방식별 토큰 측정](https://github.com/woonyong-choi/saturn/issues/7) |
| 보안 | 공급자 자식 프로세스 환경에는 판단기 키 변수가 없다. | `saturn-terminal/engine/tests/secrets.rs`의 `child_env_has_no_judge_key` |
| 보안 | 판단기 키는 기록 저장소, 로그, 오류 출력에 남지 않는다. | `saturn-terminal/engine/tests/secrets.rs`의 `masks_key_in_outputs` |
| 호환성 | 스키마를 올리기 전 백업 하나를 남긴다. | `saturn-terminal/engine/tests/store.rs`의 `migrates_forward_with_backup` |
| 사용성 | 화면 문구는 영어와 한국어를 운영체제 언어로 고른다. | `saturn-terminal/tui/tests/i18n.rs`의 `selects_language_from_locale` |
| 이식성 | 화면이 없는 파이프와 CI에서도 같은 명령이 같은 결과를 낸다. | `saturn-terminal/tui/tests/render.rs`의 `plain_and_full_screen_match` |

## 제약

- macOS, Apple Silicon
- Rust 2024 edition
- Codex CLI나 Claude Code 중 하나 이상 설치
- 판단기 API 키나 로컬 판단 모델
