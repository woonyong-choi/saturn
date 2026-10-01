# 맥락 정리

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [compaction 기준을 절대 토큰 예산으로 정한다](../decisions/2026-09-29-absolute-token-budget.md), [provider마다 입력을 계속 받는 상시 연결을 만든다](../decisions/2026-09-29-persistent-provider-connections.md), [provider 설정과 subagent 사용은 사용자 설정을 따르고 Saturn은 추적만 한다](../decisions/2026-09-29-minimal-provider-control.md) |

## 요약

맥락 정리(compaction)는 session의 맥락이 커졌을 때 줄여서 이어 가는 기능이다. engine은 턴이 끝날 때마다 맥락 크기를 기록하고, `core`의 `sessions`가 compaction 여부를 판정한다. compaction은 provider 압축을 요청하거나, Saturn 기록 원문에서 고른 패킷으로 새 session을 여는 두 방식이 있다. 그 사이 한 턴이 갑자기 커지는 경우는 provider 자동 압축이 안전망으로 받는다.

## 동기

session이 길어지면 맥락이 쌓여 토큰이 늘고 답의 품질이 떨어진다. 품질 저하는 창 크기 대비 비율이 아니라 맥락의 절대 길이를 따른다. provider 기본 압축에 맡기면 창이 큰 provider에서 맥락이 오래 쌓이고, 원격 압축 요약은 암호화되어 무엇이 남았는지 볼 수 없다. Saturn은 provider 기본 압축보다 적은 토큰으로 맥락을 유지하는 것을 목표로 한다. 이 기능이 없으면 사용자는 커진 맥락의 비용과 품질 저하를 그대로 떠안는다.

## 예시

### 맥락이 커져 새 session으로 이어 가기

1. Claude 메인 에이전트는 한 채팅에서 긴 작업을 여러 턴 이어 간다.
2. 턴이 끝날 때마다 engine은 활성 맥락 크기를 기록한다.
3. 활성 맥락 크기가 발동 기준에 닿았고, 트리 유휴이며 합칠 대기 입력이 없다.
4. 본전 턴 수가 기대 잔여 턴 이하이므로 `sessions`는 새 session으로 이어 가기로 판정한다.
5. `sessions`는 Saturn 기록 원문에서 패킷을 만들고, engine은 새 session에 패킷을 넘긴다.
6. engine은 옛 session을 닫고 채팅의 session 기록을 새 session으로 바꾼다.
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

## 상세 설계

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
| `r` | 캐시 읽기 배수 |
| `w` | 캐시 쓰기 배수 |
| `k*` | 본전 턴 수 |
| `H` | 턴당 맥락 증가량의 p95 |
| `T_hard` | provider 자동 압축 안전망 기준 |

### 맥락 크기 측정

`sessions`는 턴이 끝날 때마다 활성 맥락 크기 `A`를 잰다. `A`는 session에 쌓인 활성 맥락의 토큰 수다. engine은 턴이 끝날 때 `A`를 기록한다. 사용량 보고의 범위와 턴 값 계산은 [provider 연결과 session](providers-and-sessions.md)에 있다.

`A`를 잴 수 없으면 Saturn은 새 session을 열지 않고 provider 자동 압축에 맡긴다. 측정하지 못한 맥락으로 재시작을 판정하지 않기 위해서다.

### 발동 기준

발동 기준 `T`는 provider별 절대 토큰 기준 `T_abs`와 안전 비율 `N`으로 정한다.

```text
T = min(T_abs, N% × 창 크기)
```

