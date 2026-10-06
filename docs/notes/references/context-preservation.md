---
type: reference
date: 2026-10-06
tags: [context, memory, compaction]
---

# 맥락 보존 방식 비교

Saturn의 최소 변경은 사용자·assistant 대화 본문을 보호하고 도구 기록만 선별하는 경로다. 기억 검색, 유효 지시 이력, 같은 session 편집은 그 경로의 결과를 확인한 뒤 독립적으로 더한다. 전체 범위는 설계의 상한이며 한 번에 구현할 목록이 아니다. 제품 동작의 정본은 [맥락 정리](../../design/context-management.md#보존-우선-경로), 검증 계획은 [보존 우선 비교](../../experiments/context-preservation/design.md)다.

## 내용

### 현재 구현과 실패 범위

비교 기준은 main `58c039bab90133a6c6a9ffab223454871f6819ec`다. 실험은 다른 제품 버전 `7413264`, `7d099e6`에서 수행됐다. 현재 코드를 읽은 것과 새 버전을 실제로 시험한 것을 구분한다.

| 경로 | 확인한 구현 | 남은 문제 |
|---|---|---|
| 입력과 원문 | `engine/src/store/ledger.rs`가 입력·실행·판단 기록 조회 | 저장했다는 사실만으로 다음 session에 전달 보장 없음 |
| 수정 보존 | `engine/src/handoff.rs::explicit_amendments`가 판단된 수정 중 작업당 최근 3개 선택 | 높은 제약 점수를 받은 읽기 요청이 실제 정정을 밀어냄 |
| 패킷 예산 | `core/src/sessions/packet.rs::fit_fixed_zone`가 답·최근 턴·수정을 줄임 | 고정 구역도 원문 보존 보장 없음 |
| Jev 선별 | `engine/src/packet_select.rs`의 비동기 호출과 적용 전 키 대조 | 연결 구현 확인. 보존 정책 변경과 실제 성과 검증이 남은 범위 |
| 원문 조회 | `engine/src/evidence.rs`가 같은 채팅의 도구 기록 검색·읽기 | 사용자 입력은 조회 대상 제외. 안내만으로 조회가 실행 보장 없음 |
| 제약 | `store/constraints.rs`, `constraint_change.rs`에 상태·이벤트·revision·해제·예외 구현 | 자동 의미 판정의 품질과 새 대체 관계를 기존 구조에 연결하는 일이 남은 범위 |
| 완료 근거 | `run_changes.rs::settle_completion`과 기존 검사 기록 | 검사 통과가 과제의 정답까지 보장 없음 |

경로의 `engine/src`는 `saturn-terminal/engine/src`, `core/src`는 `saturn-terminal/core/src`를 뜻한다. 최신 구현에 없는 추상화를 현재 기능으로 표시하지 않는다.

[복구 실험 보고서](https://github.com/woonyong-choi/saturn/blob/bc29075/docs/experiments/context-recovery-effect/report.md)는 기준 조건의 전체 성공 7/8, 패킷 조건의 전체 성공 0/8을 기록한다. 0/8은 세 후속 작업을 모두 통과한 실행이 없다는 뜻이며 모든 질문을 틀렸다는 뜻이 아니다. 실제 전달 패킷 48개에는 정정 헤더가 없었다. 이 결과는 보존·복구를 포함한 제품 경로의 실패이며 Jev 모델만 분리한 평가가 아니다. 보강 진단 8회는 입력 분할로 무효이고 기본 압축 비용은 결측이다. 이 실험의 수치를 새 설계의 결과로 재사용하지 않는다.

### 외부 프로젝트와의 비교

| 대상 | 저장 | 전달·검색 | 판단 주체 | Saturn에 적용할 원리 | 비용과 한계 |
|---|---|---|---|---|---|
| fast-jev-compaction | 원래 메시지를 바탕으로 처리 | 사용자·assistant 텍스트를 유지하고 도구 호출·결과만 제거·축약 | Jev가 도구 보존을 판단, 코드가 메시지 짝과 고정 항목 보호 | 대화 본문 보호와 도구 후보 분리 | Jev 입력에서는 본문이 줄어들 수 있고 결과 전문은 생략. 모든 근거의 보존 보장 없음 |
| Mem0 | 기억 추출 결과 저장 | 질문으로 검색한 기억을 답변 전에 전달 | 추출 LLM과 검색 계층 | engine이 관련 기록을 먼저 검색·주입 | 추출·검색 비용 발생. 관리형 벤치마크를 오픈소스 또는 Saturn 성과로 대입 불가 |
| Letta | 핵심 기억과 외부 기억 구분 | 핵심 블록은 항상 맥락에 포함, 외부 기억은 도구로 검색 | 에이전트의 기억 수정과 검색 | 필수 지시와 경쟁 후보의 예산 분리 | 블록 내용의 정확성은 별도 문제. 자동 자기 수정은 도입 대상 제외 |
| Graphiti | 원자료와 시간 유효성을 가진 관계 저장 | 현재·과거 사실을 시간과 관계로 검색 | LLM 추출과 시간·그래프 관리 | 원문 근거와 대체·철회 이력 | 그래프 DB·추출 파이프라인 전체 도입은 최소 범위를 초과 |
| Saturn 기준 구현 | 입력·이벤트·판단을 같은 SQLite에 보존 | 일부 대화와 선별 도구 기록으로 새 session 구성 | RRF·Jev와 core 예산 규칙 | 기존 저장소·권한·revision·패킷 기록 재사용 | 사용자 정정 보존과 조회 범위의 결함 확인 |

### 처리 경로 비교

#### 현재 Saturn

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/context-preservation-current.ko.dark.svg">
  <img src="../../assets/context-preservation-current.ko.light.svg" alt="현재 Saturn: 기준 코드 58c039b. 일부 대화도 예산에서 탈락한다." width="100%">
</picture>

#### fast-jev-compaction

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/context-preservation-fast.ko.dark.svg">
  <img src="../../assets/context-preservation-fast.ko.light.svg" alt="fast-jev-compaction: 문서·코드 비교. Saturn과 동일 조건의 실행 결과가 아니다." width="100%">
</picture>

#### Mem0

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/context-preservation-mem0.ko.dark.svg">
  <img src="../../assets/context-preservation-mem0.ko.light.svg" alt="Mem0: 추출과 검색을 구분한다. 관리형과 OSS의 세부 구현은 다르다." width="100%">
</picture>

#### Letta

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/context-preservation-letta.ko.dark.svg">
  <img src="../../assets/context-preservation-letta.ko.light.svg" alt="Letta: 핵심 기억과 외부 기억의 접근 방식이 다르다." width="100%">
</picture>

#### Graphiti

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/context-preservation-graphiti.ko.dark.svg">
  <img src="../../assets/context-preservation-graphiti.ko.light.svg" alt="Graphiti: 관계와 유효 시간을 저장한다. 그래프 DB 전체 도입은 제외한다." width="100%">
</picture>

#### 제안 Saturn

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/context-preservation-proposed.ko.dark.svg">
  <img src="../../assets/context-preservation-proposed.ko.light.svg" alt="제안 Saturn: 설계·모형. 본문 보존과 도구 선별을 기존 경로에서 분리한다." width="100%">
</picture>

#### 최소 검증 순서

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../../assets/context-preservation-workflow.ko.dark.svg">
  <img src="../../assets/context-preservation-workflow.ko.light.svg" alt="최소 검증 순서: 설계·모형. 진단 통과는 제품 효과 입증과 다르다." width="100%">
</picture>

그림은 코드와 문서를 읽어 정리한 흐름이다. 외부 프로젝트를 같은 과제로 실행한 결과가 아니다. 제안 경로는 모형으로 표시한다. 인증·사용량·실패의 상세 분기는 [맥락 정리](../../design/context-management.md#보존-우선-경로)의 계약을 따른다.

### 전체 범위와 최소 범위

| 단계 | 사용자 결과 | 재사용할 것 | 새로 필요한 것 | 선행 조건 |
|---|---|---|---|---|
| 최소 보존 | 정정이 로그 읽기에 밀리지 않고 두 provider의 새 session에 전달됨 | ledger, handoff, packet, packet_select, 전달 해시 | 대화 본문 보호·명시적 예산 초과·동일 도구 후보 비교 | #592, #380 |
| 최소 검증 | 보존 효과와 Jev 추가 효과를 분리해 판정 | #540 실행기, 기존 원자료·채점기 | 보호 본문 해시·실제 개입·비용 결측 검증 | #7, #544 |
| 기억 복구 | 사용자 입력을 포함한 과거 근거를 현재 요청 전에 전달 | evidence, RRF, 출입증·권한·조회 기록 | 종류가 있는 원문 참조와 선주입 | [#599](https://github.com/woonyong-choi/saturn/issues/599), [#600](https://github.com/woonyong-choi/saturn/issues/600) |
| 유효 지시 | 현재 지시와 이전 지시를 원문으로 추적 | constraints와 변경 이벤트 | 범위·대체 근거·충돌 상태, 선택적 작업 LLM 제안 | [#601](https://github.com/woonyong-choi/saturn/issues/601), #382 |
| 같은 session 축약 | 지원 provider에서 session을 유지하며 도구 기록 축약 | 공통 선별 결과·원문 저장 | 능력 확인과 provider별 적용 어댑터 | [#602](https://github.com/woonyong-choi/saturn/issues/602), 실제 지원 확인 |

장기기억 전체를 구현해야 첫 검증을 할 수 있는 구조를 만들지 않는다. 모델·추론 선택 #531/#542, 입력 관계 #9, 사례집 #543, 보조 판단 #546은 맥락 보존과 별도 효과 실험으로 유지한다. Kompress, 새 그래프 DB, 별도 기억 서버, 로컬 압축 모델, 실행 중 정책·스킬 자동 변경은 이 범위에 넣지 않는다.

### 최소 작업 인수

1. 구현자는 #536에서 현재 실행 순서를 확인한다.
2. #592에서 대화 본문 보존과 예산 초과를 고친다. 최근 수정 개수나 제약 임계값만 조정하지 않는다.
3. #380에서 기존 RRF·Jev 호출을 같은 도구 후보와 예산에 연결한다. 새 selector 프레임워크를 만들지 않는다.
4. #540에서 실제 전달 본문, 미적용, cache·판단·재조회·재작업을 기존 trial에 연결한다.
5. #7에서 기능 진단을 수행하고 성공한 경우만 독립 확인 평가로 넘긴다. #544는 같은 요청의 원응답과 최종 행동 일관성을 따로 판정한다.
6. 검색·유효 지시·같은 session 편집은 각각 추가 전후 비교로 인수한다. 첫 비교에 모두 켜지 않는다.

### 이슈 전수 대조 범위

2026-10-06의 이슈 302건 중 열린 46건의 본문·댓글·완료 조건을 검토했다. 닫힌 256건은 본문·상태·코드 링크·남은 주석을 대조했다. 닫힌 이슈 전체의 실제 provider 실행을 반복한 것은 아니다. 닫힌 #33, #56, #168을 가리키는 남은 주석은 #74가 재분류한다. #525, #438, #423의 종료·등록 순서·권한 회귀는 #464가 유지한다.

이슈의 작업 순서와 완료 상태는 GitHub가 정본이다. 이 문서에 이슈 본문을 복제하지 않는다. 이전 실험의 실패 결과는 보존하고 새 범위의 성공 조건으로 바꿔 쓰지 않는다.

## 출처

| 출처 | 버전 | 확인한 날짜 |
|---|---|---|
| [fast-jev-compaction](https://github.com/tamaratran/fast-jev-compaction/tree/e3f262a7f4d42bd8dd32ced30d26176f7cb545b0) | e3f262a | 2026-10-06 |
| [Mem0](https://github.com/mem0ai/mem0/tree/c93420c49a6b14c3d446bdb156d96811908fd90a) | c93420c | 2026-10-06 |
| [Letta 기억 구조](https://github.com/letta-ai/skills/blob/6785511e7d3913d9ec598435a77ae637cd45fc89/letta/agent-development/references/memory-architecture.md) | 6785511 | 2026-10-06 |
| [Graphiti](https://github.com/getzep/graphiti/tree/2a85bbbf27f3d0d07dd3a8bf6dc8700c5193c066) | 2a85bbb | 2026-10-06 |
| [Saturn 구현](https://github.com/woonyong-choi/saturn/tree/58c039bab90133a6c6a9ffab223454871f6819ec) | 58c039b | 2026-10-06 |

## 확인하지 않은 것

- 제안 경로의 실제 성공률·비용·시간
- 각 provider에서 같은 session 기록을 안전하게 교체할 수 있는 범위
- 원문 전체를 남길 수 없는 길이에서 유효 지시 추출이 제공하는 순이득
- 외부 프로젝트와 Saturn을 같은 작업으로 실행한 직접 비교
