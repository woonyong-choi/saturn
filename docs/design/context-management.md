# 맥락 정리

| 항목 | 값 |
|---|---|
| 상태 | 제안 |
| 관련 결정 | [compaction 기준을 절대 토큰 예산으로 정한다](../decisions/2026-09-29-absolute-token-budget.md), [provider마다 입력을 계속 받는 상시 연결을 만든다](../decisions/2026-09-29-persistent-provider-connections.md), [provider 설정과 subagent 사용은 사용자 설정을 따르고 Saturn은 추적만 한다](../decisions/2026-09-29-minimal-provider-control.md), [맥락 정리는 Saturn 방식을 기본으로 두고 provider 압축 모드를 설정으로 연다](../decisions/2026-10-01-context-mode-setting.md), [경쟁 구역은 기준값 없이 router 남김 확률 순으로 예산까지 채운다](../decisions/2026-10-02-fill-packet-by-probability.md) |

## 요약

맥락 정리(compaction)는 session의 맥락이 커졌을 때 줄여서 이어 가는 기능이다. engine은 턴이 끝날 때마다 맥락 크기를 기록하고, `core`의 `sessions`가 compaction 여부를 판정한다. compaction은 provider 압축을 요청하거나, Saturn 기록 원문에서 고른 패킷으로 새 session을 여는 두 방식이 있다. 그 사이 한 턴이 갑자기 커지는 경우는 provider 자동 압축이 안전망으로 받는다. 사용자는 정리 모드 설정으로 맥락 정리를 provider 자동 압축에 맡길 수 있고, 이때도 provider 전환의 전달은 Saturn이 맡는다.

## 동기

session이 길어지면 맥락이 쌓여 토큰이 늘고 답의 품질이 떨어진다. 품질 저하는 창 크기 대비 비율이 아니라 맥락의 절대 길이를 따른다. provider 기본 압축에 맡기면 창이 큰 provider에서 맥락이 오래 쌓이고, 원격 압축 요약은 암호화되어 무엇이 남았는지 볼 수 없다. Saturn은 provider 기본 압축보다 적은 토큰으로 맥락을 유지하는 것을 목표로 한다. 이 기능이 없으면 사용자는 커진 맥락의 비용과 품질 저하를 그대로 떠안는다.

## 예시

### 맥락이 커져 새 session으로 이어 가기

