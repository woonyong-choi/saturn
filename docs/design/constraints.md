# 제약

| 항목 | 값 |
|---|---|
| 상태 | 결정 |
| 관련 결정 | [제약은 원문에서 자른 규칙 한 줄과 적용 범위로 저장하고 확신이 낮으면 사용자에게 묻는다](../decisions/2026-10-04-constraints-as-rule-lines.md), [제약 해제와 예외는 등록 기록을 본 사용자의 입력을 Jev로 판단한다](../decisions/2026-10-04-constraint-release-exception-judge.md), [경쟁 구역은 기준값 없이 router 남김 확률 순으로 예산까지 채운다](../decisions/2026-10-02-fill-packet-by-probability.md) |

## 요약

제약은 사용자가 앞으로도 지키라고 한 말이다. engine은 입력마다 router(Jev)로 제약인지 판단해 채팅의 제약 표에 규칙 한 줄과 적용 범위로 저장한다. 확신이 높으면 자동으로 등록하고, 중간이면 그 자리에서 사용자에게 묻고, 낮으면 등록하지 않는다. 등록은 대화 기록에 `제약 등록됨` 줄로 남는다. 사용자가 그 줄을 보고 `이번만 풀어`, `이 규칙은 없애`처럼 말하면 router가 해제인지 예외인지, 어느 제약인지 판단해 해제하거나 예외를 건다. 새 session에 넘기는 패킷의 제약 칸은 상한 안에서 채운다. 모든 변경은 대화 기록에 한 줄로 남고 `/constraints`에서 되돌릴 수 있다.

## 동기

긴 채팅에서 사용자는 "에러 메시지는 영어로 통일해"처럼 한 번 한 말이 끝까지 지켜지길 기대한다. provider를 바꾸거나 맥락을 정리해 새 session을 열면 그 말은 수백 턴 앞에 있어 새 session이 모른다. 제약을 입력 원문 그대로 패킷에 넣으면 쌓일수록 패킷의 자리를 차지하고, 사용자가 뒤집은 제약이 옛 제약과 나란히 들어가면 새 session이 어느 쪽을 따를지 모른다. 반대로 Saturn이 제약을 사용자 몰래 등록하거나 지우면 사용자는 무엇이 지켜지는지 알 수 없다. 이 기능은 제약을 짧게 저장하고, 상태가 바뀔 때마다 보이게 하고, 판단이 불확실하면 사용자에게 묻는다.

## 예시

### 제약이 등록되고 해제될 때

1. 사용자가 `에러 메시지는 영어로 통일해`를 입력한다.
2. engine이 입력을 접수하고 `route` 요청에 `is_constraint`를 입력 처리 질문과 함께 묻는다.
3. router가 `is_constraint`에 0.9를 답한다. 자동 등록 기준 0.8 이상이다. 입력은 평소대로 에이전트로 간다.
4. `store`가 규칙 한 줄 `에러 메시지는 영어로 통일해`와 범위 `전체`를 유효 제약으로 저장한다.
5. 대화 기록에 `제약 등록됨 · 에러 메시지는 영어로 통일해`가 한 줄 남는다.
6. 사용자가 나중에 `/constraints`를 열어 그 제약에서 `d`를 눌러 해제한다. 대화 기록에 `제약 해제됨 · 에러 메시지는 영어로 통일해`가 남는다.
7. 사용자가 변경 내역에서 해제 줄을 골라 `u`를 누르면 제약이 유효로 돌아오고 `제약 되돌림 · 에러 메시지는 영어로 통일해`가 남는다.

### 확신이 중간이라 그 자리에서 묻기

1. 사용자가 `테스트 메시지는 한국어로 둬`를 입력한다.
2. router가 `is_constraint`에 0.75를 답한다. 0.8에는 못 미치고 `constraint_ask` 0.7 이상이다.
3. engine은 제약을 `Candidate`로 저장하고 TUI에 `이 말을 앞으로 지킬 제약으로 등록할까요?` 창을 띄운다. 작업은 계속된다. 답이 오기 전에 전환 패킷을 만들면 이 제약도 유효 제약처럼 들어간다.
4. 사용자가 `등록`을 고르면 제약이 `Active`가 되고 `제약 등록됨 · 테스트 메시지는 한국어로 둬`가 남는다. `등록 안 함`을 고르면 `Released`가 되고 대화 기록에는 아무것도 남지 않는다.

### 등록 줄을 보고 이번 작업만 풀기

1. 제약 `에러 메시지는 영어로 통일해`가 유효하다. 사용자가 `이번 작업에서만 한국어로 써줘`를 입력한다.
2. router가 선택형 질문 하나로 요청 여부, 대상, 종류를 답한다. 해제 요청 확률이 0.9, 대상은 그 제약, 종류는 `이번 작업 동안`이다.
3. 제약은 지워지지 않고 그 작업 동안만 멈춘다. 대화 기록에 `제약 잠시 해제됨 · 에러 메시지는 영어로 통일해 · 이번 작업 동안`이 남는다.
4. 작업이 끝나면 예외가 사라지고 `제약 다시 유효 · 에러 메시지는 영어로 통일해`가 남는다.
5. router가 종류를 잘못 읽었다면(사용자는 영구 해제를 뜻했다) 사용자가 `/constraints`에서 예외 줄을 골라 `e`로 종류를 `영구 해제`로 바꾼다. 제약이 `Released`가 되고 `제약 해제됨 · 에러 메시지는 영어로 통일해`가 남는다.

### 제약이 많아 제약 칸이 넘칠 때

1. 유효 제약 60개가 쌓인 채팅에서 사용자가 Codex로 바꾼다.
2. `sessions`는 제약 칸 상한 안에서 범위가 `전체`인 제약, 지금 작업 파일과 범위가 겹치는 제약, 마지막 입력과 관련도가 높은 제약 순으로 채운다.
3. 들어가지 못한 제약 7개는 패킷에 `Constraints omitted: 7` 한 줄로 남고, 대화 기록에 `제약 7개 생략 · /constraints에서 확인하세요`가 남는다.
4. `store`는 전환마다 어떤 제약이 어느 단계로 들어갔고 어떤 제약이 빠졌는지 기록한다.

## 상세 설계

### 제약의 모양

