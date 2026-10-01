# 맥락 정리

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [compaction 기준을 절대 토큰 예산으로 정한다](../decisions/2026-09-29-absolute-token-budget.md), [provider마다 입력을 계속 받는 상시 연결을 만든다](../decisions/2026-09-29-persistent-provider-connections.md), [provider 설정과 subagent 사용은 사용자 설정을 따르고 Saturn은 추적만 한다](../decisions/2026-09-29-minimal-provider-control.md), [맥락 정리는 Saturn 방식을 기본으로 두고 provider 압축 모드를 설정으로 연다](../decisions/2026-10-01-context-mode-setting.md), [경쟁 구역은 기준값 없이 judge 남김 확률 순으로 예산까지 채운다](../decisions/2026-10-02-fill-packet-by-probability.md) |

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

### provider 모드에서 Claude 요약으로 Codex에 넘기기

1. 사용자는 정리 모드를 `provider`로 두고 Claude로 작업한다.
2. 맥락이 커지자 Claude 자동 압축이 맥락을 줄이고, engine은 압축 요약과 그 시점의 기록 번호를 기록한다.
3. 사용자가 Codex로 바꾸면 `sessions`는 고정 구역, Claude 압축 요약, 요약 시점 뒤 기록에서 고른 항목으로 패킷을 만든다.
4. 반대로 Codex에서 Claude로 바꿀 때는 Codex 요약을 볼 수 없으므로 Saturn 기록 원문으로 패킷을 만든다.

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
| `P_hard` | 고정 구역이 넘칠 때만 쓰는 패킷 크기 하드 상한 |
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
- Claude 압축 요약은 로컬 기록으로 읽고, Codex 원격 압축 요약은 암호화되어 읽지 않는다. Claude 압축 요약을 읽는 경로와, 그 요약과 Saturn 패킷의 전환 품질은 실측으로 확인한다([#122](https://github.com/woonyong-choi/saturn/issues/122)).

### 패킷 구성

패킷은 새 session에 넘기는 맥락 묶음이다. `sessions`는 패킷을 고정 구역과 경쟁 구역으로 나눠 채운다.

| 구역 | 항목 | 넣는 규칙 |
|---|---|---|
| 고정 구역 | 1. 사용자가 명시한 제약과 결정의 원문 2. 현재 목표와 마지막 사용자 입력의 원문 3. 끝나지 않은 항목과 효과를 모르는 항목 4. 최근 3턴의 대화(사용자 입력 원문과 에이전트 답 글) | 이 순서로 모두 넣는다 |
| 경쟁 구역 | 이 채팅의 도구 호출과 결과(최근 3턴 포함), 다른 에이전트 결과 요약, 파일 경로 | 남은 예산 안에서 고른 순서대로 넣는다 |

- 1단계의 제약은 대체된 제약을 빼고 넣는다. 제약 식별과 대체는 [맥락 고르기](context-selection.md)에 있다.
- 4단계에는 도구 결과를 넣지 않는다. 최근 턴의 큰 도구 결과가 예산을 혼자 차지하지 않게 하기 위해서다.
- 경쟁 구역의 순서는 `compact` 판단으로 정한다. 후보 전체를 묻고, 항목마다 호출과 결과 중 큰 남김 확률을 쓴다. 확률이 높은 순으로, 같은 확률이면 [맥락 고르기](context-selection.md)의 후보 순위 순으로 둔다. judge가 답하지 못한 항목은 후보 순위 순으로 뒤에 둔다.
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
- 패킷에 쓸 때는 구역마다 기록 번호 순서로 다시 정렬한다. 순위는 무엇을 넣을지만 정하고, 새 session은 일이 일어난 순서를 알아야 하기 때문이다.
- 최근 턴과 경쟁 구역은 session별 제목 아래에 묶고 항목마다 기록 번호와 시각을 적는다. 형식은 [패킷 표기](#패킷-표기)에 있다.

고정 구역만으로 `P_max`를 넘으면 `sessions`는 다음 순서로 줄인다.

1. 최근 3턴 중 오래된 턴부터 에이전트 답을 앞부분 300자만 남긴다.
2. 최근 턴 수를 3, 2, 1로 줄인다.
3. 이 패킷에 한해 `P_hard = T / 5`(초안)까지 허용하고 초과를 기록한다. 경쟁 구역은 비운다. 허용한 초과분을 고정 구역에만 쓰기 위해서다.
4. `P_hard`도 넘으면 새 session으로 옮기지 않고 그대로 이어 가며, TUI에 `고정 제약이 길어 맥락 정리를 미룹니다`와 제약 목록을 보인다.

고정 구역이 넘칠 때 provider 압축으로 대신하지 않는다. provider가 들고 있는 맥락과 Saturn 기록이 달라지고, Codex 원격 압축 요약은 무엇이 남았는지 볼 수 없기 때문이다. 4단계 뒤 맥락이 `T_hard`를 넘으면 provider 자동 압축 안전망이 받는다.

`saturn` 모드에서 이어 갈 내용은 provider 요약이 아니라 맥락의 정본인 Saturn 기록 원문에서 고른다. provider 압축 요약은 읽을 수 있을 때만 경쟁 구역의 후보 하나로 쓴다. 기록 원문을 정본으로 두기 위해서다. `provider` 모드에서 Claude 압축 요약을 넘길 때는 그 요약을 경쟁 구역의 첫 항목으로 두고, 나머지 경쟁 구역은 요약 시점 뒤의 기록에서 고른다. 요약이 경쟁 구역 예산을 넘으면 요약을 쓰지 않고 Saturn 기록 원문으로 채운다(`build_packet_with_summary`). 어느 모드든 고정 구역은 Saturn 기록 원문으로 넣고, 패킷에 제약이 요약보다 우선한다고 적는다. 요약이 대체되기 전의 제약을 담고 있을 수 있기 때문이다. provider가 스스로 읽는 문서(`AGENTS.md`, `CLAUDE.md`)는 패킷에 넣지 않는다. 중복을 막기 위해서다.

`compact` 판단은 패킷을 만들 때 한 번 묻는다. 턴마다 미리 묻지 않는다. 나눈 요청을 병렬로 보내면 후보 전체 판단의 지연이 조각 수와 거의 무관하게 약 0.3초이기 때문이다. 질문 40개와 약 7KB state 요청을 4개 동시에 보내면 0.26초, 8개 동시에 보내면 0.27초, 4개를 차례로 보내면 0.97초였다([#179](https://github.com/woonyong-choi/saturn/issues/179) 실측). 미리 판단은 후보 전체를 묻는 조건에서 패킷 때 물을 질문을 앞당길 뿐이다. 첫 턴부터 미리 판단하면 추정 입력 토큰이 1.118배 [1.088, 1.161]로 늘고, 사건에 닿지 않아 버려지는 질문이 9.7% [7.3, 13.1]이며, 미리 한 판단이 패킷 때 목표와 어긋날 위험이 남는다. 지연을 줄이는 이득은 순차 전송을 가정한 값이라 병렬 전송에서는 사라진다([후속 분석](../experiments/precompute-breakeven/report.md#후속-분석-후보-전체-판단-조건)). 이 기준은 사전 등록 판정이 아니라 설계 판단용이다.

`compact` 질문의 확률 읽기와 judge가 답하지 못할 때의 대체 규칙은 [judge](judge.md)에 있다.

### 패킷 표기

최근 턴과 경쟁 구역은 session별 제목 아래에 묶고, 항목마다 기록 번호와 기록 시각을 적는다. 패킷 96개에 날짜, session, 기록 번호 표시가 없어 날짜가 필요한 질문(384개 중 96개, 25.0%)은 근거가 모두 들어도 풀 수 없었고 `multi-session`과 `temporal` 질문은 세 조건 모두 정답률이 0%였기 때문이다([후속 분석](../experiments/handoff-packet-quality/report.md#후속-분석-원인)).

```text
## Recent turns

### Session 2

#190 2026-09-13T09:00Z User: finish the cache module
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
- 최근 턴은 사용자 입력의 기록 번호와 시각을 적는다. 제약, 목표와 마지막 입력, 끝나지 않은 항목은 표기를 붙이지 않는다.
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

session 교체는 턴이 끝난 경계에서만 한다. 교체 규칙은 [provider 연결과 session](providers-and-sessions.md)에 있다. provider마다 어느 방식을 쓸지는 품질을 지키면서 토큰이 적은 쪽을 실측으로 고른다([#7](https://github.com/woonyong-choi/saturn/issues/7)). 어느 방식도 provider 기본 압축보다 품질을 낮추지 않아야 한다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| `A` 측정 불가 | 새 session을 열지 않고 provider 자동 압축에 맡긴다. |
| 트리 유휴 전 | 판정을 다음 턴 경계까지 미룬다. |
| `A`에 합칠 대기 입력 존재 | 판정을 다음 턴 경계까지 미룬다. |
| 한 턴의 급증으로 `T` 초과 | `T_hard`에서 provider 자동 압축이 처리한다. |
| 경쟁 구역 예산 부족 | 고른 순서대로 원문, 축약본, 경로 중 들어가는 형태를 넣고 나머지는 건너뛴다. |
| 고정 구역의 `P_max` 초과 | 최근 턴을 줄이고, 그래도 넘치면 `P_hard`까지 허용한다. |
| 고정 구역의 `P_hard` 초과 | 새 session으로 옮기지 않고 사용자에게 제약 목록을 보인다. |
| 떠나는 provider의 압축 요약을 읽을 수 없음 | Saturn 기록 원문으로 패킷을 만든다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| compaction은 트리 유휴이고 합칠 대기 입력이 없을 때만 한다. | subagent가 남았거나 대기 입력이 있을 때 판정이 미뤄지는지 확인한다. |
| 유휴 복귀 조건과 기준 도달 조건에서만 새 session으로 이어 간다. | `A`, `T`, `P`, `k*`, 경과 시간 조합마다 판정 결과가 규칙과 같은지 확인한다. |
| 고정 구역이 넘치지 않으면 패킷 크기는 `T`의 10분의 1을 넘지 않는다. | `saturn-terminal/core/src/sessions/packet.rs`의 `build_packet_large_record_stays_within_packet_limit` |
| 고정 구역은 정한 순서로 모두 들어가고 최근 턴에는 도구 결과가 없다. | `saturn-terminal/core/src/sessions/packet.rs`의 `build_packet_fixed_zone_in_order_and_tool_results_only_in_competing` |
| 경쟁 구역은 기준값 없이 judge 남김 확률 순으로 예산이 찰 때까지 채운다. | `saturn-terminal/core/src/sessions/ranking.rs`의 `order_after_judge_orders_by_probability_and_keeps_low`, `saturn-terminal/core/examples/packet/tests.rs`의 `packet_judgments_put_low_probability_item_before_unanswered` |
| 경쟁 구역은 고른 순서대로 원문, 축약본, 경로 중 들어가는 형태로 채운다. | `saturn-terminal/core/src/sessions/packet.rs`의 `build_packet_small_top_item_goes_raw_before_large_lower_item`, `build_packet_fills_competing_in_chosen_order_raw_then_digest`, `build_packet_falls_back_to_path_then_skips` |
| 한 항목은 경쟁 구역 예산의 30%를 넘는 원문으로 들어가지 않는다. | `saturn-terminal/core/src/sessions/packet.rs`의 `build_packet_item_over_cap_goes_as_digest` |
| 최근 턴과 경쟁 구역은 session별 제목 아래에 묶고 항목마다 기록 번호와 기록 시각을 적는다. | `saturn-terminal/core/src/sessions/packet.rs`의 `build_packet_competing_groups_by_session_with_seq_and_time`, `build_packet_recent_turns_carry_session_title_seq_and_time`, `saturn-terminal/core/src/sessions/stamp.rs`의 `label_formats_seq_and_utc_minute`, `saturn-terminal/core/examples/packet/tests.rs`의 `packet_scenario_session_and_time_appear_before_items` |
| 시각이 없는 입력은 기록 번호만 적는다. | `saturn-terminal/core/src/sessions/packet.rs`의 `build_packet_without_time_writes_seq_only`, `saturn-terminal/core/src/sessions/stamp.rs`의 `label_without_time_is_seq_only` |
| 표기와 session 제목의 글자도 경쟁 구역 예산에 들어 패킷은 `P_max`를 넘지 않는다. | `saturn-terminal/core/src/sessions/packet.rs`의 `build_packet_many_sessions_stay_within_packet_limit` |
| 패킷은 구역마다 기록 번호 순서로 쓴다. | `saturn-terminal/core/src/sessions/packet.rs`의 `build_packet_writes_competing_in_seq_order` |
| `compact` 질문은 패킷을 만들 때 후보 전체를 한 번 judge에 보내고 턴이 끝날 때는 보내지 않는다. | 턴 종료에서 judge 호출이 없는지, 패킷을 만들 때 후보 전체가 한 번 묻히는지 확인한다. |
| 고정 구역이 `P_max`를 넘으면 오래된 턴의 답부터 줄이고, 최근 턴 수를 줄인 뒤 `P_hard`까지 허용한다. | `saturn-terminal/core/src/sessions/packet.rs`의 `build_packet_fixed_overflow_trims_oldest_answer_first`, `build_packet_fixed_overflow_drops_oldest_turns`, `build_packet_fixed_over_limit_allows_hard_limit_without_competing` |
| 실험 수집기가 `saturn-core`의 `packet` 예제(`cargo run -p saturn-core --example packet`)로 두 실험 설계의 입력에서 패킷을 만들고, judge 판단이 없으면 RRF 순서로 채운다. | `saturn-terminal/core/examples/packet/tests.rs`의 `packet_stream_input_prints_packet_text`, `packet_scenarios_input_prints_json_with_packet_and_rrf_order`, `packet_without_judgments_fills_in_rrf_order`, `packet_judgments_drop_item_judge_rejected` |
| 고정 구역이 넘쳐도 provider 압축으로 대신하지 않는다. | `saturn-terminal/core/src/sessions/packet.rs`의 `build_packet_fixed_over_hard_limit_defers_with_constraints` |
| `provider` 모드에서는 compaction을 판정하지 않고 안전망 값을 넣지 않는다. | `provider` 모드의 실행 인자와 판정 기록을 확인한다. |
| 떠나는 provider의 압축 요약을 읽을 수 있으면 요약과 요약 뒤 기록으로 패킷을 만든다. | `saturn-terminal/core/src/sessions/packet.rs`의 `build_packet_with_summary_puts_summary_first_in_competing_zone`, `build_packet_with_summary_over_competing_budget_falls_back_to_records`, `saturn-terminal/core/examples/packet/tests.rs`의 `packet_provider_mode_puts_summary_first_and_uses_records_after_it`, 전환 품질은 [#122](https://github.com/woonyong-choi/saturn/issues/122) |
| 사용자가 자동 압축 값을 정했으면 안전망 값을 넣지 않는다. | 사용자 설정에 자동 압축 값이 있을 때 안전망 인자가 빠지는지 확인한다. |
| compaction 방식은 provider 기본 압축보다 품질을 낮추지 않는다. | [#7](https://github.com/woonyong-choi/saturn/issues/7) |
| provider별 `T`와 방식이 품질을 지키며 토큰을 줄인다. | [#7](https://github.com/woonyong-choi/saturn/issues/7) |
| `H`가 subagent의 맥락 증가를 덮는다. | [#25](https://github.com/woonyong-choi/saturn/issues/25) |
| 판단 전체 패킷이 새 session의 이전 작업 질문 정답률을 패킷 없음보다 10%p 이상 높인다. | [패킷 전환 품질 실험 결과](../experiments/handoff-packet-quality/report.md): 정답률 차이 +8.4%p [6.1, 10.8]로 보류 |

## 단점

- provider별 `T_abs`를 따로 측정해야 한다.
- `T_abs`와 `N` 두 설정을 관리해야 한다.
- `A`를 잴 수 없는 경로는 provider 자동 압축에 기댄다.
- 정리 모드마다 전달 경로가 달라 두 경로를 함께 유지해야 한다.
- 제약이 아주 길면 맥락 정리를 미루고 안전망에 기댄다.
- 항목마다 표기가 붙어 같은 예산에 드는 항목 수가 줄어든다. 줄어드는 정도와 날짜가 필요한 질문의 정답률 변화는 측정 전이다.

## 대안

- 창 크기 대비 비율 `N%`만 쓰는 기준은 창이 큰 provider에서 맥락이 크게 쌓여 버렸다([결정 기록](../decisions/2026-09-29-absolute-token-budget.md)).
- 정한 순서로 쌓고 넘치면 뒤쪽 항목부터 줄이는 방식은 관련도와 무관하게 작은 도구 결과가 먼저 빠져 버렸다.
- Saturn 방식만 허용하는 방식은 provider 압축을 쓰려는 사용자 설정을 무시해 버렸다([결정 기록](../decisions/2026-10-01-context-mode-setting.md)).
- 남김 확률 0.5 이상만 경쟁 구역에 넣는 방식은 근거 항목의 88.0%를 버려 버렸다([결정 기록](../decisions/2026-10-02-fill-packet-by-probability.md)).

## 미해결 질문

- 도구 출력 자르기를 패킷을 만들 때만 할지, provider 훅으로 실행 중에 할지 ([#37](https://github.com/woonyong-choi/saturn/issues/37))
