# 맥락 정리

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [기존 정리 모드 결정](../decisions/2026-10-01-context-mode-setting.md), [원본 인계 구현](https://github.com/woonyong-choi/saturn/issues/613) |

## 요약

Saturn은 채팅 원본을 기록 저장소에 보관하고 Claude와 Codex 사이에 새 session을 열 때 그 기록으로 인계 패킷을 만든다. 같은 provider session의 맥락 정리는 기본적으로 provider에 맡긴다. 전환 패킷의 대화 본문은 줄이지 않으며 도구 결과는 남는 예산에서 최근 것부터 싣고 번호로 다시 읽을 수 있다.

## 동기

최근 몇 턴만 넘기면 앞선 사용자 정정이 빠질 수 있다. 도구 기록만 많이 압축해도 필요한 근거가 남는다는 보장이 없다. Saturn은 먼저 기록의 역할과 순서를 지키고, 보낼 수 없는 길이는 전환 보류로 드러낸다.

## 예시

### Claude에서 Codex로 전환

1. 사용자가 Claude에서 요청을 정정하고 여러 턴 뒤 Codex로 바꾼다.
2. engine은 저장된 사용자 입력, 적용된 끼워 넣기, 에이전트 글을 기록 순서로 모은다.
3. 새 Codex session은 이전 대화를 기록으로 받은 뒤 다음 입력을 별도 턴으로 받는다.
4. 빠진 도구 결과가 필요하면 기록 번호로 원문을 읽는다.

### 대화가 전송 한도를 넘음

1. 보호할 대화만으로 받는 provider의 전송 가능 상한을 넘는다.
2. Saturn은 대화를 잘라 보내지 않고 전환을 보류한다.
3. 입력과 기존 session을 남기고 사용자에게 알린다.

## 상세 설계

### 보존 우선 경로

입력은 먼저 기록 저장소에 접수된다. 패킷은 저장된 사용자 입력, 적용된 끼워 넣기, 에이전트 글을 역할과 기록 번호의 순서로 싣는다. 판단 기록이 없거나 제약 자동 등록이 꺼져 있어도 이 본문을 제외하지 않는다. 완료한 입력은 새 요청으로 취급하지 않도록 상태를 표시한다. 패킷 첫 지시는 과거 기록을 다시 실행하지 말고 다음 입력을 기다리라고 한다.

engine은 패킷의 고정 구역과 실제 발신 글의 해시·앞부분을 전송 전에 맞춘다. 패킷 항목에는 원문 종류, 번호, 해시를 기록한다. 도구 결과는 보호 본문 밖의 경쟁 구역에 두며, 호출과 결과를 한 항목으로 유지한다. 실행 결과가 불명인 호출은 그 상태를 표시한다. 결과 불명 전송은 자동 재시도하지 않는다.

### 정리 모드

| 모드 | 같은 provider session | provider 전환 |
|---|---|---|
| `provider`(기본) | provider의 압축 동작을 따른다 | Saturn 원본 기록으로 패킷을 만든다 |
| `saturn` | 턴 경계에서 새 session으로 옮길지 기존 비용 규칙으로 판정한다 | Saturn 원본 기록으로 패킷을 만든다 |

`provider`는 Saturn의 자동 session 교체와 유휴 복귀 판정을 하지 않는다. `saturn`은 기존 설정과 저장된 설정 번호를 사용하는 명시적 선택으로 남긴다. 전환할 때 떠나는 provider의 요약을 읽을 수 있으면 경쟁 구역 후보로 쓸 수 있지만, 정본인 대화 본문을 대신하지 않는다.

### compaction 판정

`provider` 기본 모드에서는 Saturn이 판정하지 않는다. 명시적으로 `saturn` 모드를 고르면 기록된 활성 맥락과 패킷 크기, 캐시 배수를 사용한다. 트리가 유휴이고 합칠 입력이 없을 때만 턴 경계에서 새 session을 연다. 맥락 크기를 모르면 새 session으로 옮기지 않는다. 이 경로의 순효과는 아직 입증되지 않아 기본값으로 쓰지 않는다.

### 패킷 구성

| 구역 | 재료 | 규칙 |
|---|---|---|
| 고정 | 유효한 명시 제약, 남은 일, 기록된 대화 본문 | 대화 본문은 개수 제한과 요약 없이 순서대로 싣는다 |
| 경쟁 | 수정 파일, 도구 호출과 결과, 읽을 수 있는 provider 요약 | 수정 파일을 앞에 두고 도구 기록은 최근 것부터 예산에 맞춘다 |

경쟁 항목은 원문, 앞부분과 메모, 경로 중 들어가는 형태로 넣는다. 빠지거나 줄어든 도구 기록의 원문은 `saturn evidence read <번호>`로 찾을 수 있다. `context.evidence.lookup`을 켜면 패킷에도 조회 안내를 붙인다. 검색은 같은 채팅의 읽기 권한을 다시 확인한다([근거 검색과 원문 조회](context-selection.md#근거-검색과-원문-조회)).

패킷 목표 예산 `P_max`는 발동 기준의 10분의 1이다. 고정 구역이 이를 넘으면 경쟁 구역을 비우고 전송 가능 상한 `P_send`까지 본문 전체를 싣는다. `P_send`도 넘거나 상한을 구할 수 없으면 전환을 보류한다. provider가 맥락 한도 초과로 확정 거절한 경우에만 경쟁 구역을 줄여 한 번 재전송한다. 고정 구역은 줄이지 않는다.

### 패킷 판단의 적용

패킷을 만들 때 Jev에 도구 후보를 묻는 실험 옵션은 사용하지 않는다. 도구 후보는 최근 기록 순으로 놓는다. 기록 번호가 있는 대화 본문의 원문과 순서는 이 선택의 영향을 받지 않는다. 원래 RRF와 Jev 비교의 결과는 실험 기록에 보존하고 제품 효과로 확대하지 않는다([실과제 진단](../experiments/context-net-effect/report.md)).

### 오류 처리

| 상황 | 동작 |
|---|---|
| 대화 본문이 `P_send`를 넘는 경우 | 본문을 자르지 않고 전환을 보류한다 |
| 발신 본문이 만든 패킷과 다름 | 전송하지 않는다 |
| 확정된 맥락 한도 거절 | 경쟁 구역만 줄여 한 번 다시 보낸다 |
| 보낸 결과가 불명 | 자동 재전송하지 않는다 |
| 근거 조회 범위 밖 또는 해시 불일치 | 원문을 돌려주지 않는다 |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 오래된 정정과 모든 대화 본문이 역할·순서·해시를 유지한다. | `saturn-terminal/engine/src/lifecycle/dialogue_preserved.rs`의 `four_reads_after_a_correction_do_not_push_it_out_of_the_switch_packet` |
| 보낼 글이 고정 본문을 잃으면 전송하지 않는다. | `saturn-terminal/engine/src/handoff.rs`의 `evidence_refuses_a_body_that_lost_or_swapped_recorded_dialogue` |
| 보호 본문이 한도를 넘으면 줄이거나 대체하지 않는다. | `saturn-terminal/engine/src/lifecycle/dialogue_preserved.rs`의 `dialogue_over_the_send_limit_is_not_applied_and_nothing_is_cut` |
| 기본 같은 session은 provider 압축에 맡긴다. | `saturn-terminal/engine/src/settings/mod.rs`의 `context_mode_defaults_to_provider_and_reads_saturn` |
| 전환 패킷은 Jev 호출 없이 도구 기록을 최근순으로 싣는다. | `saturn-terminal/engine/src/handoff.rs`의 `handoff_orders_tool_results_newest_first_without_router_ranking` |

## 단점

긴 대화 본문은 줄이지 않으므로 provider 전송 한도에 닿으면 전환할 수 없다. 도구 기록을 최근순으로 고르는 방식도 필요한 오래된 결과를 자동으로 찾는다는 보장이 없다. 원본 조회와 실제 양방향 전환 품질은 별도로 확인한다.

## 대안

- RRF와 Jev로 도구 결과를 선별하는 이전 패킷 경로는 실과제 효용이 확인되지 않아 제품 경로에서 뺐다([진단](../experiments/context-net-effect/report.md)).
- 실행 중 provider 기록을 직접 고치는 방식은 현재 어댑터 계약에 없다.