1. Claude 메인 에이전트는 한 채팅에서 긴 작업을 여러 턴 이어 간다.
2. 턴이 끝날 때마다 engine은 활성 맥락 크기를 기록한다.
3. 활성 맥락 크기가 발동 기준에 닿았고, 트리 유휴이며 합칠 대기 입력이 없다.
4. 본전 턴 수가 기대 잔여 턴 이하이므로 `sessions`는 새 session으로 이어 가기로 판정한다.
5. `sessions`는 Saturn 기록 원문에서 패킷을 만들고, engine은 새 session에 패킷을 넘긴다.
6. engine은 새 session이 열리면 옛 session을 닫고 채팅의 session 기록을 새 session으로 바꾼다. 새 session을 여는 동안 그 채팅의 다음 입력은 대기열에서 기다리고, 다른 채팅과 멈춤은 기다리지 않는다([provider 요청 작업](providers-and-sessions.md#provider-요청-작업)).
7. TUI는 `맥락 정리 후 이어서 진행` 한 줄을 보인다.

### subagent가 도는 동안 미루기

1. 메인 에이전트는 답을 냈지만 subagent 하나가 아직 돈다.
2. 활성 맥락 크기가 발동 기준을 넘었어도 트리 유휴가 아니므로 `sessions`는 판정을 다음 턴 경계로 미룬다.
3. 다음 턴이 갑자기 커져 안전망 기준을 넘으면 provider 자동 압축이 맥락을 줄인다.
4. subagent까지 끝나 트리 유휴가 되면 `sessions`는 다음 턴 경계에서 다시 판정한다.

### 오래 쉬었다 돌아오기

1. 사용자는 작업을 멈추고 캐시 유지 시간보다 오래 자리를 비운다.
2. 사용자가 돌아와 새 입력을 보낸다.
3. 패킷이 활성 맥락보다 작으므로 `sessions`는 새 session으로 이어 가기로 판정한다.

### provider 모드에서 Claude 요약으로 Codex에 넘기기

1. 사용자는 정리 모드를 `provider`로 두고 Claude로 작업한다.
2. 맥락이 커지자 Claude 자동 압축이 맥락을 줄이고, engine은 압축 요약과 그 시점의 기록 번호를 기록한다.
3. 사용자가 Codex로 바꾸면 `sessions`는 고정 구역, Claude 압축 요약, 요약 시점 뒤 기록에서 고른 항목으로 패킷을 만든다.
4. 반대로 Codex에서 Claude로 바꿀 때는 Codex 요약을 볼 수 없으므로 Saturn 기록 원문으로 패킷을 만든다.

## 상세 설계

### 보존 우선 경로

보존 우선 경로는 같은 채팅의 기록된 사용자·assistant 대화 본문을 순서대로 보호하고, 도구 기록만 RRF 또는 Jev로 선별한다. 이 절의 대화 본문 보호는 [#592](https://github.com/woonyong-choi/saturn/issues/592)로 [패킷 구성](#패킷-구성)에 구현했고, 도구 선별을 Jev로 바꾸는 부분은 [#380](https://github.com/woonyong-choi/saturn/issues/380)의 계약이다. 최근 턴·수정 개수 제한과 본문 축약은 없다. 같은 책임의 이전 조립 경로는 남기지 않는다. 일반 제품 채택은 [검증](../experiments/context-preservation/design.md) 전에 판정하지 않는다. 외부 방식 비교와 도입 상한은 [맥락 보존 방식 비교](../notes/references/context-preservation.md)에 있다.

1. engine이 사용자 입력을 저장하고 적용된 입력·답·도구 기록을 같은 채팅의 순서로 읽는다. 미전송 대기 입력은 실행된 대화와 분리한다.
2. core가 대화 본문과 필수 실행 상태를 보호 집합으로 분리한다. 제약 판단 기록이 없거나 `constraint.auto_apply`가 꺼져 있어도 사용자 본문을 보존한다. provider가 한 답을 여러 `Text` 이벤트로 나눠 내면 이벤트마다 번호·역할·순서·원문 해시를 따로 남기고 합치지 않는다. 패킷 글에서는 앞 조각에 바로 이어 붙여 `Agent:` 표지를 첫 조각에만 쓰므로 읽기 모양은 한 답이다.
3. 기존 RRF 또는 Jev가 도구 호출·결과 후보만 선별한다. 도구 호출과 결과의 짝을 유지하고 결과 없는 호출은 실행 결과 불명으로 보존한다.
4. core가 패킷을 만들 때 고정 구역(제약 칸, 남은 일, 대화 본문)을 한 번 직렬화해 두고, engine은 보내기 직전 보낼 글이 만든 패킷과 같은 글(해시 일치)이고 그 고정 구역으로 글자 그대로 시작하는지 검사한다. 구역 제목을 글에서 찾지 않고 앞에서부터 맞추므로 본문 안의 제목 모양 글이나 다른 구역에 있는 같은 글에 속지 않고, 본문이 빠지거나 바뀌거나 순서가 달라지거나 경쟁 구역으로 옮겨져도 걸러진다. 실패한 패킷은 보내지 않는다.
5. 적용 직전 입력 revision, 설정 번호, 정책 버전, 보내는 session과 받는 provider를 다시 대조한다. 이미 구현된 비동기 판단과 늦은 응답 폐기 경로를 재사용한다.
6. provider 어댑터가 패킷을 보내고 실제 적용 방식·포함과 생략·원문 참조·전송 상태를 기존 기록에 남긴다. 결과 불명 전송은 재시도하지 않는다.

보호 대상은 외부로 출력된 대화 본문이다. 숨긴 추론, provider 비밀 설정, 인증 정보는 포함하지 않는다. 현재 권한이 읽기를 금지한 자료와 router 키는 별도 보안 규칙이 우선하며 차단 사실을 남긴다. 동일한 문장의 반복 입력을 문자열 중복으로 지우지 않는다. 원래 입력·메시지 식별자로만 중복 전달을 막고 끼워 넣은 입력도 실제 적용 순서를 유지한다. assistant 본문을 보존하는 것이 그 안의 주장을 사용자 지시나 검증 사실로 승격시키지는 않는다.

제품의 정리 모드를 새로 추가하지 않는다. 보존 우선 계약은 기존 Saturn 패킷 구성의 교체 후보이며, 비교에 필요한 선택지는 실험 경로에 둔다. 채택 뒤 같은 책임의 이전 조립 경로를 상시 유지하지 않는다.

공통 계획은 기존 `PacketSource`와 패킷 항목 기록을 확장해 표현한다. 필요한 값은 보호 원문 참조, 도구 항목별 전문·발췌·생략, 적용 경로, 정책 버전, 예산 판정이다. core는 이 값을 계산하고 engine은 기록 조회·Jev 호출·전송을 맡는다. 새 기억 manager, 별도 DB, 동일 책임의 selector 계층은 만들지 않는다. 공개 타입 이름과 필드는 구현 PR에서 실제 소비자를 확인한 뒤 확정한다.

#### 예산 초과

본문을 보호한다는 이유로 provider 입력 한도를 무시하지 않는다. 전송 가능 상한 `P_send`는 provider 창에서 시스템·도구 정의·현재 입력·출력 예약량을 먼저 빼고 안전 비율을 곱해 정한다([패킷 구성](#패킷-구성)). 기존 패킷 목표 예산 `P_max`는 줄일 도구 기록의 양을 정하는 값으로만 쓴다. 보호 본문이 `P_max`를 넘으면 도구 기록을 모두 빼고 `P_send`까지 본문을 그대로 보내며 `P_max`를 넘겼다는 사실을 기록한다. 본문이 `P_send`도 넘거나 창을 알 수 없어 `P_send`를 구할 수 없으면 `보존 예산 초과`로 적용하지 않는다. `P_send`는 추정이라 통과해도 provider가 거절할 수 있다. 추정 크기는 패킷 기록의 크기 필드로, 실제 거절은 전송 상태로 따로 남는다.

자동 정리에서는 보낼 수 있는 현재 session을 유지한다. provider 전환을 완료할 수 없으면 기존 session과 입력을 보존하고 전환 미완료를 알린다. 본문을 몰래 자르거나 기본 압축으로 바꾼 결과를 Saturn 성공으로 집계하지 않는다. provider가 자체 압축한 실행은 별도 조건으로 센다. 한도 밖 장기 대화는 후속 기억 설계의 과제이며 최소 보존 실험의 성공으로 대신하지 않는다.

#### 같은 session과 새 session

최소 구현은 두 provider의 새 session 전달과 양방향 전환을 사용한다. 같은 session 내부 기록 편집은 지원 기능을 실제 확인한 어댑터에만 연결한다. 선별 계획은 공유하되 provider 기록 파일을 engine 공통 코드에서 직접 수정하지 않는다. `same_session`, `new_session`, `provider_native`, `not_applied`를 실행 기록에서 구분한다. 여기의 표기는 설계상 의미이며 아직 추가된 설정 키나 프로토콜 값이 아니다.

#### 단계별 기억 확장

| 단계 | 추가하는 동작 | 공통 계약 |
|---|---|---|
| 사용자 원문 조회 | 도구 기록 외에 사용자 입력과 출력 본문을 조회 | 같은 채팅·작업 범위, 종류와 번호·해시가 있는 원문 참조, 현재 권한 재검사 |
| 답변 전 검색 | 새 session의 첫 작업 입력 전에 관련 과거 기록을 선주입 | 같은 source 목록과 RRF 재사용, 보호 본문과 중복 제거, 검색·선택·주입을 별도 기록 |
| 유효 지시와 이력 | 현재 지시·대체·철회·작업 한정 예외를 원문으로 연결 | 기존 constraints와 변경 이벤트 확장, 자동 의미 해석은 독립 검증 전 정본 변경 금지 |
| 작업 LLM 제안 | 현재 LLM이 이미 읽은 입력에서 상태 변경과 근거 번호를 제안 | 현재 revision·출처 검사, 추가 토큰 계측, 애매한 관계는 원문 양쪽 유지 |

정책·스키마·프롬프트는 배포 버전으로 고정한다. 기록된 작업 상태만 갱신한다. 모델이 스킬을 생성하거나 판단 기준을 스스로 바꾸는 경로는 추가하지 않는다. 사용자가 지시를 반복하거나 수동으로 기억을 등록하는 일을 정상 사용의 성공 조건으로 삼지 않는다.

최소 검증은 [보존 우선 비교](../experiments/context-preservation/design.md)를 따른다. 기능 진단 통과, 제한된 실험의 품질 유지, 일반 제품 채택을 서로 다른 판정으로 둔다.

### 기호

이 문서는 다음 기호를 쓴다. 각 기호는 아래 절에서 처음 나올 때 다시 설명한다.

| 기호 | 뜻 |
|---|---|
| `A` | 활성 맥락 크기 |
| `T` | compaction 발동 기준 |
| `T_abs` | provider별 절대 토큰 기준 |
| `N` | 창 크기에 곱하는 안전 비율(%) |
| `P` | 패킷 크기 |
| `P_max` | 패킷 크기 상한 |
| `P_send` | 패킷을 새 session에 보낼 수 있는 추정 상한. `P_max`와 달리 `T`의 비율이 아니라 provider 창에서 정한다 |
| `r` | 캐시 읽기 배수 |
| `w` | 캐시 쓰기 배수 |
| `k*` | 본전 턴 수 |
| `H` | 턴당 맥락 증가량의 p95 |
| `T_hard` | provider 자동 압축 안전망 기준 |

### 맥락 크기 측정

`sessions`는 턴이 끝날 때마다 활성 맥락 크기 `A`를 잰다. `A`는 session에 쌓인 활성 맥락의 토큰 수이고 루트(메인) 에이전트 메시지로만 계산한다. engine은 턴이 끝날 때 `A`를 기록한다. 사용량 보고의 범위와 턴 값 계산은 [provider 연결과 session](providers-and-sessions.md)에 있다.

`A`를 잴 수 없으면 Saturn은 새 session을 열지 않고 provider 자동 압축에 맡긴다. 측정하지 못한 맥락으로 재시작을 판정하지 않기 위해서다.

### 발동 기준

발동 기준 `T`는 provider별 절대 토큰 기준 `T_abs`와 안전 비율 `N`으로 정한다.

```text
T = min(T_abs, N% × 창 크기)
```

`T_abs`는 provider마다 따로 정하는 토큰 수이고, `N`은 두 provider에 같이 쓰는 비율이다. `T`는 둘 중 작은 값이다. 품질 저하는 창 대비 비율이 아니라 절대 길이에 따라 생기기 때문이다([결정 기록](../decisions/2026-09-29-absolute-token-budget.md)). provider별 `T` 값은 실측으로 정한다([#7](https://github.com/woonyong-choi/saturn/issues/7)).

### compaction 판정

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/compaction-decision.ko.dark.svg">
  <img src="../assets/compaction-decision.ko.light.svg" alt="sessions는 턴이 끝날 때마다 유휴 복귀 조건, 발동 기준, 본전 턴 수를 차례로 보고 새 session으로 옮길지 계속할지 정한다" width="100%">
</picture>

`sessions`는 턴이 끝날 때마다 다음 순서로 판정한다.

1. 트리 유휴가 아니거나 `A`에 합칠 대기 입력이 있으면 다음 턴 경계까지 판정을 미룬다.
2. 유휴 복귀 조건: 마지막 턴 뒤 경과 시간이 캐시 유지 시간을 넘고 `P < A`면 새 session으로 이어 간다.
3. `A < T`면 그대로 계속한다.
4. 기준 도달 조건: `A ≥ T`이고 `k*`가 기대 잔여 턴 이하면 새 session으로 이어 간다.
5. 그 밖이면 그대로 계속한다.

1단계에서 트리 유휴는 메인 에이전트와 모든 subagent가 끝난 상태다. 2단계의 `P`는 새 session에 넘길 패킷의 크기다. 4단계의 기대 잔여 턴을 모르면 3으로 둔다.

본전 턴 수 `k*`는 새 session으로 옮기는 비용과 그대로 이어 가는 비용이 같아지는 턴 수다. `r`은 캐시 읽기 배수, `w`는 캐시 쓰기 배수다.

```text
k* = (P × w − A × r) / ((A − P) × r)
```

`k*`를 기대 잔여 턴과 비교해 여유가 있을 때 먼저 끊을지 정한다. 먼저 끊는 쪽이 이득인지를 계산으로 정하기 위해서다.

`A ≤ P`이거나 `r ≤ 0`이면 `k*`는 무한대로 보고, 계산값이 0보다 작으면 0으로 본다. 옮겨도 턴당 비용이 줄지 않으면 옮기지 않고, 바로 옮기는 쪽이 이득이면 바로 옮기기 위해서다.

### provider 자동 압축 안전망

Saturn은 provider 자동 압축을 끄지 않고, 발동점을 안전망 기준 `T_hard`에 둔다.

```text
T_hard = T + H
```

`H`는 턴당 맥락 증가량의 p95이고, 관측하기 전에는 창 크기의 10%로 둔다. 한 턴이 갑자기 커져 `T`를 넘어도 provider가 처리하게 하기 위해서다. subagent가 있을 때 맥락이 얼마나 느는지는 실측으로 확인한다([#25](https://github.com/woonyong-choi/saturn/issues/25)).

사용자가 provider 설정에 자동 압축 값을 정해 두었으면 안전망 값을 넣지 않는다. 사용자의 provider 설정을 Saturn 기본값보다 우선하기 위해서다. 안전망 값을 넘기는 실행 인자는 [provider 연결과 session](providers-and-sessions.md)에 있다.

### 정리 모드

정리 모드는 맥락 정리를 누가 맡을지 정하는 설정 `context.mode`다.

| 모드 | 맥락 정리 | provider 전환 때 넘기는 것 |
|---|---|---|
| `saturn`(기본) | `sessions`가 판정하고 패킷으로 새 session을 연다. provider 자동 압축은 안전망 | Saturn 패킷 |
| `provider` | provider 자동 압축에 맡기고 `sessions`는 compaction을 판정하지 않는다 | 떠나는 provider의 압축 요약을 읽을 수 있으면 그 요약을 넣은 패킷, 아니면 Saturn 패킷 |

- 기본은 `saturn`이다. provider 압축 없이 Saturn 기록 원문으로 전달 품질을 지키는 것이 목표이기 때문이다.
- `provider` 모드를 둔다. 사용자가 provider 압축을 쓰는 방식으로 Saturn을 맞춰 쓸 수 있게 하기 위해서다([결정 기록](../decisions/2026-10-01-context-mode-setting.md)).
- `provider` 모드에서는 안전망 값을 넣지 않는다. 사용자가 고른 provider 자동 압축을 그대로 두기 위해서다.
- `provider` 모드에서도 engine은 모든 이벤트를 기록하고, provider 전환과 결과 전달은 Saturn이 맡는다.
- 정리 모드와 발동 기준(`context.mode`, `provider.<id>.context.*`)은 engine 시작 설정이 아니라 판정 대상 에이전트가 가장 나중에 시작한 입력의 설정 번호로 읽는다. 시작한 입력이 없으면 그 채팅에서 마지막에 적용한 번호다. 맥락 정리로 새 session을 열 때도 같은 번호를 넘겨 정리 전후 설정이 유지되고, 설정이 다른 채팅끼리는 서로의 판정에 영향을 주지 않는다([설정](settings.md#병합과-설정-번호)).
- Claude 압축 요약은 로컬 기록으로 읽고, Codex 원격 압축 요약은 암호화되어 읽지 않는다. Claude 압축 요약을 읽는 경로와, 그 요약과 Saturn 패킷의 전환 품질은 실측으로 확인한다([#122](https://github.com/woonyong-choi/saturn/issues/122)).

### 패킷 구성

이 절은 [보존 우선 경로](#보존-우선-경로)의 대화 본문 보호를 반영한 현재 패킷 계약이다. 대화 본문은 선별하거나 줄이지 않는다.

패킷은 새 session에 넘기는 맥락 묶음이다. `sessions`는 패킷을 고정 구역과 경쟁 구역으로 나눠 채운다.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/packet-composition.ko.dark.svg">
  <img src="../assets/packet-composition.ko.light.svg" alt="패킷은 고정 구역을 먼저 넣고 경쟁 구역이 남은 예산을 채운다" width="100%">
</picture>

| 구역 | 항목 | 넣는 규칙 |
|---|---|---|
| 고정 구역 | 1. 사용자가 명시한 제약의 규칙 한 줄(제약 칸 상한 안) 2. 끝나지 않은 항목과 효과를 모르는 항목 3. 이 채팅에서 기록된 대화 전체(사용자 입력 원문, 끼워 넣어 적용한 입력, 에이전트 글) | 이 순서로 모두 넣는다. 1은 제약 칸 상한까지, 3은 줄이지 않는다 |
| 경쟁 구역 | 이 채팅의 도구 호출과 결과, 다른 에이전트 결과 요약, 실행별 수정 파일 목록([수정 파일 목록](providers-and-sessions.md#수정-파일-목록)), 파일 경로 | 남은 예산 안에서 고른 순서대로 넣는다 |

- 패킷은 맨 앞에 지시문을 두고(구역 예산에 든다) 그 뒤에 구역을 쓴다. 지시문은 아래가 이전 대화의 기록이지 요청이 아님, `[Finished]` 항목은 이미 끝난 일이라 다시 실행하지 않음, `[In progress]`·`[Result unknown]` 항목은 일부만 실행됐을 수 있어 현재 상태를 확인하기 전에는 믿지 않고 사용자가 요청하기 전에는 다시 하지 않음, `Queued input`·`Held input`은 보내진 적이 없어 Saturn이 따로 보냄을 알리고, 이 턴에서는 도구를 부르거나 파일을 바꾸지 말고 `Ready` 한 단어로 답한 뒤 다음 사용자 입력을 기다리라고 끝맺는다. 원문은 영어다. 지시문이 없으면 새 session이 대화의 사용자 입력을 지금 받은 요청으로 읽어, Claude에서 Codex로 전환할 때 끝난 입력의 작업을 다시 실행했다([#383](https://github.com/woonyong-choi/saturn/issues/383)).
- provider에 보낸 패킷은 시도마다 보낸 글의 해시와 들어가거나 빠진 항목, 이유를 기록 저장소에 남긴다. 줄여 다시 보낸 패킷은 시도마다 따로 남아 어느 항목이 어느 전송에 들어갔는지 섞이지 않는다([전달 패킷 근거](records.md#전달-패킷-근거)).
- 대화의 입력 항목에는 그 입력이 연 실행의 상태를 적는다. 실행이 정상으로 끝났으면 `[Finished]`, 아직 끝나지 않았으면 `[In progress]`, 실패하거나 멈췄으면 `[Result unknown]`이다. 결과를 모르는 항목은 결과 자리에 오류 결과(`Result (error): Interrupted before a result was recorded · It may have partially run`)를 붙인다. 끝난 입력은 남은 일 칸에 넣지 않는다. 남은 일 칸은 대기·보류 입력, 결과를 모르는 작업, 결과 없는 도구 호출만 담는다.
- 1단계의 제약은 유효 제약만 제약 칸 상한 `C_max` 안에서 넣고 못 넣은 수를 패킷과 대화 기록에 표시한다. 해제된 제약은 빼고 넣고 예외가 걸린 제약은 예외 표기와 함께 넣는다. 식별, 저장, 칸 채우기는 [제약](constraints.md)에 있다.
- router가 새 작업으로 판단해 여는 보조 session은 앞 맥락 없이 시작하므로 이 구성을 쓰지 않고, 제약 칸만 든 패킷을 먼저 받는다. 규칙은 [새 작업 session의 제약](constraints.md#새-작업-session의-제약)에 있다.
- 2단계에는 대기·보류 상태의 입력, 보낸 뒤 결과를 모르는 작업(`NeedsCheck`)의 입력, 결과가 없는 도구 호출을 넣는다. 결과가 없는 도구 호출은 별도 경고 문장 없이 도구 결과 자리에 오류 결과(`Interrupted before a result was recorded · It may have partially run`)를 둔다. 결과를 모르는 작업의 입력도 같은 오류 결과를 붙인다. 항목은 기록의 실제 상태로 고르고 모델을 따로 부르지 않는다. 대기·보류 입력은 보낸 적이 없으므로 이미 실행된 대화와 섞지 않는다. 남은 일을 모델이 쓰지 않고 기록으로 채우는 근거는 [실험 보고서](../experiments/packet-goal-fields/report.md)에 있다.
- 3단계는 이 채팅에서 기록된 사용자 입력, 실행 중에 끼워 넣어 적용한 입력, 메인 에이전트 글 전부다. 최근 몇 턴이나 수정 개수로 자르지 않고, 입력과 글을 줄이거나 합치지 않으며, 같은 글자의 서로 다른 입력도 문자열로 지우지 않는다. 판단 기록(`/record off` 포함), `constraint.auto_apply`, 제약 분류와 무관하게 실린다([#592](https://github.com/woonyong-choi/saturn/issues/592)). 입력이 연 실행마다 턴 하나이고 기록 번호 순서이며, 턴 안에서는 실행을 연 입력, 에이전트 글, 끼워 넣은 입력이 실제 순서다. 도구 결과는 넣지 않는다.
- 끼워 넣어 적용한 입력은 들어간 실행의 턴에 `User (sent while this turn was running): ...` 줄로 적용한 기록 번호 자리에 적용한 순서대로 붙는다. 에이전트 글은 끼워 넣은 입력에서 끊겨 앞뒤로 나뉜다. 거절되어 대기로 돌아온 입력은 실행에 들어가지 않았으므로 `Queued input`으로만 나오고, 실행된 입력으로 두 번 싣지 않는다. 이번 작업에만 해당하는 지시를 지속 제약으로 올리지 않는다([제약](constraints.md)). 기록은 `inputs.steered_run`과 `steered_after`로 [기록 저장과 보존](records.md#기록-저장소)이 정한다. 다른 session이 낸 실행만 읽는 변경분은 그 실행에 끼운 입력만 포함한다.
- provider 명령(`/compact` 등)처럼 글도 도구 호출도 내지 않고 끝난 실행의 입력도 기록된 사용자 입력이므로 같은 대화에 순서대로 들어가고 `[Finished]` 등 상태가 붙는다. 대화를 자르지 않으므로 이런 턴이 정정을 밀어낼 수 없다([#584](https://github.com/woonyong-choi/saturn/issues/584), `provider` 모드에서 정정한 헤더 이름이 8건 중 0건 남았다).
- 대화 본문의 원문 해시, 역할, 번호와 순서는 [전달 패킷 근거](records.md#전달-패킷-근거)에 남고, engine은 보내기 직전에 보낼 글이 이 본문을 같은 내용과 순서로 모두 담았는지 검사해 하나라도 빠지면 보내지 않는다.
- 판단 맥락의 최신 수정(이어 보낸 가장 나중 입력, [router](router.md#판단-요청-맥락))은 인계에 쓰지 않는다. 인계는 대화 전체를 싣기 때문이다.
- 경쟁 구역의 순서는 `compact` 판단으로 정한다. engine은 실험 옵션 `context.select.packet`이 `jev`일 때만 이 판단을 부르고(#380), 기본(`rrf`)은 후보 순위(RRF) 순서로만 채운다. 후보 전체를 묻고, 항목마다 호출과 결과 중 큰 남김 확률을 쓴다. 확률이 높은 순으로, 같은 확률이면 [맥락 고르기](context-selection.md)의 후보 순위 순으로 둔다. router가 답하지 못한 항목은 후보 순위 순으로 뒤에 둔다.
- `compact` 판단이 재시도 뒤에도 실패하면 router가 시작한 전환은 건너뛰고, 사용자가 고정했거나 맥락 크기 규칙이 시작한 전환은 후보 순위 순으로 경쟁 구역을 채운다. 규칙과 이유는 [router 실패](router.md#router-실패)에 있다.
- 확률에 기준값을 두지 않고 예산이 찰 때까지 채운다. 근거 항목의 확률은 평균 0.372, 최댓값 0.65로 낮아서 기준값 0.5가 근거 항목 624개 중 549개(88.0%)를 버렸기 때문이다([결정 기록](../decisions/2026-10-02-fill-packet-by-probability.md)). 확률은 근거와 비근거를 가르는 순서에만 쓴다.

패킷 크기 상한 `P_max`는 발동 기준의 10분의 1이다.

```text
P_max = T / 10
```

`P_max`는 재시작 비용에서 정한 값이다. 활성 맥락이 패킷의 10배 이상이면 캐시 배수와 무관하게 몇 턴 안에 재시작 비용을 돌려받기 때문이다. 패킷 크기는 네 글자를 한 토큰으로 추정한다.

경쟁 구역은 `P_max`에서 고정 구역을 뺀 예산으로 채운다. `sessions`는 고른 순서대로 항목마다 다음 중 처음으로 들어가는 형태를 넣고, 어느 형태도 들어가지 않으면 건너뛴다.

1. 원문. 단, 한 항목이 경쟁 구역 예산의 30%(초안)를 넘으면 원문을 넣지 않는다.
2. 축약본(앞부분 300자와 한 줄 메모). 메모 틀은 [맥락 고르기](context-selection.md)에 있다. 메모가 없으면 앞부분만 넣는다.
3. 파일 경로.

- 관련도가 높은 작은 항목을 관련도가 낮은 큰 항목보다 먼저 넣기 위해서다.
- 한 항목 상한은 큰 로그 하나가 경쟁 구역을 혼자 차지하지 않게 하기 위해서다.
- 도구 출력은 패킷을 만들 때만 줄이고, 실행 중인 provider의 도구 출력에는 훅을 걸지 않는다. provider 설정은 사용자 설정을 따르고 훅은 router 키 보호에만 쓰기 때문이다([결정 기록](../decisions/2026-09-29-minimal-provider-control.md), [#37](https://github.com/woonyong-choi/saturn/issues/37)). 실행 중 커지는 맥락은 위의 맥락 정리 판정이 다룬다.
- 패킷에 쓸 때는 구역마다 기록 번호 순서로 다시 정렬한다. 순위는 무엇을 넣을지만 정하고, 새 session은 일이 일어난 순서를 알아야 하기 때문이다.
- 대화와 경쟁 구역은 session별 제목 아래에 묶고 항목마다 기록 번호와 시각을 적는다. 형식은 [패킷 표기](#패킷-표기)에 있다.

고정 구역(대화 본문 포함)만으로 `P_max`를 넘어도 대화는 턴 수나 답 길이로 줄이지 않는다. `sessions`는 다음 순서로 처리한다.

1. 이 패킷에 한해 `P_send`까지 허용하고 `P_max` 초과를 기록한다. 경쟁 구역은 비운다. 허용한 초과분을 고정 구역에만 쓰기 위해서다.
2. `P_send`도 넘거나 `P_send`를 알 수 없으면 대화를 자르지 않고 새 session으로 옮기지 않으며 그대로 이어 간다. TUI에 `고정 제약이 길어 맥락 정리를 미룹니다`와 제약 목록을 보인다. 한도 밖 장기 대화는 후속 기억 설계의 과제다.

`P_send`는 `P_max`나 `T`에 비율을 곱해 정하지 않는다. `T`는 맥락 정리를 시작하는 기준이지 provider가 받는 한도가 아니기 때문이다. 받는 provider의 창 크기(`context.<provider>.window`, 어댑터 설명자가 기본값을 준다)에서 시스템 지시와 도구 정의(20,000토큰), 패킷 뒤에 보낼 현재 입력(10,000토큰), 한 턴의 출력(32,000토큰)을 예약으로 먼저 빼고 남은 양에 안전 비율 `N`을 곱한 값이다(초안, 새 설정은 없다). 현재 입력의 실제 크기는 패킷을 만들 때 모르므로 고정 예약으로 둔다. 창이 0이거나 예약보다 작으면 `P_send`는 0이라 어떤 패킷도 보내지 않는다. 한도를 모르는 채 보낸 패킷을 성공으로 세지 않기 위해서다.

패킷 크기는 `P_send`와 견줄 때 보수적으로 센다. 일반 추정은 4자에 1토큰이지만 한글처럼 ASCII가 아닌 글자는 글자마다 1토큰으로 센다. 이 값은 provider의 실제 토큰 계수가 아니라 추정이다. 추정이 한도 안이어도 provider가 거절할 수 있고, 그때는 아래 거절 처리를 따르며 추정 통과와 실제 거절은 서로 다른 기록으로 남는다. 경쟁 구역까지 더한 글이 `P_send`를 넘으면 경쟁 구역을 비운다.

패킷을 보냈는데 provider가 맥락 한도 초과로 거절하면 `engine`은 경쟁 구역을 줄여 한 번만 다시 보낸다. provider 전환과 맥락 정리로 여는 새 session이 같은 규칙을 쓴다. 거절은 보내지 않음이 확정된 실패라 다시 보내도 같은 작업이 두 번 실행되지 않는다([#162](https://github.com/woonyong-choi/saturn/issues/162) 결정).

1. 줄이는 목표는 거절 응답이 한도를 알려 주면 그 값, 알려 주지 않으면 받는 provider의 `P_max`의 절반(초안)이다. 어느 쪽이든 거절된 패킷 크기의 절반을 넘지 않는다. 한도를 알려 줘도 패킷이 그 안에 이미 들어 있으면 줄이지 못해 같은 패킷을 되풀이하게 되기 때문이다.
2. `sessions`는 고정 구역과 대화 본문을 그대로 두고 경쟁 구역만 목표에 맞춰 같은 순서로 다시 채운다. 순서가 뒤인 남길 확률이 낮은 항목부터 빠진다. 고정 구역은 줄이지 않는다.
3. 고정 구역만으로 목표를 넘거나 줄인 패킷도 거절되면 보내지 않고 입력을 작업과 함께 보류하며 TUI에 `맥락 한도 초과로 멈춤 · /continue로 다시 시도하세요`를 보인다.

맥락 정리는 기다리는 입력이 없을 때만 하므로 보류할 입력이 없다. 그래서 줄인 패킷도 거절되거나 고정 구역만으로 넘치면 새 session을 열지 않고 옛 session을 그대로 두며 같은 알림만 보인다. 맥락은 다음 턴 경계에서 다시 판정하고 그 사이에는 provider 자동 압축 안전망이 받는다.

`Deferred`(고정 구역이 `P_send`도 넘음)는 경쟁 구역이 이미 비어 있어 줄일 항목이 없다. 그래서 다시 보내지 않고 위 3과 같이 멈추되 제약 목록을 보인다. provider가 맥락 초과로 거절했는지는 provider 고유 코드(`providers/codex`)가 판정해 공통 오류 `ContextExceeded`로 올린다. Codex는 `turn/start` 거절 응답의 문구로 판정하고, 실제 거절 응답의 모양은 실측하지 못했다. Claude는 쓰기 전 거절 경로가 없어 이 판정이 없다. 진행 중인 턴이 맥락 초과로 끝나는 오류 결과(`terminal_reason`이 `prompt_too_long`)는 보내지 않음이 확정이 아니라 줄여 다시 보내지 않고, 결과 확인 필요로 둔다([providers-and-sessions](providers-and-sessions.md)).

고정 구역이 넘칠 때 provider 압축으로 대신하지 않는다. provider가 들고 있는 맥락과 Saturn 기록이 달라지고, Codex 원격 압축 요약은 무엇이 남았는지 볼 수 없기 때문이다. 4단계 뒤 맥락이 `T_hard`를 넘으면 provider 자동 압축 안전망이 받는다.

`saturn` 모드에서 이어 갈 내용은 provider 요약이 아니라 맥락의 정본인 Saturn 기록 원문에서 고른다. provider 압축 요약은 읽을 수 있을 때만 경쟁 구역의 후보 하나로 쓴다. 기록 원문을 정본으로 두기 위해서다. `provider` 모드에서 Claude 압축 요약을 넘길 때는 그 요약을 경쟁 구역의 첫 항목으로 두고, 나머지 경쟁 구역은 요약 시점 뒤의 기록에서 고른다. 요약이 경쟁 구역 예산을 넘으면 요약을 쓰지 않고 Saturn 기록 원문으로 채운다(`build_packet_with_summary`). 어느 모드든 고정 구역은 Saturn 기록 원문으로 넣고, 패킷에 제약이 요약보다 우선한다고 적는다. 요약이 대체되기 전의 제약을 담고 있을 수 있기 때문이다. provider가 스스로 읽는 문서(`AGENTS.md`, `CLAUDE.md`)는 패킷에 넣지 않는다. 중복을 막기 위해서다.

`compact` 판단은 패킷을 만들 때 한 번 묻는다. 턴마다 미리 묻지 않는다. 나눈 요청을 병렬로 보내면 실측한 범위(질문 40개, 약 7KB state, 동시 8개까지)에서 후보 전체 판단의 지연이 조각 수와 거의 무관하게 약 0.3초이기 때문이다. 질문 40개와 약 7KB state 요청을 4개 동시에 보내면 0.26초, 8개 동시에 보내면 0.27초, 4개를 차례로 보내면 0.97초였다([#179](https://github.com/woonyong-choi/saturn/issues/179) 실측). 미리 판단은 후보 전체를 묻는 조건에서 패킷 때 물을 질문을 앞당길 뿐이다. 첫 턴부터 미리 판단하면 추정 입력 토큰이 1.118배 [1.088, 1.161]로 늘고, 사건에 닿지 않아 버려지는 질문이 9.7% [7.3, 13.1]이며, 미리 한 판단이 패킷 때 목표와 어긋날 위험이 남는다. 지연을 줄이는 이득은 순차 전송을 가정한 값이고, 실측한 범위의 병렬 전송에서는 사라진다. 64KB에 가까운 요청과 동시 8개를 넘는 전송은 재지 않아 이 서술이 닿지 않는다([후속 분석](../experiments/precompute-breakeven/report.md#후속-분석-후보-전체-판단-조건)). 이 기준은 사전 등록 판정이 아니라 설계 판단용이다.

`compact` 질문의 확률 읽기와 router가 답하지 못할 때의 대체 규칙은 [router](router.md)에 있다.

### 패킷 판단의 적용

패킷을 만들 때의 `compact` 판단은 router 호출이 끝난 뒤 적용한다. 이 흐름은 실험 옵션 `context.select.packet = jev`일 때만 돈다(#380). 패킷 만들기는 세 단계다.

1. `sessions`가 기록과 제약에서 재료(후보, 고정 구역 항목)를 모은다.
2. engine이 후보 전체를 `compact` 요청으로 묻는다. 요청 처리와 별도 작업으로 돌고 그동안 다른 요청을 기다리지 않는다.
3. 적용 직전에 전환 키를 비교하고, 같으면 `sessions`가 답으로 경쟁 구역을 채운다. 제약 칸은 판단에 쓰지 않고 이 시점의 제약 목록으로 만든다.

전환 키는 채팅, 보내는 쪽 메인 session 번호, 받는 쪽 provider와 모델, 전환을 일으킨 입력 번호(맥락 정리면 `맥락 정리`)다.

- 키가 같으면 적용한다. 기록은 추가만 되므로 판단 뒤 기록이 늘어도 키는 같다. 늘어난 후보는 답이 없는 항목처럼 답이 있는 항목 뒤에 후보 순위 순으로 둔다.
- 키가 다르면 판단을 `superseded`로 기록하고 버린 뒤 재료를 다시 모아 한 번 다시 묻는다. 또 다르면 판단 없이 진행한다. router가 시작한 전환은 건너뛰고 사용자가 고정했거나 맥락 크기 규칙이 시작한 전환은 후보 순위 순서로 채운다([router 실패](router.md#router-실패)).
- 채팅 revision 전체를 비교하지 않는다. 판단이 도는 사이 작업 상태와 대기열은 자주 바뀌지만 후보를 남길 확률은 틀리지 않는다. 전환 대상이 바뀌면 그 판단은 다른 상대를 향한 것이므로 버린다.
- 맥락 정리로 여는 새 session은 키 비교에 더해 트리 유휴이고 합칠 대기 입력이 없는지 다시 확인한다. 아니면 정리를 다음 턴 경계로 미룬다.

판단은 요청 처리와 별도 작업으로 돈다(입력 판단과 같은 router 비동기 경로). 입력을 보내려던 채팅은 답이 올 때까지 다음 입력을 보내지 않지만, engine은 그동안 다른 요청과 다른 채팅의 입력을 그대로 처리한다. 답은 engine 루프로 돌아와 보관하고, 적용 직전에 계획을 다시 세우면서 그 시점의 전환 키와 비교한다. 기다림은 호출 하나의 재시도 마감(10초)을 넘지 않고, 넘어서 온 답(늦은 답)은 쓰지 않고 버린다. 답을 기다리는 사이 입력이 사라졌거나 계획이 판단을 쓰지 않게 바뀌었으면(전환 자체가 사라짐) 답은 쓰지 않고 `superseded`로 기록한다. 맥락 정리는 같은 방식으로 판단을 맡긴 채 턴 경계 처리를 멈추고, 답이 오면 경계 처리를 이어 간다(`packet_select.rs`의 `compact_gate`, `spawn_compact`, `on_compact_done`).

적용 규칙은 다음과 같다.

| 상황 | 동작 |
|---|---|
| 옵션이 꺼져 있다(기본) | router를 부르지 않고 RRF 순서로 채운다. |
| 새 session을 여는 전환과 맥락 정리 | 후보 전체를 한 번 묻고 남김 확률 순으로 채운다. 같은 예산(`P_max`)과 같은 고정 구역을 쓰고 달라지는 것은 경쟁 구역의 순서뿐이다. |
| 보관 session으로 돌아가며 변경분만 붙이는 경우, 열린 session을 그대로 쓰는 경우 | 묻지 않는다. 패킷이 아니라 변경분이다. |
| 일부 후보만 답했다 | 답한 항목을 확률 순으로 앞에 두고 나머지는 RRF 순으로 뒤에 둔다. 판단 기록에 `partial`을 남긴다. |
| 답이 전부 없다, router 실패, 응답 형식 오류, 필수 state가 크기 한도에 들어가지 않음(`input-limit`), 늦은 답 | 판단 없이 진행한다. 맥락 정리와 사용자가 고정한 전환은 RRF 순서로 채우고 로그를 남긴다. router가 고른 모델이 열린 메인과 달라 시작한 전환은 전환을 건너뛰고 현재 모델로 진행한다([router 실패](router.md#router-실패)). |
| 판단 중 전환 키가 달라졌다 | 판단을 `superseded`로 기록하고 한 번 다시 묻는다. 또 다르면 판단 없이 진행한다. |

- 전환을 누가 시작했는지는 사용자가 모델을 고정했거나(`/model` 포함) 유휴 복귀나 맥락 크기 규칙이 연 전환이 아니고, 모델 선택이 오토 모드이며, 고른 provider나 모델이 열린 메인과 다를 때만 router가 시작한 것으로 센다. 그 밖은 모두 강제한 전환이다.
- 판단은 성공이든 실패든 판단 기록에 남아 요청 합계에 센다. 늦은 답으로 끊은 호출은 응답을 받지 못해 기록하지 않고 로그만 남긴다. 연속 실패 집계(`RouterPaused`, `RouterDisconnected` 알림)에는 이 호출을 세지 않는다.
- 전달 패킷 기록의 경쟁 구역 항목은 `selector`가 `compact`(판단 순서)나 `rank`(순위 순서)다. 판단을 받지 못해 대체했으면 `rank`로 남아 기록이 실제 보낸 패킷과 맞는다.
- 판단을 요청한 패킷은 전달 패킷 시도 행에 요청한 방식(`compact`)과 실제 방식, 대체 사유, 질문에서 잘린 후보 글의 바이트, 한도를 넘은 바이트를 따로 남긴다([선별 근거 기록](context-selection.md#선별-근거-기록)). 대화 본문은 어느 쪽이든 줄지 않고, 판단이 고르는 것은 도구 호출·결과 기록뿐이다.
- 판단 state에는 지금 보내려는 입력과 기록된 사용자 입력 전부를 줄이지 않고 싣는다. 이 변경은 Jev 효과를 입증하지 않으며, 기본값은 `rrf`로 둔다. 효과 확인 전에는 기본값을 바꾸지 않는다([#7](https://github.com/woonyong-choi/saturn/issues/7), [#544](https://github.com/woonyong-choi/saturn/issues/544)).

#### 낮은 확신 대체 규칙을 쓰지 않는 이유

[#540](https://github.com/woonyong-choi/saturn/issues/540)의 오프라인 실험에서 봉인한 낮은 확신 규칙(예산 안 셋째 블록의 확률이 0.5 미만이면 전체를 코드 검색으로 대체)이 필요한 블록이 하나인 과제 24건을 모두 대체시켰다. 모델은 정답 블록에 0.95를 줬는데도 나머지 후보가 낮다는 이유로 판단이 버려졌다. 필요한 블록이 하나면 나머지 확률은 원래 낮으므로 이 규칙은 단일 블록 과제에서 늘 발동한다.

이 구현은 그 규칙을 쓰지 않는다. 판단 전체를 버리는 기준은 답의 유무뿐이고, 확률이 낮다는 이유로 대체하지 않는다. 확률 기준 이상의 블록만 남기는 방식도 검토했으나 채택하지 않았다. 남김 확률 0.5 기준은 근거 항목의 88.0%를 버렸고([결정 기록](../decisions/2026-10-02-fill-packet-by-probability.md)), 예산이 남는데 기준 미만 블록을 빼는 것은 담을 수 있는 근거를 버리는 일이다. 대신 확률 순으로 예산이 찰 때까지 채우므로 확률이 가장 높은 블록이 항상 먼저 들어가고, 필요한 블록이 하나여도 그 블록은 빠지지 않는다. 기준 이상 블록을 먼저 채우고 남은 예산을 기준 미만 블록으로 채우는 것은 이 순서와 같은 결과다. 이 성질은 `lifecycle/packet_select.rs`의 `single_needed_block_with_low_scores_elsewhere_is_kept_not_replaced`가 확인한다. 이 규칙이 실제 과제 성과를 높이는지는 #7과 #540의 온라인 단계에서 가른다.

### 패킷 표기

대화와 경쟁 구역은 session별 제목 아래에 묶고, 항목마다 기록 번호와 기록 시각을 적는다. 패킷 96개에 날짜, session, 기록 번호 표시가 없어 날짜가 필요한 질문(384개 중 96개, 25.0%)은 근거가 모두 들어도 풀 수 없었고 `multi-session`과 `temporal` 질문은 세 조건 모두 정답률이 0%였기 때문이다([후속 분석](../experiments/handoff-packet-quality/report.md#후속-분석-원인)).

```text
The records below are an archive of the earlier conversation. ...

## Conversation

### Session 2

#190 2026-09-13T09:00Z [Finished] User: finish the cache module
Agent: done

## Earlier records

### Session 1

#41 2026-09-12T10:00Z Read {"file_path":"src/auth.rs"}
auth file body

#44 2026-09-12T10:05Z src/cache.rs

### Session 2

#185 2026-09-13T08:30Z Bash {"command":"cargo test"}
test result: ok
```

- 항목 앞에 `#<기록 번호> <기록 시각>`을 적는다. 시각은 UTC 분 단위 `YYYY-MM-DDTHH:MMZ`(17글자)다. 초와 시간대 이름을 빼 토큰을 아끼고, 기록 저장소의 unix 밀리초 값을 바꿔 쓰므로 provider별 변환을 거치지 않는다.
- session 제목은 `### Session <번호>`이고 session마다 한 번 쓴다. 날짜와 session 경계를 항목마다 되풀이하지 않기 위해서다. 기록 번호 순으로 놓으면 같은 session의 항목이 이어진다.
- 대화의 사용자 입력은 기록 번호와 시각, 상태를 적는다(`#<기록 번호> <시각> [Finished] User: ...`). 에이전트 글(`Agent: ...`)과 끼워 넣은 입력은 같은 턴 안에서 줄만 바꿔 잇는다. 제약, 끝나지 않은 항목은 표기를 붙이지 않는다.
- 설정 `context.evidence.lookup`이 참이고 경쟁 구역에서 원문 아닌 모양으로 들어가거나 빠진 기록이 있으면, 경쟁 구역 끝에 `saturn evidence read <number>`로 원문을 다시 읽는 안내 한 줄을 붙인다. 안내도 경쟁 구역 예산에 든다. 기본은 거짓이다([근거 검색과 원문 조회](context-selection.md#근거-검색과-원문-조회)).
- 항목 앞 표기와 session의 첫 항목이 쓰는 제목은 경쟁 구역 예산에 든다. 표기까지 넣은 원문이 들어가지 않으면 축약본, 경로 순으로 시도한다.
- provider 압축 요약은 기록 한 건이 아니므로 표기 없이 경쟁 구역의 첫 항목으로 제목 앞에 둔다.
- 기록 저장소의 행은 모두 시각이 있다. 시각이 없는 입력(실험 예제의 표준 입력)은 `#<기록 번호>`만 적는다.

### compaction 방식

| 방식 | Codex | Claude Code |
|---|---|---|
| provider 압축 | `thread/compact/start` | `/compact` 전송 |
| 새 session | 패킷을 넘긴 새 session | 패킷을 넘긴 새 session |

1. 판정이 나면 `sessions`는 Saturn 기록 원문에서 패킷을 만든다.
2. engine은 provider별 방식으로 새 session에 패킷을 넘기거나 provider 압축을 요청한다.
3. session을 바꾸면 engine은 옛 session을 닫고 session 기록을 바꾼다.
4. TUI는 `맥락 정리 후 이어서 진행` 한 줄을 보인다.

session 교체는 턴이 끝난 경계에서만 한다. 교체 규칙은 [provider 연결과 session](providers-and-sessions.md)에 있다.

- engine은 트리가 유휴가 된 턴 끝에서 마지막 턴 값을 기록하고, 작업을 끝내고, 이 판정을 한 뒤에 기다리던 입력을 보낸다. 판정이 새 session으로 바꾸는 일이 입력 전송 사이에 끼지 않게 하기 위해서다.
- 판정은 `A`를 알고 `A ≥ T`일 때만 패킷을 만들어 `decide`를 부른다. `A < T`이면 기록을 읽지 않는다. 판정이 `Restart`면 같은 provider의 새 session을 패킷과 함께 열고, 옛 session은 `종료`로 두고 닫는다. 메인 에이전트 번호는 그대로이고 새 session의 전달 기록 번호는 패킷이 담은 마지막 번호다. 열지 못하면 옛 session을 그대로 쓴다. 패킷이 `P_send`도 넘으면 옮기지 않고 `고정 제약이 길어 맥락 정리를 미룹니다`를 보인다.
- 턴 끝에서는 마지막 턴 뒤 경과 시간을 0으로 본다. 유휴 복귀 조건은 다음 입력을 보낼 session을 정할 때 판정한다. 열린 메인이 트리 유휴이고 마지막 턴 뒤 경과 시간이 캐시 유지 시간을 넘었으면 그 입력에서 만들 패킷 `P`와 마지막 `A`를 비교해 `P < A`일 때 같은 provider와 모델의 새 session으로 패킷과 함께 이어 간다. 옛 session은 닫아 `종료`로 둔다. 마지막 턴 값을 모르거나 `P ≥ A`이면 열린 session을 그대로 쓴다. 판정하는 입력은 보내려는 입력 자신이라 합칠 대기 입력으로 세지 않고, 그 입력은 패킷에 넣지 않고 새 session의 첫 턴 뒤에 보낸다. 같은 판정을 엔진을 다시 켠 뒤 처음 보내는 입력에도 쓴다.
- 정리 모드 `context.mode`가 `provider`이면 턴 끝의 판정과 유휴 복귀 판정을 모두 하지 않고, provider 실행 인자에 안전망 값(`T_hard`)도 넣지 않는다.
- 경쟁 구역 순서는 기본이 후보 순위(RRF)이고, 실험 옵션 `context.select.packet = jev`일 때만 `compact` 판단을 부른다.

provider마다 어느 방식을 쓸지는 품질을 지키면서 토큰이 적은 쪽을 실측으로 고른다([#7](https://github.com/woonyong-choi/saturn/issues/7)). 축소 세션에서 잰 첫 결과는 [맥락 선별 순효과](../experiments/context-net-effect/report.md)에 있고, 이 측정에서는 Saturn 패킷이 provider 압축보다 비용을 줄이면서 품질을 유지한다는 근거가 없어 기본 방식은 바꾸지 않는다. 어느 방식도 provider 기본 압축보다 품질을 낮추지 않아야 한다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| `A` 측정 불가 | 새 session을 열지 않고 provider 자동 압축에 맡긴다. |
| 트리 유휴 전 | 판정을 다음 턴 경계까지 미룬다. |
| `A`에 합칠 대기 입력 존재 | 판정을 다음 턴 경계까지 미룬다. |
| 한 턴의 급증으로 `T` 초과 | `T_hard`에서 provider 자동 압축이 처리한다. |
| 경쟁 구역 예산 부족 | 고른 순서대로 원문, 축약본, 경로 중 들어가는 형태를 넣고 나머지는 건너뛴다. |
| 고정 구역의 `P_max` 초과 | 대화 본문은 줄이지 않고 경쟁 구역을 비운 채 `P_send`까지 허용한다. |
| 고정 구역의 `P_send` 초과 | 대화를 자르지 않고 새 session으로 옮기지 않으며 사용자에게 제약 목록을 보인다. |
| provider가 패킷을 맥락 한도 초과로 거절 | 경쟁 구역을 줄여 한 번만 다시 보낸다. 줄인 패킷도 거절되거나 고정 구역만으로 목표를 넘으면 보내지 않고 입력을 보류하며 사용자에게 알린다. |
| 떠나는 provider의 압축 요약을 읽을 수 없음 | Saturn 기록 원문으로 패킷을 만든다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| compaction은 트리 유휴이고 합칠 대기 입력이 없을 때만 한다. | `saturn-terminal/engine/src/lifecycle/turn_end.rs`의 `compaction_is_postponed_while_an_input_is_waiting_to_merge`, `answer_with_a_running_subagent_ends_the_turn_only_when_the_tree_is_idle` |
| session 교체는 턴 경계에서만 하고, 교체한 새 session에 패킷을 넘긴다. | `saturn-terminal/engine/src/lifecycle/turn_end.rs`의 `context_over_the_threshold_replaces_the_session_only_at_the_turn_boundary` |
| `A`를 모르면 새 session을 열지 않는다. | `saturn-terminal/engine/src/lifecycle/turn_end.rs`의 `unknown_context_size_leaves_compaction_to_the_provider` |
| 고정 구역이 `P_send`를 넘으면 맥락 정리를 미루고 사용자에게 알린다. | `saturn-terminal/engine/src/lifecycle/turn_end.rs`의 `oversized_fixed_zone_defers_compaction_and_tells_the_user` |
| 유휴 복귀 조건과 기준 도달 조건에서만 새 session으로 이어 간다. | `A`, `T`, `P`, `k*`, 경과 시간 조합마다 판정 결과가 규칙과 같은지 확인한다. |
| 캐시 유지 시간을 넘겨 돌아오고 패킷이 마지막 `A`보다 작으면 다음 입력에서 새 session으로 이어 가고, 유지 시간 안이거나 패킷이 `A` 이상이면 열린 session을 그대로 쓴다. | `saturn-terminal/engine/src/lifecycle/turn_end.rs`의 `returning_after_the_cache_window_opens_a_new_session_when_the_packet_is_smaller`, `returning_inside_the_cache_window_keeps_the_session`, `returning_after_the_cache_window_keeps_the_session_when_the_packet_is_not_smaller` |
| `context.mode`가 `provider`이면 `sessions`는 compaction을 판정하지 않고 안전망 값을 넣지 않는다. | `saturn-terminal/engine/src/lifecycle/turn_end.rs`의 `provider_mode_neither_restarts_on_return_nor_at_the_threshold`, `saturn-terminal/engine/src/lifecycle/intake.rs`의 `provider_mode_leaves_out_the_auto_compact_safety_net`, `saturn-terminal/engine/src/lifecycle/turn_end.rs`의 `connection_layer_context_mode_provider_skips_compaction_at_the_turn_end`, `connection_layer_context_budget_decides_compaction_and_survives_it`, `context_settings_of_two_chats_do_not_affect_each_other`, `saturn-terminal/engine/src/settings/mod.rs`의 `context_mode_defaults_to_saturn_and_reads_provider`, `context_mode_rejects_unknown_values` |
| 실행 중에 끼워 넣어 적용한 입력은 그 턴 안의 적용한 기록 번호 자리에 적용한 순서로 들어가고 입력 번호가 기록되며, 거절된 입력은 들어가지 않는다. | `saturn-terminal/engine/src/lifecycle/intake.rs`의 `applied_steer_is_preserved_in_handoff`, `saturn-terminal/engine/src/lifecycle/conflict_steer.rs`의 `applied_steer_is_kept_with_its_run_and_a_refused_one_is_not`, `saturn-terminal/engine/src/handoff.rs`의 `steered_inputs_keep_their_applied_position_and_order`, `a_steer_into_another_sessions_run_stays_out_of_a_catch_up_for_the_first`, `a_steer_whose_run_has_no_records_is_left_out`, `saturn-terminal/engine/src/store/ledger.rs`의 `steered_inputs_keep_acceptance_order_and_the_sequence_they_arrived_after` |
| 글도 도구 호출도 없이 끝난 provider 명령 턴도 기록된 입력이라 대화에 들어가며 정정을 밀어내지 않는다. | `saturn-terminal/engine/src/lifecycle/constraint_handoff.rs`의 `provider_command_turn_does_not_push_a_correction_out_of_the_recent_turns` |
| 정정 뒤 읽기 턴이 네 개 이어져도, 판단 기록과 제약 등록이 없어도(`/record off` 포함), engine을 다시 켠 뒤에도 정정 원문은 한 번 제자리에 남고 같은 글자의 서로 다른 입력은 합치지 않는다. | `saturn-terminal/engine/src/lifecycle/dialogue_preserved.rs`의 `four_reads_after_a_correction_do_not_push_it_out_of_the_switch_packet`, `four_reads_after_a_correction_survive_with_recording_off`, `saturn-terminal/engine/src/handoff.rs`의 `a_correction_survives_four_following_reads_without_any_judgment`, `identical_inputs_of_different_runs_are_both_kept`, `every_input_of_every_task_is_kept_in_order`, `saturn-terminal/engine/src/lifecycle/constraint_handoff.rs`의 `judged_correction_stays_in_the_packet_after_more_turns_and_a_restart`, `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_keeps_identical_inputs_and_steer_order` |
| 대화 본문의 번호·역할·원문 해시·순서를 전달 패킷 기록에 남기고, 보낼 글이 본문을 같은 내용과 순서로 담지 않으면 보내지 않는다. | `saturn-terminal/engine/src/handoff.rs`의 `evidence_refuses_a_body_that_lost_or_swapped_recorded_dialogue`, `saturn-terminal/core/src/sessions/packet/tests.rs`의 `contains_protected_rejects_missing_changed_and_misordered_dialogue`, `saturn-terminal/engine/src/lifecycle/dialogue_preserved.rs`의 `four_reads_after_a_correction_do_not_push_it_out_of_the_switch_packet` |
| 고정 구역이 넘치지 않으면 패킷 크기는 `T`의 10분의 1을 넘지 않는다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_large_record_stays_within_packet_limit` |
| 고정 구역은 정한 순서로 모두 들어가고 대화에는 도구 결과가 없다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_fixed_zone_in_order_and_tool_results_only_in_competing` |
| 경쟁 구역은 기준값 없이 router 남김 확률 순으로 예산이 찰 때까지 채운다. | `saturn-terminal/core/src/sessions/ranking.rs`의 `order_after_router_orders_by_probability_and_keeps_low`, `saturn-terminal/core/examples/packet/tests.rs`의 `packet_judgments_put_low_probability_item_before_unanswered` |
| 경쟁 구역은 고른 순서대로 원문, 축약본, 경로 중 들어가는 형태로 채운다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_small_top_item_goes_raw_before_large_lower_item`, `build_packet_fills_competing_in_chosen_order_raw_then_digest`, `build_packet_falls_back_to_path_then_skips` |
| 한 항목은 경쟁 구역 예산의 30%를 넘는 원문으로 들어가지 않는다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_item_over_cap_goes_as_digest` |
| 대화와 경쟁 구역은 session별 제목 아래에 묶고 항목마다 기록 번호와 기록 시각을 적는다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_competing_groups_by_session_with_seq_and_time`, `build_packet_conversation_carries_session_title_seq_and_time`, `saturn-terminal/core/src/sessions/stamp.rs`의 `label_formats_seq_and_utc_minute`, `saturn-terminal/core/examples/packet/tests.rs`의 `packet_scenario_session_and_time_appear_before_items` |
| 시각이 없는 입력은 기록 번호만 적는다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_without_time_writes_seq_only`, `saturn-terminal/core/src/sessions/stamp.rs`의 `label_formats_seq_and_utc_minute` |
| 표기와 session 제목의 글자도 경쟁 구역 예산에 들어 패킷은 `P_max`를 넘지 않는다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_many_sessions_stay_within_packet_limit` |
| 패킷은 맨 앞 지시문으로 기록이 요청이 아니며 끝난 일을 다시 하지 말고 다음 사용자 입력을 기다리라고 알린다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_starts_with_the_do_not_act_instruction` |
| 입력 항목에 끝남, 진행 중, 결과 모름 상태를 적고 결과 모름은 중단 결과 형식을 쓰며, 끝난 입력은 남은 일 칸에 넣지 않는다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_recent_turn_states_and_unknown_result_format`, `saturn-terminal/engine/src/handoff.rs`의 `finished_input_is_marked_finished_and_never_an_open_item`, `stopped_input_has_an_unknown_result_in_the_interrupted_format`, `running_input_is_marked_in_progress`, `saturn-terminal/engine/src/store/ledger.rs`의 `ledger_since_carries_how_the_run_ended` |
| 패킷은 구역마다 기록 번호 순서로 쓴다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_writes_competing_in_seq_order` |
| `compact` 질문은 패킷을 만들 때 후보 전체를 한 번 router에 보내고 턴이 끝날 때는 보내지 않는다. | `saturn-terminal/engine/src/lifecycle/packet_select.rs`의 `jev_order_fills_the_budget_with_the_blocks_the_router_wants`(새 session을 여는 입력에서 후보 전체를 한 번 묻는다), `option_is_off_by_default_and_asks_nothing`. 턴 종료에서 묻지 않는 것은 `compact_at_boundary`가 `Restart` 판정 뒤에만 부르는 구조로 보장하며 별도 시험은 없다. |
| 판단을 받지 못하면(실패, 전부 무응답, 형식 오류, 늦은 답) 순위 순서로 채우고 전달 기록의 `selector`가 `rank`로 남는다. 일부만 답했으면 답한 항목이 앞에 온다. | `saturn-terminal/engine/src/lifecycle/packet_select.rs`의 `without_a_judgment_the_packet_is_filled_by_rank_order`, `an_answer_that_arrives_too_late_is_not_used`, `jev_order_fills_the_budget_with_the_blocks_the_router_wants` |
| 필요한 블록 하나만 높고 나머지가 낮아도 판단을 버리지 않고 그 블록을 먼저 넣는다. | `saturn-terminal/engine/src/lifecycle/packet_select.rs`의 `single_needed_block_with_low_scores_elsewhere_is_kept_not_replaced` |
| RRF와 Jev 비교는 같은 보호 본문·후보·예산에서 도구 기록의 선택만 다르고, 요청한 방식과 실제 방식, 대체 사유, 생략 바이트, 넘친 바이트를 구분해 남긴다. | [맥락 고르기의 요구사항](context-selection.md#요구사항) 표의 `rank_and_jev_keep_the_same_basis_and_differ_only_in_the_selected_tool_ids` 등 |
| 패킷 판단은 전환 키가 같을 때만 적용하고, 다르면 한 번 다시 묻고, 또 다르면 판단 없이 진행한다. 기록이 늘어도 판단을 버리지 않는다. | `saturn-terminal/engine/src/packet_select.rs`의 `settle_applies_equal_keys_and_asks_again_once_when_they_differ`, `saturn-terminal/engine/src/lifecycle/packet_select.rs`의 `a_judgment_whose_transition_key_changed_is_discarded_and_rank_order_is_used`(판단 중 보내는 session이 끝나고 받는 provider가 바뀌어 두 판단이 `superseded`로 남고 순위 순서로 채운다) |
| 판단을 기다리는 동안 engine은 다른 요청에 지연 없이 응답한다. | `saturn-terminal/engine/src/lifecycle/packet_select.rs`의 `jev_order_fills_the_budget_with_the_blocks_the_router_wants`(판단 호출을 붙잡은 채 다른 접속의 `Version` 요청이 응답한다) |
| router가 시작한 전환은 판단이 실패하면 건너뛰고, 사용자가 고정한 전환은 순위 순서로 연다. | `saturn-terminal/engine/src/lifecycle/packet_select.rs`의 `a_failed_judgment_skips_only_the_switch_the_router_started` |
| 크기 한도를 넘는 state와 질문은 요청을 만들지 않고 순위 순서로 채운다. | `saturn-terminal/engine/src/lifecycle/packet_select.rs`의 `without_a_judgment_the_packet_is_filled_by_rank_order`(크기 한도 사례) |
| 제약 칸은 `C_max` 안에서 채우고 못 넣은 수를 표시한다. | [제약](constraints.md#요구사항)의 제약 칸 행 |
| 고정 구역이 `P_max`를 넘어도 대화 본문은 줄이거나 빼지 않고 경쟁 구역을 비운 채 `P_send`까지 허용하며, `P_send`도 넘으면 자르지 않고 보내지 않는다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_over_soft_limit_keeps_every_turn_whole_and_empties_competing`, `build_packet_dialogue_over_hard_limit_defers_instead_of_cutting`, `build_packet_fixed_over_limit_allows_hard_limit_without_competing`, `saturn-terminal/engine/src/lifecycle/dialogue_preserved.rs`의 `dialogue_over_the_target_budget_is_sent_whole_and_only_the_tools_are_dropped`, `dialogue_over_the_send_limit_is_not_applied_and_nothing_is_cut` |
| 실험 수집기가 `saturn-core`의 `packet` 예제(`cargo run -p saturn-core --example packet`)로 두 실험 설계의 입력에서 패킷을 만들고, router 판단이 없으면 RRF 순서로 채운다. | `saturn-terminal/core/examples/packet/tests.rs`의 `packet_stream_input_prints_packet_text`, `packet_scenarios_input_prints_json_with_packet_and_rrf_order`, `packet_without_judgments_fills_in_rrf_order`, `packet_judgments_put_low_probability_item_before_unanswered` |
| 고정 구역이 넘쳐도 provider 압축으로 대신하지 않는다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_fixed_over_hard_limit_defers_with_constraints` |
| provider가 맥락 한도로 거절하면 낮은 순 항목을 빼 한 번만 다시 보내고, 고정 구역과 대화 본문은 줄지 않는다. | `saturn-terminal/engine/src/lifecycle/packet_overflow.rs`의 `packet_overflow_rejection_resends_once_without_the_lowest_items`, `packet_overflow_reduction_keeps_the_fixed_zone`, `saturn-terminal/engine/src/lifecycle/dialogue_preserved.rs`의 `reduced_resend_keeps_the_same_dialogue_and_only_drops_tools`, `saturn-terminal/core/src/sessions/packet/tests.rs`의 `reduce_packet_drops_the_lowest_items_and_keeps_the_fixed_zone` |
| 맥락 정리로 여는 새 session도 거절되면 낮은 순 항목을 빼 한 번만 다시 열고, 줄인 패킷도 거절되면 옛 session을 그대로 두고 알린다. | `saturn-terminal/engine/src/lifecycle/packet_overflow.rs`의 `compaction_overflow_rejection_resends_once_without_the_lowest_items`, `compaction_overflow_after_the_reduced_resend_keeps_the_session_and_tells_the_user` |
| 줄인 패킷도 거절되거나 고정 구역만으로 목표를 넘으면 보내지 않고 멈춘 뒤 알린다. | `saturn-terminal/engine/src/lifecycle/packet_overflow.rs`의 `packet_overflow_after_the_reduced_resend_stops_and_tells_the_user`, `packet_overflow_with_only_the_fixed_zone_over_the_target_is_not_resent`, `saturn-terminal/core/src/sessions/packet/tests.rs`의 `reduce_packet_is_none_when_the_fixed_zone_alone_is_over_the_target` |
| Codex의 맥락 초과 거절을 다른 거절과 구별해 한도를 읽는다. | `saturn-terminal/engine/src/providers/codex/tests.rs`의 `rejected_turn_tells_context_overflow_from_other_rejections` |
| `provider` 모드에서는 compaction을 판정하지 않고 안전망 값을 넣지 않는다. | `provider` 모드의 실행 인자와 판정 기록을 확인한다. |
| 떠나는 provider의 압축 요약을 읽을 수 있으면 요약과 요약 뒤 기록으로 패킷을 만든다. | `saturn-terminal/core/src/sessions/packet/tests.rs`의 `build_packet_with_summary_puts_summary_first_in_competing_zone`, `build_packet_with_summary_over_competing_budget_falls_back_to_records`, `saturn-terminal/core/examples/packet/tests.rs`의 `packet_provider_mode_puts_summary_first_and_uses_records_after_it`, 전환 품질은 [#122](https://github.com/woonyong-choi/saturn/issues/122) |
| 사용자가 자동 압축 값을 정했으면 안전망 값을 넣지 않는다. | 사용자 설정에 자동 압축 값이 있을 때 안전망 인자가 빠지는지 확인한다. |
| compaction 방식은 provider 기본 압축보다 품질을 낮추지 않는다. | [#7](https://github.com/woonyong-choi/saturn/issues/7) |
| provider별 `T`와 방식이 품질을 지키며 토큰을 줄인다. | [#7](https://github.com/woonyong-choi/saturn/issues/7) |
| `H`가 subagent의 맥락 증가를 덮는다. | [#25](https://github.com/woonyong-choi/saturn/issues/25) |
| 판단 전체 패킷이 새 session의 이전 작업 질문 정답률을 패킷 없음보다 10%p 이상 높인다. | [새 패킷 규칙의 전환 품질 재측정](../experiments/handoff-packet-quality-v2/report.md): 정답률 차이 +51.4%p [45.2, 57.3]로 채택. 기준값 0.5 규칙은 [+8.4%p로 보류](../experiments/handoff-packet-quality/report.md)였다. |

## 단점

- provider별 `T_abs`를 따로 측정해야 한다.
- `T_abs`와 `N` 두 설정을 관리해야 한다.
- `A`를 잴 수 없는 경로는 provider 자동 압축에 기댄다.
- 정리 모드마다 전달 경로가 달라 두 경로를 함께 유지해야 한다.
- 고정 구역이 아주 길면 맥락 정리를 미루고 안전망에 기댄다.
- 항목마다 표기가 붙어 같은 예산에 드는 항목 수가 줄어든다. 순위 순서 패킷의 포함 항목은 평균 24.2개에서 20.2개로 줄었다. 날짜가 필요한 질문의 `router-all` 정답률은 `temporal` 0.0%에서 42.2%, `multi-session` 0.0%에서 78.1%로 올랐다([재측정 결과](../experiments/handoff-packet-quality-v2/report.md)).

## 대안

- 창 크기 대비 비율 `N%`만 쓰는 기준은 창이 큰 provider에서 맥락이 크게 쌓여 버렸다([결정 기록](../decisions/2026-09-29-absolute-token-budget.md)).
- 정한 순서로 쌓고 넘치면 뒤쪽 항목부터 줄이는 방식은 관련도와 무관하게 작은 도구 결과가 먼저 빠져 버렸다.
- Saturn 방식만 허용하는 방식은 provider 압축을 쓰려는 사용자 설정을 무시해 버렸다([결정 기록](../decisions/2026-10-01-context-mode-setting.md)).
- 남김 확률 0.5 이상만 경쟁 구역에 넣는 방식은 근거 항목의 88.0%를 버려 버렸다([결정 기록](../decisions/2026-10-02-fill-packet-by-probability.md)).