제약 한 건은 규칙 한 줄, 적용 범위, 상태로 이루어진다. 저장 열은 [기록 저장과 보존](records.md#기록-저장소)에 있다.

- 규칙 한 줄은 입력 원문에서 코드가 자른 글이고 요약하거나 생성하지 않는다. 원문을 정본으로 두고 뜻이 바뀌지 않게 하기 위해서다. 입력 원문은 `inputs`에 그대로 있고 제약은 그 입력과 조각 번호를 가리킨다.
- 입력이 한 문장이거나 200자(초안) 이하이면 입력 전체가 규칙 한 줄이다.
- 입력이 200자를 넘고 여러 문장이면 `sessions`가 줄바꿈, 문장 끝(`.`, `?`, `!`, `。`)에서 최대 20개(초안) 문장으로 나눈다. 코드 블록과 인용 줄은 나누기 전에 뺀다. engine이 문장마다 `line_<k>_is_constraint`를 묻고, `constraint_ask` 이상인 문장마다 제약 한 건을 만든다. 일과 제약이 섞인 입력의 패킷 낭비를 줄이기 위해서다. 자동 등록인지 묻기인지는 입력 전체의 `is_constraint`가 정한다.
- 문장 나누기 질문이 실패하거나 문장이 20개를 넘으면 입력 전체 원문을 한 건으로 등록한다. 입력 전체가 등록 후보인데 `constraint_ask` 이상인 문장이 하나도 없을 때도 같다. 제약을 놓치는 것보다 길게 넣는 쪽이 안전하기 때문이다. 문장 나누기 질문은 입력 전체 판단 뒤 별도 요청으로 보내고, 답이 오기 전에 입력이 취소되면 등록하지 않는다.
- 적용 범위는 `전체`이거나 경로 목록이다. `sessions`가 규칙 글에서 경로 모양 글자를 뽑고 [순위 채널](context-selection.md#순위-채널)과 같은 정규화(NFC, 앞의 `./` 제거)를 거친다. 경로가 없으면 `전체`다(초안). `테스트에서는`처럼 경로 없는 범위 표현은 `전체`로 두고, 범위 제약 판단 품질은 긴 대화 실측으로 확인한다.
- 새 제약이 앞 제약과 같은 뜻이거나 부딪쳐도 자동으로 합치거나 대체하지 않는다. 둘 다 유효로 남고, 정리는 사용자가 해제 요청이나 `/constraints`로 한다.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="../assets/constraint-lifecycle.ko.dark.svg">
  <img src="../assets/constraint-lifecycle.ko.light.svg" alt="제약은 확신에 따라 등록되거나 묻기를 거쳐 Active가 되고, 해제와 예외로 상태가 바뀐다" width="100%">
</picture>

| 상태 | 뜻 | 다음 상태 |
|---|---|---|
| `Candidate` | 등록할지 사용자에게 묻는 중이다. 패킷에는 유효 제약처럼 들어간다. | `Active`, `Released` |
| `Active` | 유효하다. 패킷의 제약 칸 후보다. | `Released` |
| `Released` | 사용자나 판단이 해제했거나 등록을 거절했다. 원문은 기록에 남고 패킷에는 들어가지 않는다. | `Active`(되돌리기) |

예외는 상태가 아니라 `Active` 제약에 붙는 별도 기록이다. 제약을 지우지 않고 일부 범위에서만 멈추기 위해서다.

| 예외 종류 | 뜻 | 끝나는 때 |
|---|---|---|
| `Once` | 그 작업 하나 동안 제약을 멈춘다 | 작업이 끝나면(완료, 실패, 멈춤) 사라지고 제약이 다시 유효해진다. 보류 중에는 유지한다(초안) |
| `Scoped` | 사용자가 말한 조건이나 범위에서만 멈춘다. 조건 문장은 입력 원문의 연속된 글이다 | 사용자가 지울 때까지. 기한과 조건 충족은 Saturn이 판정하지 않는다 |

- 제약 행은 지우지 않는다. 채팅을 지울 때만 함께 지운다. 변경 내역과 되돌리기가 원문 정본 위에서 이어지게 하기 위해서다.
- `/record off` 채팅에서도 제약은 저장한다. 판단 기록만 생략하고 제약은 패킷에 필요한 채팅 상태이기 때문이다.
- 이관 전 채팅에는 제약이 없다. 옛 입력을 소급해 판단하지 않는다.

### 식별 순서

1. engine이 입력을 기록 저장소에 접수한다.
2. 판단 차례에 engine이 `route@1.1` 요청으로 `is_constraint`를 입력 처리 질문과 함께 묻는다. 자동 적용이 켜져 있고 유효 제약이 있고 입력이 등록 대상이 아니면, 그 판단을 기록한 뒤 `constraint@1.0` 요청으로 `constraint_change`를 별도 작업으로 묻는다. 입력 처리는 이 요청을 기다리지 않는다.
3. 입력 처리 질문은 채팅 revision으로, 제약 질문은 제약 revision과 입력 상태로 따로 적용 직전에 비교한다([적용 직전 비교](#적용-직전-비교)).
4. 자동 적용(`constraint.auto_apply`)이 꺼져 있으면(기본) 이 단계에서 끝난다. `is_constraint` 답은 판단 기록에만 남고 제약 표, 사용자 질문, 문장 나누기 호출로 이어지지 않는다. 켜져 있으면 다음을 따른다. `is_constraint`가 `is_constraint` 기준값 0.8 이상이면 자동으로 등록한다. `constraint_ask` 0.7 이상 0.8 미만이면 `Candidate`로 저장하고 사용자에게 묻는다. 0.7 미만이면 등록하지 않는다.
5. 등록 후보가 긴 입력이면 문장 나누기 질문으로 규칙 한 줄을 정한다.
6. 입력이 등록 대상이 아니면(`is_constraint`가 `constraint_ask` 미만) `constraint_change`의 답을 쓴다. 요청 확률이 `constraint_release` 0.8 이상이면 해제나 예외를 적용하고, 그 미만이면 아무것도 하지 않는다. router 제안은 `constraint.auto_apply`가 꺼져 있으면(기본) 질문도 적용도 하지 않는다. 권한 모드 `full`도 이 정책을 바꾸지 않는다. 사용자의 명시 해제(`ReleaseConstraint`)는 이 설정과 권한 모드를 보지 않고 적용하며 주체를 `User`로 남긴다.
7. `store`가 판단 결과를 한 거래로 적용하고 이벤트마다 대화 기록 한 줄을 남긴다.

- 자동 적용은 사용자 설정 `constraint.auto_apply`(기본 거짓, 폴더 층에서 바꿀 수 없음)로 정한다. 끄면 router 답이 지속 제약을 만들지 못하지만 입력 원문은 작업 LLM에 그대로 가고 대화 기록과 패킷의 최근 입력, 목표 발췌로 현재 작업과 인계 맥락에 남는다. 작업 하나에만 해당하는 지시와 질문형, 인용, 붙여 넣은 지시문도 같다. 미확정을 사용자에게 묻지 않는 것은 입력마다 정답을 요구하지 않기 위해서다. 권한 모드 `full`은 이 정책을 바꾸지 않는다. `full`은 켠 상태에서 묻는 구간을 묻지 않고 등록하는 규칙일 뿐 판단 검증을 대신하지 않는다.
- 자동 적용을 기본으로 끈 것은 Jev만으로 자동 등록하는 근거가 부족해서다. 사람이 아닌 모델 합의 라벨(#382의 1,000건)로 잰 정밀도는 정답 정의에 따라 91.3%에서 55.6%, 작업 범위를 명시한 400건 재측정에서 30.2~66.7%로 갈렸고 독립 정답으로 평가한 값이 없다. 독립 평가를 통과하기 전에는 `is_constraint`, `constraint_ask` 값을 제품 기본 적용의 근거로 쓰지 않는다.
- 끄거나 켜는 것은 이미 저장한 제약을 바꾸지 않는다. 켜져 있을 때 등록된 `Active`와 `Candidate`, 답을 기다리는 확인은 끈 뒤에도 그대로 남아 패킷에 들어가고 사용자가 답하거나 해제할 수 있다. 입력은 접수 때 고정한 설정 번호의 값을 쓴다.
- `is_constraint`는 세 조건을 모두 채우는 입력이다. 사용자가 한 말이고, 이번 요청 하나가 아니라 앞으로도 적용되고, 무엇을 할지가 아니라 언어, 도구, 형식, 금지처럼 어떻게 할지를 제한한다.
- 등록 기준은 Jev `is_constraint`를 Astra 판정 868턴에 대해 잰 곡선으로 정했다. 0.8 이상은 정밀도 91.3% [87.6, 94.0]이다. 0.7 이상 0.8 미만은 입력의 12.6%이고 그중 67.0%가 진짜 제약이며, 자동과 묻기를 합친 재현율은 77.2%다. 0.7 미만은 정밀도가 84.8% 아래로 내려간다([등록 기준값의 사람 확인](../experiments/constraint-human-check/report.md)). 정답 라벨에 gpt-5.6-luna를 쓴 앞 수치는 사람 판정과 맞은 것이 Luna 5/24, Astra 21/24여서 근거로 약하다([프로젝트 대화 전수 측정](../experiments/constraint-deep/report.md)).
- 합성 입력에서는 0.7이 정밀도 91.6% [84.8, 95.5], 재현율 93.3% [86.9, 96.7]였다([제약 식별 정확도](../experiments/constraint-judge-accuracy/report.md)). 앞 말을 가리키는 간접 지시 입력은 정확도 74.5% [68.0, 80.0]이고 앞 입력의 등록 여부를 state에 넣어도 오르지 않았다([간접 지시 정확도](../experiments/indirect-constraint-accuracy/report.md)).
- 제약 판단을 입력 처리 판단과 같은 시점에 보내는 것은 입력 처리를 기다리게 하지 않기 위해서다. 등록 판단은 입력 전송을 기다리지 않는다. 입력이 전환을 일으켜도 마지막 입력 원문은 패킷의 고정 구역에 들어가므로 등록 전에 패킷을 만들어도 이 입력의 제약을 잃지 않는다.
- 등록이 끝나기 전에 입력이 취소되면 등록하지 않는다. 등록 뒤에 취소되면 그 입력의 제약을 `Released`(`InputCanceled`)로 바꾸고 해제 줄을 남긴다. 사용자가 거둔 말이 제약으로 남지 않게 하기 위해서다.
- router를 부르지 않는 입력(바로 보내기, `Tab` 대기)은 제약 판단을 받지 않는다.
- 사용자가 모델을 고정한 입력은 다른 입력과 같이 제약 판단을 받는다.
- router가 없거나 실패하면 제약 판단을 모두 건너뛴다. 입력은 평소대로 처리하고 미등록, 미해제로 둔다.

### 해제와 예외 판단

사용자가 `제약 등록됨` 줄을 본 뒤 하는 말이 그 제약을 풀려는 것인지, 어느 제약인지, 어떻게 풀려는지를 선택형 질문 하나로 묻는다. 판단은 Jev(router)가 한다. LLM으로 바꾸지 않는다.

| 질문 | 질문 세트 | 묻는 때 | 자동 | 묻기 |
|---|---|---|---|---|
| `is_constraint` | `route@1.1` | 입력마다 | `is_constraint` 0.8 이상 등록 | `constraint_ask` 이상 0.8 미만 |
| `line_<k>_is_constraint` | `constraint@1.0` | 긴 입력의 문장마다 | `constraint_ask` 이상 문장을 규칙으로 | 없음 |
| `constraint_change` | `constraint@1.0` | 유효 제약이 있는 입력마다 | `constraint_release` 0.8 이상이면 해제나 예외 적용 | 요청은 확실한데 종류의 확률이 0.8 미만이면 종류를 묻는다 |

- `constraint_change`는 `choice` 질문이다. 선택지는 `none`과 제약마다 `release`(영구 해제), `once`(이번 작업 동안 예외), `scoped`(조건·범위 예외)이다. 요청 여부는 `1 − P(none)`, 대상 제약과 종류는 가장 높은 확률의 선택지에서 읽는다. `scoped`의 조건 문장은 router가 쓰지 않고 입력 원문에서 앞뒤 공백을 뺀 연속된 글을 그대로 쓴다. 200자(초안)를 넘으면 앞 200자까지다. 요약하거나 새로 만든 글은 저장하지 않는다.
- state에는 `Active` 제약(최대 10개, [고르는 규칙](#판단에-싣을-제약-고르기))의 번호와 `제약 등록됨 · <규칙>` 기록 줄, 앞 맥락, 사용자 입력 원문을 넣는다. 질문 문장은 영어다.
- 영구 해제는 제약을 `Released`로 바꾸고 `제약 해제됨 · <규칙>`을 남긴다. 이번 작업 예외(`Once`)는 제약을 지우지 않고 그 작업 동안만 멈추며 `제약 잠시 해제됨 · <규칙> · 이번 작업 동안`을 남기고, 작업이 끝나 다시 유효해질 때 `제약 다시 유효 · <규칙>`을 한 줄 남긴다. 조건·범위 예외(`Scoped`)는 조건 문장을 기록하고 그 범위에서만 멈추며 `제약 예외 · <규칙> · <조건>` 줄을 남긴다.
- 종류 구분 정확도가 87.8%여서 예외 줄에서 사용자가 종류를 바꾸거나 되돌릴 수 있어야 한다. `/constraints`의 변경 내역에서 `e`로 종류를 바꾸고 `u`로 되돌린다([되돌리기](#되돌리기)).
- 기준값 0.8은 선택형 한 질문의 실험 권장값이다. 선택형은 쌍별 질문보다 요청·대상 결합 정확도가 +5.7%p [3.0, 8.4] 높았고 해제 아닌 말을 해제로 본 비율은 0/144였다([해제 요청 실험](../experiments/constraint-cancel-request/report.md)). 같은 흐름에서 Jev는 해제·예외 종합 정확도 91.2%(사후 계산), 요청 재현율 96.8%, 종류 87.8%, 잘못된 영구 해제 1.2%였고 응답이 0.28초, 비용이 Haiku의 약 100분의 1이었다([Jev와 Haiku 비교](../experiments/constraint-exception-judge/report.md)).
- 부분 해제("이번 작업에서만 풀어")와 조건부 요청을 영구 해제 하나로만 받으면 부분 24/30, 조건부 28/30이 잘못 처리된다. 그래서 종류를 질문 안에 넣고 예외를 기록 줄로 보인다.
- 해제나 예외로 기록해도 원문은 지우지 않는다. 기록 원문을 정본으로 두기 위해서다.
- 잘못 지운 제약은 새 session이 알 수 없으므로 판단이 없거나 기준 밖이면 해제도 예외도 하지 않는다.
- 낮은 확신이란 행동 기준에 못 미치지만 묻기 하한 이상인 확률이다. `saturn` 판단 방식에서 noul 확신도가 0.6 미만인 `is_constraint` 답도 판단 없음이 아니라 묻는 구간으로 본다. 판단이 없거나 `invalid`인 답은 묻지 않고 행동하지 않는다.

### 판단에 싣을 제약 고르기

`core`의 순수 함수(`pick_change_candidates`)가 `constraint_change`에 실을 `Active` 제약을 최대 10개 고른다. 등록 답을 기다리는 `Candidate`는 해제할 대상이 아니라 싣지 않는다. 이 함수는 `core`에 두고 engine은 제약 목록을 읽어 넘기고 요청을 보낸다. `Active` 제약을 모두 실으면 요청이 크기 상한(약 110KB)에 걸려 HTTP 400이 나기 때문이다.

1. 유효 제약이 10개 이하이면 전부 고른다.
2. 넘으면 입력과 파일 겹침, 단어 겹침 순위를 [순위 합치기](context-selection.md#순위-합치기)와 같은 RRF로 합쳐 상위 10개를 고른다. 같은 점수면 최신 제약이 위다.

- 같은 입력과 제약 목록에서 늘 같은 결과를 내야 하고 파일, 네트워크, 프로세스가 필요 없어서 `core`에 둔다. 단어 조각과 순위 합치기가 이미 `core`의 `sessions`에 있다.
- 해제 요청의 대상은 방금 등록한 제약이 대부분이다(방금 등록한 것 97.7% [92.1, 99.4]). 10개 상한과 전체 제약을 모두 보내는 조건의 비교는 긴 대화 실측으로 확인한다.

### 적용 직전 비교

판단 결과는 적용 직전에 비교한다. 제약 판단의 비교 기준은 입력 처리와 다르다.

- 제약 revision은 채팅의 마지막 `constraint_events` 번호다. 제약 판단은 제약 revision이 판단을 보낼 때와 같고 그 입력이 취소되지 않았을 때만 적용한다.
- 다르면 한 번 다시 판단하고, 또 다르면 적용하지 않는다. 사용자가 `/constraints`로 바꾼 상태를 옛 판단으로 덮어쓰지 않기 위해서다. 판단 기록은 `superseded`로 쓴다.
- 새 제약을 등록하는 판단은 새 행을 더할 뿐 다른 제약을 바꾸지 않으므로 제약 revision 비교 없이 적용한다. 비교는 대상 제약을 바꾸는 해제와 예외 판단에 쓴다.
- 입력 처리 판단의 채팅 revision이 어긋나도 그 요청의 제약 답은 위 기준만 통과하면 적용한다. 작업 상태가 바뀌었다는 이유로 사용자가 한 말의 제약 여부를 버리지 않기 위해서다. 입력 처리를 다시 판단하는 요청에는 제약 질문을 넣지 않아 같은 입력을 두 번 등록하지 않는다.
- 사용자의 해제, 예외 바꾸기, 되돌리기, 확인 답도 같은 비교를 거친다. 화면이 본 제약 revision과 다르면 engine이 `Stale`로 거절하고 TUI가 목록을 새로 읽는다.

### 사용자에게 묻기

1. engine이 `constraint_asks`에 질문을 저장하고 TUI에 알린다.
2. TUI가 확인 창을 띄운다. 작업과 입력 처리는 계속된다.
3. 사용자가 고르면 `AnswerConstraintAsk`가 engine에 가고 `store`가 한 거래로 적용한다.

| 질문 | 문구 | 선택지 |
|---|---|---|
| 등록 | `이 말을 앞으로 지킬 제약으로 등록할까요?` / `Add this as a constraint?` | `등록`, `등록 안 함` |
| 예외 종류 | `이 제약을 어떻게 풀까요?` / `How should this constraint be lifted?` | `유지`, `이번 작업 동안`, `영구 해제` |

- 질문은 한 번에 하나씩 도착 순서로 띄운다. 다른 TUI가 먼저 답하면 창이 사라지고 늦은 답은 거절한다. `Esc`는 답을 미루고 질문은 `/constraints`의 확인 필요 줄로 남는다.
- 질문은 기록 저장소에 있어 engine을 다시 켜도 되살아난다.
- 답이 오기 전의 제약은 지키는 쪽으로 둔다. 등록을 묻는 중인 제약은 유효 제약처럼, 종류를 묻는 중인 제약은 유효로 패킷에 들어간다. 놓치는 것보다 넣는 것이 안전하고 사용자가 볼 수 있기 때문이다. `등록 안 함`은 `Released`(`Declined`)로 두고 대화 기록에는 줄을 남기지 않는다. 등록 줄을 본 적이 없기 때문이다.
- 묻는 동안 대상 제약이 이미 바뀌었으면 질문을 닫고 늦은 답은 거절한다.
- 화면이 없는 실행에서는 질문을 띄우지 못하므로 답이 없는 상태로 남는다.
- 사용자의 답은 그 판단 기록의 물은 답(`asked_answer`)으로 남기고 q는 1로 둔다. router가 더 높게 본 쪽과 사용자 답이 다르면 `Wrong`이다. 확률로 고르는 피드백 질문과 달리 구간에서는 항상 묻고, [router 학습](router-training.md#사용자에게-묻기)의 20번에 1번 상한에 세지 않는다.
- 긴 입력에서 규칙이 여러 건이면 확인 창은 입력당 하나이고 답이 그 입력의 규칙 전체에 적용된다(초안).

#### 묻지 않고 진행하는 권한 모드

[권한 모드](permissions.md#권한-모드)가 `full`이면 제약 쪽 질문을 띄우지 않는다. 모든 것을 허용하고 사용자가 지켜보지 않는 실행을 확인 창이 멈추지 않게 하기 위해서다. 대신 대화 기록 줄에 확인 없이 정했음을 표시한다.

- `is_constraint`가 `constraint_ask` 이상 0.8 미만이면 `Candidate`를 거치지 않고 지키는 쪽으로 바로 `Active`로 등록하고 `제약 등록됨 · <규칙> · 확인 없이`를 남긴다.
- 요청은 확실한데 종류의 확률이 0.8 미만이면(자동 적용을 켠 때만) 종류를 묻지 않고 제약을 지우지 않는 `이번 작업 동안` 예외로 적용하고 `제약 잠시 해제됨 · <규칙> · 이번 작업 동안 · 확인 없이`를 남긴다.
- 이 줄의 이벤트는 사유 `Unconfirmed`로 기록한다. 사용자는 줄을 보고 해제 요청을 입력하거나 `/constraints`에서 `d`, `x`, `e`, `u`로 고친다.
- 모드는 engine이 판단을 적용할 때 채팅 층 값을 먼저 읽어 정한다(권한 판정과 같은 읽기). 이미 열린 확인은 모드를 바꿔도 남아 사용자가 답한다. 확인 없이 등록한 제약은 사용자가 지우면 그 판단의 결과 신호를 `Wrong`으로 남긴다.

### 되돌리기

- `/constraints`는 제약 목록 화면을 연다. 유효 제약 줄은 `번호 · 범위 · 규칙 한 줄`이고, 예외가 걸린 제약은 예외 종류와 조건을 함께 보이며, 확인 필요 줄과 마지막 전환에 들어갔는지 표시를 함께 보인다. `Tab`으로 변경 내역(등록, 해제, 예외, 다시 유효, 되돌림을 시각순)을 바꿔 본다.
- 유효 제약에서 `d`는 해제(`User`), `x`는 잘못 등록이다. `x`는 제약이 아니었다는 뜻이라 해제(`Mistaken`)로 기록하고 그 판단의 결과 신호를 `Wrong`으로 남긴다.
- 변경 내역의 예외 줄에서 `e`는 종류를 바꾼다. `이번 작업 동안`, `조건·범위`(조건 문장은 입력 원문에서 다시 고름), `영구 해제` 중 하나를 고르면 앞 예외를 닫고 새 종류로 한 거래에 쓴다. router가 종류를 잘못 읽은 판단은 그 결과 신호를 `Wrong`으로 남긴다.
- 변경 내역에서 `u`는 그 변경 한 건을 되돌린다. 해제와 예외의 되돌리기는 제약을 `Active`로 돌리고 예외를 닫는다. 등록의 되돌리기는 해제와 같다.
- 되돌리기는 그 제약의 가장 최근 변경에만 쓸 수 있다. 뒤에 같은 제약을 바꾼 이벤트가 있으면 거절한다. 예외가 끝나서 생긴 `Resumed`는 되돌리지 않는다.
- 되돌리기도 이벤트(`Restored`)로 남고 대화 기록에 한 줄이 남는다. 변경 내역을 지우지 않고 이어 쓰기 위해서다.
- router가 자동으로 한 등록, 해제, 예외를 사용자가 되돌리거나 `x`로 취소하면 그 판단의 결과 신호를 `Wrong`으로 기록한다. 대기 입력 취소와 같은 규칙이다([입력 처리](input-handling.md#대기와-취소)).
- 대화 기록의 제약 줄은 `constraint_events`에서 그리므로 채팅을 다시 열어도 같은 자리에 보인다. 문구 형식은 [TUI](tui.md)에 있다.

### 패킷의 제약 칸

패킷의 고정 구역 첫 항목인 제약 칸은 상한 안에서 채운다. 제약이 쌓여도 패킷이 제약만으로 차지 않게 하기 위해서다.

```text
C_max = P_max × context.constraint_slot_percent / 100
```

`C_max`는 제약 칸 상한이고 `P_max`는 [패킷 크기 상한](context-management.md#패킷-구성)이다. `context.constraint_slot_percent`의 기본값 25는 초안이고 긴 대화 실측으로 정한다([설정](settings.md#설정-키)). 크기는 네 글자를 한 토큰으로 추정한다.

1. 범위가 `전체`인 유효 제약을 최신 순으로 넣는다.
2. 범위가 있고 지금 작업과 겹치는 제약을 겹치는 경로 수가 많은 순, 같으면 최신 순으로 넣는다. 지금 작업은 [순위 채널](context-selection.md#순위-채널)의 기준 파일(마지막 입력에 나온 경로와 메인 session이 최근 3턴에 건드린 파일)이다.
3. 나머지를 마지막 입력과의 단어 겹침 순위가 높은 순, 같으면 최신 순으로 넣는다.

- 칸에 넣는 유효 제약은 해제되지 않은 제약(`Active`와 등록 답을 기다리는 `Candidate`)이다. 예외가 걸린 제약도 들어가고 예외 표기를 붙인다. 보관 session을 다시 열어 변경분만 붙일 때는 그 session이 이미 받은 제약이라 제약 칸을 다시 넣지 않는다.
- 칸에 들어가지 않는 제약은 건너뛰고 더 작은 제약은 계속 넣는다. 항목 하나가 큰 제약 때문에 뒤의 작은 제약이 막히지 않게 하기 위해서다.
- 예외가 걸린 제약은 규칙 한 줄 뒤에 예외 표기(`이번 작업 동안 멈춤` 또는 조건 문장)를 붙여 넣는다. 새 session이 예외를 모르고 규칙을 지키거나 제약이 없는 줄 아는 일을 막기 위해서다. 표기는 제약의 크기에 세고, 표기까지 들어가지 않으면 그 제약을 건너뛴다(초안). 해제된 제약은 표기 없이 뺀다.
- 들어가지 못한 제약이 N개면 칸 끝에 `Constraints omitted: N` 한 줄을 넣는다. 이 줄은 상한에 세지 않는다. 같은 때 대화 기록에 `제약 N개 생략 · /constraints에서 확인하세요`를 남긴다.
- 고정 구역이 `P_hard`도 넘어 맥락 정리를 미룰 때 보이는 제약 목록은 생략 없이 유효 제약 전체다([맥락 정리](context-management.md#패킷-구성)).
- 전환마다 `packet_constraints`에 새 session의 제약별 단계(`All`, `Scope`, `Relevance`, `Omitted`)를 남긴다. 무엇이 어느 전환에 들어갔는지 사용자가 `/constraints`에서 확인하게 하기 위해서다.
- 패킷에 제약이 요약보다 우선한다고 적는 규칙과 provider 요약의 취급은 그대로다.

### 실측으로 정하는 값

| 값 | 기본값 | 근거 |
|---|---|---|
| `is_constraint` | 0.8 | [등록 기준값의 사람 확인](../experiments/constraint-human-check/report.md): Astra 정답 868턴에서 정밀도 91.3% [87.6, 94.0] |
| `constraint_ask` | 0.7 | 같은 곡선: 0.7 이상 0.8 미만은 입력의 12.6%, 그중 67.0%가 제약 |
| `constraint_release` | 0.8 | [해제 요청 실험](../experiments/constraint-cancel-request/report.md)의 선택형 권장 기준값 |

| 값 | 초안 | 정하는 방법 |
|---|---|---|
| `context.constraint_slot_percent` | 25 | 긴 대화 실측(`docs/experiments/constraint-long-context/`)에서 제약 11개 이상 표본의 보존 정확도 |
| 문장 나누기 기준 | 200자, 20문장 | 같은 실측의 혼합 입력 |
| 판단에 싣는 제약 상한 | 10개 | 같은 실측의 10개 대 전체 제약 비교 |
| 종류 질문의 확률 하한 | 0.8 | 해제 종류 구분 정확도(87.8%)를 올린 뒤 사용자 답 기록 |

실제 기록의 기준값 곡선과 라벨 합의는 [프로젝트 대화 전수 측정](../experiments/constraint-deep/report.md)에 있다. 이 측정은 턴 단위 등록을 사용하므로 문장 분할과 제약 칸 상한의 근거로 쓰지 않는다. 정답 라벨에 gpt-5.6-luna를 쓴 그 수치의 한계는 [식별 순서](#식별-순서)에 적었다.

### 오류 처리

| 상황 | 동작 |
|---|---|
| 자동 적용 꺼짐 | 어떤 `is_constraint` 값이든 등록하지 않고 묻지 않는다. 판단 기록은 남는다. |
| `is_constraint` 판단 없음 | 등록하지 않는다. |
| 문장 나누기 질문 실패, 문장 20개 초과 | 입력 전체 원문을 한 건으로 등록한다. |
| `constraint_change` 판단 없음 | 해제도 예외도 하지 않는다. |
| 제약 revision 불일치 | 한 번 다시 판단하고, 또 다르면 적용하지 않는다. |
| 묻는 동안 대상 제약 변경 | 질문을 닫고 늦은 답을 거절한다. |
| 최신이 아닌 제약 revision으로 한 화면 요청 | `Stale`로 거절하고 목록을 새로 읽는다. |
| 제약 칸 초과 | 못 넣은 제약 수를 패킷과 대화 기록에 표시한다. |
| 화면이 없어 질문을 못 띄움 | 질문을 답 없이 남기고 지키는 쪽으로 둔다. |

### 요구사항

| 요구사항 | 검증 계획 |
|---|---|
| 제약은 입력 원문에서 자른 규칙 한 줄과 범위로 저장하고 원문은 바뀌지 않는다. | `saturn-terminal/engine/src/lifecycle/constraints.rs`의 `constraint_scope_comes_from_paths_in_the_rule`, `constraint_long_input_registers_only_the_sentences_the_router_calls_constraints`, `constraint_long_input_registers_the_whole_text_when_the_split_question_fails`, `saturn-terminal/core/src/constraints/tests.rs`의 `rules_are_cut_from_the_original_text`, `scope_takes_path_like_words_normalized_and_unique` |
| 등록, 해제, 예외, 다시 유효, 되돌림마다 이벤트와 대화 기록 한 줄이 남는다. | 등록과 입력 취소로 인한 해제는 `saturn-terminal/engine/src/lifecycle/constraints.rs`의 `constraint_at_or_above_the_auto_threshold_is_registered_with_a_chat_line`, `constraint_of_an_input_canceled_after_registration_is_released_with_a_line`. 해제, 예외, 다시 유효는 `saturn-terminal/engine/src/lifecycle/constraint_change.rs`의 `router_change_is_applied_by_kind_and_marked_as_the_router`, `task_exception_ends_with_its_task_and_scoped_exception_stays`(이벤트와 다시 그린 기록 줄). 되돌림은 구현 전(#381) |
| 자동 적용이 꺼져 있으면(기본, `full` 포함) 입력은 평소대로 진행하고 어떤 `is_constraint` 값도 제약 표, 질문, 줄을 만들지 않는다. 이미 저장한 제약은 그대로다. | `saturn-terminal/engine/src/lifecycle/constraints.rs`의 `constraint_is_not_applied_by_default_and_the_input_still_runs_with_its_judgment_kept`(설정 없음·거짓, 0.95·0.75, `full` 여부), `saturn-terminal/engine/src/settings/names_tests.rs`의 `security_and_cost_keys_are_user_only` |
| 자동 적용을 켜면 `is_constraint` 0.8 이상은 자동 등록하고, 0.7 이상 0.8 미만은 묻고, 0.7 미만은 등록하지 않는다. | `saturn-terminal/engine/src/lifecycle/constraints.rs`의 `constraint_at_or_above_the_auto_threshold_is_registered_with_a_chat_line`(0.85), `constraint_at_the_exact_auto_threshold_is_registered`(0.8), `constraint_in_the_ask_band_is_stored_as_candidate_and_asked`(0.75), `constraint_ask_answered_yes_registers_and_leaves_a_line`, `constraint_ask_answered_no_releases_it_without_a_chat_line`, `constraint_below_the_ask_threshold_is_not_registered`(0.65), `saturn-terminal/core/src/routers/tests.rs`의 `registration_follows_the_three_bands` |
| 답이 오기 전의 제약은 지키는 쪽으로 패킷에 들어간다. | 등록을 묻는 중(`Candidate`)인 제약은 해제된 것이 아니라서 들어간다. 종류를 묻는 창은 아직 없어 종류가 불확실하면 `full`이 아닐 때 아무것도 바꾸지 않는다(`unsure_kind_pauses_only_in_full_mode_and_otherwise_changes_nothing`). 창은 #381 |
| 해제 요청은 선택형 질문 하나로 대상과 종류를 정하고, 영구 해제는 `Released`로, 이번 작업 예외는 제약을 지우지 않고 그 작업 동안만 멈춘다. | `saturn-terminal/core/src/routers/tests.rs`의 `change_answer_reads_target_and_kind_from_the_top_option`, `change_answer_with_a_wrong_shape_or_low_confidence_changes_nothing`, `saturn-terminal/engine/src/lifecycle/constraint_change.rs`의 `router_change_is_applied_by_kind_and_marked_as_the_router`(해제, 이번 작업, 범위, 없음 네 입력) |
| router 제안의 해제·예외는 `constraint.auto_apply`가 켜져 있을 때만 질문하고 적용하며 `full`도 우회하지 못한다. 사용자의 명시 해제는 그 설정과 상관없이 `User`로 적용하고 낡은 revision은 `Stale`로 거절한다. | `saturn-terminal/engine/src/lifecycle/constraint_change.rs`의 `router_change_is_not_asked_or_applied_unless_auto_apply_is_on_even_in_full_mode`(설정 없음·거짓, `full` 여부 네 조합에서 router 호출 한 번뿐, 제약 불변), `explicit_release_is_the_user_and_ignores_auto_apply_and_full_mode`, `explicit_release_with_a_stale_revision_changes_nothing` |
| 답이 없거나 `invalid`이거나 `none`이거나 불확실하면 잘못 해제하지 않는다. 낡은 판단은 한 번 다시 판단하고 취소된 입력의 판단은 적용하지 않으며 시작할 때 사라진 작업의 예외는 닫는다. | `constraint_change.rs`의 `failed_or_invalid_change_answers_change_nothing`, `task_exception_is_skipped_when_the_input_has_no_task_yet`, `change_judged_against_an_older_constraint_revision_is_judged_again_once`, `change_of_a_canceled_input_is_not_applied`, `task_exception_of_a_vanished_task_is_closed_at_startup`, `saturn-terminal/engine/src/store/constraints.rs`의 `constraint_change_writes_nothing_for_a_stale_revision_or_an_inactive_target` |
| 이번 작업 예외는 작업이 끝나면 사라지고 `제약 다시 유효` 줄이 한 줄 남는다. | `constraint_change.rs`의 `task_exception_ends_with_its_task_and_scoped_exception_stays`, `scoped_exception_survives_the_end_of_the_task`, `store/constraints.rs`의 `task_exceptions_end_only_with_their_task_and_a_new_change_closes_the_open_one` |
| 조건·범위 예외는 조건 문장을 기록하고 패킷에 예외 표기로 들어간다. | 조건 기록은 `constraint_change.rs`의 `router_change_is_applied_by_kind_and_marked_as_the_router`(조건이 입력 원문의 연속된 글), 패킷은 `saturn-terminal/engine/src/lifecycle/constraint_handoff.rs`의 `constraint_with_an_exception_is_handed_over_with_its_exception_note`. 제약 칸 채우기와 router 연결은 #380 |
| 권한 모드 `full`에서는 제약 질문을 띄우지 않고 `확인 없이` 줄을 남긴다. | 등록은 `saturn-terminal/engine/src/lifecycle/constraints.rs`의 `constraint_in_full_mode_registers_in_the_ask_band_without_asking_and_marks_it`, `constraint_in_full_mode_above_the_auto_threshold_has_no_unconfirmed_mark`. 종류 갈림 입력은 `constraint_change.rs`의 `unsure_kind_pauses_only_in_full_mode_and_otherwise_changes_nothing` |
| 입력 처리 revision이 어긋나도 제약 판단은 제약 revision으로 적용한다. | `saturn-terminal/engine/src/lifecycle/constraints.rs`의 `constraint_is_registered_once_when_the_chat_revision_changes_during_judgment`. 해제와 예외의 제약 revision 비교는 `constraint_change.rs`의 `change_judged_against_an_older_constraint_revision_is_judged_again_once` |
| 취소된 입력의 제약은 등록하지 않거나 함께 해제한다. | `saturn-terminal/engine/src/lifecycle/constraints.rs`의 `constraint_of_an_input_canceled_while_judging_is_not_registered`, `constraint_of_an_input_canceled_after_registration_is_released_with_a_line`, `constraint_ask_is_closed_when_its_input_is_canceled` |
| router가 없거나 실패하거나 답이 없으면 등록하지 않고 입력은 평소대로 처리한다. | `saturn-terminal/engine/src/lifecycle/constraints.rs`의 `constraint_is_not_registered_when_the_router_fails`, `constraint_is_not_registered_when_the_answer_is_missing`, `constraint_input_without_the_router_is_never_judged` |
| 기준값은 설정에서 읽는다. | `saturn-terminal/engine/src/lifecycle/constraints.rs`의 `constraint_thresholds_are_read_from_settings` |
| 판단에 싣을 제약 고르기는 같은 입력에서 같은 결과를 내고 10개를 넘지 않는다. | `saturn-terminal/core/src/constraints/tests.rs`의 `change_candidates_are_all_constraints_up_to_the_cap`, `change_candidates_over_the_cap_keep_the_word_match_and_the_newest` |
| 제약 칸은 `C_max` 안에서 전체, 범위 겹침, 관련도·최신 순으로 채우고 넘친 수를 표시한다. | `saturn-terminal/core/src/sessions/constraint_slot/tests.rs`의 `tiers_go_all_then_scope_then_relevance_and_newer_first`, `a_constraint_too_big_for_the_slot_does_not_block_smaller_ones`, `saturn-terminal/core/src/sessions/packet/tests.rs`의 `omitted_constraints_add_a_count_line_that_is_not_counted_in_the_slot`, `saturn-terminal/engine/src/lifecycle/constraint_handoff.rs`의 `constraints_over_the_slot_are_omitted_and_marked`. 대화 기록의 `제약 N개 생략` 줄은 구현 전 |
| 전환마다 들어간 제약과 빠진 제약을 기록한다. | `saturn-terminal/engine/src/lifecycle/constraint_handoff.rs`의 `packet_constraints_records_what_the_new_session_got`, `constraints_over_the_slot_are_omitted_and_marked` |
| 제약이 최근 입력과 목표 발췌에서 밀려난 긴 대화에서도 새 session의 패킷 제약 구역에 남는다. 해제된 제약은 넣지 않는다. | `saturn-terminal/engine/src/lifecycle/constraint_handoff.rs`의 `constraint_stays_in_the_packet_when_it_left_the_recent_turns`(제약 원문은 어느 입력, 답, 파일 내용에도 없음), `released_constraint_is_not_handed_over` |
| `/constraints`는 유효 제약과 변경 내역을 보이고 해제, 예외 종류 바꾸기, 되돌리기를 `Stale` 검사와 함께 한다. | 구현 전(#381). 화면을 연 사이 제약을 바꿔 거절과 새로 읽기를 확인한다. |
| 등록 기준값은 사람 확인과 Astra 정답에서 근거가 있다. | [등록 기준값의 사람 확인](../experiments/constraint-human-check/report.md): 0.80 정밀도 91.3%, 0.70 정밀도 84.8%·재현율 77.2% |
| 해제·예외 판단은 Jev 선택형으로 기준을 넘거나 가깝다. | [해제 요청 실험](../experiments/constraint-cancel-request/report.md), [Jev와 Haiku 비교](../experiments/constraint-exception-judge/report.md): 요청 재현율 96.8%, 종류 87.8%, 잘못된 영구 해제 1.2% |
| 제약 칸 상한과 묻는 구간이 긴 대화에서 제약을 지킨다. | 긴 대화 실측(`docs/experiments/constraint-long-context/`)으로 확인한다. |

## 단점

- 제약 판단은 router 정확도에 기댄다. 간접 지시 입력의 정확도는 74.5%여서 놓치거나 잘못 등록할 수 있다([간접 지시 정확도](../experiments/indirect-constraint-accuracy/report.md)).
- 해제 종류 구분 정확도가 87.8%이고 잘못된 영구 해제가 1.2% [0.5, 2.8]이다. 기준(90%, 1%)에 못 미쳐 예외 줄에서 사용자가 고치게 했다.
- 실제 입력의 절반가량이 AI가 쓴 지시문을 붙여 넣은 것이다. 그런 입력은 맥락 없이 제약인지 가리기 어렵고, 작업 하나에만 해당하는 규칙이 많다. 사람 확인 24건에서 Jev 0.80은 재현율이 6/14였다. 이 입력을 어떻게 다룰지는 정하지 못했다.
- 규칙 한 줄이 원문의 문장이라 "그것도 그렇게 해"처럼 앞 말에 기대는 문장은 혼자 뜻이 통하지 않는다.
- 새 제약이 앞 제약과 같거나 부딪쳐도 자동으로 정리하지 않아 둘 다 유효로 남는다. 패킷의 제약 칸을 더 차지하고 사용자가 직접 정리해야 한다.
- 유효 제약이 있는 입력마다 `constraint_change` 요청이 한 번 더 간다. 조건 문장은 입력 원문 앞 200자까지를 그대로 쓰므로 글자는 맞지만 조건 말고 다른 말도 함께 들어갈 수 있다. 입력이 아직 작업에 배정되지 않았으면 이번 작업 예외는 걸지 않고 제약을 그대로 둔다.
- 묻기는 사용자 입력을 요구한다. 묻는 구간(입력의 12.6%)이 넓으면 질문이 잦아진다. `full` 모드에서는 묻지 않고 지키는 쪽으로 등록해 잘못 등록한 제약이 줄로만 남는다.
- router를 부르지 않는 입력은 제약 판단을 받지 않는다.
- `saturn` 판단 방식의 낮은 확신 답이 판단 없음이 아니라 질문이 되어 질문 수가 `jev`보다 많을 수 있다.
- 제약이 `C_max`를 넘으면 오래된 제약이 새 session에서 빠진다.
- provider 모드의 압축 요약에는 해제된 제약이 남아 있을 수 있다.
- 경로 없는 범위 제약은 `전체`로 취급돼 패킷에서 자리를 더 차지한다.

## 대안

- 입력마다 LLM으로 규칙 한 줄을 요약해 저장하는 방식은 호출과 출력 비용이 들고 원문 대신 생성문을 저장해 버렸다([결정 기록](../decisions/2026-10-04-constraints-as-rule-lines.md)).
- 입력 전체 원문을 제약으로 등록하는 방식은 일과 제약이 섞인 입력에서 패킷이 길어져 버렸다. 문장 나누기가 실패할 때의 대체로만 남겼다.
- 확신이 낮아도 기준값 하나로 자동 결정하는 방식은 잘못 해제한 제약을 새 session이 알 수 없어 버렸다.
- 기록 없는 대화에서 새 제약이 기존 제약을 대체하거나 합치는지 자동으로 판단하는 방식은 정밀도가 0~1.6%여서 버렸다([constraint-relation](../experiments/constraint-relation/report.md)).
- 해제·예외를 저렴한 LLM(Haiku)에 맡기는 방식은 종합 정확도 84.0% 대 91.2%, 응답 10.4초 대 0.28초, 비용 약 100배라 버렸다([Jev와 Haiku 비교](../experiments/constraint-exception-judge/report.md), 둘 다 사후 계산).
- 제약 칸 상한 없이 모든 제약을 넣는 방식은 제약이 쌓일수록 고정 구역이 `P_hard`를 넘어 맥락 정리가 계속 미뤄져 버렸다.

## 미해결 질문

- 입력의 절반가량인 AI가 쓴 지시문에서 작업 하나에만 해당하는 규칙을 어떻게 가려낼지(대응 미정)
- 해제 종류 구분 정확도를 90% 이상으로 올릴 질문 문장과 예외 조건 문장을 글자 단위로 맞추는 방법
- 독립 프로젝트 표본과 사람 확인 정답으로 0.8과 0.7을 다시 확인할 수 있는지