`T_abs`는 provider마다 따로 정하는 토큰 수이고, `N`은 두 provider에 같이 쓰는 비율이다. `T`는 둘 중 작은 값이다. 품질 저하는 창 대비 비율이 아니라 절대 길이에 따라 생기기 때문이다([결정 기록](../decisions/2026-09-29-absolute-token-budget.md)). provider별 `T` 값은 실측으로 정한다([#7](https://github.com/woonyong-choi/saturn/issues/7)).

### compaction 판정

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

### 패킷 구성

패킷은 새 session에 넘기는 맥락 묶음이다. `sessions`는 다음 순서로 항목을 넣는다.

1. 사용자가 명시한 제약과 결정의 원문을 고정한다.
2. 현재 목표와 마지막 사용자 입력의 원문을 더한다.
3. 끝나지 않은 항목과 효과를 모르는 항목을 더한다.
4. 최근 3턴의 원문을 더한다.
5. 그 이전 도구 호출 중 judge의 `compact` 질문이 남기라고 한 것만 고른다.
6. 고른 도구 결과는 앞부분 300자와 한 줄 메모로 줄인다.
7. 파일은 내용 대신 경로만 더한다.
8. 패킷이 `P_max`를 넘으면 뒤쪽 항목부터 줄인다.

패킷 크기 상한 `P_max`는 발동 기준의 10분의 1이다.

```text
P_max = T / 10
```

8단계는 파일 경로, 도구 호출, 최근 턴, 미완 항목, 목표, 고정 항목 순서로 줄이고, 절 안에서는 끝 항목부터 줄인다. 마지막으로 줄이는 항목은 남은 크기만큼 앞부분을 남기고, 고정 항목도 `P_max`를 넘으면 줄인다. 패킷 크기가 `P_max`를 넘는 일이 없게 하기 위해서다. 패킷 크기는 네 글자를 한 토큰으로 추정한다.

이어 갈 내용은 provider 요약이 아니라 맥락의 정본인 Saturn 기록 원문에서 고른다. provider 원격 압축 요약은 암호화되어 보이지 않기 때문이다. provider 압축 요약은 읽을 수 있을 때만 도구 호출 항목의 보조 후보로 쓴다. 기록 원문을 정본으로 두기 위해서다. provider가 스스로 읽는 문서(`AGENTS.md`, `CLAUDE.md`)는 패킷에 넣지 않는다. 중복을 막기 위해서다.

`compact` 질문의 기준값과 judge가 답하지 못할 때의 대체 규칙은 [judge](judge.md)에 있다.

### compaction 방식

| 방식 | Codex | Claude Code |
|---|---|---|
| provider 압축 | `thread/compact/start` | `/compact` 전송 |
| 새 session | 패킷을 넘긴 새 session | 패킷을 넘긴 새 session |

1. 판정이 나면 `sessions`는 Saturn 기록 원문에서 패킷을 만든다.
2. engine은 provider별 방식으로 새 session에 패킷을 넘기거나 provider 압축을 요청한다.
3. session을 바꾸면 engine은 옛 session을 닫고 session 기록을 바꾼다.
4. TUI는 `맥락 정리 후 이어서 진행` 한 줄을 보인다.

session 교체는 턴이 끝난 경계에서만 한다. 교체 규칙은 [provider 연결과 session](providers-and-sessions.md)에 있다. provider마다 어느 방식을 쓸지는 품질을 지키면서 토큰이 적은 쪽을 실측으로 고른다([#7](https://github.com/woonyong-choi/saturn/issues/7)). 어느 방식도 provider 기본 압축보다 품질을 낮추지 않아야 한다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| `A` 측정 불가 | 새 session을 열지 않고 provider 자동 압축에 맡긴다. |
| 트리 유휴 전 | 판정을 다음 턴 경계까지 미룬다. |
| `A`에 합칠 대기 입력 존재 | 판정을 다음 턴 경계까지 미룬다. |
| 한 턴의 급증으로 `T` 초과 | `T_hard`에서 provider 자동 압축이 처리한다. |
| 패킷의 `P_max` 초과 | 뒤쪽 항목부터 줄인다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| compaction은 트리 유휴이고 합칠 대기 입력이 없을 때만 한다. | subagent가 남았거나 대기 입력이 있을 때 판정이 미뤄지는지 확인한다. |
| 유휴 복귀 조건과 기준 도달 조건에서만 새 session으로 이어 간다. | `A`, `T`, `P`, `k*`, 경과 시간 조합마다 판정 결과가 규칙과 같은지 확인한다. |
| 패킷 크기는 `T`의 10분의 1을 넘지 않는다. | 큰 기록으로 만든 패킷이 `P_max` 안에 들고, 뒤쪽 항목부터 줄었는지 확인한다. |
| 패킷 항목은 정한 순서로 들어간다. | 고정 항목, 목표, 미완 항목, 최근 3턴, 도구 결과, 파일 경로 순서인지 확인한다. |
| 사용자가 자동 압축 값을 정했으면 안전망 값을 넣지 않는다. | 사용자 설정에 자동 압축 값이 있을 때 안전망 인자가 빠지는지 확인한다. |
| compaction 방식은 provider 기본 압축보다 품질을 낮추지 않는다. | [#7](https://github.com/woonyong-choi/saturn/issues/7) |
| provider별 `T`와 방식이 품질을 지키며 토큰을 줄인다. | [#7](https://github.com/woonyong-choi/saturn/issues/7) |
| `H`가 subagent의 맥락 증가를 덮는다. | [#25](https://github.com/woonyong-choi/saturn/issues/25) |

## 단점

- provider별 `T_abs`를 따로 측정해야 한다.
- `T_abs`와 `N` 두 설정을 관리해야 한다.
- `A`를 잴 수 없는 경로는 provider 자동 압축에 기댄다.

## 대안

- 창 크기 대비 비율 `N%`만 쓰는 기준은 창이 큰 provider에서 맥락이 크게 쌓여 버렸다([결정 기록](../decisions/2026-09-29-absolute-token-budget.md)).

## 미해결 질문

- `A`를 루트 에이전트 메시지로만 계산할지, 마지막으로 보고된 메시지로 계산할지 ([#62](https://github.com/woonyong-choi/saturn/issues/62))
- 도구 출력 자르기를 패킷을 만들 때만 할지, provider 훅으로 실행 중에 할지 ([#37](https://github.com/woonyong-choi/saturn/issues/37))
